# Daemon authentication (opt-in)

As of **0.5.x the daemon is open** on `127.0.0.1:50051` — any local
process can call `Terminal.Run`, `ListSessions`, `SendInput`, etc. For
single-user developer machines on a trusted host, that matches the threat
model (the human already controls the host and the credentials).

**0.6.0 adds opt-in bearer-token auth.** This document is how to turn it
on. [Issue #27](https://github.com/maifeeulasad/cterm/issues/27) tracks
the design.

## When to turn it on

Enable auth whenever another local process shouldn't be trusted to drive
the daemon. Common cases:

- A **multi-user host** where other UIDs can reach `127.0.0.1:50051`.
- A **container host** where other containers share the loopback.
- A **CI runner** where you want to confirm the plugin flow but don't
  want test jobs to be able to open arbitrary shells.
- **Defense in depth** in production even on a single-user host, so a
  compromised browser or code-editor extension can't trivially drive
  cterm.

If none of those apply, the default (open on loopback) is fine. The
switch is explicit on purpose.

## The model

- **One shared bearer token.** Not per-client, not rotated by the
  daemon. The daemon knows exactly one `expected` value; the client
  presents it in `authorization: Bearer <token>`. Rotation is an
  operator action (`cterm auth rotate` + restart the daemon).
- **Constant-time compare** on verification, so a mismatch doesn't leak
  which byte differed via wall-clock timing.
- **No TLS.** The transport stays plaintext gRPC over loopback —
  encryption against a local attacker who's already reading loopback
  traffic is a different conversation. Add it when the transport
  genuinely crosses a network boundary.
- **Opt-in at both ends.** The daemon enforces auth only when configured
  for it; the client attaches a header only when a token is available.
  Mismatches are legible: a configured daemon with an un-configured
  client returns `UNAUTHENTICATED`.

## Where the token lives

In order of precedence the client uses:

1. **`CTERM_AUTH_TOKEN=<secret>`** in the environment. First-class escape
   hatch for CI, containers, and headless Linux boxes without a keyring.
2. **OS native keyring** — macOS **Keychain**, Windows **Credential
   Manager**, Linux **Secret Service** (GNOME Keyring / KWallet). Managed
   with `cterm auth`. Service name `cterm`, account name `daemon-token`.
3. **Nothing** — the client connects unauthenticated. Works against an
   open daemon; rejected by a protected one.

The daemon follows the same precedence except it never reads the
keyring itself (that's a CLI responsibility; keeps the server process
dependency-light).

## Turn it on

### 1. Store a token

```bash
cterm auth enable
# → cterm auth: stored token in cterm keyring (fingerprint: 3f9a2b4c)
#   Restart cterm-daemon with CTERM_AUTH=on to enforce it.
```

Or specify it yourself (useful when you want multiple clients to share
one known value):

```bash
cterm auth enable --token hunter2hunter2hunter2hunter2hunter2
```

Or print it once to paste elsewhere:

```bash
cterm auth enable --print
# → f4b5c7...
#   Restart cterm-daemon ...
```

### 2. Tell the daemon to use it

```bash
CTERM_AUTH=on cterm-daemon
# → cterm-daemon listening on 127.0.0.1:50051 (db: cterm.db, auth=on)
```

Or pass the token directly via env (useful on hosts without a keyring):

```bash
CTERM_AUTH_TOKEN="$(cat token-file)" cterm-daemon
```

If you use `cterm service enable` to run the daemon under systemd /
launchd / Windows Startup, add `CTERM_AUTH=on` to the service unit's
environment so it survives reboots.

### 3. Verify

```bash
cterm execute --json 'echo authenticated'
# → {"event":"started", ...}
#   {"event":"chunk", ...}
#   {"event":"status", "payload":{"kind":"completed","code":0}}
```

If you forgot to run `cterm auth enable` or `CTERM_AUTH_TOKEN`:

```bash
cterm execute --json 'echo authenticated'
# → cterm execute: Run RPC: status: Unauthenticated,
#     message: "invalid or missing bearer token"
```

## Rotate

```bash
cterm auth rotate
# → cterm auth: rotated token (fingerprint: 8a4c1e02)
#   Restart cterm-daemon so the new token takes effect.
```

Rotation only matters if you've distributed the token to other clients
too — restart the daemon so it picks up the new value.

## Turn it off

```bash
cterm auth disable
# → cterm auth: cleared token from keyring
#   Restart cterm-daemon (without CTERM_AUTH=on and without CTERM_AUTH_TOKEN) to drop back to open mode.
```

The CLI only removes the stored token; the daemon keeps enforcing
whatever it loaded at startup until you restart it.

## Status

```bash
cterm auth status
# → cterm auth: token present (source: keyring, fingerprint: 3f9a2b4c)
```

The fingerprint is just the first 8 chars of the token — enough to
distinguish rotations by eye, nowhere near enough to reconstruct the
secret.

## What is **not** covered

- **Per-client identities.** There's one shared token.
- **Audit of who presented the token.** The daemon's existing
  session/execution records show *what* ran, not *which bearer*.
- **TLS / mTLS.** Not implemented. The expected deployment is loopback
  only.
- **Hardware-backed credentials.** The keyring crate uses whatever the
  OS native store provides; on macOS that is Keychain (which itself can
  be backed by the Secure Enclave), on Windows it's Credential Manager,
  on Linux it's Secret Service. We don't layer our own hardware
  binding on top.

If your deployment needs any of the above, open an issue —
[#27](https://github.com/maifeeulasad/cterm/issues/27) is a good place
to discuss.
