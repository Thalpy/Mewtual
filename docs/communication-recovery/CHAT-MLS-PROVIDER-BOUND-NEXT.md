# Actual MLS provider backing-allocation bound

## Source continuation (5 November; compilation and tests paused)

The narrow source correction is now drafted and under actual-diff review: exact upstream vendor,
explicit RandomState, actual allocation_size under the same read guard, independent output/backing
caps and opaque canonical fee threading. The source also checks actual key/value Vec capacities
against a separately paid local policy cap of twice encoded allowance; this is not a claimed SDK
allocation guarantee. New refusals preserve the provider without shrinking or compacting it.

Static Rust formatting passes. New regressions are authored, not compiled or executed. Both lock
updates and all validation remain primary-owned and currently deferred by user instruction.
The generic cold Server::restore and SDK nested JSON/group heap expansion remain separately open;
the encoder correction alone must not be described as a complete bounded restore or activation.
The provider allowance covers private initialization/restore overlap only. Active OwnedOwnerServer
memory requires separate host runtime admission before production activation; transient reservation
release after successful handoff is not a whole-lifetime provider memory bound.

Proposed correction to the OPEN work gate in canonical kernel 72c790d5. This document is not source
implementation, dependency approval or production activation. The current bounded helper correctly
limits encoded output under the SAME read guard but cannot infer raw scan work from std HashMap
reported capacity after every deletion pattern. Do not erase that limitation from the handover.

## Smallest sound route found by actual source trace

The already-locked hashbrown 0.16.1 exposes SAFE HashMap::allocation_size(). Its implementation uses
the actual allocation Layout and includes buckets/control storage even after tombstone-heavy erase.
It excludes the key/value Vec payload allocations. This is substantially narrower than duplicating
all StorageProvider methods or replacing crypto/MLS semantics with a new ordered provider.

Propose one deliberate locally vendored openmls_memory_storage 0.6.0 patch: preserve every upstream
storage label, JSON serializer, StorageProvider method and locking/error behavior; change ONLY the
internal map alias to hashbrown::HashMap<Vec<u8>,Vec<u8>,std::collections::hash_map::RandomState>.
Use explicit RandomState, NEVER hashbrown's default FoldHash. Pin hashbrown 0.16.1 (already resolved
by both workspaces) with default features disabled. Alias/default constructors and optional test-
utils decode must use the same hasher. Preserve upstream provenance/licence notices and mark the
small patch; do not silently relabel third-party source under the project combined licence.
No external download is needed to inspect/copy the existing Cargo registry package. This is still a
material provider/dependency boundary requiring independent design and actual-diff review.

Both root and separate native workspace patch tables must select the SAME reviewed local package;
unsupported/old code cannot accidentally compile one workspace against the unbounded std variant.
MLS persistence uses its exact StorageMap alias, preferably a narrow alias exported by the patched
storage package rather than duplicating generic parameters. Native/root locks are primary-owned.
No wire/signing/CID or JSON storage-key format changes. Map iteration ordering was never a shared
canonical authority; same-live-map bounded and legacy encoders MUST remain byte-identical, and
captured old SDK snapshots must still restore with their original group/signer/history.

## Independent fees and enforcement

The bounded snapshot API needs separate encoded-output and backing-allocation/work caps. Under the
ORIGINAL provider read guard, inspect len and allocation_size BEFORE the FIRST iteration, clone or
output allocation. Refuse backing above its actual prepaid cap even if payload/output bytes fit.
Then checked-sum every existing key/value length, count and outer frame while holding that SAME
guard, and allocate exactly one zeroizing encoded output. Bucket/ctrl allocation and Vec payloads
are distinct invoices; allocation_size alone is not total provider/RSS memory.

The actual canonical owner reservation pays both fees plus its existing simultaneous output,
provider/candidate, sealing and scratch allowances. A caller-provided small cap is negative budget
metadata, not a saved-head/Current witness. Legacy unbounded snapshot/duplicate APIs can retain their
existing contract; the owner path must use the actual guarded bounded encoder. Any new bounded
clone must prepay actual backing PLUS cloned key/value bytes before clone, not check after copying.
No counter reconstructed from serialized metadata, unsafe layout introspection, source compaction
before its fee, or lifetime-insert refusal cliff is needed.

The guard remains valid across partial-error SDK methods and concurrent key_package(&self): SDK
writes take the same backing lock and cannot mutate between preflight and encode. This is why a
before/after MlsDevice sample is insufficient: upstream remove_proposal may insert a missing list
then delete in one call, and optional feature writes can Err after partial insertion. No trait
forwarding/high-water/panic authority flag should be introduced to guess those hidden peaks.

## Required actual regressions and gates

- Real backed map with insert/erase tombstones: allocation_size remains charged independently of
  len/reported capacity. Refuse before an actual iterator/clone/encoding-entry hook. A deterministic
  test-only colliding hasher may exercise the SAME generic private encoder/preflight helper; do not
  claim it is a production credential/provider fixture. Also retain a real RandomState provider test.
- One-byte backing-cap refusal with ample encoded-output allowance; tiny-output refusal with ample
  backing allowance; exact caps succeed and match legacy bytes; huge retained Vec refused before
  output encode, source entries/allocation unchanged. Include empty map/no backing allocation.
- Actual generate/found/fresh selected-KP join, SDK Add/remove, provider restore/duplicate and existing
  compatibility fixtures retain the same logical state and signed data. Cover SDK mixed insert/delete,
  partial-error/unwind and concurrent KP/snapshot without a fake saved allocation scalar.
- Both workspace dependency trees select the patched storage once. Root/native metadata, focused
  resource/MLS/scope/canonical tests, strict Clippy/fmt/determinism/deny and all mandatory complete
  backend/frontend/native/build/startup gates remain primary-owned before integration/activation.

This fixes provider resource accounting only. It cannot authenticate an admission or public owner
head, detect canonical component rollback, replace the strict Add/save-before-Welcome transaction,
or complete scalable held-action/admin review. Those boundaries remain separately required.
