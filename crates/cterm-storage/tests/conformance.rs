//! One scenario, run against every `RecordStore` implementation.
//!
//! Any store that passes this behaves identically for the daemon's purposes:
//! round-tripping entities, filtering executions by session, ordering, and
//! reporting absence as `None`.

use cterm_core::{
    Command, ExecutionRecord, ExecutionStatus, Host, RecordStore, Session, SessionStatus,
    StreamChunk, StreamKind,
};
use cterm_storage::{MemoryStore, SqliteStore};
use time::{Duration, OffsetDateTime};

fn at(secs: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap() + Duration::seconds(secs)
}

/// Exercises the full `RecordStore` contract against one implementation.
async fn exercise<S: RecordStore>(store: S) {
    // --- Two sessions, opened at distinct times. ---
    let mut sess_a = Session::open(Host::named("prod-1", true), "zsh");
    sess_a.opened_at = at(10);
    let mut sess_b = Session::open(Host::local(), "bash");
    sess_b.opened_at = at(20);

    store.put_session(&sess_a).await.unwrap();
    store.put_session(&sess_b).await.unwrap();

    // Round-trip a session.
    assert_eq!(
        store.get_session(sess_a.id).await.unwrap().as_ref(),
        Some(&sess_a)
    );

    // list_sessions: most recently opened first (b before a).
    let sessions = store.list_sessions().await.unwrap();
    assert_eq!(
        sessions.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![sess_b.id, sess_a.id]
    );

    // --- Executions: two in A (out of insertion order), one in B. ---
    let mut e1 = ExecutionRecord::open(sess_a.id, Command::new("echo first", "zsh"));
    e1.started_at = at(100);
    let mut e2 = ExecutionRecord::open(sess_a.id, Command::new("echo second", "zsh"));
    e2.started_at = at(200);
    let mut e_other = ExecutionRecord::open(sess_b.id, Command::new("ls", "bash"));
    e_other.started_at = at(150);

    // Insert the later one first to prove ordering is by started_at, not insert order.
    store.put_execution(&e2).await.unwrap();
    store.put_execution(&e1).await.unwrap();
    store.put_execution(&e_other).await.unwrap();

    let in_a = store.list_executions(sess_a.id).await.unwrap();
    assert_eq!(
        in_a.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![e1.id, e2.id]
    );
    assert_eq!(store.list_executions(sess_b.id).await.unwrap().len(), 1);

    // Updating an execution in place (status transition) must overwrite.
    let mut e1_done = e1.clone();
    e1_done.transition(ExecutionStatus::Completed { code: 0 });
    store.put_execution(&e1_done).await.unwrap();
    let fetched = store.get_execution(e1.id).await.unwrap().unwrap();
    assert_eq!(fetched.status, ExecutionStatus::Completed { code: 0 });
    assert!(fetched.ended_at.is_some());

    // --- Chunks: appended out of seq order, read back in seq order. ---
    for seq in [2u64, 0, 1] {
        let data = vec![b'0' + seq as u8];
        store
            .append_chunk(&StreamChunk::now(e1.id, StreamKind::Stdout, seq, data))
            .await
            .unwrap();
    }
    let chunks = store.load_chunks(e1.id).await.unwrap();
    assert_eq!(
        chunks.iter().map(|c| c.seq).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(chunks[0].data, vec![b'0']);

    // Non-UTF-8 bytes survive the round trip.
    store
        .append_chunk(&StreamChunk::now(
            e1.id,
            StreamKind::Stderr,
            3,
            vec![0xff, 0x00],
        ))
        .await
        .unwrap();
    let chunks = store.load_chunks(e1.id).await.unwrap();
    assert_eq!(chunks.last().unwrap().data, vec![0xff, 0x00]);

    // --- Absence is None, not an error. ---
    use cterm_core::{ExecutionId, SessionId};
    assert!(store.get_session(SessionId::new()).await.unwrap().is_none());
    assert!(store
        .get_execution(ExecutionId::new())
        .await
        .unwrap()
        .is_none());
    assert!(store
        .load_chunks(ExecutionId::new())
        .await
        .unwrap()
        .is_empty());

    // Closing a session persists the terminal status.
    sess_a.close(SessionStatus::Closed);
    store.put_session(&sess_a).await.unwrap();
    assert_eq!(
        store.get_session(sess_a.id).await.unwrap().unwrap().status,
        SessionStatus::Closed
    );
}

#[tokio::test]
async fn memory_store_conforms() {
    exercise(MemoryStore::new()).await;
}

#[tokio::test]
async fn sqlite_store_conforms() {
    exercise(SqliteStore::open_in_memory().unwrap()).await;
}
