use thiserror::Error;

#[derive(Debug, Error)]
pub enum NleError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("XML parse error: {0}")]
    Xml(String),

    #[error("Archive error: {0}")]
    Archive(String),

    #[error("Invalid project format: {0}")]
    InvalidFormat(String),

    #[error("Unsupported feature: {0}")]
    Unsupported(String),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type NleResult<T> = Result<T, NleError>;
