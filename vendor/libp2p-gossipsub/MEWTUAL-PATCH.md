# Subscription hello and payload framing compatibility

This is a temporary correction of the actual patched gossipsub 0.50.1 runtime.
Replace it with an equivalent reviewed upstream release when available. It does
not change signatures, protocol names, PRUNE/GRAFT defenses or authorization.

## Provenance and licence

All 46 files from the complete crates.io archive were retained before editing.
Archive SHA-256, matching the original Mewtual shipping lock entry:

`6c28d7256540bd1a5df45cf6d7a7d8c58e913ad63ccfaa2f97983ec077b2794a`

The packaged VCS metadata reports commit
`f2244686b3ad217d6cb62e5703e6d7fa90e20d99`, `protocols/gossipsub`. The exact
repository-root licence URL returned 404 during this work, so it was not used as
evidence. The archive contains full MIT notices in its source files but omits a
root licence file. `LICENSE.upstream` reproduces the exact packaged `src/lib.rs`
MIT notice. Every original per-file notice remains. The upstream MIT licence
remains in force; the original library is not relicensed as Mewtual code.

`MEWTUAL-UPSTREAM.json` records the archive checksum and original hash of every
packaged file. The original `Cargo.toml.orig` is retained as provenance, while
the normalized `Cargo.toml` is the active manifest. Both Mewtual workspaces patch
the SAME directory, with actual production use pinned to 0.50.1. No inactive old
gossipsub dependency is retained as a substitute for fixing the runtime.

## Exact production corrections

Only these existing source files change:

- `src/lib.rs` declares the private `subscription_chunks` module.
- `src/behaviour.rs` partitions the captured initial subscriptions using the
  smaller of the configured protobuf-payload and aggregate SUB/control budgets.
  Chunks enter the same original priority FIFO consecutively within the same
  synchronous connection callback. A later unsubscribe cannot overtake a remainder.
- `src/protocol.rs` bounds the **declared first payload** before waiting for its
  body, then scans exactly that payload. It preserves publish-count and aggregate
  SUB/control limits. Prefix bytes and a following frame are not the first payload.
  Malformed varints and oversized incomplete declarations refuse immediately.

New `src/subscription_chunks.rs` measures each entry once through the actual
protobuf conversion, including explicitly present false flags and field wrappers.
It moves the same topic String through measurement and back, without another
topic-string clone. Checked accumulation is linear; a boundary entry is retained
already measured. Every fitting topic/flag/order survives. Oversized singletons
produce explicit errors and warnings, and are not announced. There are no empty
chunks or new handler continuation/backpressure states.

The original priority queue is unbounded. This patch bounds individual wire
payloads, not the complete outbox, local subscription inventory or process RSS.
The application's finite count checks are applied after decoding; its byte cap
remains the outer input bound. No complete upstream TLS/RNG memory claim follows.

The active manifest adds the used unsigned-varint 0.8 decoder already used by
prost-codec, plus a standalone test workspace. A missing QuickCheck dev dependency
is added because the published protocol unit module imports it. The standalone
test lock is consequently updated. These test dependencies are not shipping
Mewtual workspace members. No upstream test is deleted, ignored or loosened.

## Failure and evidence

Mewtual hex-encodes its transport topics. Its unchanged 2,000-topic acceptance
therefore creates a 112,000-byte (about 109.4 KiB) initial `SubscribeMany`, exceeding the
64 KiB limit when sent as one RPC. The unpatched upgraded runtime fails actual
delivery at 2,000 topics while 100 and 101 work. Raising the cap or shortening
topics would hide that defect. Byte-bounded announcements restore the actual
delivery contract; all five Net gossip acceptance tests pass (0.45s).

The old parser also rejected a buffered read containing two individually valid
frames, and did not immediately reject oversized declarations without their body.
The six dedicated regressions pass (0.01s): ordered byte-bounded reconstruction,
flags/UTF-8/varint boundaries, oversized singleton progress, exact payload sizes,
coalesced frames, partial tails, malformed/oversized prefixes and preserved
aggregate/publish limits. Run them explicitly; ordinary workspace suites exclude
third-party package tests:

```sh
cargo test --locked --manifest-path vendor/libp2p-gossipsub/Cargo.toml --lib mewtual_
```

Read-only design and implementation review found no remaining production blocker;
documentation follow-ups are included here. The complete 159-case library suite
passes (8.80s), as do both shipping dependency/license audits. The pinned
libp2p0.56/gossipsub0.49.5 old-peer probe passes actual bidirectional RR and
near-limit signed gossip after queuing2,000 subscriptions, plus controlled live-
process Mesh withdrawal. This is phased functional evidence: an earlier
overlapping bulk probe timed out, with no established production cause. Shipping
native/frontend/full suites and strict lint remain required. Chat V2 remains disabled.
