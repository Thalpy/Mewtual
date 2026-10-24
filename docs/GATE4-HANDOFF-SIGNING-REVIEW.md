# Gate 4: detached handoff preparation and individual signing turns

Status: implementation awaiting user adversarial review. Base:
`8190dc46b885f6b61efe9d84c8a1675fb041f823`. INSPECTION-TEST-001 is closed by the user;
read-only inspection and the previous durable handoff acceptance stand. Gate 4 remains active.
Code/test commit: `e65bfd89acecd4e660edb0560410e1d02cec5e21`; review destination:
`Thalpy/Mewtual`, branch `Create-suite-2`, PR #26. The user explicitly requested commit/push
of this checkpoint and the [four remaining-work handoffs](GATE4-AGENT-HANDOFFS.md).
[Review preambles](GATE4-REVIEW-PREAMBLES.md) distinguish this core review from runtime and
full-gate acceptance. The integration base `50f1f99` also preserves the independently merged
`d7ec5b9` jukebox fixture correction; it does not change this review's core code scope.

## Implemented boundary

The existing synchronous core handoff now uses the same staged implementation that runtime
integration will schedule. There is no alternate batch signing algorithm to drift from it.
The store's Prepared -> whole Source -> Completed transaction, full signed hashes, original
pending ledger, retry floor, source-required metadata link and reference inventory are unchanged.

1. `StudioOverlayState::handoff_authority` captures actual device/key, full target, receipt,
   MLS epoch and independently observed tenure under live-group custody. It verifies current
   membership and the receipt's current owner/tenure. The opaque result retains public data only.
2. `prepare_handoff_detached` consumes privately restored actual source, ledger, metadata and
   authority. It checks exact target/author/receipt, pristine Open successor identity, seed,
   projection and absence of signed operations. It prepares the original full envelopes in
   acceptance order with their original timestamps. Each change passes typed local policy,
   marker/actor/semantic checks and exact projection/checkpoint/recovery preflight. Bounded
   unsigned envelopes probe the ordinary gate's complete byte/count/author-share accounting.
   Their zero signatures are never treated as authenticated evidence. The private source's
   accounting owner must equal the independently verified receipt owner. Exact combined
   Prepared-manifest framing also passes its existing bound before signing. No secret is an input.
3. `StudioHandoffSigning::sign_next` rechecks current device/key, roster membership, original MLS
   epoch, receipt owner and observed tenure, then signs exactly one operation. It does no graph
   reconstruction, source replacement or publication. Signed operations remain private; there
   is no serializer, delta setter, prefix getter or renderer-controlled signing API.
4. `finish` consumes the whole batch, rejects incomplete signing, and revalidates full signed
   history, dependencies, typed changes, ordinary gate admission/restart and exact recovery
   bounds. It constructs the existing full-digest Prepared manifest. Any failure drops the
   private candidate. The compatibility store adapter still executes these stages synchronously;
   it is not yet a live actor scheduler.

The original installed source is never cloned by sharing its mutable gate. The private source
is independently restored from its complete authenticated snapshot. The source-before manifest
retains its existing full-snapshot digest semantics; actual vault-wrapper currency remains the
store/application's additional obligation.

## Evidence and limits

Five new core regressions cover both Index and Flipnote. The independent success oracle uses
ordinary `edit_or_reseal`, then compares every full signed envelope and complete source snapshot,
projection, Prepared evidence, completion and restart. It checks one decrement per signing turn
and unchanged original source/metadata/ledger. Negative cases cover partial finish, independently
advanced successor, wrong device/tenure, mismatched captured source-accounting owner, and a real
MLS member addition between signatures.

The MLS regression independently proves unchanged designated owner, actual membership/key and
successful current-receipt verification after the commit. The isolated mutation removes only
captured-MLS-epoch equality. It must execute exactly one failing test at the intended assertion,
restore source bytes exactly and pass the restored test. Compilation and empty filters do not
count. A second isolated mutation targets the accounting-owner match while preserving receipt
verification, target, source identity and projection. The existing acceptance-order mutation
follows the loop to its new preparation module.
All three selected local mutation checks detect their intended executed assertion, restore
source exactly, and pass restored regressions. The app run passes 25 handoff tests with two
opt-in profiles skipped; the five new core tests and strict replication/app Clippy also pass.

Execution results, maximum-count coverage and exact CI checkouts are recorded in HANDOVER.
The 256-operation regression passes both kinds locally, with complete projection/restart,
every original envelope, exactly 256 individual signing calls and unchanged durable records.
Diagnostic stage timings and their limits are in [P1-PERFORMANCE](P1-PERFORMANCE.md).
One signature per call is a work-unit bound, not a release latency or measured heap guarantee.
The existing 256-operation, signed-envelope, aggregate epoch, seed, metadata and recovery limits
still apply. Reconstruction remains expensive and must run outside actor/store custody.

## Next runtime obligations

This core object does not implement shared permit ownership, numeric-server/actor incarnation,
mount/session/request, full source/intent wrapper stamps, actual reference inventory or durable
commit. Runtime integration must supply those checks before each signing turn and transaction
barrier, retain the original preparation permit, and cancel stale candidates without losing
accepted work. Capture/rebuild/commit must not invoke expensive intent decode or reference
traversal synchronously merely because signatures have been split.

Native local Save and automatic handoff remain disabled. Manual overlay copy/export/disposition,
stale-base and preview-based work, repeated-owner tenure integration, signed repair and combined
Gate 4 acceptance remain outstanding. Gate 5 is untouched.

## Adversarial review message

```text
Please adversarially review e65bfd89acecd4e660edb0560410e1d02cec5e21 against
8190dc46b885f6b61efe9d84c8a1675fb041f823 on PR #26. Start with
docs/GATE4-HANDOFF-SIGNING-REVIEW.md and the latest HANDOVER evidence.

Challenge the opaque authority and prepared-batch provenance, exact actual-source/branch
identity, envelope/order/timestamp preservation, pre-sign typed and aggregate admission,
one-operation signing, live MLS/membership/owner/observed-tenure checks, absence of a signed
prefix API, and full final assembly. Check the synchronous store uses this same path without
weakening Prepared -> whole Source -> Completed, source-required metadata or reference fences.

Inspect the independent ordinary-edit oracle, incomplete/stale refusal tests, full-count test,
and the actual MLS-transition mutant and restored runs. Require executed intended assertions;
build errors and empty filters do not count. Distinguish per-call work count from latency proof.

Return PASS for this bounded core implementation or numbered findings with concrete failure
paths. Native activation, runtime permit/stamp/commit scheduling and full Gate 4 acceptance
remain separate; no native overlay Save is enabled by this checkpoint.
```
