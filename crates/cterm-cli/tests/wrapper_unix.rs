//! Integration test for wrapper mode: invoking the binary under a shell name
//! runs the real shell, is transparent to the caller, and records the session
//! into the shared store. Unix-only.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use cterm_core::{RecordStore, StreamKind};
use cterm_storage::SqliteStore;

fn temp_dir() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "cterm-wrap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    p
}

#[test]
fn wrapper_runs_real_shell_transparently_and_records() {
    let home = temp_dir();
    let bin = home.join(".cterm/bin");
    fs::create_dir_all(&bin).unwrap();
    let db = home.join(".cterm/cterm.db");

    // Install a shim named `bash` that is a copy of our built binary.
    let shim = bin.join("bash");
    fs::copy(env!("CARGO_BIN_EXE_cterm"), &shim).unwrap();
    let mut perm = fs::metadata(&shim).unwrap().permissions();
    perm.set_mode(0o755);
    fs::set_permissions(&shim, perm).unwrap();

    // Minimal config pointing at the real bash and our db. /bin/bash exists on
    // both Linux and macOS (unlike /usr/bin/bash).
    let config = format!(
        r#"{{"db":"{}","shells":{{"bash":"/bin/bash"}}}}"#,
        db.display()
    );
    fs::write(home.join(".cterm/config.json"), config).unwrap();

    // Invoke the shim as a shell. It must run the real bash transparently.
    // Force recording: `-c` + piped stdin is otherwise treated as a
    // non-interactive pass-through.
    let output = Command::new(&shim)
        .env("HOME", &home)
        .env("CTERM_FORCE_RECORD", "1")
        .env_remove("CTERM_WRAPPED")
        .arg("-c")
        .arg("echo WRAP_OK; exit 0")
        .output()
        .expect("run shim");

    assert!(output.status.success(), "shim exited: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("WRAP_OK"), "stdout was {stdout:?}");

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
            String::from_utf8_lossy(&out).contains("WRAP_OK"),
            "recorded stdout was {:?}",
            String::from_utf8_lossy(&out)
        );
    });

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn wrapper_records_command_string_when_copilot_cli_is_set() {
    let home = temp_dir();
    let bin = home.join(".cterm/bin");
    fs::create_dir_all(&bin).unwrap();
    let db = home.join(".cterm/cterm.db");

    let shim = bin.join("bash");
    fs::copy(env!("CARGO_BIN_EXE_cterm"), &shim).unwrap();
    let mut perm = fs::metadata(&shim).unwrap().permissions();
    perm.set_mode(0o755);
    fs::set_permissions(&shim, perm).unwrap();

    let config = format!(
        r#"{{"db":"{}","shells":{{"bash":"/bin/bash"}}}}"#,
        db.display()
    );
    fs::write(home.join(".cterm/config.json"), config).unwrap();

    let output = Command::new(&shim)
        .env("HOME", &home)
        .env("COPILOT_CLI", "1")
        .env_remove("CTERM_FORCE_RECORD")
        .env_remove("CTERM_WRAPPED")
        .arg("-c")
        .arg("echo COPILOT_WRAP_OK; exit 0")
        .output()
        .expect("run shim");

    assert!(output.status.success(), "shim exited: {:?}", output.status);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("COPILOT_WRAP_OK"),
        "stdout was {:?}",
        String::from_utf8_lossy(&output.stdout)
    );

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
            String::from_utf8_lossy(&out).contains("COPILOT_WRAP_OK"),
            "recorded stdout was {:?}",
            String::from_utf8_lossy(&out)
        );
    });

    let _ = fs::remove_dir_all(&home);
}

/// With the session marker already set (as if invoked from inside a wrapped
/// shell), the shim must pass straight through to the real shell and record
/// nothing — the loopback / infinite-recursion guard.
#[test]
fn nested_invocation_passes_through_without_recording() {
    let home = temp_dir();
    let bin = home.join(".cterm/bin");
    fs::create_dir_all(&bin).unwrap();
    let db = home.join(".cterm/cterm.db");

    let shim = bin.join("bash");
    fs::copy(env!("CARGO_BIN_EXE_cterm"), &shim).unwrap();
    let mut perm = fs::metadata(&shim).unwrap().permissions();
    perm.set_mode(0o755);
    fs::set_permissions(&shim, perm).unwrap();

    let config = format!(
        r#"{{"db":"{}","shells":{{"bash":"/bin/bash"}}}}"#,
        db.display()
    );
    fs::write(home.join(".cterm/config.json"), config).unwrap();

    // CTERM_WRAPPED set => we are "inside" a session already. Even with
    // CTERM_FORCE_RECORD, the recursion guard wins and passes through.
    let output = Command::new(&shim)
        .env("HOME", &home)
        .env("CTERM_WRAPPED", "1")
        .env("CTERM_FORCE_RECORD", "1")
        .arg("-c")
        .arg("echo NESTED; exit 0")
        .output()
        .expect("run shim");

    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("NESTED"),
        "real shell should still run"
    );

    // Nothing was recorded: the db was never even created (NullStore/no run),
    // or if present, has no sessions.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        if db.exists() {
            let store = SqliteStore::open(&db).unwrap();
            assert_eq!(
                store.list_sessions().await.unwrap().len(),
                0,
                "nested invocation must not record a session"
            );
        }
    });

    let _ = fs::remove_dir_all(&home);
}
