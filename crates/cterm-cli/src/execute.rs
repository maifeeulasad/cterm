//! `cterm execute` — a thin CLI over the daemon's gRPC `Terminal.Run`.
//!
//! The daemon's `Terminal` service is the agent-facing surface for the whole
//! system, but reaching it today means wiring up `tonic`/`grpcurl`/`protoc`.
//! That's a lot to ask of a tool-calling client that just wants to run a
//! command and get back its output. `cterm execute` is the regular-shell-shaped
//! front door: the agent spawns `cterm execute …`, reads stdout/stderr, and
//! exits on the recorded exit code. Everything still flows through the daemon
//! and is recorded against a session — this binary just hides the wire.
//!
//! Example:
//!
//! ```text
//! cterm execute --id <session-id> 'echo hello'
//! ```

use std::io::{self, Write};

use cterm_proto::v1;
use cterm_proto::TerminalClient;
use tonic::metadata::MetadataValue;

use crate::auth::{self, SystemKeyring};
use crate::error::{CliError, CliResult};

fn msg(s: impl Into<String>) -> CliError {
    CliError::Msg(s.into())
}

const DEFAULT_ADDR: &str = "http://127.0.0.1:50051";

/// Parsed `cterm execute` flags.
struct ExecuteArgs {
    addr: String,
    session: String,
    program: String,
    args: Vec<String>,
    cwd: String,
    rows: u32,
    cols: u32,
    json: bool,
    quiet: bool,
    command: String,
    /// Session title, applied only when opening a new session (no --id).
    title: String,
    /// Session labels, applied only when opening a new session (no --id).
    labels: std::collections::HashMap<String, String>,
}

/// Entry point used by `main.rs`.
pub fn run(args: &[String]) -> i32 {
    let parsed = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("cterm execute: {e}");
            eprintln!();
            eprintln!("{}", usage());
            return 2;
        }
    };

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cterm execute: tokio runtime: {e}");
            return 1;
        }
    };

    match rt.block_on(execute(parsed)) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("cterm execute: {e}");
            1
        }
    }
}

fn parse(args: &[String]) -> CliResult<ExecuteArgs> {
    let mut addr = std::env::var("CTERM_ADDR")
        .ok()
        .map(|a| {
            if a.starts_with("http://") || a.starts_with("https://") {
                a
            } else {
                format!("http://{a}")
            }
        })
        .unwrap_or_else(|| DEFAULT_ADDR.to_owned());
    let mut session = String::new();
    let mut program = String::new();
    let mut argv: Vec<String> = Vec::new();
    let mut cwd = String::new();
    let mut rows: u32 = 0;
    let mut cols: u32 = 0;
    let mut json = false;
    let mut quiet = false;
    let mut command: Option<String> = None;
    let mut title = String::new();
    let mut labels: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--addr" => addr = take_value("--addr", &mut it)?,
            "--session" | "--id" => session = take_value("--session", &mut it)?,
            "--program" => program = take_value("--program", &mut it)?,
            "--arg" => argv.push(take_value("--arg", &mut it)?),
            "--cwd" => cwd = take_value("--cwd", &mut it)?,
            "--rows" => rows = parse_u32("--rows", &take_value("--rows", &mut it)?)?,
            "--cols" => cols = parse_u32("--cols", &take_value("--cols", &mut it)?)?,
            "--title" => title = take_value("--title", &mut it)?,
            "--label" => {
                let raw = take_value("--label", &mut it)?;
                let (k, v) = raw
                    .split_once('=')
                    .ok_or_else(|| msg(format!("--label expects KEY=VALUE, got {raw:?}")))?;
                labels.insert(k.to_owned(), v.to_owned());
            }
            "--json" => json = true,
            "--quiet" => quiet = true,
            "-h" | "--help" => {
                print!("{}", usage());
                std::process::exit(0);
            }
            "--" => {
                // Rest is the command; join with spaces so a caller who
                // forgot to quote still works for the simple cases.
                let rest: Vec<String> = it.by_ref().cloned().collect();
                if !rest.is_empty() {
                    command = Some(rest.join(" "));
                }
            }
            other if other.starts_with("--") => {
                return Err(msg(format!("unknown flag '{other}'")));
            }
            other => {
                if command.is_some() {
                    return Err(msg(
                        "only one command may be given (quote it as a single string)",
                    ));
                }
                command = Some(other.to_owned());
            }
        }
    }

    let command = command.ok_or_else(|| msg("missing command"))?;

    // --title / --label are only meaningful when opening a new session.
    // Error loudly if the caller passed both --id and metadata so they
    // don't silently assume the daemon applied it.
    if !session.is_empty() && (!title.is_empty() || !labels.is_empty()) {
        return Err(msg(
            "--title/--label only apply when opening a new session (no --id)",
        ));
    }

    Ok(ExecuteArgs {
        addr,
        session,
        program,
        args: argv,
        cwd,
        rows,
        cols,
        json,
        quiet,
        command,
        title,
        labels,
    })
}

fn take_value(flag: &str, it: &mut std::slice::Iter<'_, String>) -> CliResult<String> {
    it.next()
        .cloned()
        .ok_or_else(|| msg(format!("{flag} needs a value")))
}

fn parse_u32(flag: &str, raw: &str) -> CliResult<u32> {
    raw.parse::<u32>()
        .map_err(|_| msg(format!("{flag}: not a non-negative integer: {raw}")))
}

async fn execute(args: ExecuteArgs) -> CliResult<i32> {
    let channel = tonic::transport::Endpoint::from_shared(args.addr.clone())
        .map_err(|e| msg(format!("addr {}: {e}", args.addr)))?
        .connect()
        .await
        .map_err(|e| msg(format!("connect {}: {e}", args.addr)))?;

    // If the operator has set up auth (env or keyring), attach a bearer
    // header on every request via an interceptor. With no token configured,
    // the interceptor is a no-op and the client stays identical to pre-0.6.0.
    let header: Option<MetadataValue<_>> = match auth::resolve_token(&SystemKeyring) {
        Some(t) => Some(
            format!("Bearer {t}")
                .parse()
                .map_err(|e| msg(format!("bearer token is not a valid header: {e}")))?,
        ),
        None => None,
    };
    let mut client =
        TerminalClient::with_interceptor(channel, move |mut req: tonic::Request<()>| {
            if let Some(h) = header.clone() {
                req.metadata_mut().insert("authorization", h);
            }
            Ok(req)
        });

    let request = v1::RunRequest {
        session: args.session.clone(),
        command: args.command.clone(),
        program: args.program.clone(),
        args: args.args.clone(),
        cwd: args.cwd.clone(),
        rows: args.rows,
        cols: args.cols,
        title: args.title.clone(),
        labels: args.labels.clone(),
    };

    let mut stream = client
        .run(request)
        .await
        .map_err(|e| msg(format!("Run RPC: {e}")))?
        .into_inner();

    let stdout = io::stdout();
    let stderr = io::stderr();

    let mut last_status: Option<v1::Status> = None;

    while let Some(event) = stream
        .message()
        .await
        .map_err(|e| msg(format!("stream: {e}")))?
    {
        let Some(ev) = event.event else { continue };
        match ev {
            v1::execution_event::Event::Started(record) => {
                if args.json {
                    emit_json(
                        "started",
                        &serde_json::json!({
                            "execution": record.id,
                            "session": record.session,
                        }),
                    );
                } else if !args.quiet {
                    eprintln!(
                        "cterm: started execution {} in session {}",
                        record.id, record.session
                    );
                }
            }
            v1::execution_event::Event::Chunk(chunk) => {
                if args.quiet {
                    continue;
                }
                if args.json {
                    emit_json(
                        "chunk",
                        &serde_json::json!({
                            "kind": stream_kind_name(chunk.kind),
                            "seq": chunk.seq,
                            "at": chunk.at,
                            "data_b64": base64_encode(&chunk.data),
                        }),
                    );
                } else {
                    match v1::StreamKind::try_from(chunk.kind)
                        .unwrap_or(v1::StreamKind::Unspecified)
                    {
                        v1::StreamKind::Stderr => {
                            let mut h = stderr.lock();
                            let _ = h.write_all(&chunk.data);
                            let _ = h.flush();
                        }
                        _ => {
                            let mut h = stdout.lock();
                            let _ = h.write_all(&chunk.data);
                            let _ = h.flush();
                        }
                    }
                }
            }
            v1::execution_event::Event::Status(status) => {
                last_status = Some(status);
            }
        }
    }

    let Some(status) = last_status else {
        return Err(msg("stream ended with no terminal status"));
    };

    if args.json {
        emit_json("status", &status_json(&status));
    } else if !args.quiet {
        eprintln!("cterm: {}", status_human(&status));
    }

    Ok(exit_code_for(&status))
}

fn stream_kind_name(k: i32) -> &'static str {
    match v1::StreamKind::try_from(k).unwrap_or(v1::StreamKind::Unspecified) {
        v1::StreamKind::Stdin => "stdin",
        v1::StreamKind::Stdout => "stdout",
        v1::StreamKind::Stderr => "stderr",
        v1::StreamKind::Unspecified => "unspecified",
    }
}

fn status_json(s: &v1::Status) -> serde_json::Value {
    let kind = match v1::status::Kind::try_from(s.kind).unwrap_or(v1::status::Kind::Pending) {
        v1::status::Kind::Pending => "pending",
        v1::status::Kind::Running => "running",
        v1::status::Kind::Completed => "completed",
        v1::status::Kind::Interrupted => "interrupted",
        v1::status::Kind::Failed => "failed",
    };
    let mut o = serde_json::json!({ "kind": kind });
    let obj = o.as_object_mut().unwrap();
    if kind == "completed" {
        obj.insert("code".into(), s.code.into());
    }
    if kind == "interrupted" && s.has_signal {
        obj.insert("signal".into(), s.signal.into());
    }
    o
}

fn status_human(s: &v1::Status) -> String {
    match v1::status::Kind::try_from(s.kind).unwrap_or(v1::status::Kind::Pending) {
        v1::status::Kind::Pending => "status pending".into(),
        v1::status::Kind::Running => "status running".into(),
        v1::status::Kind::Completed => format!("exited with code {}", s.code),
        v1::status::Kind::Interrupted => {
            if s.has_signal {
                format!("interrupted by signal {}", s.signal)
            } else {
                "interrupted".into()
            }
        }
        v1::status::Kind::Failed => "failed to run".into(),
    }
}

fn exit_code_for(s: &v1::Status) -> i32 {
    match v1::status::Kind::try_from(s.kind).unwrap_or(v1::status::Kind::Pending) {
        v1::status::Kind::Completed => s.code,
        v1::status::Kind::Interrupted => {
            if s.has_signal {
                128 + s.signal
            } else {
                130
            }
        }
        v1::status::Kind::Failed => 1,
        // Pending/Running shouldn't be the last event; treat as failure if seen.
        _ => 1,
    }
}

fn emit_json(kind: &str, payload: &serde_json::Value) {
    let line = serde_json::json!({ "event": kind, "payload": payload });
    println!("{line}");
}

/// Minimal URL-safe base64 for chunk bytes, no padding. Keeps the output JSON
/// clean when `--json` is set; we don't need `base64` as a dependency for this.
fn base64_encode(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let b = &data[i..i + 3];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(A[((n >> 18) & 0x3f) as usize] as char);
        out.push(A[((n >> 12) & 0x3f) as usize] as char);
        out.push(A[((n >> 6) & 0x3f) as usize] as char);
        out.push(A[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = data.len() - i;
    if rem == 1 {
        let n = u32::from(data[i]) << 16;
        out.push(A[((n >> 18) & 0x3f) as usize] as char);
        out.push(A[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = (u32::from(data[i]) << 16) | (u32::from(data[i + 1]) << 8);
        out.push(A[((n >> 18) & 0x3f) as usize] as char);
        out.push(A[((n >> 12) & 0x3f) as usize] as char);
        out.push(A[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

/// Help text for `cterm execute`.
pub fn usage() -> String {
    "cterm execute — run a command through the daemon's gRPC Terminal.Run.

USAGE:
    cterm execute [FLAGS] <COMMAND>

    <COMMAND> is a single string (quote it). It is passed as the raw command
    (and by default is run under the daemon's default shell via -lc).

FLAGS:
    --id <ID>            Attach to an existing session (alias of --session).
    --session <ID>       Same as --id. Empty opens a new session on the host.
    --addr <URL>         Daemon gRPC URL. Default: $CTERM_ADDR or
                         http://127.0.0.1:50051.
    --program <PATH>     Shell program to use (default on the daemon: /bin/bash).
    --arg <VALUE>        Argument for --program; may be repeated.
                         Default on the daemon: [-lc, <COMMAND>].
    --cwd <PATH>         Working directory on the daemon host.
    --rows <N>           PTY rows (default 24).
    --cols <N>           PTY cols (default 80).
    --title <TEXT>       Title for a new session (only when --id is not set).
                         <= 256 bytes, no newlines or ANSI escapes.
    --label KEY=VALUE    Attach a label to a new session (may be repeated).
                         <= 32 entries, key/value each <= 128 bytes. Only
                         applied when --id is not set.
    --json               Emit one NDJSON event per line on stdout instead of
                         streaming raw output. Chunks are base64 in data_b64.
    --quiet              Suppress chunk output; print only the final status on
                         stderr and exit with the command's code.
    -h, --help           Show this help.

EXIT CODE:
    Mirrors the executed command: the process exit code on Completed,
    128+signal on Interrupted (130 when the signal is unknown), 1 on Failed.
"
    .to_owned()
}
