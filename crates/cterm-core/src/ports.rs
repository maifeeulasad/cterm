//! Ports: the abstract interfaces the rest of the system plugs into.
//!
//! These traits are the seams of the architecture (a hexagonal / ports-and-
//! adapters style). The domain depends only on these abstractions; concrete
//! adapters live in outer crates:
//!
//!   * [`RecordStore`] — persistence. Adapter: `cterm-storage` (SQLite).
//!   * [`TerminalControl`] — the agent-facing control surface. Adapter:
//!     `cterm-daemon`, exposed over gRPC by `cterm-proto`.
//!
//! Keeping them here lets us unit-test the daemon against an in-memory fake and
//! swap backends without touching call sites.

use async_trait::async_trait;

use crate::error::Result;
use crate::ids::{ExecutionId, SessionId};
use crate::record::ExecutionRecord;
use crate::session::Session;
use crate::stream::StreamChunk;

/// Durable storage for sessions, execution records, and their stream chunks.
///
/// Implementations must be safe to share across tasks (`Send + Sync`); the
/// daemon holds one and serves many concurrent agents from it.
#[async_trait]
pub trait RecordStore: Send + Sync {
    /// Inserts or updates a session.
    async fn put_session(&self, session: &Session) -> Result<()>;

    /// Inserts or updates an execution record.
    async fn put_execution(&self, record: &ExecutionRecord) -> Result<()>;

    /// Appends one stream chunk. Chunks are immutable once written.
    async fn append_chunk(&self, chunk: &StreamChunk) -> Result<()>;

    /// Fetches a session by id, or `None` if unknown.
    async fn get_session(&self, id: SessionId) -> Result<Option<Session>>;

    /// Fetches an execution by id, or `None` if unknown.
    async fn get_execution(&self, id: ExecutionId) -> Result<Option<ExecutionRecord>>;

    /// Lists sessions, most recently opened first.
    async fn list_sessions(&self) -> Result<Vec<Session>>;

    /// Lists executions for a session, oldest first.
    async fn list_executions(&self, session: SessionId) -> Result<Vec<ExecutionRecord>>;

    /// Loads all stored chunks for an execution, in `seq` order.
    async fn load_chunks(&self, execution: ExecutionId) -> Result<Vec<StreamChunk>>;
}

/// The control surface agents use to observe and (where permitted) drive
/// sessions. This is the trait the gRPC service is a thin adapter over, so the
/// transport can be tested independently of the daemon's internals.
#[async_trait]
pub trait TerminalControl: Send + Sync {
    /// All sessions currently known.
    async fn list_sessions(&self) -> Result<Vec<Session>>;

    /// Executions within a session.
    async fn list_executions(&self, session: SessionId) -> Result<Vec<ExecutionRecord>>;

    /// A single execution's record, erroring with [`crate::error::Error::NotFound`]
    /// if unknown.
    async fn get_execution(&self, execution: ExecutionId) -> Result<ExecutionRecord>;

    /// Writes bytes to a session's stdin. May be rejected with
    /// [`crate::error::Error::Forbidden`] for observe-only (e.g. production)
    /// sessions.
    async fn send_input(&self, session: SessionId, data: Vec<u8>) -> Result<()>;

    /// Requests interruption of a running execution (e.g. deliver SIGINT).
    async fn interrupt(&self, execution: ExecutionId) -> Result<()>;
}
