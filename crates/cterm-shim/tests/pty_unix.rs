//! Real-PTY behaviour tests. Unix-only: they drive `/bin/bash` under a tty.
#![cfg(unix)]

use std::time::Duration;

use cterm_core::ExecutionStatus;
use cterm_shim::{OutputReceiver, PtySession, Spawn};

/// Drains output to EOF, with an overall safety timeout so a hang fails fast.
async fn drain(mut rx: OutputReceiver) -> Vec<u8> {
    let mut out = Vec::new();
    while let Ok(Some(chunk)) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
        out.extend_from_slice(&chunk);
    }
    out
}

#[tokio::test]
async fn runs_command_captures_output_and_exit_code() {
    let (session, rx) =
        PtySession::spawn(Spawn::command("/bin/bash", &["-c", "printf OUT_MARKER; exit 7"]))
            .expect("spawn");

    let out = drain(rx).await;
    let status = session.wait().await.expect("wait");

    assert!(
        String::from_utf8_lossy(&out).contains("OUT_MARKER"),
        "output was: {:?}",
        String::from_utf8_lossy(&out)
    );
    assert_eq!(status, ExecutionStatus::Completed { code: 7 });
}

#[tokio::test]
async fn interactive_input_is_delivered_and_evaluated() {
    // No args => bash reads commands from its (tty) stdin: an interactive shell.
    let (session, rx) = PtySession::spawn(Spawn::command("/bin/bash", &[])).expect("spawn");

    // Compute something so a literal echo of the input line can't satisfy the
    // assertion by itself — only genuine evaluation yields "val-42".
    session
        .write_input(b"echo val-$((6*7))\n".to_vec())
        .await
        .expect("write echo");
    session.write_input(b"exit\n".to_vec()).await.expect("write exit");

    let out = drain(rx).await;
    let status = session.wait().await.expect("wait");

    assert!(
        String::from_utf8_lossy(&out).contains("val-42"),
        "output was: {:?}",
        String::from_utf8_lossy(&out)
    );
    assert_eq!(status, ExecutionStatus::Completed { code: 0 });
}

#[tokio::test]
async fn interrupt_stops_a_running_command_and_reports_interrupted() {
    let (session, rx) =
        PtySession::spawn(Spawn::command("/bin/bash", &["-c", "sleep 30"])).expect("spawn");

    // Let the sleep actually start before interrupting.
    tokio::time::sleep(Duration::from_millis(400)).await;
    session.interrupt().await.expect("interrupt");

    // Should finish promptly now, well under the 30s sleep.
    let out = tokio::time::timeout(Duration::from_secs(5), drain(rx)).await;
    assert!(out.is_ok(), "command did not stop after interrupt");

    let status = session.wait().await.expect("wait");
    assert_eq!(status, ExecutionStatus::Interrupted { signal: Some(2) });
}

#[tokio::test]
async fn pid_is_reported() {
    let (session, rx) =
        PtySession::spawn(Spawn::command("/bin/bash", &["-c", "true"])).expect("spawn");
    assert!(session.pid().is_some());
    let _ = drain(rx).await;
    let _ = session.wait().await;
}
