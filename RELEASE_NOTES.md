# cterm v0.4.1

A terminal integration / shim tool that lets agents observe, record, stream, and
drive shell command executions **without ever holding the credentials used to
open the session**.

> Early release. Tri-platform; shared for feedback. Expect rough edges.

## 0.4.1 — tool-callable CLI over gRPC

- **`cterm execute [--id <session>] <COMMAND>`** (alias `exec`) — a regular
  shell-shaped front door to the daemon's `Terminal.Run` RPC (issue #24).
  Tool-calling clients no longer need `tonic`/`grpcurl`/`protoc`: spawn
  `cterm execute …`, read stdout/stderr, exit on the command's own exit code
  (128+signal on interrupt, 1 on failure). Flags mirror the proto
  `RunRequest` (`--session/--id`, `--addr`, `--program`, `--arg`, `--cwd`,
  `--rows`, `--cols`). `--json` emits one NDJSON event per line with
  base64-encoded chunks for machine consumers; `--quiet` suppresses chunk
  output and reports only the final status.
- **proto**: `Session.status` is now a typed enum (`ACTIVE` / `CLOSED` /
  `DROPPED`) on the wire. Previously a free-form string; `Status.Kind` on
  the same service was already an enum, so this removes an inconsistency and
  the risk of typos on either side.
- **docs**: the workspace's own top-of-file layering comment now lists
  `cterm-ipc` alongside the other crates.
- **deps**: lockfile bump of the `pulldown-cmark` build-dep (pulled in
  transitively by `prost-build`) to 0.13.6 so `cargo audit` on CI stops
  flagging RUSTSEC-2026-0433. The advisory concerns `ENABLE_WIKILINKS`,
  which the build never sets.

## 0.4.0 — self-serving commands

`cterm` can now manage its own lifecycle (issue #23):

- **`cterm update`** (alias **`upgrade`**) — downloads the latest release for
  this platform, **verifies it against the published `sha256`** (and refuses to
  install on a mismatch), then replaces the running binary and the sibling
  `cterm-daemon`/`cterm-web` binaries in place. `--check` only reports whether
  an update is available; it's a no-op when you're already current.
- **`cterm link [--system|--user]`** — puts the `cterm`/`cterm-daemon`/
  `cterm-web` binaries next to the current one onto a bin directory. `--user`
  (default) targets `~/.local/bin` (or `%LOCALAPPDATA%\cterm\bin`); `--system`
  targets `/usr/local/bin` (or `%ProgramFiles%\cterm`). Symlinks on Unix, copies
  on Windows. It never edits `PATH` silently — it prints how if the dir isn't on
  it.
- **`cterm uninstall --all` / `--purge`** — `--all` also disables autostart and
  removes the config/runtime/shim directories (recordings kept); `--purge`
  additionally deletes the whole install root **including recordings** and
  requires an explicit `--yes`.
- **`cterm doctor`** — a read-only diagnosis of the whole install: version and
  update availability, the companion binaries, installed shims and `PATH`, the
  records database, daemon reachability, and autostart state.

## 0.3.0 — terminal-emulated Zsh transcripts

- Web transcripts for Zsh now replay the recorded PTY output through xterm.js,
  preserving colors, cursor movement, Powerlevel10k prompts, and other terminal
  control sequences instead of displaying them literally.
- The transcript API exposes ordered raw stream chunks so the webview can
  reproduce terminal output faithfully.
- Interactive input is retained in the record but is not rendered a second time
  when PTY echo has already produced it in the output stream.

## 0.2.3 — stable download URLs + README download badges

- Release assets now have **version-less names** (e.g. `cterm-x86_64-linux.tar.gz`),
  so `…/releases/latest/download/<name>` always points to the newest build.
- README gained per-platform **download badges** (Windows / Linux / macOS
  Apple Silicon / macOS Intel) and a downloads-count badge.

## 0.2.2 — docs & help for agents

- Rewrote `cterm --help` to describe the whole system (the gRPC `Terminal` API,
  the `cterm-daemon`/`cterm-web` companions, env vars, and the per-platform
  wrappable shells), and added `--version`. Help now prints to stdout.
- README: corrected the crate table (all crates are shipped, not planned) and
  the shim description (reversible PATH shim, no shell renaming), and added an
  agent-oriented Usage section. No functional changes.

## 0.2.1 — broader shells, no loopback

- **More shells.** The shim can now wrap a wide set per platform: Linux/macOS —
  `bash zsh sh dash fish ksh tcsh csh ash mksh nu elvish xonsh pwsh`; Windows —
  `cmd powershell pwsh bash sh zsh fish nu ksh`. (The shim is shell-agnostic — it
  just runs the real program under a PTY — so coverage is about which names it
  intercepts.) Default install stays the common interactive shells.
- **Loopback / infinite-recursion guard.** Every PTY-spawned process tree is
  marked (`CTERM_WRAPPED`); a shim invoked anywhere beneath a cterm session
  passes straight through to the real shell instead of wrapping again. This stops
  a wrapped shell's subshells, scripts, and command substitutions from endlessly
  re-wrapping and re-connecting to the daemon.
- **Only interactive sessions are recorded** by default: `-c`/`/c` command
  invocations and piped (non-tty) input pass through unrecorded, cutting noise
  from scripts and tooling. Plus a self-exec guard so the real shell can never
  resolve back to the shim.

## 0.2.0 — tri-platform

cterm now runs on **Linux, macOS, and Windows** — all three are built and tested
in CI, with signed, provenance-backed binaries for each. This minor bump marks
that coverage milestone (adding macOS was a big expansion).

- **macOS:** CLI/shim, daemon (gRPC + Unix-socket IPC), storage, and webview;
  autostart via a **launchd** LaunchAgent; binaries for **Apple Silicon (arm64)**
  and **Intel (x86_64)**.
- Everything from 0.1.x: transparent shell shim, gRPC agent API, live
  shim↔daemon IPC, SQLite records, webview, and daemon autostart (systemd /
  launchd / Startup).

## Earlier

- **v0.1.3 — dependency maintenance:** `portable-pty` 0.9 (drops the unmaintained
  `serial` crate — **RUSTSEC-2017-0008 fully resolved**), `nix` 0.31,
  `windows-sys` 0.61, `terminal_size` 0.4, `codeql-action` v4.
- **v0.1.2 — security:** HTTP stack to **hyper 1.x / tonic 0.14 / axum 0.8 /
  tower 0.5** (`h2` ≥ 0.4.20), resolving **RUSTSEC-2026-0258** (`h2` DoS).

## Highlights

- **Transparent shell shim** (`cterm install`) — a reversible, per-user PATH shim
  wraps your real shell (`bash`/`zsh`, or `cmd`/`powershell` on Windows) under a
  PTY/ConPTY, recording the session. If anything about cterm is unavailable it
  falls back to just starting your shell, so it can't lock you out.
- **gRPC agent API** (`cterm-daemon`) — `Run`, `StreamExecution`, `SendInput`,
  `Interrupt`, and session/execution queries, with live server-streaming.
- **Live shim ↔ daemon IPC** — a human opens an interactive shell via the
  wrapper; an agent observes and drives it in real time over gRPC, never opening
  its own connection. Unix socket on Linux, named pipe on Windows.
- **Durable records** — every session, execution, status, and input/output
  stream is stored in SQLite.
- **Webview** (`cterm-web`) — a small UI to browse recorded sessions and
  transcripts.
- **Daemon autostart** — `cterm service enable` (systemd `--user` on Linux, a
  per-user Startup entry on Windows); undo with `cterm service disable`.

## Install

Binaries: `cterm` (CLI + shim), `cterm-daemon`, `cterm-web`.

Asset names are version-less, so these `latest/download` URLs always fetch the
newest release.

### Linux (x86_64)

```bash
curl -fsSL "https://github.com/maifeeulasad/cterm/releases/latest/download/cterm-x86_64-linux.tar.gz" | tar -xz
sudo install -m 0755 cterm cterm-daemon cterm-web /usr/local/bin/
cterm --help
```

### macOS (Apple Silicon or Intel)

```bash
ARCH=$([ "$(uname -m)" = "arm64" ] && echo aarch64 || echo x86_64)
curl -fsSL "https://github.com/maifeeulasad/cterm/releases/latest/download/cterm-$ARCH-macos.tar.gz" | tar -xz
# Gatekeeper quarantines downloaded binaries; clear it, then install:
xattr -dr com.apple.quarantine cterm cterm-daemon cterm-web 2>/dev/null || true
sudo install -m 0755 cterm cterm-daemon cterm-web /usr/local/bin/
cterm --help
```

### Windows (x86_64, PowerShell)

```powershell
Invoke-WebRequest "https://github.com/maifeeulasad/cterm/releases/latest/download/cterm-x86_64-windows.zip" -OutFile cterm.zip
Expand-Archive cterm.zip -DestinationPath "$env:LOCALAPPDATA\cterm" -Force
# Add "$env:LOCALAPPDATA\cterm" to your PATH, then:
cterm.exe --help
```

## Verify downloads

Checksums (`*.sha256`) are attached to this release.

Each archive also ships with **signed build provenance** (SLSA, generated by
GitHub Actions). Verify that an archive was built by this repo's release
workflow:

```bash
gh attestation verify cterm-x86_64-linux.tar.gz --repo maifeeulasad/cterm
```

## Known limitations

- On Windows, the wrapper's window-resize (SIGWINCH analogue) isn't wired yet.
- Live IPC drives one interactive session per wrapper process.

## License

GPL-3.0-or-later.
