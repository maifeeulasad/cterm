---
description: Replay a stored execution and then follow it live if still running.
argument-hint: <execution-id>
allowed-tools: Bash(cterm:*)
---

The user wants to watch an existing execution — either one a human started
in a wrapped shell, or one Claude ran earlier through `/cterm:exec`. The
argument is the execution id (e.g. `exe-…`).

There is no first-class `cterm tail` subcommand yet in 0.5.0, so for now
read the stored chunks through the gRPC `StreamExecution` RPC. The
pragmatic shortcut is to point the user at `cterm-web`, which renders the
full history and keeps streaming while the execution is still running:

```bash
cterm-web
```

at `http://127.0.0.1:8088/executions/$ARGUMENTS`.

If the user insists on CLI output, tell them the native `cterm tail` is
tracked as a follow-up (we wired `cterm execute` first in 0.4.1), and that
`grpcurl -plaintext -d '{"execution":"'$ARGUMENTS'","replay":true}'
127.0.0.1:50051 cterm.v1.Terminal/StreamExecution` is the escape hatch
until then. Don't offer to run `grpcurl` without checking whether they
have it installed.
