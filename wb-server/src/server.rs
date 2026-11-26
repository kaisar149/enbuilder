//! The server: accept loop, one thread per connection, and ONE session-actor thread that owns the `wb_app::Session`.
//! Connection threads never touch the session; they forward text messages to the actor, which executes them, writes the response to the
//! asking client FIRST, and then pushes any events that the command caused (so a client never sees an event before its own ack).
use crate::http::{self, Request};
use crate::protocol::{self, Outgoing};
use crate::ws::{self, Incoming};
use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use wb_app::{Command, Session};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// 0 = pick a free port.
    pub port: u16,
    pub preset: String,
    /// Parameter overrides applied after loading the preset (`name`, value).
    pub params: Vec<(String, f64)>,
    /// Directory with the built client (`index.html`, assets). None = a built-in "client not built" page.
    pub static_dir: Option<PathBuf>,
    /// Start a solve as soon as the server is up.
    pub solve_on_start: bool,
    /// Test hook: slow every solve step down (so tests can watch progress and cancel).
    pub step_delay: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self { ServerConfig { port: 8787, preset: "earthlike".into(), params: vec![], static_dir: None, solve_on_start: false, step_delay: Duration::ZERO } }
}

/// A connected WebSocket client. All writes go through the mutex, so a response and a pushed event never interleave.
pub struct ClientWriter {
    pub id: u64,
    stream: Mutex<TcpStream>,
    dead: AtomicBool,
}

impl ClientWriter {
    fn send(&self, opcode: u8, payload: &[u8]) -> bool {
        if self.dead.load(Ordering::SeqCst) {
            return false;
        }
        let frame = ws::encode_frame(opcode, payload);
        let ok = match self.stream.lock() {
            Ok(mut s) => s.write_all(&frame).and_then(|_| s.flush()).is_ok(),
            Err(_) => false,
        };
        if !ok {
            self.dead.store(true, Ordering::SeqCst);
        }
        ok
    }
    fn send_out(&self, o: &Outgoing) -> bool {
        match o {
            Outgoing::Text(t) => self.send(ws::OP_TEXT, t.as_bytes()),
            Outgoing::Binary(b) => self.send(ws::OP_BINARY, b),
        }
    }
    fn close_socket(&self) {
        self.dead.store(true, Ordering::SeqCst);
        if let Ok(s) = self.stream.lock() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

enum Msg {
    Request { client: Arc<ClientWriter>, text: String },
    Stop,
}

type Clients = Arc<Mutex<Vec<Arc<ClientWriter>>>>;

struct Shared {
    tx: Sender<Msg>,
    clients: Clients,
    static_dir: Option<PathBuf>,
    next_id: AtomicU64,
    stop: AtomicBool,
}

pub struct ServerHandle {
    addr: SocketAddr,
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl ServerHandle {
    pub fn addr(&self) -> SocketAddr { self.addr }
    pub fn url(&self) -> String { format!("http://127.0.0.1:{}", self.addr.port()) }
    pub fn ws_url(&self) -> String { format!("ws://127.0.0.1:{}/ws", self.addr.port()) }

    /// Stops accepting, closes every client and joins the actor (which cancels a running solve).
    pub fn shutdown(&mut self) {
        if self.shared.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = TcpStream::connect(self.addr); // wake the accept loop
        for c in self.shared.clients.lock().unwrap().drain(..) {
            c.close_socket();
        }
        let _ = self.shared.tx.send(Msg::Stop);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }

    /// Blocks until the server stops (used by the binary).
    pub fn wait(&mut self) {
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) { self.shutdown(); }
}

fn broadcast(clients: &Clients, events: Vec<wb_app::Event>) {
    if events.is_empty() {
        return;
    }
    let mut list = clients.lock().unwrap();
    for e in events {
        let text = protocol::event_message(&e);
        for c in list.iter() {
            c.send(ws::OP_TEXT, text.as_bytes());
        }
    }
    list.retain(|c| !c.dead.load(Ordering::SeqCst));
}

fn actor(rx: Receiver<Msg>, clients: Clients, cfg: ServerConfig, ready: Sender<Result<(), String>>) {
    let built = (|| -> Result<Session, wb_app::ApiError> {
        let mut s = Session::with_preset(&cfg.preset)?;
        for (k, v) in &cfg.params {
            s.execute(Command::SetParam { name: k.clone(), value: *v })?;
        }
        s.set_step_delay(cfg.step_delay);
        if cfg.solve_on_start {
            s.execute(Command::Solve)?;
        }
        Ok(s)
    })();
    let mut session = match built {
        Ok(s) => {
            let _ = ready.send(Ok(()));
            s
        }
        Err(e) => {
            let _ = ready.send(Err(e.to_string()));
            return;
        }
    };
    // The WorldLoaded event of the initial build is for nobody: clients fetch world info after connecting.
    let _ = session.poll_events();
    loop {
        match rx.recv_timeout(Duration::from_millis(15)) {
            Ok(Msg::Request { client, text }) => {
                let out = protocol::handle_request(&mut session, &text);
                client.send_out(&out);
            }
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        broadcast(&clients, session.poll_events());
    }
    // dropping the session cancels and joins a running solve
}

pub fn start(cfg: ServerConfig) -> std::io::Result<ServerHandle> {
    let listener = TcpListener::bind(("127.0.0.1", cfg.port))?;
    let addr = listener.local_addr()?;
    let (tx, rx) = channel();
    let clients: Clients = Arc::new(Mutex::new(vec![]));
    let (ready_tx, ready_rx) = channel();
    let (c2, cfg2) = (clients.clone(), cfg.clone());
    let actor_thread = std::thread::spawn(move || actor(rx, c2, cfg2, ready_tx));
    match ready_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(m)) => {
            let _ = actor_thread.join();
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, m));
        }
        Err(_) => return Err(std::io::Error::new(std::io::ErrorKind::Other, "the session thread died")),
    }
    let shared = Arc::new(Shared { tx, clients, static_dir: cfg.static_dir, next_id: AtomicU64::new(1), stop: AtomicBool::new(false) });
    let sh = shared.clone();
    let accept = std::thread::spawn(move || {
        for conn in listener.incoming() {
            if sh.stop.load(Ordering::SeqCst) {
                break;
            }
            if let Ok(stream) = conn {
                let sh2 = sh.clone();
                std::thread::spawn(move || {
                    let _ = handle_conn(stream, sh2);
                });
            }
        }
    });
    Ok(ServerHandle { addr, shared, threads: vec![actor_thread, accept] })
}

fn handle_conn(mut stream: TcpStream, sh: Arc<Shared>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_nodelay(true)?;
    let req = match http::read_request(&mut stream) {
        Ok(r) => r,
        Err(_) => return http::write_all(&mut stream, &http::response(400, "Bad Request", "text/plain", b"bad request", false)),
    };
    if let Err(why) = http::check_local(&req) {
        return http::write_all(&mut stream, &http::response(403, "Forbidden", "text/plain", why.as_bytes(), false));
    }
    let wants_ws = req.header("upgrade").map(|u| u.eq_ignore_ascii_case("websocket")).unwrap_or(false);
    if req.path.split('?').next() == Some("/ws") {
        return if wants_ws { ws_session(stream, req, sh) } else { http::write_all(&mut stream, &http::response(426, "Upgrade Required", "text/plain", b"use a WebSocket client", false)) };
    }
    if req.method != "GET" && req.method != "HEAD" {
        return http::write_all(&mut stream, &http::response(405, "Method Not Allowed", "text/plain", b"method not allowed", false));
    }
    if req.path == "/api/hello" {
        let body = protocol::hello();
        return http::write_all(&mut stream, &http::response(200, "OK", "application/json", body.as_bytes(), req.method == "HEAD"));
    }
    let bytes = match &sh.static_dir {
        Some(dir) => http::serve_static(dir, &req),
        None if req.path == "/" => http::response(200, "OK", "text/html; charset=utf-8", http::NO_CLIENT_PAGE.as_bytes(), req.method == "HEAD"),
        None => http::response(404, "Not Found", "text/plain", b"not found", false),
    };
    http::write_all(&mut stream, &bytes)
}

fn ws_session(mut stream: TcpStream, req: Request, sh: Arc<Shared>) -> std::io::Result<()> {
    let key = req.header("sec-websocket-key");
    let conn_ok = req.header("connection").map(|c| c.to_ascii_lowercase().contains("upgrade")).unwrap_or(false);
    if key.is_none() || !conn_ok || req.header("sec-websocket-version") != Some("13") {
        return http::write_all(&mut stream, &http::response(400, "Bad Request", "text/plain", b"bad websocket handshake", false));
    }
    let accept = ws::accept_key(key.unwrap());
    let head = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n");
    http::write_all(&mut stream, head.as_bytes())?;
    stream.set_read_timeout(None)?;
    let writer_stream = stream.try_clone()?;
    writer_stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let client = Arc::new(ClientWriter { id: sh.next_id.fetch_add(1, Ordering::SeqCst), stream: Mutex::new(writer_stream), dead: AtomicBool::new(false) });
    client.send(ws::OP_TEXT, protocol::hello().as_bytes());
    sh.clients.lock().unwrap().push(client.clone());
    let result = (|| -> std::io::Result<()> {
        let mut reader = ws::MessageReader::new();
        loop {
            match reader.read(&mut stream, ws::MAX_MESSAGE)? {
                Incoming::Text(text) => {
                    if sh.tx.send(Msg::Request { client: client.clone(), text }).is_err() {
                        return Ok(());
                    }
                }
                Incoming::Binary(_) => {
                    client.send_out(&Outgoing::Text(r#"{"id":null,"ok":false,"error":{"code":"bad_request","message":"binary frames from clients are not supported"}}"#.into()));
                }
                Incoming::Ping(p) => {
                    client.send(ws::OP_PONG, &p);
                }
                Incoming::Close(p) => {
                    client.send(ws::OP_CLOSE, &p);
                    return Ok(());
                }
            }
        }
    })();
    client.close_socket();
    sh.clients.lock().unwrap().retain(|c| c.id != client.id);
    result
}
