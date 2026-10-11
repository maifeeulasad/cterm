# cterm

[![CI](https://github.com/maifeeulasad/cterm/actions/workflows/ci.yml/badge.svg)](https://github.com/maifeeulasad/cterm/actions/workflows/ci.yml)
[![CodeQL](https://github.com/maifeeulasad/cterm/actions/workflows/codeql.yml/badge.svg)](https://github.com/maifeeulasad/cterm/actions/workflows/codeql.yml)
[![Security audit](https://github.com/maifeeulasad/cterm/actions/workflows/audit.yml/badge.svg)](https://github.com/maifeeulasad/cterm/actions/workflows/audit.yml)
[![Release](https://img.shields.io/github/v/release/maifeeulasad/cterm?sort=semver)](https://github.com/maifeeulasad/cterm/releases/latest)
[![License](https://img.shields.io/github/license/maifeeulasad/cterm)](LICENSE)

A terminal integration / shim tool. `cterm` sits between a process and its
shell so that command executions can be **streamed, tracked, recorded, and
driven by agents** — without an agent ever needing to hold the credentials used
to open the session.

> **Downloads:** prebuilt **Linux, macOS (Apple Silicon + Intel), and Windows**
> binaries (with signed build provenance) are on the
> [releases page](https://github.com/maifeeulasad/cterm/releases/latest).
> See the release notes for install commands.

## Why

Agents (e.g. Claude) are forbidden from SSHing into production. A human,
though, can. `cterm` turns that into a *hybrid* workflow:

- A human opens the session on the host (local or prod).
- A shim masquerades as the shell, runs the real shell under a PTY, and streams
  everything to a local daemon.
- The daemon records executions (command, input, output, status, timing) and
  exposes a gRPC streaming API.
- An agent observes the session and, where policy permits, drives it — through
  the daemon, never by SSHing in itself.

## Status

Working end to end on **Linux, macOS, and Windows**. The daemon, gRPC API,
SQLite store, shell shim/wrapper, webview, daemon autostart (systemd on Linux,
launchd on macOS, a Startup entry on Windows), and live **shim→daemon IPC** all
build and are tested on all three in CI. The IPC makes the hybrid
flow live: a human opens an interactive shell through the wrapper, and an agent
observes and drives it in real time over gRPC — never opening its own connection.
Transport is a Unix socket on Linux and a named pipe (`\\.\pipe\cterm-daemon`) on
Windows.

## Architecture

Ports-and-adapters. Dependencies point inward toward `cterm-core`.

```
real shell (zsh/bash)
     ▲  PTY
     │
cterm-shim ──IPC──▶ cterm-daemon ──persist──▶ RecordStore (SQLite)
(masquerades as                 │
 the shell)                     └──gRPC(stream)──▶ agents (Claude), webview
```

| Crate           | Role                                                              |
| --------------- | ----------------------------------------------------------------- |
| `cterm-core`    | Domain model + ports (traits). Runtime-agnostic, no I/O.          |
| `cterm-proto`   | gRPC/protobuf contract (tonic). The agent-facing API.             |
| `cterm-ipc`     | shim↔daemon IPC protocol + length-delimited framing.              |
| `cterm-storage` | `RecordStore` adapters: SQLite (durable), in-memory, null.        |
| `cterm-shim`    | PTY supervision + the recording runner.                           |
| `cterm-daemon`  | IPC + gRPC servers, session/run registry (the session authority). |
| `cterm-cli`     | The `cterm` binary: operator CLI **and** the shell shim/wrapper.  |
| `cterm-web`     | Web UI + JSON API over recorded executions.                       |

### Integration strategy (the shim)

Rather than patch or rename the system shell, `cterm` installs a reversible
**PATH shim**: `cterm install` puts a per-user directory (`~/.cterm/bin`) early
on `PATH` containing copies of the `cterm` binary named after each wrapped shell
(e.g. `bash`, `zsh`). Invoked under a shell's name, the binary looks up the
**real** shell's absolute path (recorded at install time, so there's never a
self-exec loop), runs it under a PTY, and records the session. Uninstalling just
removes that directory. A `CTERM_WRAPPED` marker on the spawned process tree
makes nested shells pass straight through, preventing recursion/loopback.

## Usage

Three binaries: **`cterm`** (CLI + shell shim), **`cterm-daemon`** (the
agent-facing service), **`cterm-web`** (records browser). Run `cterm --help` for
the full reference.

**For an agent** — the daemon is the interface; the agent never opens its own
shell/SSH:

```bash
# 1. Start the daemon (records to CTERM_DB, serves gRPC on CTERM_ADDR).
CTERM_DB=~/.cterm/cterm.db CTERM_ADDR=127.0.0.1:50051 cterm-daemon &

# 2. Drive it over the gRPC `cterm.v1.Terminal` service (see proto/cterm.proto):
#      Run(command)            -> server-stream of ExecutionEvent (Started/Chunk/Status)
#      StreamExecution(id)     -> live tail or replay of a recorded execution
#      SendInput(id, bytes)    -> write to a running execution's stdin
#      Interrupt(id)           -> Ctrl-C a running execution
#      ListSessions / ListExecutions(session) / GetExecution(id)  -> query records
```

**To record a human's interactive shells** (the hybrid flow), install the PATH
shim and put it on `PATH`:

```bash
cterm install              # wraps common interactive shells found on the system
cterm status               # show what's wrapped and whether it's on PATH
# add ~/.cterm/bin to PATH (or: cterm install --rc ~/.bashrc)
```

A human opens a shell (now wrapped); the agent observes/drives it live over the
same gRPC API. Browse records with `cterm-web` (`http://127.0.0.1:8088`).

**Key environment variables:** `CTERM_DB` (records path), `CTERM_ADDR` (daemon
gRPC addr), `CTERM_SOCK` (shim↔daemon IPC socket/pipe), `CTERM_WEB_ADDR`
(cterm-web addr), `CTERM_FORCE_RECORD` (record non-interactive shim calls too).

## Building

Requires a Rust toolchain and `protoc`. On Ubuntu:

```bash
sudo apt install -y build-essential pkg-config protobuf-compiler
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
cargo test
```

macOS needs only `protoc` (clang ships with Xcode CLT). On Windows, CI uses the
MSVC toolchain; a `protoc` binary must be on `PATH` or in `$env:PROTOC`. (The GNU
toolchain also works if a mingw-w64 `gcc`, e.g. from Strawberry Perl, is on
`PATH`.)

```powershell
$env:PROTOC = "C:\path\to\protoc.exe"   # from a protobuf release zip
cargo test
```

Autostart (daemon on login): `cterm service enable` — systemd `--user` on Linux,
a launchd LaunchAgent on macOS, a per-user Startup entry on Windows; undo with
`cterm service disable`.

## Acknowledgements

The Windows host-console raw mode (`cterm-cli`'s `win_console`) adapts the
console-mode flag choices from **[zhiburt/conpty](https://github.com/zhiburt/conpty)**
(MIT, Copyright (c) 2021 Maxim Zhiburt) — its `console.rs` raw-stdin setup and
`process.rs` VT-output-processing setup. We call the Win32 console API directly
rather than vendoring the code. See that module's docs for details, including
the limitations noted in the upstream library.

Terminal PTYs use [portable-pty](https://github.com/wez/wezterm) (wezterm).
Investigation note: ConPTY itself is **not** a bottleneck for the Windows
limitations we hit — both portable-pty and zhiburt/conpty use the same OS
`CreatePseudoConsole`; the issue was configuring the *host* console, which the
`win_console` module now does.

## License

GPL-3.0-or-later.
