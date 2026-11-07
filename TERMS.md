# Terms of Service

Last updated: 2026-11-07.

## Short version

cterm is open-source software released under the GPL-3.0-or-later. You
may use, study, modify, and redistribute it under that license.
**There is no service** behind these terms — the maintainers host no
backend, API, or hosted instance. The "service" each cterm install
provides is one you run yourself, on your own machine, under your own
control.

## The software

- **License**: GNU General Public License, version 3 or later. The full
  text is in [LICENSE](LICENSE), and the GPL's own disclaimers of
  warranty and limitation of liability apply to your use of cterm.
- **Distribution**: official prebuilt binaries are the files published
  to [GitHub Releases](https://github.com/maifeeulasad/cterm/releases)
  for this repository. Verifying downloads against the published
  `sha256` is your responsibility (and is what `cterm update` does for
  you automatically).

## Your responsibilities

By using cterm you confirm that:

- You control the host it runs on, the shell sessions it wraps, and the
  commands that get executed through it. cterm does not authenticate
  commands or check who issued them; that is the surrounding system's
  job.
- You will not use cterm to carry out activity that violates law
  applicable to you, your employer's acceptable-use policy, the terms
  of services you reach from inside the recorded shell, or the rights
  of others.
- If you operate cterm on behalf of an organization, you have authority
  to accept these terms on that organization's behalf.
- You keep the recordings stored in your local `cterm.db` only as long
  as your own retention policy allows. cterm will record whatever you
  run; deciding whether a given command *should* be recorded is a
  policy question for you, not for the tool.

## No warranty

cterm is provided "as is", without warranty of any kind, express or
implied. See the GPL-3.0 text in [LICENSE](LICENSE) for the full
disclaimer. The maintainers are not liable for lost data, lost shells,
mis-run deployments, leaked secrets that passed through a wrapped shell,
or any other damages arising from use of the software.

## The Claude plugin

The plugin in [`plugin/`](plugin/) is covered by these same terms and
the same GPL-3.0 license. It installs no binaries of its own: its slash
commands drive the local `cterm execute` CLI, which talks to your local
`cterm-daemon`. Running the plugin inside Claude is subject to
[Anthropic's own terms](https://www.anthropic.com/legal/consumer-terms)
for the Claude surface you are using (claude.ai, Claude Code, Cowork,
or the desktop app) — these cterm terms do not modify them.

## Changes to these terms

Material changes appear as ordinary commits in this file's git history
and, when they ride a release, in [RELEASE_NOTES.md](RELEASE_NOTES.md).
There is no mailing list; use the repository's watch feature to be
notified.

## Contact

Open an issue at [github.com/maifeeulasad/cterm/issues](https://github.com/maifeeulasad/cterm/issues),
or email `maifeeulasad@gmail.com`.
