# cterm — a Claude plugin

Run and observe shell executions through the local
[cterm](https://github.com/maifeeulasad/cterm) daemon so every command is
recorded, streamable, and replayable, without an agent ever holding SSH
keys or credentials.

## What this plugin adds

| Kind    | Name                 | What it does                                               |
| ------- | -------------------- | ---------------------------------------------------------- |
| Command | `/cterm:exec`        | Run a command through the daemon; stream stdout/stderr. Supports `--title` / `--label` on new sessions (0.6.0+). |
| Command | `/cterm:sessions`    | List the daemon's known sessions, including their titles/labels. |
| Command | `/cterm:tail`        | Replay then follow a recorded execution.                   |
| Skill   | `cterm-skill`        | Teaches Claude when to prefer `cterm execute` over `Bash`, when to name a session, and how to handle auth rejections. |

Every command is a thin wrapper over the
[`cterm execute`](https://github.com/maifeeulasad/cterm#usage) CLI — the
plugin ships no binaries of its own and never asks for credentials
directly. When the operator has turned on [daemon auth](https://github.com/maifeeulasad/cterm/blob/main/docs/auth.md),
`cterm execute` picks the token up from `CTERM_AUTH_TOKEN` or the OS
keyring; the plugin itself does nothing special for this.

## Prerequisites

The cterm binaries must be installed and the daemon must be reachable. One
line on any platform:

```bash
cterm link --user && cterm service enable
```

Full install instructions:
[docs/claude-plugin/installing.md](https://github.com/maifeeulasad/cterm/blob/main/docs/claude-plugin/installing.md).

## Try it

In any Claude surface that loads the plugin:

```text
/cterm:sessions
/cterm:exec echo hello from cterm
/cterm:exec --title "deploy webapp" --label env=staging kubectl apply -f manifest.yaml
/cterm:tail <execution-id>
```

If `/cterm:sessions` errors with a connect failure, the daemon isn't
running — `cterm doctor` diagnoses the install. If it errors with
`UNAUTHENTICATED`, the daemon has auth on but this client doesn't have a
token — `cterm auth status` / `cterm auth enable`.

## Docs

- [Overview and positioning](https://github.com/maifeeulasad/cterm/blob/main/docs/claude-plugin/README.md)
- [Install paths](https://github.com/maifeeulasad/cterm/blob/main/docs/claude-plugin/installing.md)
- [Usage and when-not-to-use](https://github.com/maifeeulasad/cterm/blob/main/docs/claude-plugin/usage.md)
- [Directory submission notes](https://github.com/maifeeulasad/cterm/blob/main/docs/claude-plugin/submission.md)

## License

GPL-3.0-or-later. Same as the rest of the repository.
