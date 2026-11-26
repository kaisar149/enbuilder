//! Part 9b-1 tests: codec vectors, WebSocket framing, protocol, and end-to-end over real sockets (localhost, port 0).
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use wb_server::codec::{base64_encode, hex, sha1};
use wb_server::protocol::{self, unpack_binary, Outgoing};
use wb_server::ws::{self, accept_key, encode_frame, encode_masked, read_frame, Incoming, MessageReader};
use wb_server::{start, ServerConfig, ServerHandle};

// ------------------------------------------------------------------ codec and framing

#[test]
fn sha1_and_base64_vectors() {
    assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(hex(&sha1(&[b'a'; 1000])), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
    assert_eq!(base64_encode(b"Man"), "TWFu");
    assert_eq!(base64_encode(b"Ma"), "TWE=");
    assert_eq!(base64_encode(b"M"), "TQ==");
    assert_eq!(base64_encode(b""), "");
}

#[test]
fn rfc6455_accept_key() {
    assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}

#[test]
fn frame_round_trips_for_all_length_classes() {
    for n in [0usize, 5, 125, 126, 200, 65535, 65536, 70000] {
        let payload: Vec<u8> = (0..n).map(|i| (i * 7 % 251) as u8).collect();
        let f = read_frame(&mut &encode_frame(ws::OP_BINARY, &payload)[..], false, 1 << 20).unwrap();
        assert_eq!((f.fin, f.opcode, f.payload.len()), (true, ws::OP_BINARY, n));
        assert_eq!(f.payload, payload);
        let m = read_frame(&mut &encode_masked(ws::OP_TEXT, true, &payload, [1, 2, 3, 4])[..], true, 1 << 20).unwrap();
        assert_eq!(m.payload, payload);
    }
}

#[test]
fn protocol_violations_are_rejected() {
    // unmasked client frame
    assert!(read_frame(&mut &encode_frame(ws::OP_TEXT, b"hi")[..], true, 100).is_err());
    // reserved bit
    assert!(read_frame(&mut &[0xC1u8, 0x80, 0, 0, 0, 0][..], true, 100).is_err());
    // too large
    assert!(read_frame(&mut &encode_masked(ws::OP_TEXT, true, &[0u8; 300], [9, 9, 9, 9])[..], true, 100).is_err());
    // fragmented control frame
    assert!(read_frame(&mut &encode_masked(ws::OP_PING, false, b"x", [1, 1, 1, 1])[..], true, 100).is_err());
}

#[test]
fn fragmented_message_and_control_frames() {
    let mut wire = encode_masked(ws::OP_TEXT, false, b"hel", [1, 2, 3, 4]);
    wire.extend(encode_masked(ws::OP_PING, true, b"p", [5, 6, 7, 8]));
    wire.extend(encode_masked(ws::OP_CONT, false, b"lo ", [9, 9, 9, 9]));
    wire.extend(encode_masked(ws::OP_CONT, true, b"world", [4, 3, 2, 1]));
    let (mut r, mut m) = (&wire[..], MessageReader::new());
    assert_eq!(m.read(&mut r, 1000).unwrap(), Incoming::Ping(b"p".to_vec()));
    assert_eq!(m.read(&mut r, 1000).unwrap(), Incoming::Text("hello world".into()), "a ping in the middle must not lose the partial message");
    // the reader is reusable for the next message
    let next = encode_masked(ws::OP_BINARY, true, &[1, 2, 3], [7, 7, 7, 7]);
    assert_eq!(m.read(&mut &next[..], 1000).unwrap(), Incoming::Binary(vec![1, 2, 3]));
    // continuation without a start
    assert!(MessageReader::new().read(&mut &encode_masked(ws::OP_CONT, true, b"x", [1, 1, 1, 1])[..], 1000).is_err());
}

// ------------------------------------------------------------------ protocol (no sockets)

fn small_session() -> wb_app::Session {
    let mut s = wb_app::Session::with_preset("earthlike").unwrap();
    s.execute(wb_app::Command::SetParam { name: "resolution".into(), value: 2.0 }).unwrap();
    s
}

fn text(o: Outgoing) -> Value {
    match o {
        Outgoing::Text(t) => serde_json::from_str(&t).unwrap(),
        Outgoing::Binary(_) => panic!("expected a text response"),
    }
}

#[test]
fn protocol_errors_and_envelopes() {
    let mut s = small_session();
    let e = text(protocol::handle_request(&mut s, "not json"));
    assert_eq!((e["ok"].clone(), e["error"]["code"].clone()), (json!(false), json!("bad_request")));
    let e = text(protocol::handle_request(&mut s, r#"{"id":3,"command":{"type":"explode"}}"#));
    assert_eq!((e["id"].clone(), e["error"]["code"].clone()), (json!(3), json!("bad_request")));
    let e = text(protocol::handle_request(&mut s, r#"{"id":4}"#));
    assert_eq!(e["error"]["code"], "bad_request");
    // typed API errors keep their code
    let e = text(protocol::handle_request(&mut s, r#"{"id":5,"command":{"type":"load_preset","name":"nope"}}"#));
    assert_eq!(e["error"]["code"], "unknown_preset");
    let e = text(protocol::handle_request(&mut s, r#"{"id":6,"query":{"type":"get_field","name":"temperature"}}"#));
    assert_eq!(e["error"]["code"], "not_solved");
    // a command
    let ok = text(protocol::handle_request(&mut s, r#"{"id":"abc","command":{"type":"set_param","name":"seed","value":12345}}"#));
    assert_eq!((ok["id"].clone(), ok["ok"].clone(), ok["kind"].clone()), (json!("abc"), json!(true), json!("command")));
    assert_eq!(ok["result"]["api_version"], wb_app::API_VERSION);
    assert_eq!(ok["result"]["invalidated"][0], "elevation");
    // a JSON-only query
    let info = text(protocol::handle_request(&mut s, r#"{"id":8,"query":{"type":"get_world_info"}}"#));
    assert_eq!(info["result"]["domain"]["kind"], "sphere");
}

#[test]
fn binary_frames_are_aligned_and_decodable() {
    let mut s = small_session();
    let Outgoing::Binary(b) = protocol::handle_request(&mut s, r#"{"id":1,"query":{"type":"get_field","name":"elevation"}}"#) else { panic!("expected binary") };
    let (head, payload) = unpack_binary(&b).unwrap();
    assert_eq!(head["id"], 1);
    let n = head["result"]["count"].as_u64().unwrap() as usize;
    assert_eq!(payload.len(), n * 4);
    assert_eq!(head["payload_bytes"].as_u64().unwrap() as usize, payload.len());
    assert_eq!((b.len() - payload.len()) % 4, 0, "payload starts on a 4-byte boundary so clients can view it as Float32Array");
    // elevation values round-trip
    let direct = s.field("elevation", 0).unwrap().values;
    let first = f32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    assert_eq!(first, direct[0] as f32);
    // mesh: layout offsets all multiples of 4
    let Outgoing::Binary(m) = protocol::handle_request(&mut s, r#"{"id":2,"query":{"type":"get_mesh"}}"#) else { panic!() };
    let (mh, mp) = unpack_binary(&m).unwrap();
    for item in mh["result"]["layout"].as_array().unwrap() {
        assert_eq!(item["offset"].as_u64().unwrap() % 4, 0);
    }
    assert_eq!(mp.len() as u64, mh["result"]["bytes"].as_u64().unwrap());
}

// ------------------------------------------------------------------ end to end

struct Client {
    s: TcpStream,
}

impl Client {
    fn connect(h: &ServerHandle) -> Client { Self::connect_with(h, &format!("127.0.0.1:{}", h.addr().port()), None) }

    fn connect_with(h: &ServerHandle, host: &str, origin: Option<&str>) -> Client {
        let mut s = TcpStream::connect(h.addr()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        let o = origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default();
        write!(s, "GET /ws HTTP/1.1\r\nHost: {host}\r\n{o}Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").unwrap();
        Client { s }
    }

    fn read_head(&mut self) -> String {
        let mut head = vec![];
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            self.s.read_exact(&mut b).unwrap();
            head.push(b[0]);
        }
        String::from_utf8(head).unwrap()
    }

    fn send(&mut self, v: Value) { self.s.write_all(&encode_masked(ws::OP_TEXT, true, v.to_string().as_bytes(), [0x12, 0x34, 0x56, 0x78])).unwrap(); }

    /// Next server message: text JSON or a decoded binary frame header (+ payload).
    fn next(&mut self) -> (Value, Vec<u8>) {
        let f = read_frame(&mut self.s, false, 1 << 26).unwrap();
        match f.opcode {
            ws::OP_TEXT => (serde_json::from_slice(&f.payload).unwrap(), vec![]),
            ws::OP_BINARY => {
                let (h, p) = unpack_binary(&f.payload).unwrap();
                (h, p.to_vec())
            }
            o => panic!("unexpected opcode {o}"),
        }
    }

    /// Next message that answers request `id`; events seen on the way are collected.
    fn reply(&mut self, id: u64, events: &mut Vec<Value>) -> (Value, Vec<u8>) {
        loop {
            let (v, p) = self.next();
            if v.get("event").is_some() {
                events.push(v["event"].clone());
            } else if v["id"] == id {
                return (v, p);
            }
        }
    }
}

fn server(extra: impl FnOnce(&mut ServerConfig)) -> ServerHandle {
    let mut cfg = ServerConfig { port: 0, params: vec![("resolution".into(), 2.0)], ..Default::default() };
    extra(&mut cfg);
    start(cfg).unwrap()
}

fn http_get(h: &ServerHandle, path: &str, host: &str) -> (String, String) {
    let mut s = TcpStream::connect(h.addr()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let (head, body) = out.split_once("\r\n\r\n").unwrap_or((&out, ""));
    (head.to_string(), body.to_string())
}

#[test]
fn binds_to_loopback_only_and_serves_a_fallback_page() {
    let h = server(|_| {});
    assert!(h.addr().ip().is_loopback());
    let (head, body) = http_get(&h, "/", "localhost");
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(body.contains("wb-server is running"));
    let (head, body) = http_get(&h, "/api/hello", "127.0.0.1");
    assert!(head.starts_with("HTTP/1.1 200"));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["hello"]["api_version"], wb_app::API_VERSION);
}

#[test]
fn rejects_foreign_host_and_origin() {
    let h = server(|_| {});
    let (head, _) = http_get(&h, "/", "evil.example");
    assert!(head.starts_with("HTTP/1.1 403"), "{head}");
    let mut c = Client::connect_with(&h, &format!("127.0.0.1:{}", h.addr().port()), Some("http://evil.example"));
    assert!(c.read_head().starts_with("HTTP/1.1 403"));
    let mut c = Client::connect_with(&h, "evil.example", None);
    assert!(c.read_head().starts_with("HTTP/1.1 403"));
    // a browser page served by this very server is allowed
    let mut c = Client::connect_with(&h, &format!("localhost:{}", h.addr().port()), Some(&format!("http://localhost:{}", h.addr().port())));
    assert!(c.read_head().starts_with("HTTP/1.1 101"));
}

#[test]
fn serves_static_files_and_blocks_traversal() {
    let dir = std::env::temp_dir().join(format!("wb-static-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<h1>client</h1>").unwrap();
    std::fs::write(dir.join("assets/app.js"), "console.log(1)").unwrap();
    std::fs::write(dir.parent().unwrap().join("wb-secret.txt"), "secret").unwrap();
    let d2 = dir.clone();
    let h = server(move |c| c.static_dir = Some(d2));
    let (head, body) = http_get(&h, "/", "localhost");
    assert!(head.starts_with("HTTP/1.1 200") && body == "<h1>client</h1>");
    let (head, body) = http_get(&h, "/assets/app.js", "localhost");
    assert!(head.contains("text/javascript") && body == "console.log(1)");
    let (head, _) = http_get(&h, "/assets/missing.js", "localhost");
    assert!(head.starts_with("HTTP/1.1 404"));
    let (head, body) = http_get(&h, "/some/route", "localhost");
    assert!(head.starts_with("HTTP/1.1 200") && body == "<h1>client</h1>", "extension-less paths fall back to index.html");
    for evil in ["/../wb-secret.txt", "/%2e%2e/wb-secret.txt", "/assets/..%2f..%2fwb-secret.txt", "/..%5cwb-secret.txt"] {
        let (head, body) = http_get(&h, evil, "localhost");
        assert!(!body.contains("secret"), "{evil} leaked: {head}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn websocket_session_end_to_end() {
    let h = server(|_| {});
    let mut c = Client::connect(&h);
    let head = c.read_head();
    assert!(head.starts_with("HTTP/1.1 101") && head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="), "{head}");
    let (hello, _) = c.next();
    assert_eq!(hello["hello"]["api_version"], wb_app::API_VERSION);
    assert_eq!(hello["hello"]["wire_version"], protocol::WIRE_VERSION);
    let mut ev = vec![];

    c.send(json!({"id": 1, "query": {"type": "get_schema"}}));
    let (r, _) = c.reply(1, &mut ev);
    assert!(r["result"]["fields"].as_array().unwrap().len() > 10);

    c.send(json!({"id": 2, "query": {"type": "get_mesh"}}));
    let (r, p) = c.reply(2, &mut ev);
    assert_eq!(p.len() as u64, r["result"]["bytes"].as_u64().unwrap());
    let n_cells = r["result"]["n_cells"].as_u64().unwrap();

    // unsolved field -> typed error over the wire
    c.send(json!({"id": 3, "query": {"type": "get_field", "name": "temperature"}}));
    assert_eq!(c.reply(3, &mut ev).0["error"]["code"], "not_solved");

    // solve: ack first, then progress events and job_done, then fields are available
    c.send(json!({"id": 4, "command": {"type": "solve"}}));
    let (ack, _) = c.reply(4, &mut ev);
    let job = ack["result"]["job"].as_u64().unwrap();
    assert!(ev.is_empty(), "no event may arrive before the ack of the command that caused it");
    let t0 = Instant::now();
    let mut done = false;
    while !done && t0.elapsed() < Duration::from_secs(60) {
        let (v, _) = c.next();
        if let Some(e) = v.get("event") {
            ev.push(e.clone());
            done = e["type"] == "job_done";
        }
    }
    assert!(done, "job_done arrives as a pushed event");
    assert!(ev.iter().any(|e| e["type"] == "job_progress" && e["job"] == job));
    assert!(ev.iter().any(|e| e["type"] == "field_changed" && e["names"].as_array().unwrap().iter().any(|n| n == "temperature")));

    c.send(json!({"id": 5, "query": {"type": "get_field", "name": "temperature"}}));
    let (r, p) = c.reply(5, &mut ev);
    assert_eq!(r["result"]["count"].as_u64().unwrap(), n_cells);
    assert_eq!(p.len() as u64, n_cells * 4);
}

#[test]
fn events_reach_every_client_and_errors_do_not_kill_the_connection() {
    let h = server(|_| {});
    let (mut a, mut b) = (Client::connect(&h), Client::connect(&h));
    a.read_head();
    b.read_head();
    a.next();
    b.next();
    a.send(json!({"id": 1, "command": {"type": "set_param", "name": "seed", "value": 99}}));
    let mut ev_a = vec![];
    assert_eq!(a.reply(1, &mut ev_a).0["ok"], true);
    // client b did not ask, but is told that the world changed
    let (v, _) = b.next();
    assert_eq!(v["event"]["type"], "field_changed");
    // garbage does not close the socket
    a.s.write_all(&encode_masked(ws::OP_TEXT, true, b"{{{", [1, 2, 3, 4])).unwrap();
    let (v, _) = a.next();
    let mut skip = v;
    while skip.get("event").is_some() {
        skip = a.next().0;
    }
    assert_eq!(skip["error"]["code"], "bad_request");
    a.send(json!({"id": 2, "query": {"type": "get_world_info"}}));
    assert_eq!(a.reply(2, &mut vec![]).0["ok"], true);
    // ping gets a pong
    a.s.write_all(&encode_masked(ws::OP_PING, true, b"hey", [1, 2, 3, 4])).unwrap();
    loop {
        let f = read_frame(&mut a.s, false, 1 << 20).unwrap();
        if f.opcode == ws::OP_PONG {
            assert_eq!(f.payload, b"hey");
            break;
        }
    }
}

#[test]
fn cancel_over_the_wire_and_clean_shutdown() {
    let mut h = server(|c| c.step_delay = Duration::from_millis(40));
    let mut c = Client::connect(&h);
    c.read_head();
    c.next();
    let mut ev = vec![];
    c.send(json!({"id": 1, "command": {"type": "solve"}}));
    c.reply(1, &mut ev);
    std::thread::sleep(Duration::from_millis(200));
    c.send(json!({"id": 2, "command": {"type": "cancel"}}));
    c.reply(2, &mut ev);
    let t0 = Instant::now();
    loop {
        let (v, _) = c.next();
        if v["event"]["type"] == "job_done" {
            assert_eq!(v["event"]["status"], "cancelled");
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(30));
    }
    let t = Instant::now();
    h.shutdown();
    assert!(t.elapsed() < Duration::from_secs(10), "shutdown is prompt");
}
