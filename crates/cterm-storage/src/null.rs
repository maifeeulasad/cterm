//! A [`RecordStore`] that discards everything.
//!
//! Used by the shim when it is connected to the daemon over IPC: the PTY runner
//! needs a store to satisfy its interface, but persistence is the daemon's job,
//! so the shim drops its own copy rather than double-writing.

use async_trait::async_trait;
use cterm_core::{
    ExecutionId, ExecutionRecord, RecordStore, Result, Session, SessionId, StreamChunk,
};

/// A store that accepts all writes and returns nothing on reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullStore;

impl NullStore {
    /// Creates a `NullStore`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl RecordStore for NullStore {
    async fn put_session(&self, _session: &Session) -> Result<()> {
        Ok(())
    }
    async fn put_execution(&self, _record: &ExecutionRecord) -> Result<()> {
        Ok(())
    }
    async fn append_chunk(&self, _chunk: &StreamChunk) -> Result<()> {
        Ok(())
    }
    async fn get_session(&self, _id: SessionId) -> Result<Option<Session>> {
        Ok(None)
    }
    async fn get_execution(&self, _id: ExecutionId) -> Result<Option<ExecutionRecord>> {
        Ok(None)
    }
    async fn list_sessions(&self) -> Result<Vec<Session>> {
        Ok(Vec::new())
    }
    async fn list_executions(&self, _session: SessionId) -> Result<Vec<ExecutionRecord>> {
        Ok(Vec::new())
    }
    async fn load_chunks(&self, _execution: ExecutionId) -> Result<Vec<StreamChunk>> {
        Ok(Vec::new())
    }
}
