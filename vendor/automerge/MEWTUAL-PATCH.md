# Pinned Automerge readonly history observer

This is a temporary additive patch of the published `automerge` **0.10.0** crate, not a new
Automerge wire format or an ingestion/security change. Mewtual's retained admin history reader
needs to enumerate the actual applied graph and dependency-blocked queue in bounded turns.
The upstream public alternatives either materialize all change metadata, reconstruct entire
changes, or close an AutoCommit transaction. This patch only borrows existing inventory storage.
Replace it with an equivalent reviewed upstream observer if one becomes available.

## Exact provenance

The complete 184-file crates.io archive was extracted without pruning. Its SHA-256 matches BOTH
original Mewtual workspace lockfile entries:

```text
crate:     automerge-0.10.0.crate
source:    registry+https://github.com/rust-lang/crates.io-index
SHA-256:   09b78abcbba93428b9465b26cb2816a5b4654cce507f099a84a8c1b311cb3633
VCS:       a4f584c86358dd07f83f36708573e1c8d1bd8161
VCS path:  rust/automerge
```

The published `.cargo_vcs_info.json`, `Cargo.toml`, `Cargo.toml.orig`, README, tests, nested
licenses and upstream Cargo.lock are preserved. The published archive omits its repository-root
license. `LICENSE.upstream` is the unchanged MIT license from the exact VCS revision:
[upstream license](https://github.com/automerge/automerge/blob/a4f584c86358dd07f83f36708573e1c8d1bd8161/LICENSE).
No dependency versions or crate feature definitions were changed. Both Mewtual workspaces patch
crates.io to this SAME directory; their lockfiles differ only by Automerge becoming a local source.
The vendor directory is excluded from Mewtual workspace membership, so normal full-suite commands
do not silently turn every upstream integration test into a new Mewtual workspace test target.

## Complete patch inventory

Only these upstream source files differ from the archive:

- `src/lib.rs`: declare the private `history` module and re-export its opaque public token.
- `src/automerge.rs`: one nonserialized instance-identity field and initialization in all three
  direct construction paths (`new`, `new_with_encoding`, `from_parts`). Its private custom Clone
  makes a document clone/fork a fresh instance without changing copied document content.
- `src/change_graph.rs`: a private borrowed slice over the existing applied hash vector.
- `src/change_queue.rs`: a private borrowed slice over the existing queued Change vector.
- `src/autocommit.rs`: readonly ROOT-value metadata forwarding with explicit isolation refusal.
- `src/op_set2/op_set.rs`: declare the private metadata-only ROOT lookup module.

New files are this note, `LICENSE.upstream`, `src/history.rs`, `src/history/tests.rs`,
`src/value_metadata.rs` and `src/op_set2/op_set/value_metadata.rs`.
The history module adds four readonly inherent methods on BOTH Automerge and AutoCommit:
`applied_change_hashes`, `queued_changes`, `has_applied_change`, `history_instance_id`.
The metadata modules add `root_value_metadata` and bounded borrowed `root_bytes` on both types,
plus four fixed metadata/limit/error types. The ROOT lookup uses the same property range and indexed last-visible winner as current
unisolated get, while reading only visibility, action and type/length columns. It never constructs
an Op, a conflict vector or actor identity, and never decodes/copies raw scalar bytes. Explicit
operation/key-column limits precede lookup. Physical object-counter/object-actor/key/visibility/action/type metadata slab counts
are checked in O(1) before key-byte summation/lookup, and raw slab count is checked before a nonempty
raw read, each at most checked `max_ops + 1`. Padding slabs cannot fund a hidden scan. Isolated
AutoCommit views refuse before clock creation.
Existing constructor bodies initialize the new process-local identity. No parser, serializer,
encoding, ingestion, verification or admission rule changes. No unsafe code, private-layout
assumptions, change reconstruction or all-history metadata collection is added.

## Reader and lifecycle contract

Both slices borrow existing storage in constant time. Membership uses the existing applied-graph
hash table. Applied order is internal insertion order, not timestamps or canonical authority.
Queued Changes retain actual raw bytes accessible through upstream `Change::raw_bytes`; they
are not original signed envelopes. Applied hashes do NOT promise retained original raw frames.
This patch deliberately does not wrap `get_change_by_hash`, reconstruct applied changes, decode
operations, attribute authors or infer membership from Automerge actor IDs.

AutoCommit forwarding takes `&self` and NEVER calls `ensure_transaction_closed`. Pending local
operations are omitted: a reader needing a complete committed observation must check
`Transactable::pending_ops() == 0` and refuse a pending transaction without committing it.
Queuing, mutation and later dependency arrival can reorder or drain the orphan slice; there is
NO pending-queue MVCC or stable queue offset across mutations. Callers hold borrows only within
one readonly observation and must revalidate their own revision/namespace/policy before resuming.

`HistoryInstanceId` is an opaque nonserialized Arc-allocation identity. Tokens cloned from one
instance compare equal and retain the allocation, preventing replacement ABA by address reuse.
New/load/clone/fork instances differ even with identical counts/content; ordinary mutation and
moving the original document keep its identity. The token is not a mutation revision, a durable
epoch, a permission or a guarantee that no history changed. It makes no full-ledger completeness
or production-admin authorization claim.

`root_value_metadata` addresses a distinct trap: upstream `get` returns an owned legacy ScalarValue,
so checking a Bytes/String length AFTER get is too late to bound allocation. Metadata alone is also
NOT sufficient preflight for a later get: get decodes losing conflicting scalars and clones an actor
ID. For bounded Bytes reads use `root_bytes`, which checks winner kind and byte cap BEFORE touching
only that winner's raw slice. No losing scalar, actor ID or content copy is read/created. Bodies
cannot change while that immutable slice is borrowed. Retained operation/key-metadata saturation
returns WorkLimit, and isolated views return UnsupportedIsolation. Neither is permission to modify
history or fall back to an unbounded getter. Pending local operations remain open; the host supplies
its committed-only guard.

## Authored regression tests

`src/history/tests.rs` covers pending-transaction rollback after every getter, pointer-identical
borrowed graph/queue storage, exact unsigned orphan bytes after a real save/load, queue-to-applied
transition on the missing parent, fresh new/clone/fork/load identity, and stable identity on moves
and ordinary edits. Incremental load also pins upstream's existing distinction between replacing
an empty document and merging into a populated one. The primary integration agent owns compilation
and all required suites.
The vendor is not a workspace member. Run its focused upstream-style unit tests with
`cargo test --manifest-path vendor/automerge/Cargo.toml --locked --lib history::tests`, then the
affected replication tests and both full Mewtual workspaces before handoff. No execution is implied
by the presence of these tests.

Nine metadata/borrow regressions in `src/op_set2/op_set/value_metadata.rs` cover huge Bytes and wrong-type
String without any raw-value-column access (the test removes that private column), normal get kind
and size equivalence, current conflict winners in opposite arrival orders, work/key refusal, and
rollbackable pending operations plus explicit isolation refusal. Exact borrowed winner pointer
identity is pinned with a retained huge losing String and huge winning actor, plus unchanged saved
history. Benign get equivalence, missing raw storage, zero-length Bytes and physical padding slabs
have explicit cases, including a one-operation huge-key refusal.
The focused selector is
`cargo test --manifest-path vendor/automerge/Cargo.toml --locked --lib value_metadata::tests`.
