//! `cterm` — operator CLI and shell shim (multi-call binary).
//!
//! Invoked as `cterm`, it runs subcommands (install/uninstall/status/service,
//! plus --help/--version). Invoked under a wrapped shell's name (e.g. as
//! `~/.cterm/bin/bash`), it acts as that shell's wrapper. Dispatch is on
//! `argv[0]`'s basename.

// `deny` (not `forbid`) so the Windows console module can scope-allow the
// unavoidable SetConsoleMode FFI; everything else stays unsafe-free.
#![deny(unsafe_code)]

mod commands;
mod config;
mod error;
mod layout;
mod service;
#[cfg(windows)]
mod win_console;
mod wrapper;

use std::path::Path;

use layout::Layout;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg0 = Path::new(&args[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("cterm")
        .to_owned();

    // Wrapper mode: invoked under a shell's name (normalized across .exe/case).
    let shell_name = commands::normalize_shell_name(&arg0);
    if commands::is_wrappable(&shell_name) {
        std::process::exit(wrapper::run(&shell_name, &args[1..]));
    }

    std::process::exit(cli(&args[1..]));
}

fn cli(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("install") => cmd_install(&args[1..]),
        Some("uninstall") => cmd_uninstall(),
        Some("status") => cmd_status(),
        Some("service") => cmd_service(&args[1..]),
        Some("-h") | Some("--help") | Some("help") | None => {
            // Help goes to stdout so it can be captured by scripts/agents.
            println!("{}", usage());
            0
        }
        Some("-V") | Some("--version") | Some("version") => {
            println!("cterm {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some(other) => {
            eprintln!("cterm: unknown command '{other}'\n\n{}", usage());
            2
        }
    }
}

fn layout_or_die() -> Layout {
    match Layout::default_home() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cterm: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_install(args: &[String]) -> i32 {
    // Flags: --rc <file> appends the PATH export to a shell rc file.
    let mut shells = Vec::new();
    let mut rc: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--rc" => match it.next() {
                Some(path) => rc = Some(path.clone()),
                None => {
                    eprintln!("cterm: --rc needs a file path");
                    return 2;
                }
            },
            other => shells.push(other.to_owned()),
        }
    }

    let layout = layout_or_die();
    match commands::install(&layout, &shells) {
        Ok(installed) => {
            println!("Installed cterm shims for: {}", installed.shims.join(", "));
            for (name, real) in &installed.config.shells {
                println!("  {name} -> {real}");
            }
            println!("Records: {}", installed.config.db);
            println!();
            if let Some(rc_path) = rc {
                match commands::append_rc(Path::new(&rc_path), &layout) {
                    Ok(true) => {
                        println!("Added PATH entry to {rc_path}. Open a new shell to activate.")
                    }
                    Ok(false) => println!("{rc_path} already contains the cterm PATH entry."),
                    Err(e) => {
                        eprintln!("cterm: could not update {rc_path}: {e}");
                        return 1;
                    }
                }
            } else {
                println!("To activate, add this to your shell rc (or re-run with --rc <file>):");
                println!("  {}", commands::path_export_line(&layout));
            }
            0
        }
        Err(e) => {
            eprintln!("cterm: install failed: {e}");
            1
        }
    }
}

fn cmd_uninstall() -> i32 {
    let layout = layout_or_die();
    match commands::uninstall(&layout) {
        Ok(removed) if removed.is_empty() => {
            println!(
                "Nothing to uninstall (no shims at {}).",
                layout.bin_dir().display()
            );
            0
        }
        Ok(removed) => {
            println!("Removed shims: {}", removed.join(", "));
            println!("Remove the cterm PATH entry from your shell rc if you added one.");
            0
        }
        Err(e) => {
            eprintln!("cterm: uninstall failed: {e}");
            1
        }
    }
}

fn cmd_status() -> i32 {
    let layout = layout_or_die();
    match commands::status(&layout) {
        Ok(st) => {
            println!("cterm install: {}", st.base.display());
            println!(
                "  shims: {}",
                if st.installed_shims.is_empty() {
                    "(none)".to_owned()
                } else {
                    st.installed_shims.join(", ")
                }
            );
            println!("  on PATH: {}", if st.on_path { "yes" } else { "no" });
            match st.config {
                Some(cfg) => {
                    println!("  records: {}", cfg.db);
                    for (name, real) in &cfg.shells {
                        println!("  wrap {name} -> {real}");
                    }
                }
                None => println!("  config: (not installed)"),
            }
            0
        }
        Err(e) => {
            eprintln!("cterm: status failed: {e}");
            1
        }
    }
}

fn cmd_service(args: &[String]) -> i32 {
    let layout = layout_or_die();
    let result = match args.first().map(String::as_str) {
        Some("enable") => service::enable(&layout),
        Some("disable") => service::disable(&layout),
        Some("status") => service::status(&layout),
        _ => {
            eprintln!("usage: cterm service <enable|disable|status>");
            return 2;
        }
    };
    match result {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            0
        }
        Err(e) => {
            eprintln!("cterm: {e}");
            1
        }
    }
}

fn usage() -> String {
    format!(
        "cterm {version} — terminal integration shim.

Record, stream, and drive shell sessions so an agent can observe and control
them through a local daemon, WITHOUT the agent holding SSH keys or credentials.
A human opens a shell; the agent interacts with it over the daemon's gRPC API.

USAGE:
    cterm <command> [args]

COMMANDS:
    install [SHELL...] [--rc <rcfile>]
        Install the reversible PATH shim for the named shells (default: the
        common interactive shells present on this system). --rc <file> appends
        the PATH entry to that shell rc file. Wrappable names on this platform:
        {wrappable}.
    uninstall                     Remove installed shims (records are kept).
    status                        Show install state (shims, PATH, records, shells).
    service <enable|disable|status>
        Daemon autostart for the current user: systemd --user (Linux),
        a launchd LaunchAgent (macOS), or a Startup entry (Windows).
    -h, --help                    Show this help (on stdout).
    -V, --version                 Show the version.

SHELL-SHIM MODE:
    Invoked under a wrapped shell's name (e.g. ~/.cterm/bin/bash), this binary
    runs the real shell under a PTY and records the session. Nested invocations
    pass straight through (via the CTERM_WRAPPED marker) to avoid recursion and
    loopback. Only interactive sessions are recorded by default; set
    CTERM_FORCE_RECORD=1 to also record `-c`/`/c` and piped (non-tty) invocations.

COMPANION BINARIES:
    cterm-daemon  The agent-facing service. Serves a gRPC `Terminal` API on
                  CTERM_ADDR (default 127.0.0.1:50051) and a local shim IPC
                  endpoint on CTERM_SOCK. Persists to CTERM_DB. gRPC methods:
                  Run, StreamExecution, SendInput, Interrupt, ListSessions,
                  ListExecutions, GetExecution (see proto/cterm.proto).
    cterm-web     Read-only web UI over the records on CTERM_WEB_ADDR
                  (default 127.0.0.1:8088), reading CTERM_DB.

ENVIRONMENT:
    CTERM_DB            SQLite records path (daemon/web/shim; default cterm.db).
    CTERM_ADDR          daemon gRPC listen address (default 127.0.0.1:50051).
    CTERM_SOCK          shim<->daemon IPC endpoint (Unix socket path, or a
                        Windows named pipe; default <home>/.cterm/run/daemon.sock).
    CTERM_WEB_ADDR      cterm-web listen address (default 127.0.0.1:8088).
    CTERM_FORCE_RECORD  record non-interactive shim invocations too.

Docs & gRPC schema: https://github.com/maifeeulasad/cterm",
        version = env!("CARGO_PKG_VERSION"),
        wrappable = commands::WRAPPABLE.join(" "),
    )
}
