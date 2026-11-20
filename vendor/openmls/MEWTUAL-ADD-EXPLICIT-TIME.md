# Separate explicit-time commit build hook

This patch is separate from the reviewed KeyPackageIn clock seam and numerical
layout helper. It retains the same published OpenMLS 0.9.2 package and license;
`MEWTUAL-PATCH.md` records original archive/VCS provenance. No manifest, feature,
version, wire, persistence or error-type change is needed for this slice.

The newly added method is:

`CommitBuilder<LoadedPsks>::build_with_time(rand, crypto, signer, now, f)`.

It delegates the exact existing `build_internal` algorithm using a private
Legacy/Explicit clock selector. Existing `build` and `build_with_new_signer`
choose Legacy. No clock is sampled at entry. The original proposal filtering,
proposal-type support, key uniqueness, suite/version, group-context extensions,
leaf capabilities, removals, PSKs, updates and remaining commit checks keep
both their order and original error conversions. The new public method does not
introduce a signer callback: `f` is precisely the existing proposal filter.

`PublicGroup::validate_add_proposals_with_time` shares the entire existing Add
validation body with Legacy callers. The full leaf-validation body is shared
too. Only at the existing leaf lifetime check does Legacy retain the original
Lifetime::validate sampling and original tree/KAT skip behavior. Explicit
always calls Lifetime::validate_with_time(now) for each selected added
KeyPackage, regardless of tree lifetime policy or the upstream test-only skip
flag. One captured time is used for all added packages. No validation is skipped.

On native targets, the time is std::time::SystemTime; on wasm32, it is
web_time::SystemTime, exactly as in the existing Lifetime API. The caller must
convert its trusted local catcoms-rt Clock time before entering the SDK. A peer's
time is never authoritative. The API is not an all-SDK clock override: receive,
Welcome joining, other leaf validation and default lifetime generation retain
legacy behavior. Those consumers need separate reviewed explicit-time adapters
before claiming a fully strict-clock lifecycle.

The six new native Mls tests use real MlsDevice identities, SDK-built and fully
signature-validated fixed-lifetime KeyPackages, and actual build/stage/merge.
They cover boundary refusals/success, unchanged legacy expiry, suite/capability
and uniqueness/remove rejection order, one time for multiple Add packages,
pre-UNIX errors and unchanged proposal filtering. They verify actual selected
member keys, epoch and pending/merged state. No test-utils, unchecked conversion,
mock signer or replacement receipt supplies authentication.

This DATA/SDK slice grants no mutation admission to Mewtual's closed restored
graph. A future private candidate must reserve an independent full operation
invoice before copies, storage work, or key dispatch; preserve its immutable
original graph; enter Add once; cache original outputs before late refusal; and
cover its own persistence/publication ambiguity. The 42 wrapper-size terms in
the separate helper exclude dynamic allocations, stack frames and this clock
selector/call-parameter overlap. They are not a numerical Add bound. No existing
restore/replay budget is credited here and there is no actual Root payer claim.

Root Source/UI/member authority, durable governance intent, MLS candidate
persistence, Commit/Welcome publication and native activation remain unimplemented
by this patch. The new test fixture's unrestricted fresh devices are not an
operation permit for the production closed graph.

The primary applied this packet and ran all six focused tests (0.12s) and the
complete MLS library suite (121 passed, one existing fixture generator ignored,
6.10s). Strict all-target/all-feature MLS Clippy also passes. The first compile
found a missing lifetime on the test helper's borrowed device; the helper now
retains that device for the same lifetime as the SDK builder. Production code
needed no compile correction. Complete root/native suites and wasm checks remain
separate open gates; see docs/HANDOVER.md for subsequent integration results.

The private Add operation also needs an explicit merge timestamp and a bounded
execution path through SDK tree work. This build hook supplies neither. The
separate reviewed merge hook is recorded in `MEWTUAL-MERGE-EXPLICIT-TIME.md`;
ordinary legacy merge still uses the ambient clock and ordinary native path work
still uses Rayon. A separately reviewed serial route and the actual operation
invoice remain necessary before claiming a deterministic, fully paid private Add.
