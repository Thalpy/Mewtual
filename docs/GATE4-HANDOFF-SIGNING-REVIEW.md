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

## Review verdict (2026-10-06): bounded PASS, two medium test-coverage findings open

Independent adversarial review of exactly `8190dc46b885f6b61efe9d84c8a1675fb041f823` to
`e65bfd89acecd4e660edb0560410e1d02cec5e21`, run with the preamble in
`GATE4-REVIEW-PREAMBLES.md` and the common reviewer contract. Reviewer: an Opus agent, read-only,
in its own detached worktree at the head SHA.

**Verdict: PASS for the bounded core handoff preparation and signing split (production code).**
No blocker or high finding, and no production correction required for the PASS. Not Gate 4
acceptance.

**What the reviewer executed**: the five core `studio_handoff_preparation` tests (5 passed); the
committed signing mutation harness, with both mutants (`mls`, `owner`) failing at their named
assertions with exactly one test executed each, byte-exact restoration and restored passes; and two
probe mutants of its own (below). **Not executed by the reviewer**: the app crate's 25 handoff
tests, both 256-operation fixtures, the ten store mutants, app Clippy, fmt, the ambient gate and
cargo deny. Those were assessed from CI run 34981381873 (`prepared-signing` and `handoff` jobs),
after the reviewer verified that the run's merge checkout is tree-equivalent to `e65bfd8` under
`crates/` and `.github/`, apart from an unrelated jukebox fixture.

**Verified sound**: opaque authority (private, not `Clone`, redacted `Debug`; `check_live` at capture
and before every signature: device id and key, roster key, MLS epoch, tenure, `verify_current_owner`);
actual source binding (`check_overlay_successor`); envelope, order and timestamp preservation against
the ordinary-edit oracle for both kinds; exactly one `sign_domain` per call; no signed-prefix
escape; complete final assembly; the store's Prepared -> whole Source -> Completed, source-required
metadata and reference fences unchanged.

| id | severity | finding | status |
|---|---|---|---|
| SIGN-TEST-001 | MEDIUM, test coverage | Pre-sign typed and aggregate admission is unreached by any test: `local_policy` (`epoch/handoff/preparation.rs:39`), per-operation `recovery::preflight` (`:69`), the probe gate's `admit_local` (`:83-91`) and the manifest framing probe (`overlay/handoff/preparation.rs:115-121`). One reviewer mutant disabled all four and every core test passed. Line 39 is the handoff path's only editor-cap check, and it is reachable through a vault-decoded branch, which is how the 256-operation fixture builds one. No handoff test has a non-owner author | **open**: regression driving a vault-decoded over-cap branch to its exact `EpochBound`, a positive non-owner-author handoff, and isolated mutants for `:39` and `:83`; then re-review |
| SIGN-TEST-002 | MEDIUM, test coverage | Nothing pins the binding between the captured authority and the metadata's receipt (`overlay/handoff/preparation.rs:106`). Replacing the comparison with `false` passed every core test. Without it, authority captured for receipt RA could sign a branch prepared against RB from an earlier tenure of the same owner key (A -> B -> A) | **open**: regression capturing authority from one state and preparing a second with a different receipt (`rejoining_owner`), asserting `EpochScope` from `prepare_handoff_detached` specifically, plus an isolated mutant; then re-review |
| SIGN-PERF-001 | LOW, production | The compatibility adapter restores the private copy before the cheap pristine check, so a refused `handoff_studio_overlay` against a successor with progress pays one extra full restore under custody, and it clones the ledger and metadata twice | follow-up: run `check_overlay_successor`, or a cheap op-count/phase/opening check, before `copy_handoff_source`. Not reachable from native today |
| SIGN-DESIGN-001 | LOW, design | Authority binds at receipt level (target, author, receipt), not the branch fingerprint; the tenure equality in `check_live` is implied by the receipt check | note; binding `active.basis()` at capture would make provenance exact rather than relying on runtime stamps |

### SIGN-TEST-001 and SIGN-TEST-002: re-reviewed; 002 closed, 001 narrowed to 001b

**Finding re-review (2026-10-06, Opus, static, against the fix at `c9566b82` plus the tests and
harness below):** SIGN-TEST-002 **CLOSED**. SIGN-TEST-001's editor-cap and aggregate halves
**closed**; what remains is the positive handoff by a non-owner author, re-filed as
**SIGN-TEST-001b** (open). The re-review confirmed each mutant changes only its own guard, so a
killed mutant proves the unmutated error came from that guard, and that the over-gate branch is a
faithful honest branch (valid local work under every append-path check).

Fixed at the branch head, not at `e65bfd8`: the guards are unchanged, so these pin the pinned
code's behaviour as it stands now. All in `studio/epoch/owner/tests/handoff.rs`, named
`studio_handoff_preparation_*` so CI's `prepared-signing` job runs them, each with an isolated
mutant in `check-studio-handoff-signing-mutations.py` that must fail at its named assertion with
exactly one test executed, restore byte-for-byte and pass again.

| finding | regression | mutant |
|---|---|---|
| SIGN-TEST-002 | `..._refuses_authority_captured_for_another_receipt`: authority from a branch on receipt R1, then a real second close cycle producing R2, its branch and successor; R1 still verifies as current, so only the receipt binding can refuse; `EpochScope` asserted; positive control signs with the second branch's own authority | `receipt`: the comparison replaced by `false` |
| SIGN-TEST-001, editor cap (`:39`) | `..._refuses_a_vault_decoded_branch_over_the_local_cap`: an Index branch built honestly to `MAX_INDEX_OBJECTS`, its last entry retargeted in the record bytes at one more `PutObject`, decoded structurally; `EpochBound` from preparation; the replaying decoder refuses the same bytes; the honest record is the positive control | `local-policy`: the result ignored |
| SIGN-TEST-001, aggregate (`:83`) | `..._refuses_an_honest_branch_over_the_successor_gate`: 48 honest near-maximal title edits within `MAX_INTENT_BYTES_PER_DOCUMENT`, whose signed form exceeds the successor's `MAX_EPOCH_BYTES`; `EpochBound` from preparation, before any signature | `probe-gate`: the admission skipped |

**SIGN-TEST-001b: regressions and mutants submitted.** A second member joins the owner's group.
The member's copy of the owner's sealed Closing source is the owner's snapshot restored under the
member's group with the member as actor, an approximation of a fully synced member that skips the
member's own inbound decrypt and admission and its own receipt seal. The member then settles and
checkpoints **its own** copy, so the successor checked against the basis is one the member built.

| regression | pins | mutant |
|---|---|---|
| `..._signs_a_branch_authored_by_a_non_owner_member` (Index and Flipnote) | the member's branch on the owner's close prepares, signs every turn and assembles, and every signed operation is the member's | `author-is-owner`: requiring the source's actor to be the gate owner |
| `..._charges_only_a_non_owner_the_per_device_cap` | an owner-authored branch between `MAX_DEVICE_BYTES` and `MAX_EPOCH_BYTES` once signed (asserted) hands off; a member-authored branch of the same shape by construction (the same deterministic titles) is refused `EpochBound` before signing | `owner-charged` (local admission charges the owner the cap; it mutates `admit_local` only, because the shared `admit_open` would also charge the owner during the fixture's own inbound fill) and `device-exempt` (no one is charged it) |

**Harness run** (at `d319b37b` plus this change, in an isolated worktree): all eight mutants
(`mls`, `owner`, `receipt`, `local-policy`, `probe-gate`, `author-is-owner`, `owner-charged`,
`device-exempt`) detected at their named assertions with exactly one test executed, restored
byte-for-byte, and the restored tests passed. The first version of `owner-charged` mutated the
shared `admit_open`, and the harness refused it as the short re-review predicted: it died in the
fixture's own fill instead of at its assertion. It now mutates `admit_local` only.

**Not done, and why.**
- **Residual, not part of the original correction:** no isolated mutant for the per-operation
  `recovery::preflight` in `PreparedOverlayChanges::prepare`, nor for the manifest framing probe
  (the `set_prepared` call in `prepare_handoff_detached`). Signed bytes are about twice the live
  title bytes, so the checkpoint bound and the probe gate are crossed at about the same
  operation, and making preflight the first refusal needs a base seed near 2 MiB; the framing
  bound only fails with large retained completed and disposed manifests, a third-generation
  branch. Neither is cheap.

**A product gap the aggregate test exposed, stated at its full width.** The branch it builds is
valid local work: it fits the intent budget and every append accepts it. Its signed form can never
fit the successor epoch, so it can never be handed off automatically. The handoff refuses safely
before signing. But:

- For a **non-owner** author the binding limit is the per-device `MAX_DEVICE_BYTES` (1 MiB), not
  `MAX_EPOCH_BYTES` (4 MiB), so a member's branch becomes permanently untransferable at about a
  quarter of the size. A rail sized at the epoch budget would leave that case open.
- P2's classifier is not merely silent: `studio_overlay_eligibility` has no size reason and
  reports such a branch as **Transferable**, and its comment classes capacity as a transient
  retryable refusal. Here every retry fails the same way, because the successor must stay
  untouched.
- No existing 8.3 rail bounds a branch's signed size; this needs a new rail or a new manual reason.

That is Agent 2's capacity rails and classifier. It is recorded here and in the Agent 1 status
ledger, and handed to Agent 2 for their own documents.

Residual risks recorded by the reviewer: `finish` does not re-check live authority, so the runtime
must re-check before commit; strict MLS-epoch equality means membership churn can keep a large batch
from completing; batch plaintext is not zeroized on drop (consistent with `StudioEpoch`); the
per-call work count is the only measurement. The reviewer also observed that both medium gaps
appear to persist at the current branch head.
