//! CLI error type.

/// Errors from CLI operations (install/uninstall/status and the wrapper).
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// A filesystem or process I/O failure.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Config (de)serialization failure.
    #[error("config: {0}")]
    Json(#[from] serde_json::Error),
    /// Anything else, with a message.
    #[error("{0}")]
    Msg(String),
}

/// Result alias for CLI operations.
pub type CliResult<T> = std::result::Result<T, CliError>;
