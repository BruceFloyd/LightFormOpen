/// Errors from reading or writing a document.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unexpected end of data at byte {0}")]
    UnexpectedEof(usize),
    #[error("unknown chunk tag {tag:#04x} at byte {offset}")]
    UnknownTag { tag: u8, offset: usize },
    /// The bytes parse as chunks but don't match the document layout.
    /// The message names the chunk path where it went wrong. Paths can
    /// include keys from the file, so control characters are escaped when
    /// the message is displayed.
    #[error("{}", escape_controls(.0))]
    Malformed(String),
    #[error("invalid JSON document: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Escape control characters so text taken from a file can't drive the terminal.
fn escape_controls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

pub(crate) fn malformed<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Malformed(msg.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_messages_escape_control_characters() {
        let err = Error::Malformed("object/\u{1b}[2Jkey".into());
        assert_eq!(err.to_string(), "object/\\u{1b}[2Jkey");
    }
}
