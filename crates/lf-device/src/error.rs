use bytes::Bytes;

use crate::proto::ErrorType;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("couldn't connect to {target}: {reason}")]
    Connect { target: String, reason: String },
    #[error("gRPC error: {0}")]
    Rpc(#[from] tonic::Status),
    /// Most replies carry no payload, so a failed command may simply never be
    /// answered. Treat a timeout as "unknown outcome", not as success.
    #[error("no reply to {request} within {seconds:.1} s")]
    Timeout { request: String, seconds: f64 },
    #[error("the device closed the connection")]
    Disconnected,
    #[error("device error {kind:?}: {message}")]
    Device { kind: ErrorType, message: String },
    /// A reply whose payload didn't decode to the reply for the request sent.
    /// `raw` is the whole frame, for comparing against the recovered schema.
    #[error("unexpected reply to {request}: {}", describe_raw(.raw))]
    UnexpectedReply { request: String, raw: Bytes },
    #[error("discovery failed: {0}")]
    Discovery(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

fn describe_raw(raw: &Bytes) -> String {
    format!("{} bytes", raw.len())
}
