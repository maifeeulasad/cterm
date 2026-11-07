# cterm as a Claude plugin

**Status:** shipped in 0.5.0. Discovery and submission to Anthropic's directory
tracked in [#21](https://github.com/maifeeulasad/cterm/issues/21).

## Why a plugin and not a connector

Anthropic's [connector submission docs](https://claude.com/docs/connectors/building/submission#submit-your-connector)
require a remote HTTPS endpoint for an MCP connector. `cterm-daemon` is a
local service by design — it listens on `127.0.0.1:50051` (gRPC) and a
Unix socket / named pipe (shim IPC). Exposing it over HTTPS would undo the
whole threat model (the agent is supposed to go through a *local* broker, not
reach across the network with its own credentials).

So we ship cterm as a [Claude plugin](https://claude.com/docs/plugins/overview)
instead. A plugin bundles **skills**, **commands**, **agents**, and optional
**MCP servers** into a single unit a user installs from Customize > Plugins
in claude.ai, or from the directory, or straight from a Git URL in Claude Code.

## What this plugin ships

The plugin lives in [plugin/](../../plugin/) at the repo root.

| Component                | What it gives Claude                                                  |
| ------------------------ | --------------------------------------------------------------------- |
| `skills/cterm/SKILL.md`  | Teaches Claude when to reach for `cterm execute` in place of `Bash`, when to name a session (0.6.0 `--title` / `--label`), and how to handle `UNAUTHENTICATED` from an auth-enabled daemon. |
| `commands/exec.md`       | `/cterm:exec [--id <id>] [--title <t>] [--label k=v] <command>` — run through the daemon, stream output. |
| `commands/sessions.md`   | `/cterm:sessions` — list the daemon's known sessions, with any titles/labels they carry. |
| `commands/tail.md`       | `/cterm:tail <execution-id>` — replay + tail an execution.            |

Every command is a thin wrapper around the `cterm execute` CLI added in
0.4.1 (issue [#24](https://github.com/maifeeulasad/cterm/issues/24)), so the
plugin itself installs **no new code on the user's machine** — it only tells
Claude how to use the binaries they already have. When the operator turns on
[daemon bearer-token auth](../auth.md) (0.6.0, issue
[#27](https://github.com/maifeeulasad/cterm/issues/27)), the token is picked
up from `CTERM_AUTH_TOKEN` or the OS keyring; no plugin change needed.

## Three ways to use it

1. **From this repository, with Claude Code** — add the repo as a marketplace:
   see [installing.md](installing.md).
2. **Uploaded to claude.ai** — zip the `plugin/` folder and upload it from
   Customize > Plugins > Add > Upload plugin.
3. **Via the directory** — pending Anthropic review; see
   [submission.md](submission.md) for the checklist and status.

Usage patterns, flags, and how skills kick in automatically are in
[usage.md](usage.md).

## Scope for 0.5.0

What is **in**:

- The `plugin/` folder, validated with `claude plugin validate`.
- The three commands above, the SKILL, and the metadata needed by the
  directory (license, repository, homepage, keywords, icon, policy URLs).

What is deliberately **out** (follow-up work):

- A bundled MCP server. The daemon speaks gRPC, not MCP. Wrapping it in an
  MCP server is its own crate; we'll track that separately.
- A SessionStart hook that auto-launches `cterm-daemon`. For 0.5.0 we expect
  the user to run `cterm service enable` once (or launch the daemon manually)
  and the plugin trusts that it's up.

Both are tracked in follow-up issues once this lands.
