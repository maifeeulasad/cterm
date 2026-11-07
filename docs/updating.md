# Updating cterm

cterm uses a **two-path install model**. This is deliberate, and some
operational quirks (Linux `ETXTBSY` on `cterm update`, `cterm-web`
keeping an old UI until you restart it) are consequences of that model
rather than bugs.

## The two paths

| Path | Who writes it | Who reads it |
| --- | --- | --- |
| `~/.cterm/bin/` | `cterm update` (and the installer). Authoritative home. | `cterm-daemon.service`'s `ExecStart` points here directly. |
| `~/.local/bin/` (`--user`, default) or `/usr/local/bin/` (`--system`) | `cterm link` — **symlinks on Unix** into `~/.cterm/bin/`, **copies on Windows** (Windows can't exec a symlink to a loose binary cleanly). | Your shell's `PATH` — the `cterm` CLI you type. `cterm-web.service`'s `ExecStart` points here. |

The split exists so you can:

- **Drop a hand-built or vendored binary into `~/.local/bin/`** (by
  replacing the symlink with a file on Linux, or overwriting the copy on
  Windows) and have the services pick it up on restart, without
  `cterm update` later stomping on it.
- **Pin a stable version on PATH** while experimenting with a different
  build out of `~/.cterm/bin/` — the two can diverge intentionally.
- **Keep `cterm update` deliberate**: it never touches the services'
  running state, so a long-running recording isn't cut off by a
  background update.

## What `cterm update` does — and doesn't

**Does:**

- Downloads the latest release tarball for your platform.
- Verifies it against the published sha256.
- Writes fresh `cterm`, `cterm-daemon`, and `cterm-web` into
  `~/.cterm/bin/`.

**Does not:**

- Touch the copies `cterm link` placed on Windows (symlinks on Unix
  follow `~/.cterm/bin/` automatically, so no copy step is needed
  there).
- Stop, restart, or reload any `cterm-daemon.service` /
  `cterm-web.service` (nor the launchd / Windows Startup equivalents).
- Try to overwrite a binary that is currently running — on Linux that's
  `ETXTBSY` ("Text file busy"); on Windows the file is locked.
  `cterm update` reports the failure clearly for each binary it couldn't
  rewrite and keeps going on the ones it could.

## The intended update flow

After `cterm update` finishes, the running services are still on the old
binary. The rollout step is yours, so you can pick the moment —
mid-pipeline is usually a bad moment.

### Linux (systemd `--user`)

`cterm link --user` on Linux creates **symlinks**, so `~/.local/bin/`
automatically tracks `~/.cterm/bin/`. The only work is stopping the
services long enough to let `cterm update` overwrite the on-disk
binaries, then starting them again.

```bash
# 1. Stop anything that's holding a running binary open so cterm update
#    can overwrite the files under ~/.cterm/bin/.
systemctl --user stop cterm-daemon.service cterm-web.service

# 2. Fetch the new binaries into ~/.cterm/bin/. With the services down
#    this succeeds for all three; with them up, cterm-daemon /
#    cterm-web will each report "Text file busy" for their own binary
#    and skip it (harmless — the others still update, and you'd just
#    repeat step 1-2 later).
cterm update

# 3. Restart the services. They re-resolve the symlinks and run the
#    fresh binaries.
systemctl --user restart cterm-daemon.service
systemctl --user start   cterm-web.service
```

If you ever see `~/.local/bin/cterm-web` (or any of the three) as a
plain file instead of a symlink — e.g. because an older install path
dropped a copy there, or because someone hand-placed one — `cterm link
--user` rewrites the link targets atomically. After that the symlink
flow above applies again.

### macOS (launchd)

Same shape, launchd commands:

```bash
launchctl unload ~/Library/LaunchAgents/com.maifeeulasad.cterm-daemon.plist
launchctl unload ~/Library/LaunchAgents/com.maifeeulasad.cterm-web.plist
cterm update
launchctl load   ~/Library/LaunchAgents/com.maifeeulasad.cterm-daemon.plist
launchctl load   ~/Library/LaunchAgents/com.maifeeulasad.cterm-web.plist
```

### Windows

`cterm link --user` on Windows places **copies**, so after `cterm
update` you need to re-run `cterm link` to refresh them. Stop the
Startup-invoked processes first (Task Manager or log out/in).

```powershell
cterm update
cterm link --user  # re-copies ~/.cterm/bin/* into %LOCALAPPDATA%\cterm\bin\
# then start cterm-web.exe / cterm-daemon.exe again
```

## FAQ

**Q: Why not have `cterm update` do all of this automatically?**
A: Two reasons. First, bouncing `cterm-web` / `cterm-daemon` mid-session
cuts off whatever is being recorded; the operator is the only one who
knows when it's safe. Second, having a hand-built binary at
`~/.local/bin/cterm-web` is a real dev loop — an automatic post-update
copy/symlink would quietly replace it.

**Q: `cterm update` printed "Text file busy" for `cterm-daemon`. Is that
a problem?**
A: Only until you restart the daemon. If the file wasn't rewritten, the
old binary is still on disk and the service keeps running it; on
Linux, stop the service, re-run `cterm update`, start the service.

**Q: After `cterm update`, my web UI still looks old at
`http://localhost:8088/`.**
A: Expected if `cterm-web` was running when `cterm update` tried to
overwrite its binary (same `ETXTBSY` story as the daemon). On Linux:

```bash
systemctl --user stop cterm-web.service
cterm update              # now succeeds on cterm-web
systemctl --user start cterm-web.service
```

Then hard-reload the tab (Ctrl-Shift-R / Cmd-Shift-R) to bypass the
cached HTML.

**Q: After `cterm update`, my `cterm` on PATH reports the old version.**
A: Linux/macOS — `cterm link --user` creates symlinks, so your PATH
`cterm` should already point at the fresh binary; if it doesn't, the
link target is a stale file and `cterm link --user` fixes it. Windows —
re-run `cterm link --user` to refresh the copy.

## See also

- [docs/auth.md](auth.md) — restarting the daemon to pick up a newly
  configured auth token follows the same stop → restart pattern.
- Release notes: [RELEASE_NOTES.md](../RELEASE_NOTES.md).
