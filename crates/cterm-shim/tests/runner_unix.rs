//! End-to-end: run a real command under a PTY and verify the recorded
//! execution (status lifecycle + stream chunks) in a RecordStore. Unix-only.
#![cfg(unix)]

use std::sync::Arc;
use std::time::Duration;

use cterm_core::{Command, ExecutionEvent, ExecutionStatus, RecordStore, SessionId, StreamKind};
use cterm_shim::{start_recorded, RunSpec};
use cterm_storage::MemoryStore;

fn concat(kind: StreamKind, chunks: &[cterm_core::StreamChunk]) -> Vec<u8> {
    chunks
        .iter()
        .filter(|c| c.kind == kind)
        .flat_map(|c| c.data.clone())
        .collect()
}

#[tokio::test]
async fn records_output_and_terminal_status() {
    let store = Arc::new(MemoryStore::new());
    let session = SessionId::new();
    let run = start_recorded(
        store.clone(),
        session,
        Command::new("printf AAABBB; exit 3", "bash"),
        RunSpec::new("/bin/bash", ["-c", "printf AAABBB; exit 3"]),
    )
    .await
    .expect("start");
    let exec = run.execution();

    let status = run.done.await.expect("join").expect("run result");
    assert_eq!(status, ExecutionStatus::Completed { code: 3 });

    // Persisted record reflects the terminal status and has an end time.
    let rec = store.get_execution(exec).await.unwrap().unwrap();
    assert_eq!(rec.status, ExecutionStatus::Completed { code: 3 });
    assert!(rec.ended_at.is_some());
    assert_eq!(rec.session, session);

    // Output stream captured.
    let chunks = store.load_chunks(exec).await.unwrap();
    let out = String::from_utf8_lossy(&concat(StreamKind::Stdout, &chunks)).into_owned();
    assert!(out.contains("AAABBB"), "stdout was: {out:?}");

    // Listed under its session.
    let listed = store.list_executions(session).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, exec);
}

#[tokio::test]
async fn records_input_and_evaluates_it() {
    let store = Arc::new(MemoryStore::new());
    let run = start_recorded(
        store.clone(),
        SessionId::new(),
        Command::new("interactive", "bash"),
        RunSpec::new("/bin/bash", Vec::<String>::new()),
    )
    .await
    .expect("start");
    let exec = run.execution();

    run.send_input(b"echo val-$((6*7))\n".to_vec()).await.unwrap();
    run.send_input(b"exit\n".to_vec()).await.unwrap();

    let status = run.done.await.expect("join").expect("run result");
    assert_eq!(status, ExecutionStatus::Completed { code: 0 });

    let chunks = store.load_chunks(exec).await.unwrap();
    // Input was recorded.
    let stdin = String::from_utf8_lossy(&concat(StreamKind::Stdin, &chunks)).into_owned();
    assert!(stdin.contains("echo val-"), "stdin was: {stdin:?}");
    // And it was genuinely evaluated (not just echoed): 6*7 = 42.
    let stdout = String::from_utf8_lossy(&concat(StreamKind::Stdout, &chunks)).into_owned();
    assert!(stdout.contains("val-42"), "stdout was: {stdout:?}");
}

#[tokio::test]
async fn live_events_are_published_in_order() {
    let store = Arc::new(MemoryStore::new());
    let mut run = start_recorded(
        store.clone(),
        SessionId::new(),
        Command::new("printf LIVE", "bash"),
        RunSpec::new("/bin/bash", ["-c", "printf LIVE"]),
    )
    .await
    .expect("start");
    let mut rx = run.take_primary().expect("primary available once");

    let mut saw_started = false;
    let mut output = Vec::new();
    let mut final_status = None;
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Ok(ExecutionEvent::Started(rec))) => {
                assert_eq!(rec.status, ExecutionStatus::Running);
                saw_started = true;
            }
            Ok(Ok(ExecutionEvent::Chunk(c))) if c.kind == StreamKind::Stdout => {
                output.extend_from_slice(&c.data);
            }
            Ok(Ok(ExecutionEvent::Chunk(_))) => {}
            Ok(Ok(ExecutionEvent::Status(s))) => {
                final_status = Some(s);
                break;
            }
            Ok(Err(_)) | Err(_) => break,
        }
    }

    assert!(saw_started, "did not observe Started");
    assert!(String::from_utf8_lossy(&output).contains("LIVE"));
    assert_eq!(final_status, Some(ExecutionStatus::Completed { code: 0 }));

    run.done.await.expect("join").expect("run result");
}

#[tokio::test]
async fn interrupt_is_recorded() {
    let store = Arc::new(MemoryStore::new());
    let run = start_recorded(
        store.clone(),
        SessionId::new(),
        Command::new("sleep 30", "bash"),
        RunSpec::new("/bin/bash", ["-c", "sleep 30"]),
    )
    .await
    .expect("start");
    let exec = run.execution();

    tokio::time::sleep(Duration::from_millis(400)).await;
    run.interrupt().await.unwrap();

    let status = tokio::time::timeout(Duration::from_secs(5), run.done)
        .await
        .expect("did not finish after interrupt")
        .expect("join")
        .expect("run result");
    assert_eq!(status, ExecutionStatus::Interrupted { signal: Some(2) });

    let rec = store.get_execution(exec).await.unwrap().unwrap();
    assert_eq!(rec.status, ExecutionStatus::Interrupted { signal: Some(2) });
}
