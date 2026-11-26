//! Minimal HTTP/1.1: request head parsing, host/origin checks (localhost only), static file serving.
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>, // lower-case names
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> { self.headers.get(name).map(|s| s.as_str()) }
}

/// Reads the request head (up to the blank line, max 16 KiB). Any bytes after the head are not consumed beyond the buffer read,
/// so this reads one byte at a time from the head only (the head is short and the socket is local).
pub fn read_request<R: Read>(r: &mut R) -> io::Result<Request> {
    let mut head = Vec::with_capacity(512);
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "request head too large"));
        }
        if r.read(&mut b)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed before the request head ended"));
        }
        head.push(b[0]);
    }
    parse_head(&String::from_utf8_lossy(&head))
}

pub fn parse_head(text: &str) -> io::Result<Request> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let mut lines = text.split("\r\n");
    let first = lines.next().ok_or_else(|| bad("empty request"))?;
    let mut parts = first.split(' ');
    let (method, path, version) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method.is_empty() || !path.starts_with('/') || !version.starts_with("HTTP/1.") {
        return Err(bad("malformed request line"));
    }
    let mut headers = HashMap::new();
    for l in lines {
        if l.is_empty() {
            break;
        }
        let (k, v) = l.split_once(':').ok_or_else(|| bad("malformed header"))?;
        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
    }
    Ok(Request { method: method.to_string(), path: path.to_string(), headers })
}

/// True for `localhost`, `127.0.0.1` and `[::1]`, with or without a port.
pub fn is_local_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    let name = if let Some(rest) = h.strip_prefix('[') { rest.split(']').next().unwrap_or("").to_string() } else { h.split(':').next().unwrap_or("").to_string() };
    matches!(name.as_str(), "localhost" | "127.0.0.1" | "::1")
}

/// Guards against DNS rebinding (Host) and cross-site WebSocket hijacking (Origin). A missing Origin is allowed (non-browser clients).
pub fn check_local(req: &Request) -> Result<(), &'static str> {
    match req.header("host") {
        Some(h) if is_local_host(h) => {}
        _ => return Err("host is not localhost"),
    }
    if let Some(o) = req.header("origin") {
        let rest = o.split_once("://").map(|x| x.1).unwrap_or(o);
        if !is_local_host(rest) {
            return Err("origin is not localhost");
        }
    }
    Ok(())
}

pub fn response(status: u16, reason: &str, content_type: &str, body: &[u8], head_only: bool) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    if !head_only {
        out.extend_from_slice(body);
    }
    out
}

pub fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Maps a URL path to a file below `root`. None when the path tries to escape (`..`, backslashes, NUL, absolute segments).
pub fn resolve_static(root: &Path, url_path: &str) -> Option<PathBuf> {
    let path = url_path.split(['?', '#']).next().unwrap_or("/");
    let decoded = percent_decode(path)?;
    if decoded.contains('\0') || decoded.contains('\\') {
        return None;
    }
    let mut out = root.to_path_buf();
    for seg in decoded.split('/') {
        match seg {
            "" | "." => {}
            ".." => return None,
            s => out.push(s),
        }
    }
    Some(out)
}

/// Serves a file (or `index.html` for `/` and for extension-less unknown paths, so client-side routes work).
pub fn serve_static(root: &Path, req: &Request) -> Vec<u8> {
    let head_only = req.method == "HEAD";
    let Some(mut file) = resolve_static(root, &req.path) else {
        return response(403, "Forbidden", "text/plain", b"forbidden", head_only);
    };
    if file.is_dir() {
        file.push("index.html");
    }
    if !file.is_file() && file.extension().is_none() {
        file = root.join("index.html");
    }
    match std::fs::read(&file) {
        Ok(bytes) => response(200, "OK", mime(&file), &bytes, head_only),
        Err(_) => response(404, "Not Found", "text/plain", b"not found", head_only),
    }
}

pub const NO_CLIENT_PAGE: &str = "<!doctype html><meta charset=utf-8><title>wb-server</title>\
<body style=\"font:16px system-ui;max-width:42em;margin:3em auto;padding:0 1em\">\
<h1>wb-server is running</h1><p>The web client is not built yet, so there is nothing to show here.</p>\
<pre>npm install\nnpm run build -w ui-web</pre><p>then reload this page, or start the server with <code>--static &lt;dir&gt;</code>. \
The API is available at <code>ws://127.0.0.1:PORT/ws</code>.</p>";

pub fn write_all<W: Write>(w: &mut W, bytes: &[u8]) -> io::Result<()> {
    w.write_all(bytes)?;
    w.flush()
}
