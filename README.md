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

> **Downloads:** prebuilt Linux and Windows binaries (with signed build
> provenance) are on the [releases page](https://github.com/maifeeulasad/cterm/releases/latest).
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
