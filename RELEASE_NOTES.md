# cterm v0.6.1

A terminal integration / shim tool that lets agents observe, record, stream, and
drive shell command executions **without ever holding the credentials used to
open the session**.

> Early release. Tri-platform; shared for feedback. Expect rough edges.

## 0.6.1 — Claude plugin + docs refresh for 0.6.0 features

Docs-only patch. No daemon, CLI, or wire-format changes. Bumps the plugin
manifest too so the Anthropic directory re-scans the plugin and users on
0.6.0 pick up the updated skill + command content.

- **Claude plugin** (`plugin/`) now teaches Claude about 0.6.0:
  - `commands/exec.md` advertises `--title` / `--label K=V` passthrough on
    new sessions, with the open-time-only constraint.
  - `commands/sessions.md` mentions that titles/labels surface in
    `cterm-web` and `ListSessions`.
  - `skills/cterm/SKILL.md` gets a "Naming a session" section (which
    keys to reach for, which to avoid), and a "When the daemon rejects
    the call" section covering `UNAUTHENTICATED` (auth on, no token) and
    `InvalidArgument` (metadata rejected).
- **Install/usage docs** (`docs/claude-plugin/`) catch up: an "Optional:
  opt-in daemon auth" section in `installing.md`, verifying-the-install
  error cases split between connect failures and `UNAUTHENTICATED`,
  usage.md documents the new flags and the four things the skill does.
- **`docs/auth.md`**: tense fix now that 0.6.0 has shipped — makes clear
  that the default is still open (backward compat) and auth is opt-in.

## 0.6.0 — opt-in auth + session metadata

Two features ship together:

### Opt-in bearer-token auth for the daemon

**Security.** The gRPC `Terminal` service can now require a bearer
token on every RPC ([#27](https://github.com/maifeeulasad/cterm/issues/27)).
Default behavior is unchanged — single-user developer machines keep the
open-on-loopback model — but multi-user hosts, containers, CI runners, and
defense-in-depth setups can flip it on with two commands:

```bash
cterm auth enable            # stores a fresh token in the OS keyring
CTERM_AUTH=on cterm-daemon   # enforces it on every RPC
```

Full flow, token precedence, rotation, and the `CTERM_AUTH_TOKEN` escape
hatch for headless hosts are in [docs/auth.md](docs/auth.md).

### What's new

- **`cterm-core::AuthVerifier`** — a port the daemon calls on every RPC
  without knowing how the credential was issued or stored. `NoopVerifier`
  is the default and preserves today's behavior.
- **`cterm-daemon::TokenVerifier`** — constant-time bearer-token compare
  (byte-level fold so neither length nor prefix match leak via timing).
- **`cterm-daemon::AuthInterceptor`** — tonic interceptor that pulls
  `authorization: Bearer <t>` from metadata; wired via
  `InterceptedService` so no service method can skip the check.
- **`cterm-daemon::verifier_from_env`** — the env-driven factory:
  `CTERM_AUTH_TOKEN=<secret>` (first), `CTERM_AUTH=on` + keyring token
  (second), then `NoopVerifier`. The daemon startup banner now prints
  `auth=on` / `auth=off` so operators can see the state at a glance.
- **`cterm-cli::auth::KeyStore`** — trait abstracted over the OS keyring.
  The shipped `SystemKeyring` impl uses the
  [`keyring`](https://crates.io/crates/keyring) 3.x crate (macOS
  Keychain, Windows Credential Manager, Linux Secret Service). Tokens
  live under `(service = "cterm", user = "daemon-token")`.
- **`cterm auth` subcommand group** — `enable`, `disable`, `rotate`,
  `status`. `enable --print` / `rotate --print` echo the token once so
  operators on keyring-less hosts can paste it into `CTERM_AUTH_TOKEN`.
  `status` reports only a short fingerprint (first 8 chars), never the
  secret.
- **`cterm execute`** now attaches the bearer header on every RPC when a
  token is available (env > keyring > none); unchanged when nothing is
  configured.

### What's not covered

Per-client identities, audit of who presented the token, TLS/mTLS, and
hardware-backed credentials are all explicitly out of scope — the
expected deployment is loopback-only. If you need any of these, open an
issue on [#27](https://github.com/maifeeulasad/cterm/issues/27).

### Session titles and labels

Sessions can now carry an optional one-line `title` and a small
`BTreeMap<String, String>` of `labels`
([#28](https://github.com/maifeeulasad/cterm/issues/28)). Attach them at
open time; they show up in `ListSessions` and in the `cterm-web` session
list, and do not affect scheduling, storage identity, or how the shell
runs.

```bash
cterm execute --title "deploy webapp" --label env=staging --label ticket=ABC-123 \
  'kubectl apply -f manifest.yaml'
```

- **Validation is strict at the daemon boundary**: title ≤ 256 bytes, no
  newlines/ANSI; ≤ 32 labels, key/value each ≤ 128 bytes, no control
  characters in either, `=` banned in keys. Violations land as a clear
  `InvalidArgument` rather than silent truncation.
- **Metadata is set at open time only** in this release. A dedicated
  `UpdateSessionMetadata` RPC is a follow-up.
- **No schema migration, no wire break**: new proto3 fields default
  empty; `Session` in `cterm-core` carries `#[serde(default,
  skip_serializing_if = "...is_empty")]` on both fields so pre-0.6
  session blobs deserialize unchanged and fresh sessions without
  metadata serialize byte-for-byte as before.

Full flow, limits, and non-goals in
[docs/session-metadata.md](docs/session-metadata.md).

### Compatibility

No wire-format or Cargo-API breaking changes. Existing clients continue
to work against a default (open) daemon, and an existing open daemon
continues to work with upgraded clients. The new `terminal_server_with_auth`
factory is additive; the previous `terminal_server` factory still exists
for callers (and tests) that explicitly want no auth. All new proto
fields (`Session.title`, `Session.labels`, `RunRequest.title`,
`RunRequest.labels`) are proto3-default-empty, so older clients and
older daemons interoperate with upgraded peers in either direction.

### Web UI hardening

While adding title/labels rendering to `cterm-web`, every session-row
field is now HTML-escaped through a new `esc()` helper — including
fields that were previously interpolated raw. Loopback-only service,
but worth doing once rather than leaving the sharp edge.

## 0.5.2 — plugin listing: privacy policy + terms of service

Docs-only patch so the Claude directory listing can surface a policy pair
([#21](https://github.com/maifeeulasad/cterm/issues/21)). No daemon,
CLI, or wire-format changes.

- **`PRIVACY.md`** — explicitly local-first: no server behind cterm, no
  analytics, no telemetry, no crash reporting. The daemon writes its
  SQLite file on the user's disk and listens on `127.0.0.1` by default;
  nothing phones home beyond the GitHub Releases fetch that backs
  `cterm update`.
- **`TERMS.md`** — the GPL-3.0's own warranty and liability disclaimers
  plus a short "you control the host, you're responsible for the
  commands, don't use this to break the law" section. The plugin
  inherits both documents.
- **`plugin.json`** — now carries `privacyPolicyUrl` and
  `termsOfServiceUrl` pointing at those files on GitHub, so the
  directory listing can link them.

## 0.5.1 — directory-review fixes for the Claude plugin

Patch release to unblock the Anthropic plugin-directory submission started
in 0.5.0 ([#21](https://github.com/maifeeulasad/cterm/issues/21)). No
daemon, CLI, or wire-format changes.

- **Drop `allowed-tools` frontmatter from the three slash commands.** The
  directory refused the 0.5.0 submission with: *"`allowed-tools` entries
  beyond Read/Grep/Glob/LS accepted only from Verified organizations. If
  your organization isn't Verified, remove them from the plugin."* Our
  org isn't Verified. Removing the frontmatter means Claude Code now
  prompts the user the first time each `/cterm:*` command calls `cterm`
  — a one-time approval, same as any other Bash call.
- **Quote the `description` / `argument-hint` values** in the command
  frontmatter so they stop tripping the "may be misread" YAML lint. An
  `argument-hint` that started with `[` was being parsed as a YAML flow
  sequence.
- **Ship a listing icon**: `plugin/.claude-plugin/icon.png` (1024×1024
  PNG converted from `assets/cterm.webp` — the portal rejects WebP/SVG).
  Referenced from `plugin.json`. The portal warns the listing icon may be
  locked on first submit, so this may not take effect on the current
  review, but it is in place for any resubmission and for local installs.
- **gitignore the SQLite WAL companion files** (`cterm.db-shm`,
  `cterm.db-wal`) so a daemon run at the repo root no longer leaves
  untracked files.

## 0.5.0 — cterm as a Claude plugin

cterm now ships as a [Claude plugin](docs/claude-plugin/README.md)
([#21](https://github.com/maifeeulasad/cterm/issues/21)). Install it into
Claude Code with `/plugin marketplace add maifeeulasad/cterm`, or upload the
zipped [plugin/](plugin/) folder in claude.ai > Customize > Plugins > Add >
Upload plugin. Three slash commands and a skill come with it:

- **`/cterm:exec [--id <session>] <command>`** — run through the daemon
  with full NDJSON event streaming. The command's own exit code propagates
  into the agent's tool output.
- **`/cterm:sessions`** — the daemon's session list, with a shortcut to
  `cterm-web` for the full catalog.
- **`/cterm:tail <execution-id>`** — replay-plus-tail of a stored
  execution; points at `cterm-web` while a native `cterm tail` subcommand
  is still on the follow-up pile.
- **`cterm-skill`** — teaches Claude when to prefer `cterm execute` over
  the raw Bash tool (long builds, deploys, anything auditable) and when
  not to (small reads, full-screen TUIs, daemon unreachable), and to stay
  in one session across a turn via `--id`.

The plugin ships no binaries of its own — every command wraps the
`cterm execute` CLI that landed in 0.4.1, so a user who already has
cterm installed gets the full plugin surface without any new code on
their machine. Prerequisites and three install paths (Claude Code, zip
upload, directory submission) are in
[docs/claude-plugin/installing.md](docs/claude-plugin/installing.md).

Directory submission is tracked in
[docs/claude-plugin/submission.md](docs/claude-plugin/submission.md) and
stays open for a plan upgrade before the first submit; the plugin is
usable today from the repo as a Claude Code marketplace.

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
