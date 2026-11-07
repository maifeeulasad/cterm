# Using the cterm plugin

Once installed (see [installing.md](installing.md)) the plugin shows up as
one skill and three slash commands, namespaced under `cterm:`.

## The commands

### `/cterm:exec <command>`

Runs `<command>` through the daemon and streams its output back into the
conversation. Equivalent to `cterm execute --json "$ARGUMENTS"` on the host.

- Everything the daemon streams (stdout, stderr, status) is recorded against
  a session, so the execution is observable through gRPC *and* the
  `cterm-web` browser later.
- The command's exit code is reflected in the tool output; Claude reads it
  the same way it reads a `Bash` exit code.
- To attach to an existing session instead of opening a new one, invoke the
  command with `--id <session-id>` in the arguments.

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
will want recorded or want me to observe later"*. It does two things:

1. **Nudges Claude to prefer `cterm execute` over `Bash`** when a cterm
   daemon is reachable. Rule of thumb in the skill: if `cterm status`
   succeeds and `CTERM_ADDR` is set (or the default is live), the command
   goes through the daemon.
2. **Captures the session id** after the first `/cterm:exec` so follow-up
   commands can reuse `--id` instead of opening a new session per call.

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
