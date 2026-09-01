# Retention integration contract

Status: implementation contract written 2026-08-27; wire and persistence work pending.

`catcoms-storage::RetentionIndex` already implements expiry precedence, jitter, pinning,
holder-aware eviction, rehydration, and typed missing states. It is not wired into the desktop.
The integration must preserve its defining invariant: ordinary expiry never removes this device's
bytes unless a fresh authenticated response proves another current member holds every chunk being
removed.

## Durable local index

Store one vault-sealed retention record per server. Each row contains only:

- ciphertext/plaintext CID as already used by the blob store;
- creation and last-access timestamps;
- logical size and blob kind;
- pin/expiry state and decorrelation jitter;
- whether local bytes were intentionally evicted.

The record is local metadata, not a CRDT. Save it atomically under the vault database key. On load,
reconcile it against the sealed blob store and the live file index:

- a listed chunk missing from retention metadata is inserted conservatively with `created=now`;
- metadata claiming local bytes exist never overrides `BlobStore::has`;
- an unknown/orphan blob is not deleted during reconciliation;
- wiki-derived pins and explicit `FileExpiry::Never` are applied before any GC pass;
- malformed metadata disables automatic GC and surfaces a storage-health finding.

## Authenticated holder probe

Add a members-only signed request/response kind. A request binds:

`group_id, requester device, nonce, issued_at, [CID]`

The CID list is deduplicated and capped by count and encoded bytes. A response binds the request
hash and returns a bitset only; it must not enumerate unrelated storage. The responder sets a bit
only after `BlobStore::has` and its normal seal/CID verification succeed. Responses use the
existing authenticated member request path, freshness window, per-peer rate limit, and bounded
pending request table.

A holder observation is usable by GC only when:

- its signer is a current roster member and is not this device;
- its request nonce/hash matches the outstanding probe;
- it is younger than the short holder-proof TTL;
- it says yes for every chunk of the file/listing being evicted;
- the connection used for the answer was authenticated, though it need not remain connected.

Presence, a cached peer record, a PEX entry, a prior download provider, or an unsigned claim is
never holder proof.

## GC transaction

1. Snapshot eligible listings and their chunk CIDs.
2. Remove wiki-pinned and explicit-forever items.
3. Probe at least one other current member for the complete chunk set.
4. Re-check pins, expiry, listing identity, and local presence after the await.
5. Under the store lock, delete chunks only when another live listing does not reference them.
6. Atomically persist the retention state after deletion.
7. Emit a typed report: evicted bytes/files, retained-last-copy, stale decision discarded, and
   verification/probe failures.

Forced “clear local copy” is a separate explicit user operation. It may bypass holder proof only
after warning that the file can become unavailable, and must never apply to wiki-pinned content
without first removing the pinning reference.

## Required tests

- sealed index round-trip, tamper refusal, missing-index conservative rebuild;
- expiry/pin/listing changes during an in-flight probe discard the stale decision;
- forged, replayed, expired, wrong-group and removed-member responses cannot authorize deletion;
- partial chunk possession never authorizes whole-file eviction;
- deduplicated chunks survive while any other live listing references them;
- automatic GC keeps the last copy offline and online;
- crash between blob deletion and index save reconciles honestly on restart;
- all queues, lists, byte totals and probe rates hit deterministic configured bounds.
