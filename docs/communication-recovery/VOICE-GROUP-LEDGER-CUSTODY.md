# Original Group and ledger custody prerequisite

29 October 2026. Focused correction passes at production snapshot34336b2f:
rt authority11, Resources60, helper26, source32, Query/Support12, runtime42 and
route9. The revised actual Root transport tests pass69/69 (89.47s) at4b9d2241.
These are separate overlapping filters, not an aggregate unique total. Final
read-only adversarial review found no blocker/high/medium. Strict lint and complete
integration remain primary-owned pending gates.

The first actual invoice measured3106 bytes and correctly failed the candidate
3072 guard. The fee was explicitly raised to4096, preserving every inventory term.
The subsequent source-corrected H8 complete invoice measures3750/4096 and its
production preallocation guard passes. This fixes a real underinvoice as well as
the separate diagnostic mistake below; it is not merely a test correction.

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
to3750/4096; actual compiled confirmation of that revised guard passes. No
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

Focused regressions pause actual Group/ledger destruction, hold escaped both-ledger
owners across real Runtime return, concurrently return final aliases and require
the fee until actual completion. A genuine request-authority revocation future
uses a one-shot panic waker to exercise Runtime unwind. Other tests cover guarded
constructor refusal, sparse/churn/empty bucket return, unchanged live service
horizon, pool-fence serialization and the pinned1.94.0 counter-capacity profile up
to1024 groups. Detached producer threads use RAII release and a sanctioned Clock
watchdog, without blocking joins or ambient sleeps.

The broader Root transport run initially passed59 and failed10 old assertions
expecting refund at the last EXTERNAL alias return. A strong registry row is still
an actual owner. Updated tests run ordinary Root collect_expired_voice, which
prunes before its Clock/cadence test and is invoked by the real Actor loop as well
as ingress/preparation. They preserve exact zero/refund/attempt oracles and also
collect WHILE actual bytes/requests/preparations/responders remain held, requiring
those loans to remain charged. One newly added assertion initially overexpected
registry custody after a prior prune: upper cancellation had returned Gate
authority while the two real responders retained both Lower loans. It now requires
both loans after that prune and exact zero immediately on actual responder return.
The corrected69-test run passes without a production change or weaker payment.

Shared-target provenance matters: an older rt artifact from the native checkout
initially produced two route failures, although that checkout lacked the c57 rt
custody corrections. Touching only the unchanged local rt source forced a rebuild
of the local dependency chain; all9 route and11 authority tests then passed,
followed by the remaining focused filters. The failed logs remain retained. Final
integration must include the actual rt corrections and build one coherent checkout;
the warm cross-worktree artifact alone was insufficient evidence.

Execution used the shared native target, -j1 and one test thread. Commands were
`cargo test -p catcoms-rt --all-features --lib authority_tests -j1 -- --test-threads=1 --nocapture`
and the equivalent catcoms-sync command with each filter:
`voice_v2::resources::`, `voice_v2::helper::`, `voice_v2::source::`,
`admitted_control`, `voice_v2::runtime::`, `voice_transport::forwarding::tests`,
and `voice_runtime::tests`. Narrow formatting and diff checks passed. Strict lint,
complete backend/frontend suites and TCP integration are still primary-owned gates.

This declared application graph is not an allocator trace or proof that every
generic Resources caller sponsors its own usage. It does not activate V2 or
complete neutral/round/Ready/RTC/audio consumers, and makes no general guarantee
for arbitrary reentrant custom transport or signer callbacks.
