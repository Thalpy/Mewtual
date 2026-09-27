# Bounded history page work

Cursor-capable catch-up pages inspect at most 256 uncertified retained operations. They spend
at most 2,048 charged steps on popped hashes, scheduled dependency edges and copied range
certificates, including unknown and duplicate hashes. The input frontier and pending stack are
each capped at 2,048 hashes (the authenticated wire frontier already permits at most 512).
At most 2,048 ranges are sorted/merged per query; this work does not depend on archive length.
The closure is conservative: omitted facts can cause duplicate transmission, never omitted
history. A provider cursor advances across already-held regions even if its page is empty.

## Locally certified ancestry ranges (review R4)

Each legacy `EncryptedDoc` keeps a derived cache of at most 4,096 change certificates. A
certificate contains up to eight exact half-open signed-log ranges, obtained only from its own
canonical Automerge change and locally certified dependencies. One insertion reads at most 32
dependencies and merges at most 257 ranges. Overlapping or adjacent certified intervals merge;
an unproved sibling gap never does. If more than eight intervals remain, the broadest are retained,
with earlier positions winning ties. The certificate becomes incomplete so queries may also walk
its dependencies. Oldest distinct entries are evicted; duplicate envelopes do not grow the queue.
The fixed key/certificate/queue payload is about 512 KiB per full cache plus bounded container
overhead. Empty and small documents allocate only for their accepted changes.

Acceptance adds certificates incrementally, and ordinary restore builds a fresh bounded cache
while it already reads the saved signed log. A network request never rebuilds an archive index.
A buffered child whose dependencies are absent from the canonical graph grants no certificate.
Positions that cannot fit the existing u32 cursor representation grant no certificate. P1 epoch
documents keep their existing separate pager and do not allocate this index. The cache is absent
from saved bytes and snapshot keys. Restoring/replacing a document discards the former source's
ranges; appending only adds positions beyond previous certificates.

A recent head in a causally ordered linear history carries its whole prefix through cache eviction. This corrects
the 10,000-operation, one-missing-operation case without preserving peer-specific query state or
adding wire fields. Certificates are shared local facts, so peer churn cannot allocate another
graph. The existing peer/runtime cursor binding still resets foreign positions. Remote heads
that cannot be resolved locally exclude nothing. A malicious requester can suppress its own
response with a valid same-provider cursor but cannot create or alter a certificate for others.

The durable-send candidate path already snapshots and restores a document before preparing an
operation. Its restore now also constructs this bounded derived index, adding constant-bounded
work per retained log entry to that existing archive-sized operation. No cache is separately
cloned or saved, and a discarded candidate cannot alter the live document's certificates.

This is a bounded optimization for linear and low-fragmentation histories. An evicted old head,
out-of-order dependencies or a highly fragmented DAG can still require fallback pages and resend
duplicates. In particular, a large linear history received in reverse causal order can still
resend most of its known prefix, including after restore; the index is built in retained log order.
No universal transfer-efficiency bound for arbitrary arrival orders or DAGs is claimed. The provider
retains bounded work instead of falling back to an unbounded walk for cursor-capable requests.

The requester grants at most eight advancing empty pages per document/provider without a stall
penalty, then applies the existing eight-round non-progress bound and cooldown. Changing provider
identities or repeating a position is not advancement. Cooldown cannot renew empty-page grace;
actual accepted history or signed completion clears it. The tracking map uses the existing bound.
No new wire tags, authority, operation identity or storage schema are introduced.

Compatibility limitation: clients without provider cursors retain the previous scan behavior.
Bounding their duplicate-only prefix without a continuation would strand them at that prefix.
This feature does not claim a universal CPU/time bound for legacy operation parsing, no-cursor
requests, or the older full-history exchange. Those require a separate negotiated boundary.

Prior bounded-page verification on Windows: two replication regressions passed (duplicate-prefix continuation and
conservative/unknown-head closure bounds). All 278 sync library tests pass, including three new
signed requester regressions for honest empty pages, malicious unlimited advancement, and
repeated/rotating provider cursors. Raw outputs: `logs/cr-page-work-tests.log` and
`logs/cr-bounded-pages-sync-final.log`. Independent adversarial review found no remaining
blocker/high; its unnecessary honest-provider cooldown finding is corrected by the bounded grace.

R4 adds index bounds/fragmentation/duplicate tests; document tests for tiny-cache eviction, cold
restore, incomparable branches in both log orders, changing heads and appends during paging,
buffered children, and foreign document heads; and a real signed requester/provider test with
4,098 shared operations (larger than both work/cache caps) that counts every transferred operation. That requester test also
restores the provider before a second request and rejects a stale runtime cursor. These new
regressions have not yet been executed in this implementation branch; the root agent owns the
serialized Cargo run and separate public-API transfer-cost regression.
