//! Real-PTY behaviour tests on Windows (ConPTY), via cmd.exe / ping.
#![cfg(windows)]

use std::time::Duration;

use cterm_core::ExecutionStatus;
use cterm_shim::{OutputReceiver, PtySession, Spawn};

async fn drain(mut rx: OutputReceiver) -> Vec<u8> {
    let mut out = Vec::new();
    // Output arrives within ms; a short inter-chunk gap means we're done. On
    // ConPTY the final EOF can lag, so we stop on the gap rather than wait for it.
    while let Ok(Some(chunk)) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
        out.extend_from_slice(&chunk);
    }
    out
}

#[tokio::test]
async fn runs_command_captures_output_and_exit_code() {
    let (session, rx) = PtySession::spawn(Spawn::command(
        "cmd.exe",
        &["/c", "echo WINOUT& exit 7"],
    ))
    .expect("spawn");

    let out = drain(rx).await;
    let status = session.wait().await.expect("wait");

    assert!(
        String::from_utf8_lossy(&out).contains("WINOUT"),
        "output was: {:?}",
        String::from_utf8_lossy(&out)
    );
    assert_eq!(status, ExecutionStatus::Completed { code: 7 });
}

#[tokio::test]
async fn interrupt_stops_a_running_command_and_reports_interrupted() {
    // ping runs ~30s; spawn it directly so killing the child ends it promptly.
    // Keep rx alive but don't drain to EOF: on ConPTY the output pipe doesn't
    // reliably EOF after a kill, so completion is observed via wait(), which
    // reaps the terminated child.
    let (session, _rx) =
        PtySession::spawn(Spawn::command("ping.exe", &["-n", "30", "127.0.0.1"])).expect("spawn");

    tokio::time::sleep(Duration::from_millis(500)).await;
    session.interrupt().await.expect("interrupt");

    let status = tokio::time::timeout(Duration::from_secs(10), session.wait())
        .await
        .expect("wait did not return after interrupt")
        .expect("wait");
    // No POSIX signals on Windows, so signal is None.
    assert_eq!(status, ExecutionStatus::Interrupted { signal: None });
}

#[tokio::test]
async fn pid_is_reported() {
    let (session, rx) =
        PtySession::spawn(Spawn::command("cmd.exe", &["/c", "exit 0"])).expect("spawn");
    assert!(session.pid().is_some());
    let _ = drain(rx).await;
    let _ = session.wait().await;
}
