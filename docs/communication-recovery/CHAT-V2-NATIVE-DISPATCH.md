# Native text dispatch (implementation in progress)

Chat V2 remains disabled and incomplete. PR #124 adds original-runtime member
save/read operations. This continuation connects real native server rows and
the frontend to that path. No native factory or successful usable-server result
exists yet.

## Original per-row fence

`NativeChatSourceOwner::close_handle` issues an opaque handle to its original
counter. It has no Source/Host/issuer backlink or positive key permission.
Calling close is the existing SC atomic fence; it performs no notification,
allocation, wait or payer release. Dropping a handle closes nothing. Retained
handles preserve the original counter's trusted custody; the actual Root
producer supplies fee-only RuntimeMetadata, without an SDK/physical reference.
Handle disposal still belongs outside native guards.

Root captures that handle before moving its real SourceOwner into the canonical
Entry. Its one paid runtime selection caches a clone and exposes only
`close_original_cached`. Native row retirement can therefore stop new primitives
without borrowing the runtime, Source book or issuer. Existing whole-owner
retirement and actual key/loan/SDK/TCP drainage remain separate requirements.
No usable Leave or durable Retired transition is implemented by this fence.

Five new RT regressions cover original prepared purposes, lock/unlock, admitted
Ends, another source, passive payer lifetime and the exact Source-counter CAS
with no notification under a guard. A real two-row NativeRequired Root test
preborrows the first runtime and holds its actual book while closing its cached
counter, then requires a real signature from the other row. UI credentials in
these Root fixtures remain synthetic. All 32 RT lifecycle cases passed, as did
all five selector cases (27.99 seconds) and the genuine runtime-loan case
(5.79 seconds). The initial reused 27-case RT binary was rejected as stale;
only the current 32-case execution after package-specific cleanup is evidence.
The full RT package also passed all 74 cases, and the complete affected Source
cohort passed all 79 cases (303.28 seconds), zero failures/ignored. Strict RT
Clippy passed (2.51 seconds). Full Root/native/frontend validation of this
continuation and actual native integration remain pending.

## Remaining warm vertical

The existing Legacy shutdown barrier owns both UI commit and the entire server
table while native Sources drain. Native final preparation is counted work:
never await that UI guard with such a preparation alive. Final admission must
try the actual owned guard, returning/dropping whole preparation outside guards
on contention before any wait/retry. The real typed-table shutdown must also
avoid locking the already-held table from inside that barrier, using an
explicitly scoped native close or splitting/revalidating the barrier. These are
implementation requirements, not completed native acceptance.

The actual server table needs explicit Legacy and TextV2 variants, with actor
selectors refusing TextV2 before effects. Real founder/Create/own-history routes
must capture original ACL, UI generation, authenticated mount and exact row
before scheduling; retain actual owned native commit custody through the file
job; and fund serialization separately from the paid Root page. Founder and
member retries preserve their exact token/settings/body. SavedLocal alone
does not prove peer receipt or provide a message ID; same-request reconciliation
recovers the actual saved action/ordinal.

The frontend needs its real backend DTO branch: native founder intent has a
stable 32-byte ID, member intent has a 32-byte token, and stored own-original
pages cannot masquerade as complete group history. Successful native founding
must not call the current unsupported shared-name/livery writer afterward.

Native boot currently configures one member-outbox context. The real multi-row
consumer must choose and test a resource policy that permits saves in both
rows. No two-row-save or production-capacity claim follows from this slice.

Joined/Add production sources, multi-member archive, transport, full projection
and admin ledger, operational cold restore and combined 20–50-member acceptance
remain open after this warm local vertical. Physical two-device acceptance is
not claimed.
