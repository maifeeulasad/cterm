//! `cterm` — operator CLI and shell shim (multi-call binary).
//!
//! Invoked as `cterm`, it runs subcommands (install/uninstall/status/service,
//! plus --help/--version). Invoked under a wrapped shell's name (e.g. as
//! `~/.cterm/bin/bash`), it acts as that shell's wrapper. Dispatch is on
//! `argv[0]`'s basename.

// `deny` (not `forbid`) so the Windows console module can scope-allow the
// unavoidable SetConsoleMode FFI; everything else stays unsafe-free.
#![deny(unsafe_code)]

mod auth;
mod commands;
mod config;
mod error;
mod execute;
mod layout;
mod selfcmd;
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
        Some("uninstall") => cmd_uninstall(&args[1..]),
        Some("status") => cmd_status(),
        Some("service") => cmd_service(&args[1..]),
        Some("doctor") => selfcmd::doctor(&layout_or_die()),
        Some("link") => cmd_link(&args[1..]),
        Some("execute") | Some("exec") => execute::run(&args[1..]),
        Some("auth") => cmd_auth(&args[1..]),
        Some("update") | Some("upgrade") => cmd_update(&args[1..]),
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

fn cmd_uninstall(args: &[String]) -> i32 {
    let (mut all, mut purge, mut yes) = (false, false, false);
    for a in args {
        match a.as_str() {
            "--all" => all = true,
            "--purge" => purge = true,
            "--yes" | "-y" => yes = true,
            other => {
                eprintln!("cterm: uninstall: unknown flag '{other}' (use --all, --purge, --yes)");
                return 2;
            }
        }
    }
    selfcmd::uninstall(&layout_or_die(), all, purge, yes)
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

fn cmd_update(args: &[String]) -> i32 {
    let mut check_only = false;
    for a in args {
        match a.as_str() {
            "--check" => check_only = true,
            other => {
                eprintln!("cterm: update: unknown flag '{other}' (use --check)");
                return 2;
            }
        }
    }
    selfcmd::update(check_only)
}

fn cmd_link(args: &[String]) -> i32 {
    let mut scope = selfcmd::LinkScope::User;
    for a in args {
        match a.as_str() {
            "--system" => scope = selfcmd::LinkScope::System,
            "--user" => scope = selfcmd::LinkScope::User,
            other => {
                eprintln!("cterm: link: unknown flag '{other}' (use --system or --user)");
                return 2;
            }
        }
    }
    selfcmd::link(scope)
}

fn cmd_auth(args: &[String]) -> i32 {
    let sub = args.first().map(String::as_str);
    let rest = if args.is_empty() { &[][..] } else { &args[1..] };
    let store = auth::SystemKeyring;
    match sub {
        Some("enable") => {
            let (mut token, mut print) = (None, false);
            let mut it = rest.iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--token" => {
                        let Some(v) = it.next() else {
                            eprintln!("cterm: auth enable: --token needs a value");
                            return 2;
                        };
                        token = Some(v.clone());
                    }
                    "--print" => print = true,
                    other => {
                        eprintln!("cterm: auth enable: unknown flag '{other}'");
                        return 2;
                    }
                }
            }
            match auth::enable(&store, token, print) {
                Ok(t) => {
                    if !print {
                        println!(
                            "cterm auth: stored token in {} keyring (fingerprint: {})",
                            auth::SERVICE,
                            &t[..8.min(t.len())]
                        );
                    }
                    println!("Restart cterm-daemon with CTERM_AUTH=on to enforce it.");
                    0
                }
                Err(e) => {
                    eprintln!("cterm: {e}");
                    1
                }
            }
        }
        Some("disable") => match auth::disable(&store) {
            Ok(()) => {
                println!("cterm auth: cleared token from keyring");
                println!("Restart cterm-daemon (without CTERM_AUTH=on and without CTERM_AUTH_TOKEN) to drop back to open mode.");
                0
            }
            Err(e) => {
                eprintln!("cterm: {e}");
                1
            }
        },
        Some("rotate") => {
            let print = rest.iter().any(|a| a == "--print");
            match auth::rotate(&store, print) {
                Ok(t) => {
                    if !print {
                        println!(
                            "cterm auth: rotated token (fingerprint: {})",
                            &t[..8.min(t.len())]
                        );
                    }
                    println!("Restart cterm-daemon so the new token takes effect.");
                    0
                }
                Err(e) => {
                    eprintln!("cterm: {e}");
                    1
                }
            }
        }
        Some("status") => {
            let st = auth::status(&store);
            if st.has_token {
                let source = match st.source {
                    Some(auth::AuthSource::Env) => "env (CTERM_AUTH_TOKEN)",
                    Some(auth::AuthSource::Keyring) => "keyring",
                    None => "none",
                };
                println!(
                    "cterm auth: token present (source: {source}, fingerprint: {})",
                    st.fingerprint.unwrap_or_default()
                );
            } else {
                println!("cterm auth: no token (client will connect unauthenticated)");
            }
            0
        }
        _ => {
            eprintln!("usage: cterm auth <enable|disable|rotate|status> [--token <t>] [--print]");
            2
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
    uninstall [--all] [--purge [--yes]]
        Remove installed shims. --all also disables autostart and removes the
        config/runtime/shim dirs (recordings kept). --purge additionally deletes
        the whole install root including recordings, and requires --yes.
    status                        Show install state (shims, PATH, records, shells).
    service <enable|disable|status>
        Daemon autostart for the current user: systemd --user (Linux),
        a launchd LaunchAgent (macOS), or a Startup entry (Windows).
    update [--check]  (alias: upgrade)
        Download the latest release for this platform, verify it against the
        published sha256, and replace this binary (and the sibling daemon/web
        binaries) in place. --check only reports whether an update is available.
    link [--system|--user]
        Put the cterm/cterm-daemon/cterm-web binaries next to this one onto a
        bin dir (--user: ~/.local/bin or %LOCALAPPDATA%\\cterm\\bin [default];
        --system: /usr/local/bin or %ProgramFiles%\\cterm). Symlinks on Unix,
        copies on Windows. Never edits PATH; prints how if the dir isn't on it.
    execute [--id <session>] [--addr <url>] [--json|--quiet] <COMMAND>
        (alias: exec) Run <COMMAND> on the daemon over gRPC and stream the
        output. Exits with the command's own exit code. Agent-friendly front
        door over the Terminal.Run RPC — no tonic/grpcurl/protoc needed on
        the client side. `cterm execute --help` for all flags.
    auth <enable|disable|rotate|status> [--token <t>] [--print]
        Opt-in bearer-token auth for the daemon's gRPC. Tokens live in the
        OS keyring (macOS Keychain / Windows Credential Manager / Linux
        Secret Service). Env var CTERM_AUTH_TOKEN overrides the keyring on
        both daemon and client. Daemon must be restarted with CTERM_AUTH=on
        (or CTERM_AUTH_TOKEN set) for enforcement to kick in.
    doctor                        Diagnose the install (version/update, binaries,
                                  shims, PATH, records db, daemon, autostart).
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
