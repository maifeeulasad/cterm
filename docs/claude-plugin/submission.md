# Submitting to Anthropic's plugin directory

Tracks the public-directory listing for the cterm plugin.
[Issue #21](https://github.com/maifeeulasad/cterm/issues/21) is the running thread.

## Status

| Stage                         | Status      | Notes                                              |
| ----------------------------- | ----------- | -------------------------------------------------- |
| Plugin shipped in-tree         | **done**    | 0.5.0: `plugin/` validates with `claude plugin validate`. |
| Repository public              | done        | `github.com/maifeeulasad/cterm` is already public. |
| Submitted at `claude.ai/directory/manage` | **pending** | Needs a Max/Team/Enterprise plan to submit. |
| Automated validation + scan    | pending     | Runs on submission.                                |
| Human review                   | pending     | Required for a first listing.                      |
| Listed                         | pending     | Appears under Discover after review clears.        |

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
