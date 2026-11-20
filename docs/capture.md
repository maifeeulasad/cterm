# Command capture

> Status: in progress (issue [#19](https://github.com/maifeeulasad/cterm/issues/19)).
> The source catalog, host probe, config, plan executor, and `cterm capture`
> CLI are implemented. Wiring the event stream into the daemon is the remaining
> step.

cterm's shim records a shell it launches. But a lot of what runs on a host never
goes through it: `sh -c` / `bash -c` from editors and build tools, scripts,
shells launched by absolute path, static binaries. **No single interception
point sees every command.** Command capture addresses that by modelling capture
as a set of independent *sources*, each belonging to a *layer*, that emit a
common event — so coverage is the union of complementary mechanisms rather than
one fragile hook.

## The layers

| Layer | What it is | Sees builtins? | Bypass-proof? |
|---|---|:---:|:---:|
| `shim` | cterm's PTY shim — full stream for shells it launches | ✅ | ❌ |
| `interactive-hook` | a shell's "before each command" hook (bash `DEBUG`, zsh `preexec`) | ✅ | ❌ |
| `noninteractive-hook` | a startup entry honored for `-c` too (bash `BASH_ENV`, cmd `AutoRun`) | ✅ | partial |
| `system-audit` | kernel/OS exec auditing (auditd/eBPF, Event 4688/Sysmon, Endpoint Security) | ❌ | ✅ |

The shell-hook layers are the only ones that see shell **builtins** (`cd`,
loops) and the exact as-typed line; the system-audit layer is the only one that
catches a command **however it was launched**, but it sees process *launches* so
it's blind to builtins. They're complementary — completeness is their union.

## Architecture (ports & adapters)

```
cterm-core::capture        the domain: CommandEvent, Layer, Privilege,
  (no I/O, pure)           Availability, SourceInfo, + the ports:
                             - HostProbe       (read-only view of the machine)
                             - CaptureSource   (a source: info/availability/plan)
                             - EnablePlan/Action (declarative enable-steps)
        ▲
        │ implements
        │
cterm-capture              adapters:
                             - SystemProbe     (the live HostProbe)
                             - catalog()       (CaptureSource per mechanism)
                             - CaptureConfig   (persisted enabled set)
                             - executor::apply (run unprivileged steps,
                                                defer elevated/manual ones)
        ▲
        │ uses
        │
cterm-cli `capture`        list / enable / disable / emit, config + prompts
```

Everything a source does to turn itself on is a **declarative plan** — an
ordered list of `Action`s (`WriteFile`, `AppendOnce`, `SetRegistryValue`,
`RunCommand`, `RemoveFile`, `RemoveBlock`, `DeleteRegistryValue`, `Manual`),
each flagged `elevated` or not. That's what lets the CLI show the operator
exactly what will change, and flag what needs root/admin, **before** anything
happens.

## The source catalog

| id | platform | layer | privilege | catches | notes / limits |
|---|---|---|---|---|---|
| `bash-hook` | Linux, macOS | noninteractive-hook | user | interactive bash + `bash -c` | `BASH_ENV` only fires if it's in the spawning process's env |
| `zsh-hook` | Linux, macOS | interactive-hook | user | interactive zsh | zsh has no `-c` hook |
| `cmd-autorun` | Windows | noninteractive-hook | user (HKCU) | `cmd /c` via `%CMDCMDLINE%` | can't see commands typed in an interactive cmd; overwrites an existing `AutoRun` (chaining is a TODO) |
| `linux-auditd` | Linux | system-audit | root | every exec, incl. `sh -c` | needs the `audit` package; `PROCTITLE` truncates long cmdlines |
| `windows-process-audit` | Windows | system-audit | admin | every process launch + cmdline | Event 4688; only affects processes started after enabling |
| `macos-endpoint-security` | macOS | system-audit | entitlement | every exec | needs Apple's ES-client entitlement + a notarized helper — a guided manual step |

See the per-shell / per-OS research in
[issue #19](https://github.com/maifeeulasad/cterm/issues/19) for the full
coverage matrix and the sources behind each mechanism.

## Using it

```bash
cterm capture list                         # every source: layer, privilege,
                                           # per-host availability, on/off
cterm capture enable bash-hook zsh-hook    # enable the user-level shell hooks
cterm capture enable linux-auditd          # prompts: auditd needs root
cterm capture disable cmd-autorun          # undo
```

`enable` applies the unprivileged steps of the plan directly. When a source
needs elevation, cterm prints those steps and asks you to **accept or reject**
before continuing (skip the prompt with `--yes`); it **never escalates on its
own**. Steps that need root/admin — or that are inherently manual, like the
macOS entitlement — are printed for you to run yourself. The enabled set is
stored in `~/.cterm/capture.json`.

### How a hook reports a command

An installed shell hook shells out, per command, to:

```
cterm capture emit --shell bash --command "<cmd>" --cwd "<dir>"
```

which appends one `CommandEvent` (JSON) to `~/.cterm/capture-events.ndjson`.
This is the v1 transport: an append-only log a consumer can tail.

## Known limits & roadmap

- **Daemon ingestion.** Events land in `capture-events.ndjson` today; feeding
  them into the daemon's record store (and correlating with shim sessions by
  pid + time) is the next step.
- **Scale of the shell-hook transport.** Spawning `cterm` per command is fine
  for interactive use but heavy under load; a socket / batched fast path is the
  scale answer.
- **`BASH_ENV` reach.** A per-user `BASH_ENV` only covers shells whose parent
  exported it; truly bypass-proof non-interactive bash needs the system-audit
  layer.
- **cmd `AutoRun` chaining.** Enabling overwrites any existing `AutoRun`
  (e.g. Clink); it should chain instead. (The accept/reject prompt shows the
  change first, so it isn't silent.)
- **Privilege.** The bypass-proof layers need root/admin, and macOS needs an
  Apple-approved entitlement — unprivileged installs reach only the shell-hook
  layer.
- **Secrets.** Command lines can carry secrets; a capture-all deployment needs a
  retention / redaction policy.
