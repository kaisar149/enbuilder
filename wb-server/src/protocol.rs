//! The wire protocol (version 1) on top of the wb-app API. Pure functions: bytes/strings in, bytes/strings out, no sockets.
//!
//! Client -> server (text frame, JSON): `{"id": <any>, "command": {...Command}}` or `{"id": <any>, "query": {...Query}}`.
//! Server -> client:
//!  * `hello` text frame once after connecting: `{"hello": {"wire_version", "api_version", "server"}}`
//!  * response text frame: `{"id", "ok": true, "kind": "command"|"query", "result": ...}` or `{"id", "ok": false, "error": {"code","message"}}`
//!  * a query that has binary data answers with ONE binary frame instead (layout below) whose JSON header is the same response envelope
//!  * event text frame, pushed: `{"event": {...Event}}`
//!
//! Binary frame: `"WBF1"` | u32 LE json_len | json (UTF-8) | zero padding so the payload starts at a multiple of 4 | payload (little-endian arrays,
//! described by the JSON `result` exactly as in `ARCHITECTURE_9.md`). `payload_bytes` in the envelope gives the payload length.
use serde_json::{json, Value};
use wb_app::{Command, Event, Query, Session, API_VERSION};

pub const WIRE_VERSION: u32 = 1;
pub const MAGIC: &[u8; 4] = b"WBF1";

#[derive(Debug, Clone, PartialEq)]
pub enum Outgoing {
    Text(String),
    Binary(Vec<u8>),
}

pub fn hello() -> String { json!({ "hello": { "wire_version": WIRE_VERSION, "api_version": API_VERSION, "server": "wb-server" } }).to_string() }

pub fn event_message(e: &Event) -> String { json!({ "event": e }).to_string() }

/// Packs a JSON header and a payload into one binary frame body.
pub fn pack_binary(header: &Value, payload: &[u8]) -> Vec<u8> {
    let j = header.to_string().into_bytes();
    let mut out = Vec::with_capacity(8 + j.len() + 4 + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(&j);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out.extend_from_slice(payload);
    out
}

/// Inverse of `pack_binary` (used by tests and by anything that wants to read frames in Rust).
pub fn unpack_binary(frame: &[u8]) -> Option<(Value, &[u8])> {
    if frame.len() < 8 || &frame[..4] != MAGIC {
        return None;
    }
    let n = u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]) as usize;
    let header: Value = serde_json::from_slice(frame.get(8..8 + n)?).ok()?;
    let start = (8 + n + 3) / 4 * 4;
    Some((header, frame.get(start..)?))
}

fn error(id: &Value, code: &str, message: String) -> Outgoing { Outgoing::Text(json!({ "id": id, "ok": false, "error": { "code": code, "message": message } }).to_string()) }

/// Handles one client text message against the session.
pub fn handle_request(session: &mut Session, text: &str) -> Outgoing {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return error(&Value::Null, "bad_request", format!("not JSON: {e}")),
    };
    let id = v.get("id").cloned().unwrap_or(Value::Null);
    if let Some(c) = v.get("command") {
        let cmd: Command = match serde_json::from_value(c.clone()) {
            Ok(c) => c,
            Err(e) => return error(&id, "bad_request", format!("bad command: {e}")),
        };
        return match session.execute(cmd) {
            Ok(ack) => Outgoing::Text(json!({ "id": id, "ok": true, "kind": "command", "result": ack }).to_string()),
            Err(e) => error(&id, &e.code, e.message),
        };
    }
    if let Some(q) = v.get("query") {
        let query: Query = match serde_json::from_value(q.clone()) {
            Ok(q) => q,
            Err(e) => return error(&id, "bad_request", format!("bad query: {e}")),
        };
        return match session.query(query) {
            Ok(r) if r.binary.is_empty() => Outgoing::Text(json!({ "id": id, "ok": true, "kind": "query", "result": r.json }).to_string()),
            Ok(r) => {
                let env = json!({ "id": id, "ok": true, "kind": "query", "result": r.json, "payload_bytes": r.binary.len() });
                Outgoing::Binary(pack_binary(&env, &r.binary))
            }
            Err(e) => error(&id, &e.code, e.message),
        };
    }
    error(&id, "bad_request", "expected a \"command\" or a \"query\"".into())
}
