//! A SQLite-backed [`RecordStore`].
//!
//! Each entity is stored as its JSON encoding plus a few indexed columns (id,
//! parent, sort key). Encoding through `serde` keeps the three stores
//! (memory, SQLite, and anything future) byte-for-byte consistent and avoids a
//! hand-written column mapping per type.
//!
//! `rusqlite` is blocking, so every method runs its work on a blocking thread
//! via [`tokio::task::spawn_blocking`]; the connection is shared behind a
//! `Mutex` (SQLite serializes writes anyway).

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cterm_core::{
    Error, ExecutionId, ExecutionRecord, RecordStore, Result, Session, SessionId, StreamChunk,
};
use rusqlite::{params, Connection, OptionalExtension};
use time::OffsetDateTime;

const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS sessions (
    id         TEXT    PRIMARY KEY,
    opened_at  INTEGER NOT NULL,
    json       TEXT    NOT NULL
);
CREATE TABLE IF NOT EXISTS executions (
    id         TEXT    PRIMARY KEY,
    session    TEXT    NOT NULL,
    started_at INTEGER NOT NULL,
    json       TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_exec_session ON executions(session, started_at);
CREATE TABLE IF NOT EXISTS chunks (
    execution  TEXT    NOT NULL,
    seq        INTEGER NOT NULL,
    json       TEXT    NOT NULL,
    PRIMARY KEY (execution, seq)
);
";

/// A durable store backed by a single SQLite database.
#[derive(Clone)]
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// Opens (creating if needed) a database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path).map_err(sql_err)?;
        Self::from_conn(conn)
    }

    /// Opens a private in-memory database (used in tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(sql_err)?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        // WAL lets the daemon read while a shim writes the same database from
        // another process; busy_timeout rides out brief write-lock contention
        // instead of failing with SQLITE_BUSY. (WAL is a no-op for :memory:.)
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(sql_err)?;
        conn.execute_batch(SCHEMA).map_err(sql_err)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs `f` with the connection on a blocking thread.
    async fn with_conn<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn
                .lock()
                .map_err(|_| Error::Storage("sqlite mutex poisoned".to_owned()))?;
            f(&guard)
        })
        .await
        .map_err(|e| Error::Storage(format!("blocking task join failed: {e}")))?
    }
}

fn sql_err(e: rusqlite::Error) -> Error {
    Error::Storage(e.to_string())
}

fn json_err(e: serde_json::Error) -> Error {
    Error::Serialization(e.to_string())
}

/// Nanoseconds since the Unix epoch, used as a monotonic, numerically
/// sortable column (string RFC3339 sorts wrongly when some values carry
/// fractional seconds and others do not).
fn nanos(t: OffsetDateTime) -> i64 {
    t.unix_timestamp_nanos() as i64
}

#[async_trait]
impl RecordStore for SqliteStore {
    async fn put_session(&self, session: &Session) -> Result<()> {
        let id = session.id.to_string();
        let opened = nanos(session.opened_at);
        let json = serde_json::to_string(session).map_err(json_err)?;
        self.with_conn(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO sessions(id, opened_at, json) VALUES (?1, ?2, ?3)",
                params![id, opened, json],
            )
            .map_err(sql_err)?;
            Ok(())
        })
        .await
    }

    async fn put_execution(&self, record: &ExecutionRecord) -> Result<()> {
        let id = record.id.to_string();
        let session = record.session.to_string();
        let started = nanos(record.started_at);
        let json = serde_json::to_string(record).map_err(json_err)?;
        self.with_conn(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO executions(id, session, started_at, json) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, session, started, json],
            )
            .map_err(sql_err)?;
            Ok(())
        })
        .await
    }

    async fn append_chunk(&self, chunk: &StreamChunk) -> Result<()> {
        let execution = chunk.execution.to_string();
        let seq = chunk.seq as i64;
        let json = serde_json::to_string(chunk).map_err(json_err)?;
        self.with_conn(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO chunks(execution, seq, json) VALUES (?1, ?2, ?3)",
                params![execution, seq, json],
            )
            .map_err(sql_err)?;
            Ok(())
        })
        .await
    }

    async fn get_session(&self, id: SessionId) -> Result<Option<Session>> {
        let key = id.to_string();
        self.with_conn(move |c| {
            let row: Option<String> = c
                .query_row(
                    "SELECT json FROM sessions WHERE id = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql_err)?;
            row.map(|j| serde_json::from_str(&j).map_err(json_err))
                .transpose()
        })
        .await
    }

    async fn get_execution(&self, id: ExecutionId) -> Result<Option<ExecutionRecord>> {
        let key = id.to_string();
        self.with_conn(move |c| {
            let row: Option<String> = c
                .query_row(
                    "SELECT json FROM executions WHERE id = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql_err)?;
            row.map(|j| serde_json::from_str(&j).map_err(json_err))
                .transpose()
        })
        .await
    }

    async fn list_sessions(&self) -> Result<Vec<Session>> {
        self.with_conn(move |c| {
            let mut stmt = c
                .prepare("SELECT json FROM sessions ORDER BY opened_at DESC")
                .map_err(sql_err)?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(sql_err)?;
            let mut out = Vec::new();
            for row in rows {
                let j = row.map_err(sql_err)?;
                out.push(serde_json::from_str(&j).map_err(json_err)?);
            }
            Ok(out)
        })
        .await
    }

    async fn list_executions(&self, session: SessionId) -> Result<Vec<ExecutionRecord>> {
        let key = session.to_string();
        self.with_conn(move |c| {
            let mut stmt = c
                .prepare(
                    "SELECT json FROM executions WHERE session = ?1 ORDER BY started_at ASC",
                )
                .map_err(sql_err)?;
            let rows = stmt
                .query_map(params![key], |r| r.get::<_, String>(0))
                .map_err(sql_err)?;
            let mut out = Vec::new();
            for row in rows {
                let j = row.map_err(sql_err)?;
                out.push(serde_json::from_str(&j).map_err(json_err)?);
            }
            Ok(out)
        })
        .await
    }

    async fn load_chunks(&self, execution: ExecutionId) -> Result<Vec<StreamChunk>> {
        let key = execution.to_string();
        self.with_conn(move |c| {
            let mut stmt = c
                .prepare("SELECT json FROM chunks WHERE execution = ?1 ORDER BY seq ASC")
                .map_err(sql_err)?;
            let rows = stmt
                .query_map(params![key], |r| r.get::<_, String>(0))
                .map_err(sql_err)?;
            let mut out = Vec::new();
            for row in rows {
                let j = row.map_err(sql_err)?;
                out.push(serde_json::from_str(&j).map_err(json_err)?);
            }
            Ok(out)
        })
        .await
    }
}
