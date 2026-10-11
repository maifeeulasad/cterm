//! Integration test for wrapper mode on Windows: invoking the binary as a
//! shell runs the real shell (cmd.exe) under ConPTY, transparently, and records
//! the session into the shared store. Exercised entirely in a temp dir — it
//! never touches the real PATH, registry, or startup.
#![cfg(windows)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use cterm_core::{RecordStore, StreamKind};
use cterm_storage::SqliteStore;

fn temp_dir() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "cterm-wrapwin-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    p
}

#[test]
fn wrapper_runs_real_cmd_transparently_and_records() {
    let home = temp_dir();
    let bin = home.join(".cterm").join("bin");
    fs::create_dir_all(&bin).unwrap();
    let db = home.join(".cterm").join("cterm.db");

    // Install a shim named cmd.exe that is a copy of our built binary.
    let shim = bin.join("cmd.exe");
    fs::copy(env!("CARGO_BIN_EXE_cterm"), &shim).unwrap();

    // Config points the wrapped `cmd` at the real cmd.exe (from ComSpec).
    let real_cmd =
        std::env::var("ComSpec").unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".to_owned());
    let cfg = serde_json::json!({
        "db": db.to_string_lossy(),
        "shells": { "cmd": real_cmd },
    });
    fs::write(
        home.join(".cterm").join("config.json"),
        serde_json::to_vec(&cfg).unwrap(),
    )
    .unwrap();

    // Invoke the shim as cmd. HOME steers Layout::default_home to our temp dir.
    // Force recording: `/c` + piped stdin is otherwise a non-interactive
    // pass-through.
    let output = Command::new(&shim)
        .env("HOME", &home)
        .env("CTERM_FORCE_RECORD", "1")
        .env_remove("CTERM_WRAPPED")
        .args(["/c", "echo WRAP_WIN & exit 0"])
        .output()
        .expect("run shim");

    assert!(output.status.success(), "shim exited: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("WRAP_WIN"), "stdout was {stdout:?}");

    // The session was recorded into the shared store.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let store = SqliteStore::open(&db).unwrap();
        let sessions = store.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1, "expected one recorded session");

        let execs = store.list_executions(sessions[0].id).await.unwrap();
        assert_eq!(execs.len(), 1, "expected one execution");

        let chunks = store.load_chunks(execs[0].id).await.unwrap();
        let out: Vec<u8> = chunks
            .iter()
            .filter(|c| c.kind == StreamKind::Stdout)
            .flat_map(|c| c.data.clone())
            .collect();
        assert!(
            String::from_utf8_lossy(&out).contains("WRAP_WIN"),
            "recorded stdout was {:?}",
            String::from_utf8_lossy(&out)
        );
    });

    let _ = fs::remove_dir_all(&home);
}
