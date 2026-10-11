//! An in-memory [`RecordStore`].
//!
//! Used by the daemon as a fast cache / test double and as the reference
//! implementation the SQLite store is checked against. State lives behind a
//! plain `Mutex`; every method only holds the lock long enough to touch the
//! maps, so there is no `.await` while locked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cterm_core::{
    Error, ExecutionId, ExecutionRecord, RecordStore, Result, Session, SessionId, StreamChunk,
};

#[derive(Default)]
struct State {
    sessions: HashMap<SessionId, Session>,
    executions: HashMap<ExecutionId, ExecutionRecord>,
    chunks: HashMap<ExecutionId, Vec<StreamChunk>>,
}

/// A thread-safe, cloneable in-memory store. Clones share the same state.
#[derive(Clone, Default)]
pub struct MemoryStore {
    state: Arc<Mutex<State>>,
}

impl MemoryStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| Error::Storage("memory store mutex poisoned".to_owned()))
    }
}

#[async_trait]
impl RecordStore for MemoryStore {
    async fn put_session(&self, session: &Session) -> Result<()> {
        self.lock()?.sessions.insert(session.id, session.clone());
        Ok(())
    }

    async fn put_execution(&self, record: &ExecutionRecord) -> Result<()> {
        self.lock()?.executions.insert(record.id, record.clone());
        Ok(())
    }

    async fn append_chunk(&self, chunk: &StreamChunk) -> Result<()> {
        self.lock()?
            .chunks
            .entry(chunk.execution)
            .or_default()
            .push(chunk.clone());
        Ok(())
    }

    async fn get_session(&self, id: SessionId) -> Result<Option<Session>> {
        Ok(self.lock()?.sessions.get(&id).cloned())
    }

    async fn get_execution(&self, id: ExecutionId) -> Result<Option<ExecutionRecord>> {
        Ok(self.lock()?.executions.get(&id).cloned())
    }

    async fn list_sessions(&self) -> Result<Vec<Session>> {
        let mut v: Vec<Session> = self.lock()?.sessions.values().cloned().collect();
        // Most recently opened first.
        v.sort_by_key(|s| std::cmp::Reverse(s.opened_at));
        Ok(v)
    }

    async fn list_executions(&self, session: SessionId) -> Result<Vec<ExecutionRecord>> {
        let mut v: Vec<ExecutionRecord> = self
            .lock()?
            .executions
            .values()
            .filter(|e| e.session == session)
            .cloned()
            .collect();
        // Oldest first.
        v.sort_by_key(|e| e.started_at);
        Ok(v)
    }

    async fn load_chunks(&self, execution: ExecutionId) -> Result<Vec<StreamChunk>> {
        let mut v = self
            .lock()?
            .chunks
            .get(&execution)
            .cloned()
            .unwrap_or_default();
        v.sort_by_key(|c| c.seq);
        Ok(v)
    }
}
