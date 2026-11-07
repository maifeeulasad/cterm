# Installing the cterm plugin

Three paths. Pick the one that matches where you want to use cterm.

## Prerequisites (any path)

cterm's binaries must already be on the client machine where the plugin
runs. The plugin itself ships no code — its commands call `cterm execute`
and the SKILL expects `cterm status` / `cterm-daemon` to be available.

```bash
# 1. Install cterm (prebuilt release, any platform)
curl -L https://github.com/maifeeulasad/cterm/releases/latest/download/cterm-x86_64-linux.tar.gz | tar -xz
# 2. Put the three binaries on PATH (symlinks on Unix, copies on Windows)
./cterm link --user
# 3. Start the daemon on login (systemd --user / launchd / Windows Startup)
cterm service enable
```

See the [top-level README](../../README.md) for Windows/macOS commands and
alternative install methods.

### Optional: opt-in daemon auth (0.6.0+)

If you're on a multi-user host or a container host where another UID or
container can reach `127.0.0.1:50051`, turn on bearer-token auth before
running the plugin so the daemon rejects anything that isn't your Claude
client:

```bash
cterm auth enable                     # stores a fresh token in the OS keyring
CTERM_AUTH=on cterm-daemon            # or add CTERM_AUTH=on to the service unit
```

The plugin's commands then pick the token up automatically from the
keyring (or `CTERM_AUTH_TOKEN` on headless hosts). Full flow in
[docs/auth.md](../auth.md). Skip this on a single-user developer box —
the default (open on loopback) is fine there.

## 1. Claude Code — from this GitHub repository

In an interactive `claude` session:

```text
/plugin marketplace add maifeeulasad/cterm
/plugin install cterm@cterm
```

Claude Code reads `plugin/.claude-plugin/plugin.json` directly from the repo
and pins to the default branch. Updates arrive on `/plugin update` (or every
`claude` start with auto-update on). To pin to the released tag instead:

```text
/plugin marketplace add maifeeulasad/cterm@v0.5.0
```

## 2. claude.ai or the desktop app — upload the plugin

The claude.ai Customize > Plugins page accepts a `.zip` of the plugin folder:

```bash
cd plugin
zip -r ../cterm-plugin.zip .
```

In claude.ai: **Customize > Plugins > Add > Upload plugin**, pick the zip.
The archive must contain one `.claude-plugin/plugin.json`; both "zip the
folder itself" and "zip the folder's contents" work.

Once uploaded the plugin is on your account, so it also loads in Cowork and
Claude Code when you're signed in to the same account.

## 3. The directory (pending)

Claude's own directory requires a review pass; status in
[submission.md](submission.md). When live, the install path is:

- claude.ai > **Customize > Plugins > Discover > cterm > Add**

## Verifying the install

In any Claude surface, type `/` and look for `cterm:` entries (`exec`,
`sessions`, `tail`). On Claude Code:

```text
/cterm:sessions
```

should print the daemon's session list. If it errors:

- **Connect failure**: the daemon isn't running — `cterm service status`
  and `cterm doctor` diagnose the install.
- **`UNAUTHENTICATED`**: the daemon has 0.6.0 auth on but this client
  doesn't have the token. `cterm auth status` confirms; `cterm auth
  enable` (or set `CTERM_AUTH_TOKEN`) wires one up.
