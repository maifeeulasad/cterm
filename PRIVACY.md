# Privacy Policy

Last updated: 2026-11-07.

## Short version

cterm is a **local-first** tool. It runs on your machine, it stores your
data on your machine, and it talks only to `127.0.0.1` by default. The
maintainers run **no server, no analytics, no telemetry, and no crash
reporting**. Nothing is collected or transmitted to us by anything in
this repository.

## What cterm handles

cterm is a terminal integration tool. Running it produces three kinds of
data, all handled on your machine:

- **Session and execution records.** `cterm-daemon` writes an SQLite
  file (default: `~/.cterm/cterm.db`, overridable with `CTERM_DB`) that
  contains the commands you (or an agent going through the daemon) ran,
  their raw stdout/stderr/stdin bytes, exit status, timing, host name,
  shell name, and generated UUIDs. **This file never leaves your machine
  through anything in this repository.** Delete it to delete the records;
  the schema is in [crates/cterm-storage/src/sqlite.rs](crates/cterm-storage/src/sqlite.rs).
- **gRPC and shim traffic.** `cterm-daemon` listens on `CTERM_ADDR`
  (default `127.0.0.1:50051`, gRPC) and on `CTERM_SOCK` (a Unix socket
  or a Windows named pipe for the shim). Both bind to localhost by
  default. If you choose to bind to a non-loopback address, that is your
  decision, and whatever reaches that address is subject to that
  network's exposure — cterm does nothing special with it beyond the
  same local recording.
- **Config and credentials.** cterm reads `~/.cterm/config.json` (the
  list of wrapped shells and their real paths). It does not read, write,
  or transmit your `.ssh`, keychain, password manager, browser profile,
  or any other authentication store. The threat model is explicitly that
  **the human holds the credentials** — the agent reaches through
  cterm's local broker rather than opening its own connection.

## What the Claude plugin handles

The `plugin/` folder in this repository is a Claude plugin that bundles
a skill and three slash commands (`/cterm:exec`, `/cterm:sessions`,
`/cterm:tail`). The plugin ships **no binaries of its own**; every
command tells Claude to invoke the already-installed `cterm execute`
CLI, which talks to the local daemon described above.

The plugin itself therefore collects, stores, and transmits **nothing**.
Any data flow is between your Claude client, the Claude app or CLI's own
usual channels (covered by [Anthropic's privacy policy](https://www.anthropic.com/privacy)),
and your local cterm install.

## Network connections cterm initiates

- **`cterm update` / `cterm upgrade`**: downloads a release asset from
  `https://github.com/maifeeulasad/cterm/releases/...` and verifies it
  against the published sha256. GitHub (as the download host) has its
  own [privacy policy](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
- **`cterm doctor`**: queries the GitHub Releases API to compare your
  installed version against the latest tag.
- **Nothing else.** The daemon, shim, web UI, and plugin do not phone
  home.

## Third-party services

cterm links against third-party open-source libraries (listed in
[Cargo.lock](Cargo.lock)) and fetches release assets from GitHub. We do
not process your data through any third-party analytics, advertising,
error-reporting, or CDN services.

## Children

cterm is a developer tool. It is not directed at, designed for, or
suitable for children under 18.

## Changes

If this policy changes materially, the change appears as an ordinary
commit in this file's git history. Breaking changes to the data cterm
itself handles would also be called out in [RELEASE_NOTES.md](RELEASE_NOTES.md).

## Contact

Open an issue at [github.com/maifeeulasad/cterm/issues](https://github.com/maifeeulasad/cterm/issues),
or email `maifeeulasad@gmail.com`.
