//! Crate-wide error type. Commands print `error: <message>` and exit 1.

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("HTTP request failed: {0}")]
    Http(#[from] ureq::Error),
    #[error("DevTools websocket failed: {0}")]
    WebSocket(#[from] tungstenite::Error),
}

impl Error {
    pub(crate) fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}
