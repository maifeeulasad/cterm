# Submitting to Anthropic's plugin directory

Tracks the public-directory listing for the cterm plugin.
[Issue #21](https://github.com/maifeeulasad/cterm/issues/21) is the running thread.

## Status

| Stage                                     | Status       | Notes                                                                                                                                                 |
| ----------------------------------------- | ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Plugin shipped in-tree                    | **done**     | 0.5.0: `plugin/` validates with `claude plugin validate`.                                                                                             |
| Repository public                         | done         | `github.com/maifeeulasad/cterm` is already public.                                                                                                    |
| Submitted at `claude.ai/directory/manage` | **done**     | 0.5.0 submitted against commit `a51f027`; tracked branch `main`, plugin path `plugin`.                                                                |
| GitHub push webhook                       | **done**     | Configured so future `main` pushes that touch `plugin/` are picked up within minutes rather than the 6-hour poll.                                     |
| Auto-publish passing versions             | **enabled**  | Later versions that pass checks go live without a human click. First version still needs a human publish.                                             |
| Automated validation + scan               | **done**     | Passed with 3 warnings (see follow-ups below). Portal scan: "No local code execution."                                                                |
| Human review                              | pending      | Required for the first listing.                                                                                                                       |
| Listed                                    | pending      | Appears under Discover after review clears.                                                                                                           |

## Follow-ups from the 0.5.0 submission scan

The portal flagged three non-blocking warnings on commit `a51f027`. We
submitted anyway on the "release first, polish second" plan; tracked here
so they don't get forgotten in a 0.5.x bump.

| # | Warning                                                                 | Where                                                                                 | Fix                                                                                                                                                                                                                                                                                      |
| - | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1 | **No icon**                                                             | `plugin/.claude-plugin/icon.png`                                                      | **Done post-submission** in a follow-up commit (1024×1024 PNG derived from `assets/cterm.webp`). The portal warns the listing icon may be locked on first submit, so for the current submission this may not take effect — it's in place for any future resubmission and for local installs. |
| 2 | **Unrecognized field in `plugin.json`** (`documentationUrl`, `supportUrl`) | `plugin/.claude-plugin/plugin.json`                                                   | The manifest reference says these are directory-only fields for the listing. The portal is now flagging them as unknown — either they got renamed, or they're accepted only once a plugin is Verified. Decide whether to drop or keep based on what the portal returns on the first scheduled re-scan. |
| 3 | **`allowed-tools` beyond Read/Grep/Glob/LS accepted only from Verified organizations** | `plugin/commands/exec.md` (and the pattern applies to `sessions.md` / `tail.md` too) | Not refused today, but will be later. Either drop the `allowed-tools` frontmatter from the three command files (Claude Code will simply prompt the user on each `Bash(cterm:*)` call — acceptable), or narrow it, or pursue Verified-organization status. Default plan: drop it in 0.5.1. |

Also while we're there:

- **Frontmatter value may be misread** in `commands/exec.md` — the lint
  flagged a quoting issue in the description/argument-hint lines. Fix as
  part of the `allowed-tools` cleanup in 0.5.1.

Tracked in [issue #21](https://github.com/maifeeulasad/cterm/issues/21)'s
follow-up discussion for 0.5.1.

## Pre-submission checklist

Anthropic's docs enumerate it under
[Submit a plugin](https://claude.com/docs/plugins/submit); here is where
each item lives in this repo.

- **`plugin.json`** — [plugin/.claude-plugin/plugin.json](../../plugin/.claude-plugin/plugin.json).
  Carries `name`, `displayName`, `version`, `description`, `author`,
  `homepage`, `repository`, `license`, `keywords`, the directory-listing
  URLs (`documentationUrl`, `supportUrl`, `privacyPolicyUrl`,
  `termsOfServiceUrl`), and the components (skill, three commands).
- **`README`** — [plugin/README.md](../../plugin/README.md). The directory
  shows the first paragraph in the listing.
- **`LICENSE`** — the repo's GPL-3.0 file covers the plugin. The listing
  reads the SPDX id from `plugin.json`.
- **Validator**: `cd plugin && claude plugin validate .` must pass. The
  portal re-runs this plus a security scan on each submission.

## Submitting (when ready)

1. Open [`claude.ai/directory/manage`](https://claude.ai/directory/manage)
   and select **Submit new > Plugin bundle**.
2. **Source**: `maifeeulasad/cterm`, Plugin path `plugin`, Branch
   `main` (or pin to a tag, e.g. `v0.5.0`).
3. **Validate** — fix findings in `plugin/` and re-push.
4. **Listing details** — cross-check what the portal reads back against
   what's in [plugin.json](../../plugin/.claude-plugin/plugin.json).
5. **Data handling** — the plugin does not read or store personal data of
   its own; it records only what the user runs under their own shell.
   Nothing is sent to third parties by the plugin itself.
6. **Compliance** — the four acknowledgements, plus the contact email on
   the GitHub profile.
7. **Review and submit** — keep "GitHub push webhook" on so a 0.5.x bump
   publishes a new version through the directory without re-submitting.

## After submission

Each new commit on `main` under `plugin/` triggers a version scan. If a
version is held for review, the reasons appear on the submission's
Versions tab; fix and re-push.

When a listing is live:

- Make sure the repository stays public (the listing goes offline if the
  portal can no longer fetch it).
- Keep `version` in `plugin.json` monotonic across commits, so Claude Code
  users don't get silently downgraded.
- When the daemon's wire format changes (the `cterm.v1.*` proto), bump the
  plugin's major version too: a client on an old plugin will keep calling
  the old flags on the new binary.

## Why we might *not* list it in the directory

Open question for the 0.5.x line: the directory review bar is for plugins
that are generally useful out of the box. cterm's plugin only becomes
useful when the user has also installed the native binaries and started
the daemon — otherwise every command is a connect-refused. We may decide
to keep the plugin hosted here (installable via `/plugin marketplace add
maifeeulasad/cterm`) and skip the directory submission entirely.

Both paths stay open; this file gets updated when we pick one.
