# Session titles and labels

As of #28, a session can carry two pieces of caller-supplied metadata
that ride alongside its id but don't affect scheduling, storage identity,
or how the shell runs:

- **`title`** — a short, human-readable one-liner. Shows in `cterm-web`
  as the session's heading.
- **`labels`** — a small `BTreeMap<String, String>`, intended for
  shape-light tags like `env=prod` or `ticket=ABC-123`. Not indexed.

Both are optional. Sessions written before this feature deserialize
unchanged (both default to empty, and empty fields are omitted on write,
keeping on-disk JSON byte-for-byte stable).

## Set from the CLI

```bash
# Open a new session with a title and a couple of labels:
cterm execute \
  --title "deploy webapp" \
  --label env=staging --label ticket=ABC-123 \
  'kubectl apply -f manifest.yaml'
```

`--title` and `--label` are rejected when combined with `--id`, because
metadata is applied at open-time only in this release. A dedicated
"update session metadata" RPC is a follow-up if you need it.

## Set from gRPC

`RunRequest` carries two new fields (proto3 default empty):

```proto
message RunRequest {
  // ... existing fields ...
  string title = 8;                      // only honored when session == ""
  map<string, string> labels = 9;        // only honored when session == ""
}
```

`Session` on `ListSessions` grows matching return fields:

```proto
message Session {
  // ... existing fields ...
  string title = 7;
  map<string, string> labels = 8;
}
```

Older clients that don't populate `RunRequest.title` / `.labels` keep
working; newer daemons just see empty values. Older daemons that don't
know the fields ignore them (proto3 unknown-field behavior).

## Limits

The daemon rejects out-of-bounds metadata with `InvalidArgument` at the
gRPC boundary (never silently truncated):

| Rule                                       | Limit (see `cterm_core::session`) |
| ------------------------------------------ | --------------------------------- |
| Title length                                | `MAX_TITLE_BYTES` (256)           |
| Title content                               | No control characters             |
| Labels per session                          | `MAX_LABELS` (32)                 |
| Label key / value length (each)             | `MAX_LABEL_BYTES` (128)           |
| Label key must be non-empty, no `=`, no control chars | — |
| Label value: no control chars               | —                                 |

The control-char rule is a hardening choice: a title or label value is
rendered in the webview, and we don't want a `\x1b]0;...` OSC title
sequence (or an arbitrary ANSI escape) sneaking through a UI surface.

## What this release does not do

- **Update metadata after open.** Only set at the first `Run` that opens
  a session. A follow-up RPC (`UpdateSessionMetadata`) can land once we
  see a need.
- **Query/filter by label.** `ListSessions` returns all sessions; filter
  client-side for now.
- **Validate label key charset beyond "no `=`, no control chars".** We
  deliberately don't impose a k8s-style [a-z0-9-.] rule so people can
  use whatever characters match their own tagging conventions.
