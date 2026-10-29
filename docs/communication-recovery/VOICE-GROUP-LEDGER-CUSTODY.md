# Original Group and ledger custody prerequisite

29 October 2026. Correction draft. Runtime42 tests PASS; Resources57 tests PASS
and1 capacity-profile diagnostic FAILS atd989ad21. The H8 correction and new churn
regressions below are **UNRUN**. The first actual
complete-invoice test measured3106 bytes and correctly failed the candidate3072
guard. The fee is explicitly raised to4096, preserving every inventory term.
Query/Support's previous12 focused
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
alignment. The measured3106-byte H4 graph now has a4KiB fee guarded BEFORE payment
or allocation. The source-corrected H8 allowance increases the COMPLETE invoice
to3750/4096; actual compiled confirmation of that revised guard is pending. No
formula or bound is trimmed to fit. Group/process byte and work/record caps remain
unchanged; the first retained Group infrastructure costs more even while V2 is OFF.

All three actual empty-counter removals (Credit drop, retained-byte release and
actual progress-stamp release) use one pure pool-locked helper. It preserves the
hasher/service horizon for survivors, shrinks sparse backing and replaces an empty
table using the same hasher before releasing the lock. No ambient RNG or new
group entry is introduced. Admission/diagnostic observers cannot enter between
logical refund and physical bucket return. Existing generic Resources APIs remain
unchanged.

The failed first profile test made a diagnostic mistake: it treated changes in
reported HashMap capacity as physical reallocation. Matching local primary code
shows `RawTable::capacity = items + growth_left`, a lower bound that tombstones
can change without resizing. This is distinct from the REAL H4 underinvoice:
`reserve_rehash_inner` may grow early when DELETED controls exhaust growth, at
n>7B/16, allocating up to2B alongside oldB. Thus old+new3B/n<48/7<8; the immediate
sparse shrink has the same bound. Small4/8-bucket native tables cannot retain
DELETED controls and need at most4n overlap. H8 retains BOTH tables' control-byte
and alignment terms. Every-removal shrink and one-at-a-time insertion are part
of this proof, and empty backing stays under its final payer and pool lock.

This proof is pinned to [Rust1.94.0](../../rust-toolchain.toml) and the matching
[hashbrown0.16.1 raw implementation](https://github.com/rust-lang/hashbrown/blob/1876e4f02708b93903d55ef598f68e82a826518f/src/raw/mod.rs):
`capacity_to_buckets`, `shrink_to`, `capacity`, `reserve_rehash_inner`, and `erase`.
The installed Rust hashbrown rlib embeds0.16.1 source paths, and the corresponding
cached source's `.cargo_vcs_info.json` identifies that exact commit. Inspection was
offline. Compiler/target updates must revisit the allocation model.

The corrected monotone diagnostic derives minimum raw buckets after
`shrink_to(len)` rather than from logical capacity. A separate actual Resources
churn test grinds bounded keys against the EXISTING hasher, preserves dense
DELETED controls and inserts into the untouched half-table. It must witness
reported capacity112 at47 rows, proving raw growth64→128: old+new192 exceeds
H4's188 and remains within H8's376. The structural resize proof is not an
allocator trace; actual sparse/empty backing and admission-fence tests remain
separate execution oracles.

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
