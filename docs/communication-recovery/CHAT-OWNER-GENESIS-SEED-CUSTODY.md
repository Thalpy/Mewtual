# Actual source-to-genesis seed resource custody

The actual owning App read→SDK consumer is authored with six UNRUN regressions. Design review
and final actual source/documentation static reviews are clear. The cold reader is authored
at0cdaeb5c, and the Sync DATA seed at65f666e9. This consumer supplies the actual separate
provider/metadata/SDK payments required by that seed. It creates neither a Current host nor a
native result. No compilation/tests/builds ran.

## Owned source and actual independent payments

Consume the real `RetainedBootstrapSourceData` by value. Refusal must return that SAME original
DATA/payment, not discard it and reread files. Validate its original mount/controls before work,
then acquire fees outside all admission/pin/canonical/Root/pool writers from the SAME actual
configured source-read account. The original read fee stays live while its opened source exists.

A trusted local negative `GenesisSeedBudget` holds explicit source limits and SDK byte/work
caps; it does not authenticate a source or act as payment. Its complete checked invoice is:

- actual SDK graph reservation, retained LAST in BOTH real device/group by the existing loader;
- separate raw provider-map backing and up to2× configured MLS encoded payload capacity;
- `V2_GENESIS_SEED_METADATA_BYTES`, actual App wrapper/return/Arc layouts and bounded error/copy
  scratch, retained independently of SDK and opened-source fees;
- Runtime-parts/refusal layouts are NOT part of this first consumer. A later T/R/Clock handoff
  must separately reserve them before allocation. Arbitrary caller T/Clock allocations and
  broad host/provider execution are not paid by a scalar limit.

Construct the documented trusted-host `OwnerSdkGraphReservation` only from an actual non-Clone
catalog-pool debit plus immutable cached caps. No Store, key, provider, source owner or sender
may live in that reservation. Calling the actual Sync seed/SDK loader happens outside writers.
On SDK/metadata failure, partial graphs drain before their fees refund. A successful graph
still needs final original source validation after its potentially long decode/verification.

The retained App wrapper stores the actual seed FIRST, original read DATA next, and separate
provider/metadata fees LAST. It exposes borrowed seed/public DATA, not an owned raw seed,
SDK getter, Server, run method or Current setter. A caller cannot move the seed out and silently
drop its independent non-SDK fees. Root's later actual Current factory may consume this entire
resource capsule while preserving those fees through node/graph teardown.

## Runtime attachment without losing payments

This section remains the contract for a future consuming runtime method. The current capsule
exposes borrowed seed/data/source only; it cannot extract an owned seed, attach runtime or
release its independent fees while a moved graph remains alive.

Any consuming runtime attachment must preserve the complete wrapper on refusal, together with
the SAME original runtime parts. Use the Sync owning checked pairing and return both original
owners; never reconstruct T/R/Clock or rewind RNG. Infallible final assembly moves the actual
prepared node and all resource owners into another retained wrapper, without allocation.
Cold inputs remain DATA with no Fresh life; warm extraction retains its actual original Fresh
life internally and Root independently keeps the original plan/creation sender alive.

Before runtime Box/refusal allocation, compare its exact generic layout against the ACTUAL
already acquired runtime allowance or acquire another actual fee outside writers. A cap tested
after allocation is insufficient. No last fee may refund under a writer or SDK lock.

This capsule does not bypass namespace3, group-pending/immutable pin, actual activation/head
barriers, scope Current, native UI-session generation or the private Current factory. Those
actual source/authority checks remain mandatory even for a genuine paid DATA graph.

## Required regressions

- Genuine source creation→terminal→new physical read→actual paid SDK seed, exact original
  source/genesis/metadata, and SAME-pool charge lifetime after Store/read caller Drop.
- One-byte-short provider/metadata or SDK credit refuses BEFORE SDK entry; source returned
  intact for a later genuinely paid attempt, without another large source read/write unit.
- Wrong original Store/mount/root/control/public source and cancellation after SDK decode
  refuse final handoff; partial device/group cleanup precedes last payment release.
- Borrowed seed access cannot extract a raw owned seed and drop its non-SDK resources.
- Checked runtime mismatch returns the same capsule/parts; cold/warm success retains fees
  through actual node teardown; unsupported generic layout refuses before allocation.

The initial reader/loader remains closed genesis epoch0/sole-founder, not a live/member loader.
Live SDK operation admission, root Current/governor/join/send/receive/admin/native integration
and all compiled validation remain open.

Actual API: `GenesisSeedBudget::new` supplies trusted negative caps and checked byte/work
limits; `ServerStore::prepare_genesis_seed(source, budget)` consumes the real retained source.
It validates that source before fees, after resource waits before SDK entry, and after decode.
Success owns `RetainedBootstrapGenesisSeed`; fixed-reason refusal owns the SAME source and
read fee for a genuinely new SDK attempt, without another full source read or write unit.
Six genuine cold-source tests cover independent charge/drop lifetime, one-byte-short combined
pressure before SDK, provider/setup refusal cleanup, cancellation after fee wait, actual final
control replacement and foreign Store/account rejection. The work1 test refuses SDK setup
before typed getters; partial typed graph cleanup is covered by the prior MLS loader tests.
Only formatting/whitespace/static safety gates have run, not compilation or test execution.
