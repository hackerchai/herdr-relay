pub mod access;
pub mod client;
pub mod control;
pub mod frame;
pub mod host;
pub mod identity;
pub mod native;
pub mod pairing;
pub mod relay;
pub mod retry;
pub mod secure;
pub mod service;
pub mod stream;
pub mod targets;
pub mod terminal;
pub mod transport;
pub mod user_service;

#[derive(Debug)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
    pub retry_after: Option<std::time::Duration>,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retry_after: None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
