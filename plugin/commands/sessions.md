---
description: "List the sessions the cterm daemon currently knows about."
---

The user wants to see the terminal sessions currently tracked by the cterm
daemon. There is no dedicated subcommand for this yet, so use the gRPC
listing by running a tiny probe:

```bash
cterm execute --json --quiet 'true' 2>/dev/null
```

Then read the daemon's state from the `started` event's `session` field for
a fresh one, and tell the user to run the gRPC `ListSessions` RPC directly
if they need the full catalog. If they do want the full list, suggest:

```bash
cterm-web
```

and point them at `http://127.0.0.1:8088`, which renders every stored
session and its executions.

If you have a prior session id from an earlier `/cterm:exec` this turn,
include it in the summary so the user knows which one Claude is currently
attached to.

Keep the response one short paragraph — this command is a shortcut, not a
full dashboard.
