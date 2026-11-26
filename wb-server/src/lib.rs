//! wb-server: the local adapter (UI_ARCHITECTURE layer "Adapter"). HTTP for the built client files, WebSocket for the API.
//! Std-only transport (hand-rolled HTTP/1.1 + RFC 6455) on top of `wb-app`; the only dependency besides wb-app is serde_json.
//! One thread (the session actor) owns the `Session`; connection threads only parse bytes. Binds to 127.0.0.1 only.
pub mod codec;
pub mod http;
pub mod protocol;
pub mod server;
pub mod ws;

pub use server::{start, ServerConfig, ServerHandle};
