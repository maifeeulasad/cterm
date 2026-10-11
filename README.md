# cterm

A terminal integration / shim tool. `cterm` sits between a process and its
shell so that command executions can be **streamed, tracked, recorded, and
driven by agents** — without an agent ever needing to hold the credentials used
to open the session.

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

Working end to end on **Linux and Windows** (macOS next). The daemon, gRPC API,
SQLite store, shell shim/wrapper, webview, daemon autostart, and live
**shim→daemon IPC** all build and are tested on both. The IPC makes the hybrid
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

| Crate           | Role                                                        |
| --------------- | ----------------------------------------------------------- |
| `cterm-core`    | Domain model + ports (traits). Runtime-agnostic, no I/O.    |
| `cterm-proto`   | gRPC/protobuf contract (tonic). *(planned)*                 |
| `cterm-storage` | SQLite adapter implementing `RecordStore`. *(planned)*      |
| `cterm-daemon`  | IPC + gRPC servers, session authority. *(planned)*          |
| `cterm-shim`    | The shell shim; PTY supervision + IPC reporting. *(planned)*|
| `cterm-cli`     | Operator CLI: install/remove shim, status, tail. *(planned)*|
| `cterm-web`     | Basic webview over recorded executions. *(planned)*         |

### Integration strategy (the shim)

Rather than patch the shell, `cterm` installs an **abstraction layer**: the real
shell is moved aside (e.g. `zsh` → `zsh-original`) and `cterm`'s shim takes its
place on `PATH`. The shim runs `zsh-original` under a PTY and hooks the stream.
This keeps the real shell untouched and reversible.

## Building

Requires a Rust toolchain. On Ubuntu:

```bash
sudo apt install -y build-essential pkg-config protobuf-compiler
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
cargo test
```

On Windows, the GNU toolchain works with a mingw-w64 `gcc` on `PATH` (e.g. from
Strawberry Perl) and a `protoc` binary:

```powershell
rustup-init.exe -y --default-host x86_64-pc-windows-gnu --profile minimal
$env:PROTOC = "C:\path\to\protoc.exe"   # from a protobuf release zip
cargo test
```

Autostart (daemon on login): `cterm service enable` (systemd `--user` on Linux,
a per-user Startup entry on Windows); undo with `cterm service disable`.

## License

GPL-3.0-or-later.
