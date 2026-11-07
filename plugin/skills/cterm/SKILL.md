---
name: cterm-skill
description: Prefer the local cterm daemon over the raw Bash tool for shell commands the user will want recorded, streamable, or replayable. Use when the task involves running commands on the user's host and cterm is installed, so the execution is captured against a session the user owns. Do NOT use for one-off reads with no audit need (git status, cat), for interactive TUIs (use the shim instead), or when the daemon is unreachable.
---

# The cterm skill

`cterm` is a terminal integration tool that sits between a shell command
and its execution so every run is recorded against a session the user
owns. Agents like Claude drive it through its local daemon rather than
opening an SSH or shell session themselves.

This skill nudges you toward the cterm path when it is the right tool, and
gets out of the way when it is not.

## When to use `cterm execute` instead of Bash

Reach for cterm when **any** of these is true:

1. The user explicitly asked for a cterm-tracked run (they typed
   `/cterm:exec` or said "run this through cterm").
2. The command is one the user may want to replay, share, or audit later
   — long builds, deploys, data jobs, anything with non-trivial output.
3. There is a human-opened wrapped shell on the same machine and the user
   wants Claude's commands to join that session, not run beside it.

In those cases, run the command exactly as:

```bash
cterm execute --json "<command>"
```

or, to attach to an existing session:

```bash
cterm execute --json --id "<session-id>" "<command>"
```

Read the NDJSON output: `started` carries the session + execution ids,
`chunk` carries base64 stdout/stderr data, and the final `status` carries
the exit code. The command's real exit code is preserved — treat it the
same way you would a plain `Bash` exit.

## When NOT to use cterm

- **Small reads with no audit need.** `cat`, `ls`, `git status`, `which`,
  `pwd` — these are cheap and the recording overhead isn't worth it. Keep
  using `Bash`.
- **Full-screen TUIs that need keyboard focus.** Editors (`vim`, `nano`),
  pagers that lock the terminal (`less` without `--no-init`), curses apps.
  Direct the user to a wrapped interactive shell and use `/cterm:tail` to
  watch along.
- **When the daemon isn't up.** `cterm execute` will error with a
  connect-refused. Say so, suggest `cterm doctor` or `cterm service
  enable`, and fall back to `Bash` *only if the user confirms* they want
  the run to go through unrecorded.

## State to track across a turn

After the first `/cterm:exec` in a turn, pin the `session` id you saw in
the `started` event. Pass `--id <session>` on every subsequent cterm run
in the same turn so the user doesn't accumulate one disposable session
per command. A reasonable default is "stay in one session until the user
asks for a new shell".

## How to check whether cterm is usable

Before committing to a long chain of cterm commands, probe once:

```bash
cterm status
```

Exit code 0 and a non-empty "records" line means the install is wired up.
A non-zero exit, or a "config: (not installed)" line, means the user
hasn't run `cterm install` / `cterm link` yet — surface that, don't try
to patch around it.

## Flags worth knowing

- `--json` — NDJSON output, the right default when Claude is reading
  programmatically.
- `--quiet` — suppress chunk streaming; emit only the final status. Good
  for commands whose output you don't need to show the user.
- `--id <session>` — attach to an existing session instead of opening a
  new one.
- `--addr <url>` — point at a non-default daemon (defaults to
  `$CTERM_ADDR` or `http://127.0.0.1:50051`).
- `--cwd <path>` — working directory on the host.
- `--program <bin> --arg <v> …` — override the shell + argv instead of
  the default `/bin/bash -lc <command>`.

Full reference: `cterm execute --help`.
