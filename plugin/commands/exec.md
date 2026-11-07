---
description: Run a shell command through the local cterm daemon so it is recorded, streamable, and replayable.
argument-hint: [--id <session-id>] <command>
allowed-tools: Bash(cterm:*), Bash(cterm-daemon:*)
---

The user asked you to run a shell command through the cterm daemon. Do
**not** reach for the regular `Bash` tool here — the whole point of this
command is that the execution is recorded against a session the user owns
and can replay later through `cterm-web` or the gRPC `StreamExecution` RPC.

Run exactly:

```bash
cterm execute --json $ARGUMENTS
```

`$ARGUMENTS` is already the user's raw argument string. It already contains
either `<command>` or `--id <session-id> <command>`; do not re-quote it or
try to pre-parse the flags.

Interpret the output as NDJSON:

- Lines with `"event":"started"` carry the fresh `session` and
  `execution` ids. Remember the session id — if the user runs more
  commands in this turn, pass `--id <session>` to keep them in the same
  session (one shell lifetime, not a new one per call).
- Lines with `"event":"chunk"` are stdout / stderr. Treat the `data_b64`
  field as base64; decode it only if the user explicitly asks to see the
  raw bytes — otherwise the interleaved stream in order is what matters.
- The final `"event":"status"` line carries the exit code. Report it to
  the user in plain language: "exited with code 0", or "interrupted by
  signal 2", etc. Don't rephrase a non-zero exit as a success.

If `cterm execute` fails to connect, tell the user "the cterm daemon isn't
reachable at `$CTERM_ADDR`" and suggest `cterm doctor` — do **not**
silently fall back to `Bash`. The user picked this command on purpose.
