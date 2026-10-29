# Original Group and ledger custody prerequisite

29 October 2026. Correction draft, **UNRUN**. Query/Support's previous12 focused
passes remain exactly snapshotc5c8b7d8; they do not validate these changes.

The inherited1024-byte Group payment covered Group, two separately allocated empty
Existing ledgers, Runtime BTree nodes and Resources counter buckets without a full
invoice. Measured Windows bodies were304/216/184 bytes; the complete original
constructor/Arc/tree/hash invoice already exceeded1024 before Control7's fields.
The Group also dropped its embedded credit before ledger fields, and raw escaping
ledger Arcs could survive actual Group/Runtime return. A larger fee alone cannot
repair that ownership order.

Private GroupOwner and public opaque HelperOwner/SourceOwner replace those raw Arc
aliases. They dereference only the original body; no production raw Arc/Weak escape
exists. Every clone, Actor/registration, Runtime snapshot and transport Work retains
the SAME outside FundingOwner. It has no body/Clock/native backedge. Actual body
and Arc deallocation precede funding return. The final FundingOwner uses
Arc::into_inner on every drop, so the funding allocation/header is also freed before
its plain Credit refunds capacity. Helper/Source identity and state survive actor
replacement unchanged; identity itself grants no current authority.

Runtime installs a private map-retirement guard before invoking revocation wakers.
Its destructor structurally pops entries, retains the last payer while freeing the
empty BTree backing, then returns that owner. This order runs during first-waker
unwind as well as ordinary shutdown. Existing first-effect session/actor negatives
remain in place. Test Clock probes use only a hidden funded Weak observation to
avoid Clock↔Group cycles; they expose no raw Arc/Weak and retain the same payer
through that test-only header return.

The full production invoice includes Group/empty-ledger/Funding construction and
Arc headers, opaque owners, collection/retirement overlap, full sparse BTree nodes,
and complete Resources key/Counts buckets with old+new growth/shrink controls and
alignment. The candidate3KiB fee is guarded BEFORE payment or allocation and must
be measured; it will be raised explicitly if the complete graph exceeds it. No
formula or bound is trimmed to fit. Group/process byte and work/record caps remain
unchanged; the first retained Group infrastructure costs more even while V2 is OFF.

All three actual empty-counter removals (Credit drop, retained-byte release and
actual progress-stamp release) use one pure pool-locked helper. It preserves the
hasher/service horizon for survivors, shrinks sparse backing and replaces an empty
table using the same hasher before releasing the lock. No ambient RNG or new
group entry is introduced. Admission/diagnostic observers cannot enter between
logical refund and physical bucket return. Existing generic Resources APIs remain
unchanged.

Draft regressions pause actual Group/ledger destruction, hold escaped both-ledger
owners across real Runtime return, concurrently return final aliases and require
the fee until actual completion. A genuine request-authority revocation future
uses a one-shot panic waker to exercise Runtime unwind. Other tests cover guarded
constructor refusal, sparse/churn/empty bucket return, unchanged live service
horizon, pool-fence serialization and the pinned1.94.0 counter-capacity profile up
to1024 groups. Detached producer threads use RAII release and a sanctioned Clock
watchdog, without blocking joins or ambient sleeps.

Focused execution, actual revised invoice, strict lint and complete integration
remain pending. This does not activate V2 or complete neutral/round/Ready/RTC/audio
consumers, and makes no general guarantee for arbitrary reentrant custom transport
or signer callbacks.
