//! RFC 6455 WebSocket: handshake key, frame reader (client frames are masked), frame writer (server frames are not).
use crate::codec::{base64_encode, sha1};
use std::io::{self, Read};

pub const OP_CONT: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xA;

/// Largest message we accept from a client (commands are tiny).
pub const MAX_MESSAGE: usize = 1 << 20;

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub fn accept_key(client_key: &str) -> String { base64_encode(&sha1(format!("{}{}", client_key.trim(), GUID).as_bytes())) }

#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub fin: bool,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

fn bad(msg: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, msg.to_string()) }

/// Reads one frame. Errors with `InvalidData` on protocol violations (unmasked client frame, reserved bits, oversized payload).
pub fn read_frame<R: Read>(r: &mut R, require_mask: bool, max: usize) -> io::Result<Frame> {
    let mut h = [0u8; 2];
    r.read_exact(&mut h)?;
    let (fin, rsv, opcode) = (h[0] & 0x80 != 0, h[0] & 0x70, h[0] & 0x0F);
    if rsv != 0 {
        return Err(bad("reserved bits set"));
    }
    let masked = h[1] & 0x80 != 0;
    if require_mask && !masked {
        return Err(bad("client frame is not masked"));
    }
    let mut len = (h[1] & 0x7F) as u64;
    if len == 126 {
        let mut b = [0u8; 2];
        r.read_exact(&mut b)?;
        len = u16::from_be_bytes(b) as u64;
    } else if len == 127 {
        let mut b = [0u8; 8];
        r.read_exact(&mut b)?;
        len = u64::from_be_bytes(b);
    }
    if opcode >= 0x8 && (len > 125 || !fin) {
        return Err(bad("bad control frame"));
    }
    if len > max as u64 {
        return Err(bad("frame too large"));
    }
    let mut mask = [0u8; 4];
    if masked {
        r.read_exact(&mut mask)?;
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload)?;
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok(Frame { fin, opcode, payload })
}

/// Encodes one unfragmented server frame.
pub fn encode_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.push(0x80 | opcode);
    let n = payload.len();
    if n < 126 {
        out.push(n as u8);
    } else if n <= 0xFFFF {
        out.push(126);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    out.extend_from_slice(payload);
    out
}

/// Encodes a masked client frame (used by tests and the Rust test client).
pub fn encode_masked(opcode: u8, fin: bool, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = vec![(if fin { 0x80 } else { 0 }) | opcode];
    let n = payload.len();
    if n < 126 {
        out.push(0x80 | n as u8);
    } else if n <= 0xFFFF {
        out.push(0x80 | 126);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// What the connection loop should do with an assembled message.
#[derive(Debug, PartialEq, Eq)]
pub enum Incoming {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Close(Vec<u8>),
}

/// Assembles messages from frames. Keeps the partial message across interleaved control frames (a ping may arrive mid-message).
#[derive(Default)]
pub struct MessageReader {
    acc: Vec<u8>,
    first: Option<u8>,
}

impl MessageReader {
    pub fn new() -> Self { Self::default() }

    /// Reads frames until a whole message (or a control frame) is available. Pongs are ignored.
    pub fn read<R: Read>(&mut self, r: &mut R, max: usize) -> io::Result<Incoming> {
        loop {
            let f = read_frame(r, true, max)?;
            match f.opcode {
                OP_PING => return Ok(Incoming::Ping(f.payload)),
                OP_PONG => continue,
                OP_CLOSE => return Ok(Incoming::Close(f.payload)),
                OP_TEXT | OP_BINARY => {
                    if self.first.is_some() {
                        return Err(bad("new message inside a fragmented one"));
                    }
                    self.first = Some(f.opcode);
                    self.acc = f.payload;
                }
                OP_CONT => {
                    if self.first.is_none() {
                        return Err(bad("continuation without a start"));
                    }
                    if self.acc.len() + f.payload.len() > max {
                        return Err(bad("message too large"));
                    }
                    self.acc.extend(f.payload);
                }
                _ => return Err(bad("unknown opcode")),
            }
            if f.fin {
                let kind = self.first.take();
                let data = std::mem::take(&mut self.acc);
                return match kind {
                    Some(OP_TEXT) => String::from_utf8(data).map(Incoming::Text).map_err(|_| bad("text frame is not UTF-8")),
                    _ => Ok(Incoming::Binary(data)),
                };
            }
        }
    }
}
