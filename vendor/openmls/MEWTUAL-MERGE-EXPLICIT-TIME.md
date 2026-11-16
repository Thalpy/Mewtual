# Explicit timestamp for pending-commit merge

This separate local patch adds
`MlsGroup::merge_pending_commit_with_time(provider, now)` to the same pinned
OpenMLS 0.9.2 package. Package provenance and the original validation hooks are
recorded in `MEWTUAL-PATCH.md`.

The new method runs the same pending/staged/commit merge bodies as the legacy
method. Only the replacement of current message secrets receives the supplied
timestamp. The legacy path continues to call the original replacement helper,
which reads `SystemTime::now()` at its original statement. This changes no wire
or persistence grammar, and does not alter receive, join, expiry, new-group or
default KeyPackage clocks. Native and wasm time types match the existing SDK.

The private operation owner must obtain the supplied timestamp from its trusted
local Clock and cache it with the original operation before SDK entry. The API
does not authenticate that provenance. It supplies no Source, membership, UI,
storage, resource or publication permission.

Merge failures retain their existing partial-effect semantics. In particular,
the SDK consumes its pending state before the first fallible storage write. A
refused write can leave the in-memory group Operational while stored rows still
describe the pending operation. A subsequent `Ok(())` from the no-pending path
therefore proves no successful original merge. The owning private operation must
keep its failed/uncertain state and cached outputs, and must not retry Add or
claim a saved result from that no-op. This hook is not a recovery algorithm.

`MlsGroup::text_add_merge_time_layout_bytes()` reports only the numeric inline
timestamp-selector layout. It is not the complete operation's allocation bound
or a resource debit. Native parallel tree work and all transitive SDK/provider
allocations remain separately reviewed and paid obligations.

The primary's focused regressions inspect the actual stored timestamp, genuine
Commit/Welcome output and joiner key agreement, a cold snapshot round-trip,
byte-exact operational no-op, and both legacy/explicit first-write refusals.
Compilation and runtime results are recorded in `docs/HANDOVER.md` only after
the reviewed production patch is integrated and those tests execute.
