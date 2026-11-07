# Using the cterm plugin

Once installed (see [installing.md](installing.md)) the plugin shows up as
one skill and three slash commands, namespaced under `cterm:`.

## The commands

### `/cterm:exec [--id <id>] [--title <t>] [--label k=v]… <command>`

Runs `<command>` through the daemon and streams its output back into the
conversation. Equivalent to `cterm execute --json "$ARGUMENTS"` on the host.

- Everything the daemon streams (stdout, stderr, status) is recorded against
  a session, so the execution is observable through gRPC *and* the
  `cterm-web` browser later.
- The command's exit code is reflected in the tool output; Claude reads it
  the same way it reads a `Bash` exit code.
- To attach to an existing session instead of opening a new one, invoke the
  command with `--id <session-id>` in the arguments.
- To **name** a new session so the user can find it later in `cterm-web`:
  pass `--title "one-line description"` and optionally one or more
  `--label key=value` tags. Both are open-time only (rejected when
  combined with `--id`). Rules: title <= 256 bytes, no newlines/ANSI;
  labels <= 32, key/value each <= 128 bytes, no control characters,
  `=` banned in keys. See [docs/session-metadata.md](../session-metadata.md)
  for the full rules.
- If the daemon has 0.6.0 bearer-token auth turned on, `cterm execute`
  picks the token up from `CTERM_AUTH_TOKEN` or the OS keyring; nothing
  to pass through the slash command.

### `/cterm:sessions`

Calls `Terminal.ListSessions` via `cterm execute` (actually a tiny wrapper
that Prints JSON list from the daemon). Use it when the user asks what
shells are live, when you need to pick an existing session for `--id`, or
before offering to tail a specific execution.

### `/cterm:tail <execution-id>`

Prints the stored history of an execution and then follows it live if it is
still running. Thin wrapper over `cterm execute`'s tail mode (currently
piped through `StreamExecution` with `replay=true`). Useful when a human
started something in a wrapped shell and you want to watch along without
interrupting them.

## The skill

`cterm-skill` loads whenever a task matches *"run a shell command the user
will want recorded or want me to observe later"*. It does four things:

1. **Nudges Claude to prefer `cterm execute` over `Bash`** when a cterm
   daemon is reachable. Rule of thumb in the skill: if `cterm status`
   succeeds and `CTERM_ADDR` is set (or the default is live), the command
   goes through the daemon.
2. **Captures the session id** after the first `/cterm:exec` so follow-up
   commands can reuse `--id` instead of opening a new session per call.
3. **Names new sessions** with `--title` / `--label` when the task is
   user-visible (a build, a deploy, a data job). Doesn't invent noisy
   tags — only well-known keys like `env`, `ticket`, `pr`, `service`,
   `commit`.
4. **Surfaces auth and validation rejections cleanly** instead of
   retrying — on `UNAUTHENTICATED` it points at `cterm auth status`;
   on `InvalidArgument` for bad metadata it relays the daemon's reason.

The skill is written so it loses gracefully — if the daemon is down or
cterm isn't installed, it says so and falls back to regular `Bash`. It
never leaves the user without a way to run the command.

## When *not* to use the plugin

- **One-off reads with no audit need** — a `git status` or a `cat` stays
  cheaper under `Bash`. The skill lets these through.
- **Interactive TUIs** — `cterm execute` runs through a PTY, so simple
  TUIs work, but anything that expects keyboard focus or a pipe-less tty
  can get awkward. Use the shim (a wrapped interactive shell the human
  opens) and the `/cterm:tail` command instead.

## Environment the plugin reads

Nothing specific to the plugin. Its commands inherit `CTERM_ADDR` from the
caller's env (default `http://127.0.0.1:50051`), and `cterm execute`
prefers `$CTERM_ADDR` over the built-in default. See the main README for
every variable.
