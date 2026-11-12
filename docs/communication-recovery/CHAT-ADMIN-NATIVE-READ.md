# Native retained-ledger read boundary

The two main-window commands are `get_admin_chat_ledger({server,cursor,limit})` and
`get_admin_chat_ledger_detail({server,rowId,expectedRevision,frameOffset})`. Their native router
owns the literal `COMMANDS` inventory; ordinary generated handlers remain inside `with_commands`.
Both names are also in native command policy and the main capability. No new all-member body
projection or fallback through `get_moderation` exists.

Tauri 2.12.5 parses an async command's arguments inside its scheduled future. A custom async
CommandArg therefore cannot capture the original dispatch generation. These two commands use a
synchronous invoke handler instead. Tauri's real Webview `on_message` resolves exact-command ACL
before `run_invoke_handler`; the router additionally refuses absent resolved ACL, non-main window
or non-main webview. It reads only closed, bounded JSON arguments and captures the current native
generation, original account/registry-row observers and exact actor/instance before spawning.
Registry contention is `busy`, never delayed permission to adopt a replacement row.

Native owns a single active admin-server scope, a shared 64 MiB actor-index metadata budget and at
most 8 in-flight calls. A new server pane retires the previous scope. Explicit lock retires it before
the first cleanup await. Account and registry-row retirement remain independent parents, so an
unlock or numeric-ID reuse cannot revive the old scope. The real Actor checks current membership,
full-key reader role and scope before cursor/revision metadata or privileged originals. These role
reads retain the documented locally-known roster freshness limitation (R7).

Pure read waits select against that original scope's cancellation, including stale-revision recovery.
They release their reply receivers and permits even if the old Actor is blocked at an unrelated
durable-send Ready barrier. A server-pane switch retires its old scope before trying the global read
permit, so saturation cannot prevent that cancellation or starve another healthy server indefinitely.
The cached native scope owns no strong Actor sender. Dropping a genuine registry row and its in-flight
reads can therefore end the old Actor without waiting for a later ledger read or account lock.

Replies are encoded through a writer capped at 1 MiB; a decoded projection too large after JSON
escaping falls back to an explicitly unavailable projection while preserving the exact raw chunk.
Decoded counters, sequence numbers, operation IDs and 64-bit scalar values become canonical text,
never rounded JavaScript numbers. Mark-containing changes are explicitly unsupported by this
projection because the current DTO cannot preserve mark names/expansion. Exact original bytes remain
readable in 64 KiB chunks. Raw offsets and lengths are exact JavaScript integers up to 2^53 - 1,
so a combined signed frame is not restricted to a single field's u32 length. Unsupported historical
actor widths or zero operation counters are not presented as a decoded projection. No Automerge
ExpandedChange is directly serde-serialized to the frontend.

Final publication obtains the original UI-generation commit guard, then the actual native registry
guard, checks the original actor/instance and complete cancellation observer and resolves the
pre-encoded Tauri response synchronously without an intervening await. Lock cannot report completion
ahead of a reply already admitted under that guard. If lock/replacement wins, only a closed unavailable
reply is emitted. A stale reset gets its revision from the same Actor's actual current captured view
under the same original scope; no fabricated reset token or caller revision is substituted.

This is the retained signed-original vertical. Every page/detail reports incomplete coverage;
typed/owner evidence is not active and unattributed graph inventory remains unknown. It is not
full Chat V2 activation or a claim to possess every message ever authored. Native tests and the
primary integration gates must pass before this implementation is handed off as validated.
