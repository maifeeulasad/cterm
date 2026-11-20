# Registering cterm as a terminal profile

cterm's hybrid flow records a shell a **human** opens: your terminal launches
the cterm *shim* (`~/.cterm/bin/<shell>`) instead of the raw shell, the shim
reports to the local daemon, and an agent can observe (and, where policy
allows, drive) the session over gRPC — without ever holding your credentials.

The clean way to opt a terminal into this is a dedicated **profile** that runs
the shim. `cterm register` sets that up.

## Prerequisites

1. Install the shims for the shells you want to record:

   ```bash
   cterm install            # wraps the common interactive shells found
   cterm status             # confirm which shims exist
   ```

2. Have the daemon running (so sessions are recorded):

   ```bash
   cterm service enable     # autostart, or run cterm-daemon yourself
   cterm doctor             # confirm the daemon is reachable
   ```

## `cterm register`

```text
cterm register [SHELL] [--print | --remove]
```

- `SHELL` — which installed shim the profile launches. Defaults to the first
  installed shim.
- `--print` — show what would be registered without writing anything.
- `--remove` — remove a profile cterm previously registered.

What it does depends on the platform, because terminal emulators don't share a
profile format.

### Windows — Windows Terminal (automatic)

`cterm register` adds a `cterm (<shell>)` profile directly to Windows Terminal's
`settings.json`:

```powershell
cterm register              # adds the profile to settings.json
cterm register --print      # preview the profile JSON without writing
cterm register --remove     # delete the profile
```

Windows Terminal **hot-reloads** `settings.json`, so the profile appears in the
new-tab (▾) menu right away — no restart. Open a tab from it to start a recorded
session.

> **Why not a JSON fragment?** Windows Terminal's [fragment extensions][frag]
> look like the natural fit, but on the packaged/Store build the mere presence
> of a per-user `Fragments\` folder can make Terminal fail to load its settings
> and fall back to defaults ([microsoft/terminal#20893][bug]) — a valid fragment
> doesn't rescue it. So `cterm register` edits `settings.json` instead (and
> removes any `Fragments\cterm` folder an older cterm left behind). It writes a
> profile with a stable GUID, preserving everything else in the file; re-running
> updates in place, and `--remove` deletes it by GUID.

[frag]: https://learn.microsoft.com/windows/terminal/json-fragment-extensions
[bug]: https://github.com/microsoft/terminal/issues/20893

### macOS / Linux — paste the settings

Terminal emulators on macOS and Linux keep profiles in their own, non-shared
formats, so `cterm register` **prints** the settings to add by hand:

```text
Profile name          cterm (<shell>)
Command / Custom      "/home/you/.cterm/bin/<shell>"
Working directory     preserve / inherit the current directory ("Always")
When command exits    close the terminal
```

The three behavioral settings matter:

- **Command** — run the cterm shim as the profile's command (not the raw
  shell). The shim execs your real shell under a PTY and records it.
- **Working directory: preserve / Always** — so a new terminal opens where you
  are, not at `$HOME`. (This is the key setting called out in
  [issue #26](https://github.com/maifeeulasad/cterm/issues/26).)
- **Exit the terminal when the command exits** — so closing the shell closes
  the tab, as a normal shell would.

Per-emulator notes:

- **iTerm2** — Settings → Profiles → `+` → *General* → *Command* = “Command”,
  set it to the shim path; *Working Directory* = “Reuse previous session's
  directory”.
- **GNOME Terminal / Tilix** — Preferences → Profiles → new profile → *Command*
  → “Run a custom command instead of my shell”, set the shim; enable “When
  command exits: Exit the terminal”.
- **Konsole** — Settings → Manage Profiles → New → *General* → *Command* = the
  shim path.
- **Terminal.app (macOS)** — Settings → Profiles → *Shell* → “Run command” =
  the shim path, with “Run inside shell” **off**.

## Why not `cterm execute`?

`cterm execute` is the agent-facing front door: an agent runs a one-off command
through the daemon. Registration is the **human-facing** side of the same
system — you open a normal interactive shell that happens to be recorded, and
the agent joins that session. Use registration for day-to-day interactive work;
use `cterm execute` when an agent wants to run a specific command itself.
