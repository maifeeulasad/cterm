//! The crate-wide error type shared across ports.

/// Errors surfaced by domain ports ([`crate::ports`]).
///
/// Implementations map their concrete failures (SQLite, sockets, serde) onto
/// these variants so callers can react without depending on a specific backend.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The requested entity does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// A persistence backend failed.
    #[error("storage error: {0}")]
    Storage(String),

    /// An IPC/gRPC transport failed.
    #[error("transport error: {0}")]
    Transport(String),

    /// (De)serialization failed.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// The operation is not permitted by policy (e.g. writing to a
    /// production session an agent may only observe).
    #[error("forbidden: {0}")]
    Forbidden(String),

    /// Anything not otherwise classified.
    #[error("{0}")]
    Other(String),
}

/// Convenient result alias for port methods.
pub type Result<T> = std::result::Result<T, Error>;
