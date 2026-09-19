# Gate 4 Agent 2: overlay lifecycle, provisional local work and repeated tenure

Status: **design ACCEPTED. Adversarial review PASS for all three boundaries at
`a6d8170f6ab1f0d46287808fc8051ac2a387521c`, 2026-09-16, with no findings.** (b) passed at revision 4
and was not reopened; (a) and (c) passed at revision 6. Revision 7 is the accepted text plus two
non-blocking refinements the reviewer offered while passing, folded in below and marked as such.

**This PASS accepts the design only.** No production code is written, no test has been added, no
Cargo command was executed and no measurement exists. It grants no implementation acceptance, no
native Save exposure, no acceptance of Agent 1's or Agent 3's work and no part of full Gate 4.
Agent 1's registration prerequisite P5 remains **false**, and `studio_overlay_save` stays
unregistered. The next adversarial boundary is implementation: whether the accessor split, the v1
migration, the M-1 receive rule, the `DraftArchive` family, A-1's ordering, N-T7/N-T7b and the
mutation anchors are built and actually execute as specified.

Revision 1 (`a901f6b`) received CHANGES REQUIRED with nine findings. Revision 2 (`21ca8fa`)
received CHANGES REQUIRED as SEC-PAIR-001 with five. Revision 3 (`9097207`) received CHANGES
REQUIRED with two. Revision 4 (`37fa877`) **passed boundary (b)** and received CHANGES REQUIRED on
(a) and (c) for a single Medium gap. Revision 5
(`f2257b018d396a835529742c40d4b282bbc127d9`) closed that gap, and received CHANGES REQUIRED on (a)
and (c) for one new Medium finding introduced by its own accessor-removal hardening: two app
wrappers were classified as pure authoring although their accepted store ordering deliberately
acknowledges and recovers before requiring a tenure. This revision corrects that with invariant A-1.
Section 0 maps each finding to its correction; verify each against the code, not the prose.

Design base for this revision: `f2257b018d396a835529742c40d4b282bbc127d9`. Earlier bases:
`37fa87753d32a2c4f5d1172cc865910a93d90fa1`,
`909720739b6c0a455d37761e8149d7eec21cb6f4`, `21ca8fa93c07b8bb65a00bc0bbc555518f8a7132`,
`a901f6b0f64df2b4ea9cc0221b64ac98276f582d`, `1bcb1bca204d721b848b17c0835faf931ae930e3`. Scope is
[Agent 2 of the four handoffs](GATE4-AGENT-HANDOFFS.md); progress is in
[GATE4-AGENT-2-STATUS](GATE4-AGENT-2-STATUS.md). The applicable review scope is
[review preamble 2](GATE4-REVIEW-PREAMBLES.md#review-2-manualprovisional-overlay-lifecycle-and-repeated-tenure).

Three separable review boundaries, unchanged from revision 1:

1. the manual lifecycle (inspect, export, copy, disposition), stale bases and repeated tenure;
2. the **separately reviewed** extension for durable local work based only on an
   `AwaitingTenureReceipt` preview (section 8);
3. the **correction to locally observed owner tenure** (section 9.3), which changes an
   authority-bearing observation in `catcoms-sync` and now also touches `catcoms-mls`.

Accepted work this design must not weaken: the Closing-overlay foundation (`b1b0ec9`,
OVERLAY-TEST-001 closed), the handoff design (HANDOFF-001) and its bounded core/store
implementation (`62f06d4`, HANDOFF-002 closed), the detached-inspection proposal (`0b28f06`) and
its read-only implementation (`c47ae0b`, INSPECTION-TEST-001 closed), the combined scheduling
block (`6b71d96`), and the accepted provisional read-only preview contract (`a89bde6`/`134394e`,
NATIVE-TEST-001 and TAIL-TEST-001 closed). No closure is reopened.

**Unmet dependencies.** The [core signing split](GATE4-HANDOFF-SIGNING-REVIEW.md) at `e65bfd8` is
unreviewed. **Agent 1's runtime design has since passed user review** (PASS for the bounded runtime
design, 2026-09-15, design only, no implementation acceptance), so its Flow S stages, holds and
prerequisites are a stable target rather than a moving one; this design consumes its seams by name
only and
section 14 records what breaks if they change. Agent 3's design is separate and is not consumed
here except through the tenure seam of section 9.4.

## 0. Design acceptance, and the two refinements folded into revision 7

The revision-6 re-review returned **PASS for (a) and (c) with no findings**, with (b) already passed
at revision 4. Every finding raised across revisions 1 to 6 is closed at the design boundary. The
reviewer offered two non-blocking refinements while passing; both are adopted, and neither changes
an accepted decision:

| Refinement | Adopted as | Where |
|---|---|---|
| A-1 should be read specifically as the rule for **store-delegating lifecycle wrappers**. An app-level operation that itself performs irreversible authoring before reaching a store stage cannot defer its authority check merely because A-1 exists. | A-1 gains an explicit scope sentence. The reviewer judged the existing single-stage exception sufficient and did not raise it as a finding; it is tightened anyway, because this document now becomes implementation instructions and the previous finding showed how brittle a whole-function reading is. | 9.3 part 5, A-1 |
| N-T7b could assert `is_prepared()` and that the fixture's initial `completed_branch(.., current_basis)` is `None`. Harmless, not required: the reviewer independently established that completion advances the basis floor, so a Prepared branch cannot legitimately carry a completed handoff's old basis and the short-circuit cannot mask the resolution path. | Added to N-T7b as diagnostics. The reasoning is recorded with it so a later reader does not mistake a cheap assertion for the actual argument. | 17.1 N-T7b |

## 0b. Disposition of the revision-5 re-review findings

Boundary **(b), the preview-local-work extension, PASSED** at revision 4 and is not changed here.

| Finding | Disposition in revision 6 | Where |
|---|---|---|
| **1 (Medium)** the accessor-removal table moved tenure refusal ahead of legitimate retry and recovery paths: `save_studio_closing_overlay` and `handoff_studio_overlay` were classified as pure authoring, although their accepted store ordering deliberately acknowledges and recovers first | **Corrected, and generalised into one rule.** A-1: an app wrapper reads `authoring_owner_tenure_start()` and passes the `Option<u64>` through unchanged; the store owns every refusal, at the stage that needs it. The committed orderings that make this necessary are cited by line. A-1 replaces a per-wrapper "pure or mixed" classification deliberately, because a classification invites the next person to judge a new wrapper and judging it wrong is exactly this finding. V1 is refined to "new authoring", V8 states the complementary reachability, and N-T7b plus M24c make it executable. | 9.3 part 5 A-1, 9.4 V1/V8, 15 L11, 17.1 N-T7b, 17.2 M24c |

Revision 5's own correction, the `Imported` app-seam anchoring, was accepted by that re-review and
is unchanged here.

## 0c. Disposition of the revision-4 re-review findings

| Finding | Disposition in revision 5 | Where |
|---|---|---|
| **1 (Medium)** `Imported` is not mutation-isolated at the app authoring seam: N-T7 and M24 cover only `Unknown`, so an implementation could satisfy N-T6d, M22e and M22f while still mapping `Imported(S)` to `Known(S)` at the app boundary | **Corrected.** N-T7 is pulled out of the retained set and runs the full V1 authoring-refusal matrix for **both** fail-closed values, using a genuinely migrated v1 fixture rather than an injected one, and asserting in the same run that verification stays unaffected so it cannot pass by making `Imported` behave as `Unknown` everywhere. M24's scope is narrowed to `Unknown`, and the two halves of the app-boundary invariant get separate anchors: M22g for the conversion, M24b for the accessor. V7 states the invariant. | 9.4 V7, 17.1 N-T7, 17.2 M22g, M24, M24b |
| Hardening note: `observed_owner_tenure_start`'s "independently observed" contract would become false for `Imported` | **Adopted, and taken further.** The accessor is **removed** rather than repointed, so no call site can inherit the wrong semantics by default and the compiler enumerates every one. `verification_owner_tenure_start` and `authoring_owner_tenure_start` replace it, with the intended per-site mapping listed. | 9.3 part 5 |
| Hardening note: P4 still cited V1-V5 | Corrected to V1-V7 in both documents. | 13, status note |
| 16.1 product decision | **Adopted as (a)**, over my own recommendation of (b): `Imported` ships fail-closed with no operator-adoption override. The "no fourth option" claim is refined to cover reconstruction only. | 16, 15 L11 |

## 0d. Disposition of the revision-3 re-review findings

| Finding | Disposition in revision 4 | Where |
|---|---|---|
| **1 (High)** v1 tenure snapshots can promote historically ambiguous tenure into `Known`: a stale `start` written by the old preserve branch, with the live leaf digest grafted on | **Corrected, and the reviewer's escape hatch proved rather than assumed.** `start == Some(epoch)` is shown to be exactly the promotable set, because the old preserve branch cannot produce it. A bare downgrade of the rest is rejected as worse than the risk, since it would strip every existing founder of rotation permanently, so the value is split by consumer: a new `Imported(u64)` state is sound for verification and fail-closed for authoring, `prepare_receipt_head_snapshot` moves to a new authoring accessor, and a version flag stops a save/reload laundering it. | 9.3 part 5, 9.4 V1/V5/V6, 15 L11, 16.1, 17.1 N-T6d, 17.2 M22e/M22f |
| **2 (Medium)** revision 3 still contained the archive placement it declares impossible, in O2 | **Corrected**, together with the two stale cross-references the reviewer named: Agent 1's design is recorded as user PASS, and the pre-`Unmatched` "unknown id returns Stale" shorthand is removed from the status note. | 3 O2, header, GATE4-AGENT-2-STATUS |

Revision-3's own corrections to SEC-PAIR-001 were accepted by that re-review and are unchanged here;
the table below is retained as their record.

## 0e. Disposition of the SEC-PAIR-001 findings (retained record)

| Finding | Disposition in revision 3 | Where |
|---|---|---|
| **1 (High)** the generation scheme has no legal first-acceptance state: `classify_request` returns `Stale` for any unknown id, including the legitimate first Save of the next generation | **Corrected.** `classify_request` is a structural, basis-free classifier returning `Active`, `Transferred`, `Disposed` or **`Unmatched`**. `Unmatched` is not terminal: it is resolved into `New` or `Stale` at the authorizing stage, where the basis is minted anyway, by comparing the request against the **derived expected-next** identity. Agent 1's rule that acknowledgement classification performs no basis mint, tenure read, source lookup or media work is preserved. | 5.1, 6.6, 17.1 N17a, M10b |
| **2 (High)** the archive has no unambiguous physical identity in the Intents inventory | **Corrected, adopting the reviewer's 16.1 answer.** A distinct physical record kind `EpochRecordKind::DraftArchive` with its own suffix, sealing domain, scope, canonical path, reader and sealed cap, its own inventory key, final/temporary recognition, `epoch_files_absent` participation, I-4 invalidation and reference collection, while remaining in the **Intents accounting class** through `includes_intents()` and `EpochIntentBudget`. Revision 2's "one more record kind inside the existing family" is withdrawn as unrepresentable. | 5.4, 6.5, 14, 17.1 N19b |
| **3 (High)** the archive's declared bound cannot contain its own stated maximum payload | **Corrected.** A strict size formula derived from the actual field bounds, with three separate constants threaded through the reader cap, inventory per-record cap, budget, sub-cap and native bound, plus a static assertion and maximal-shape tests. The archive is no longer bounded by the intent record's cap, which it never had to be once it is its own record kind. | 5.3, 6.5, 15 L3b, 17.1 N19c |
| **4 (High)** the same-commit tenure residual is blocked only in the local commit builder | **Corrected.** The prohibition moves to the **receive side**, into the existing pre-merge staged-commit inspection that already walks `add_proposals()` and `remove_proposals()`, as a `DeviceId`-based rule independent of leaf allocation. The invite ledger is explicitly demoted to honest-join admission and is no longer load-bearing for this invariant. | 9.3 part 4, 17.1 N-T6c, M22d |
| **5 (Medium)** N14/M5 cannot test a missing or wrong confirmation at the store layer | **Corrected.** Split by layer: the native adapter tests absence and every wrong literal; the store tests that the typed confirmation is required for the destructive branch. M5's literal mutation moves to the native parser. | 17.1 N14, N14n, 17.2 M5, M5n |

Revision-1 findings 1, 2, 4, 5, 6, 8 and 9 are closed by SEC-PAIR-001. Their corrections are
unchanged in this revision and the table below is retained as the record of them.

## 0f. Disposition of the revision-1 findings (retained record)

| Finding | Disposition in revision 2 | Where |
|---|---|---|
| **1 (High)** `Copied` disposal has no valid terminal representation: `copy` is required by the mode, forbidden without `active`, and cleared by the transition | **Corrected by removing the cause.** Copy bookkeeping is deleted from the durable record entirely. Disposal no longer depends on copy. The terminal `disposed` manifest is self-contained and independently valid with `active` absent, and positive encode/decode/reopen cases exist for both modes. | 5.1, 6.4, 6.5, 17.1 N11-N13, N23 |
| **2 (High)** the copy proof can account for the wrong source work; projection-level copying is not envelope-level preservation | **Corrected twice.** (a) Copy is no longer a disposal precondition, so no destructive action rests on it. (b) `source_entry` is deleted from the request: `restore::plan` now *derives* the source operation ids it consumes, and the design states plainly that copy is projection-level and lossy for superseded operations, conflicts and original provenance. Preservation is supplied by the archive, not by copy. | 6.3 C-P, 6.5, 17.1 N6-N10 |
| **3 (High)** repeated disposal erases the only defence against an old Save retry | **Corrected by a durable branch-generation namespace.** A monotonic `branch_generation` is bound into the branch identifier every request must carry, so an old request matches nothing and returns **Stale**, never a new acceptance. One retained manifest still gives a terminal acknowledgement for the most recent disposal; forgetting older ones degrades to refusal, never to acceptance. | 5.1, 6.6, 17.1 N17, N17b, M10b |
| **4 (Medium)** the unchanged inspection capture cannot supply the destination inputs copy needs | **Corrected.** A composite capture taken in the same custody visit under the same single permit: the existing source capture plus the destination's authenticated Studio source and recovery record plaintexts with their digest and physical size, rechecked at preview completion and at application. The "only the rebuild function changes" claim is withdrawn. | 5.2, 6.3 C1-C4, 17.1 N8b |
| **5 (Medium)** a structurally valid, non-replayable branch has no lossless export path | **Corrected, and it simplified the archive.** Export and the archive are built from the **structural** record plus the ledger envelopes and do not require typed reconstruction. A reconstruction failure is labelled, not fatal, and `Preserved` disposal therefore works for a non-replayable branch. | 5.3, 6.2, 6.4, 7 S3, 17.1 N22 |
| **6 (Medium)** the preview mint has no way to obtain its required seed-only bytes | **Corrected.** `UnconfirmedStudioSeed` retains the exact verified checkpoint bytes it parsed, exposed only through the existing scoped callback, with explicit memory accounting. The mint does not trust that retention: the detached stage **re-parses** those bytes against the candidate receipt before the branch is built. | 8.1, 8.2, 8.3, 17.1 N25b |
| **7 (High)** L5's safety argument overlooks Unknown-tenure readers | **Corrected; the false argument is withdrawn.** `complete_checkpoint_head_scoped`'s `is_some_and` means an Unknown-tenure reader accepts a proof's claimed tenure, so disagreement is not self-correcting. Agreement is now established structurally: `Position` gains the committer's leaf identity so every witness recognises the same authenticated membership discontinuity, plus a membership rule for the one residual case. The inference is rephrased in terms of current continuous membership. | 9.3, 17.1 N-T6, N-T6b, M22b |
| **8 (Medium)** the native storage-refusal rule is false after a partially completed copy | **Corrected.** C5 is deleted with the copy bookkeeping, so the partial-copy state no longer exists. The three-state write outcome is specified for the write sequence that does exist: archive durable with disposal pending. | 6.4, 11, 12 |
| **9 (Medium)** explicit discard confirmation is required but absent from the schema | **Corrected.** An exact required confirmation token in both the Rust request and the native argument list, with a typed constructor that only that literal produces. | 5.1, 5.5, 6.4 D5 |
| Reviewer's N19 precision point: destination operations do not retain the branch's base-only, superseded and removed references | **Accepted and corrected.** N19 is rewritten. After `Preserved` disposal the **archive's** reference collection retains them; after `Discarded` they become reclaimable, which is what the user confirmed. | 6.4, 10 R3, 17.1 N19 |
| Reviewer's answers to questions 16.1-16.6 | Adopted, including 16.1's ruling that a newly authored projection edit is not a lossless substitute, and 16.2's preference for a v3 terminal record with the archive's durability established first. | 6.4, 16 |

## 1. Outcome and boundary

Every retained local branch has a bounded, authorized way to be inspected, exported, copied into
current authorized work, or explicitly disposed of, and none of those is available by accident.
Repeated owner changes and newcomers preserve the difference between unconfirmed history, local
work and verified authority, and a fail-closed Unknown tenure has an implemented, legitimate way to
become Known on which every participant agrees.

In scope: the four manual operations and their durable records; the draft archive; stale, rewound
and nonpristine bases; preview-based local work; A -> B -> A succession and rejoining; the
live-tenure contract consumed by Agents 1 and 3; truthful native results, events and UI-hooks rows.

Out of scope: actor-scheduled Save and automatic handoff runtime, preparation permits, the
inventory cursor and transient reference holds (Agent 1); signed fault repair (Agent 3);
integration, shared-document edits and full-gate acceptance (Agent 4). **Nothing here registers
`studio_overlay_save`**; section 13 states exactly which of Agent 1's registration prerequisites
this design satisfies, and the status note carries the authoritative answer.

There is **no import path**. Neither the export nor the archive can re-enter a vault as authority.

## 2. What was audited

Revision 1's audit table stands. It is not repeated in full. Facts added or corrected for this
revision, each read at the pinned source:

- **A1.** `ServerStore::read_studio_record(&scope)` and `read_epoch_studio_plain` return
  `AuthenticatedEpochFileBytes { plain, physical_bytes }` for a Studio source record under
  `MAX_SEALED_BYTES`, with the parent-directory and regular-file restrictions already applied
  ([epoch_studio.rs:648-700](../crates/catcoms-app/src/store/epoch_studio.rs#L648)). The recovery
  family has the equivalent reader. **Finding 4's composite capture needs no new reader**, only
  wider visibility and a second stamp.
- **A2.** `UnconfirmedStudioSeed::parse` ends by requiring
  `projection.checkpoint(receipt.close_record_hash)?.bytes() == bytes`
  ([provisional.rs:41-44](../crates/catcoms-replication/src/studio/provisional.rs#L41)), so the
  canonical seed is byte-identical to what was fetched **at parse time**. It does **not** retain
  those bytes, and the tail subsequently advances `projection`, so the identity cannot be
  recomputed later. `ProvisionalStudioSeedPreparation` owns `raw` and drops it in `prepare()`.
  This confirms finding 6 and fixes its shape: retain the bytes, and re-prove the binding rather
  than trusting the retention.
- **A3.** `openmls::group::Member` exposes `index`, `credential`, `encryption_key` and
  `signature_key`. `ServerGroup` currently surfaces only `index` and `signature_key`
  ([group.rs:138-171](../crates/catcoms-mls/src/group.rs#L138)). A `DeviceId` is derived from the
  signature key, so a changed signature key is already an owner change; the **credential** is the
  field that distinguishes a re-add from an ordinary self-update, because
  [group.rs:247-274](../crates/catcoms-mls/src/group.rs#L247) binds a joiner's KeyPackage
  credential to `(this group, invite_nonce)` while an update rotates keys and not the credential.
  This is the discriminator section 9.3 uses.
- **A4.** `complete_checkpoint_head_scoped` rejects a proof/observation tenure mismatch only under
  `observed_owner_tenure_start().is_some_and(|t| t != proof.tenure_start_group_epoch)`
  ([detached.rs:172-180](../crates/catcoms-sync/src/receipt_head/detached.rs#L172)). With local
  observation `None` a valid fresh proof mints a `HeadSelection` carrying the proof's claimed
  tenure. The reviewer is right: this is deliberate accepted reader behaviour and it means a wrong
  value is **not** universally refused. Revision 1's safety argument is withdrawn.
- **A6 (SEC-PAIR-001 finding 2).** `EpochRecordKind` is a closed five-variant enum, and each variant
  owns exactly one `suffix()`, one `domain()`, one `scope()` and one `sealed_cap()`
  ([inventory.rs:48-99](../crates/catcoms-app/src/store/epoch_recovery/inventory.rs#L48)).
  `storage_name` iterates that fixed family list gated by coverage, so a file is assigned to a
  family **by filename suffix before decryption**; `decode_record_scope` then requires the
  authenticated scope's leading domain to equal `family.domain()` and requires the scope to
  re-derive canonically; the inventory is keyed by `(EpochRecordKind, hash)`. A second record under
  the same family therefore has no representable identity: sharing the scope collides on the key,
  changing the scope fails the domain check, and a new suffix is simply not recognised. The
  reviewer is right and revision 2's placement was unimplementable.
- **A7 (SEC-PAIR-001 finding 4).** `ServerGroup::process_incoming` already inspects the staged
  commit **before** merging it: it walks `staged.add_proposals()` for the defence-in-depth
  credential binding check and `staged.remove_proposals()` for the `ns_secret_L` rotation signal,
  and only then calls `merge_staged_commit`
  ([group.rs:466-490](../crates/catcoms-mls/src/group.rs#L466)). Its own comment states that
  single-use nonce enforcement stays with the admitting committer's ledger and that this is the
  binding check every applier can make without the invite token. That is exactly why revision 2's
  part 4 was insufficient, and it is also the seam the correction needs: the pre-merge roster is
  still the pre-commit roster at that point, so the pre-commit designated committer and the removed
  leaves' device ids are both available.
- **A5.** `StudioRecoveryItem`'s `value` field already names the source operation id of the value
  the planner selects (`frame_value`/`index_value` match `source.op_id`), and the deletion and
  creation arms take `tombstones[id].first()` and `creations.first()`
  ([restore.rs:72-83, 198-202, 264-267](../crates/catcoms-app/src/studio/restore.rs#L72)). The
  planner therefore **can** report the exact source operation ids it consumed, which is finding 2's
  correction (a).

## 3. Audit observations

Revision 1's O1 (the foundation's hold is a removal filter, not a lifecycle), O3 (`StudioRecovery`
is the wrong container), O4 (`restore::plan` is the copy engine), O5 (a preview's durable content is
content-addressed) and O8 (classification needs no reconstruction) stand unchanged.

**O2, corrected twice.** Revision 1 concluded from the 64 KiB metadata ceiling that no archive was
possible and therefore that a verified copy had to serve as preservation. That conclusion was wrong:
the constraint rules out putting bodies **in the extension**, not out of an archive as such.
Revision 2 then over-corrected in the other direction and placed the archive as a second record
inside the existing Intents family, which O12 and A6 show is unrepresentable. The settled position,
and the only one this design holds, is: the archive is a distinct **physical** record kind,
`EpochRecordKind::DraftArchive`, with its own suffix, sealing domain, scope, canonical path, reader,
sealed cap and inventory key, while sharing the Intents **accounting class**, its vault budget, the
`includes_intents()` coverage policy and Agent 1's mutation-generation coordination. Section 6.5
specifies its schema, provenance, accounting, reference collection, ordering and quotas, which is
what 16.1 requires of any archive representation.

**O9 (new).** Copy and preservation are different jobs and revision 1 conflated them. `restore::plan`
is a projection planner: it recovers the *selected value* of an element, so a branch entry that was
superseded within the branch, a conflict, and the original authorship of an accepted envelope have
no representation in what it produces. Copy is therefore a genuinely useful way to carry work
forward and a genuinely invalid proof that work was preserved. Revision 2 keeps the first role and
deletes the second.

**O11 (new, finding 1).** Revision 2 made `Stale` the answer to every identity the record does not
know, which is safe against an old request and fatal to a new one. The two cases are distinguishable
only against a freshly derived basis, because `branch_id` is a hash and cannot be inverted. So the
classifier cannot be the place that decides: the cheap structural stage can only say "none of the
identities I hold", and the stage that already mints the basis must resolve it. That stage exists in
Agent 1's Flow S as S1b, which already compares a request identifier against a freshly minted basis
and answers stale on mismatch, so the correction adds a comparison there rather than a new stage.

**O12 (new, finding 2).** A6 shows the family assignment happens by filename before decryption and
the scope domain is then required to match that family. "Same family, different sealing domain" is
therefore self-contradictory in this codebase. The archive needs its own physical kind. What it does
**not** need is its own accounting family: coverage, budget and the vault ceiling are separate
concerns from physical identity, and `includes_intents()` can gate both kinds together.

**O10.** The rollover defence that `completed` provides comes from advancing
`minimum_new_basis_closed_epoch`, which disposal deliberately does not do (finding 3). The cheap
replacement is not a history of acknowledgements but a **namespace**: if every request carries a
branch identifier that includes a monotonic generation, an old request cannot collide with a new
branch at all, and the worst outcome of forgetting an old manifest is a refusal.

## 4. Design principles

1. **Reuse the accepted read machinery.** Export, archiving and copy planning use the existing
   inspection capture, permit, stamp, currency contract and delivery fence. Where copy needs more
   than that capture holds, the capture is **extended explicitly** (5.2), not claimed to be
   unchanged.
2. **Read-only operations change no durable byte.**
3. **Removal is a transition with durable evidence.** Work leaves the ledger only through one
   explicit, authorized, exactly retryable transaction that has either established lossless
   preservation first or recorded that the user waived it.
4. **Preservation is lossless or it is not preservation.** A newly authored projection edit carries
   work forward; it does not preserve envelopes, ordering, conflicts or provenance.
5. **Copy authors new work** and is never a precondition for destroying anything.
6. **Provenance is carried, never inferred.**
7. **Expiry governs the preview, not the work.**
8. **Tenure is observed or Unknown**, and where an inference is made, every participant must be able
   to make the same one from the same authenticated evidence.
9. **Identity is generational.** An old request must be distinguishable from new work by
   construction, not by retained history.
10. **Refusal retains work**, and every refusal names an actionable state.

## 5. Concrete APIs

New leaf modules owned by Agent 2:

```
crates/catcoms-replication/src/studio/overlay/disposal.rs      (v3 arm, terminal manifest)
crates/catcoms-app/src/store/epoch_intents/disposal.rs         (the disposal transaction)
crates/catcoms-app/src/store/epoch_intents/archive.rs          (the draft archive record)
crates/catcoms-app/src/studio/lifecycle.rs                     (structural classification)
crates/catcoms-app/src/studio/overlay/copy.rs                  (composite capture, copy driver)
apps/desktop/src-tauri/src/studio/overlay.rs                   (native surface)
```

### 5.1 Core: provenance, generation, and the v3 terminal record

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StudioOverlayProvenance { Closing, Unconfirmed }

impl StudioOverlayState {
    pub fn provenance(&self) -> Option<StudioOverlayProvenance>;
    /// Monotonic per logical document. Incremented exactly once when a branch is first
    /// accepted where none existed. Never reset, never reused, never decremented.
    pub fn branch_generation(&self) -> u64;
    /// The identity every request must carry: H("catcoms/studio-overlay-branch/v1",
    /// basis fingerprint, branch_generation). An identifier, never authority.
    pub fn branch_id(&self) -> Option<[u8; 32]>;
    pub fn disposed(&self) -> Option<&StudioOverlayDisposal>;

    /// Structural classification for an incoming Save request, evaluated before any basis mint,
    /// tenure read, source lookup or media admission. Exactly one arm can match. It cannot
    /// distinguish a legitimate next-generation request from a stale one, because `branch_id` is
    /// a hash of a basis it deliberately does not derive; that is what `Unmatched` means.
    pub fn classify_request(&self, target: StudioTarget, branch: [u8; 32], intent: &LocalIntent)
        -> Result<StudioOverlayRequestClass, ReplError>;

    /// Resolution of `Unmatched`, called ONLY from the authorizing stage that has just minted a
    /// fresh basis under live authority. Returns `New` exactly when there is no active branch and
    /// `branch == H(domain, fresh.fingerprint(), branch_generation + 1)`, and `Stale` otherwise.
    pub fn admit_new_branch(&self, target: StudioTarget, branch: [u8; 32],
                            fresh: &StudioClosingOverlayBasis)
        -> Result<StudioOverlayAdmission, ReplError>;

    /// The one transition that drops an active branch without transferring it. The caller has
    /// already proved authorization, archive durability (for `Preserved`) and explicit user
    /// confirmation (for `Discarded`); this rebuilds and validates state only.
    pub fn dispose(&self, ledger: &IntentLedger, decision: StudioDisposalDecision,
                   sequence: u64, at: u64) -> Result<(Self, BTreeSet<[u8; 32]>), ReplError>;
}

pub enum StudioOverlayRequestClass {
    /// `branch` names the live branch; ordinary exact-retry or append applies.
    Active,
    /// `branch` names the retained `completed` manifest with a matching envelope.
    Transferred(StudioHandoffOutcome),
    /// `branch` names the retained `disposed` manifest with a matching envelope.
    Disposed(StudioOverlayDisposal),
    /// None of the identities this record holds. NOT a verdict: the authorizing stage resolves
    /// it with `admit_new_branch`. Reaching this arm implies no terminal acknowledgement is
    /// owed, so the request must either be a first acceptance or stale.
    Unmatched,
}
pub enum StudioOverlayAdmission {
    /// First acceptance of the derived next generation. The increment and the first accepted
    /// envelope become durable in the SAME sealed replacement; there is no reserved, uncommitted
    /// generation and no second durable transition.
    New { generation: u64 },
    /// An older generation, a skipped generation, an unrelated basis or an unknown identity.
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StudioDisposalMode {
    /// A durable draft archive for this exact branch content existed before the removal.
    Preserved { archive: [u8; 32] },   // the archive record's content digest
    /// The user explicitly destroyed the bodies after observing the branch.
    Discarded,
}

/// Terminal and SELF-CONTAINED. Valid with `active`, `prepared` and `copy` all absent, because
/// no field of it refers to them (finding 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudioOverlayDisposal {
    pub target: StudioTarget, pub author: DeviceId,
    pub provenance: StudioOverlayProvenance,
    pub basis: [u8; 32], pub branch: [u8; 32], pub content: [u8; 32],
    pub generation: u64, pub mode: StudioDisposalMode,
    pub accepted: usize, pub sequence: u64, pub at: u64,
    /* private: Vec<Entry> (id, envelope, sequence, ts), <= MAX_STUDIO_OVERLAY_OPS */
}

/// Only this literal constructs it; no bool and no defaulted field can stand in (finding 9).
pub struct StudioDiscardConfirmation(());
impl StudioDiscardConfirmation {
    pub const TOKEN: &'static str = "destroy-local-draft";
    pub fn parse(value: &str) -> Option<Self>;
}
```

`StudioCopyProgress` and the `copy` record arm of revision 1 **do not exist**. Copy writes nothing
to the branch's record (findings 1, 2, 8).

**Encoding rules**, following the accepted v1/v2 discipline:

1. A state expressible as v1 still encodes as v1 byte-for-byte (the existing `legacy` rule).
2. A state expressible as v2, meaning `provenance == Closing`, `generation == 1`, no `disposed`,
   still encodes as **v2 byte-for-byte**. Existing records are never rewritten, so Agent 1's C-2
   digest fence and the accepted canonical re-encode equality are unaffected for every current
   vault. A v1 or v2 record decodes with `generation = 1`.
3. Tag `3` is emitted only when `provenance == Unconfirmed`, `generation != 1`, or
   `disposed.is_some()`. Its layout is the v2 layout followed by: `generation`, a provenance byte
   (0 `Closing`, 1 `Unconfirmed` with `provider`, `observed_mls_epoch`, `observed_at_ms`), and an
   optional `disposed` block.
4. A reader accepts exactly one complete v1, v2 or v3 form. Unknown version bytes, trailing bytes,
   duplicate ids, noncanonical order and a wrong `sequence` reject, and the existing canonical
   re-encode equality check runs unchanged. Old readers already fail closed on tag 3.
5. For `Unconfirmed` the nested v1 basis blob is untouched: `source_id` and `source_version` must
   be canonically zero and the provenance fields live in the v3 outer record.

**Validation additions to `StudioOverlayState::validate`**, inside the existing method so the
decoder and `encode_vault` share one predicate:

- no operation id occurs in both `completed.entries` and `disposed.entries`, and none of either
  occurs in `active`;
- `disposed.accepted == disposed.entries.len()`, `validate_manifest(disposed.entries)` passes, and
  a ledger entry still present for a disposed id must match its author and envelope (the tolerance
  `completed` already has);
- `disposed.generation <= branch_generation`, and `branch_generation >= 1`;
- **no rule of `disposed` refers to `active`, `prepared` or any live field**, so a terminal record
  with `active == None` is valid, re-encodes canonically and reopens (finding 1);
- an `active` branch may coexist with a `disposed` manifest, in which case
  `active.generation > disposed.generation` is implied by `branch_id` construction and asserted;
- `provenance == Unconfirmed` forbids `prepared` and a nonzero nested `source_id`/`source_version`;
- the combined metadata ceiling is charged as today. With `copy` gone, two 256-entry manifests plus
  headers occupy approximately 43 KiB of the 64 KiB budget; the encoder refuses over it and the
  branch stays retained (limit L3).

**The provenance guard.** `prepare_handoff`, `prepare_handoff_detached` and `prepared_manifest`
refuse unless `provenance == Closing`, before any authority work. This single core fence makes an
unconfirmed branch structurally incapable of becoming signed history.

### 5.2 Store: the composite capture (finding 4)

Revision 1's claim that only the rebuild function changes is withdrawn. The source capture holds
one record's plaintext and stamp and nothing about a destination, so copy needs a second bounded
capture taken in the **same custody visit under the same single preparation permit**:

```rust
pub(crate) struct StudioOverlayCopyCapture {
    /// The accepted capture, unchanged: one bounded authenticated intent record plus
    /// `StudioInspectionStamp` (mount, server, document, target, author, digest, physical size).
    source: StudioInspectionCapture,
    destination: StudioDestinationCapture,
}

/// Authenticated bytes and their currency stamp. No projection is materialized under custody;
/// both projections are built on the detached worker, preserving "custody is spent on evidence".
pub(crate) struct StudioDestinationCapture {
    stamp: StudioDestinationStamp,
    source: Zeroizing<Vec<u8>>,            // read_studio_record, <= MAX_SEALED_BYTES (A1)
    recovery: Option<Zeroizing<Vec<u8>>>,  // <= MAX_RECOVERY_SLOTS_BYTES + 1024
}
pub(crate) struct StudioDestinationStamp {
    mount: Arc<()>, server: u64, document: LogicalDocument, target: StudioTarget,
    /// (blake3 of authenticated plaintext, physical bytes) for each record, the same currency
    /// contract `studio_inspection_is_current` already uses.
    source: (blake3::Hash, u64),
    recovery: Option<(blake3::Hash, u64)>,
}

impl ServerStore {
    /// Caller holds live membership custody and one shared preparation permit. Two bounded
    /// authenticated reads. Decodes nothing.
    pub(crate) fn capture_studio_overlay_copy(
        &self, server: u64, group: &[u8], source: StudioTarget, destination: StudioTarget,
        author: DeviceId,
    ) -> Result<StudioOverlayCopyCapture, AppError>;

    /// Re-reads BOTH destination records and compares digest and physical size, in addition to
    /// the existing `studio_inspection_is_current` check on the source record.
    pub(crate) fn studio_destination_is_current(
        &self, server: u64, group: &[u8], destination: StudioTarget,
        stamp: &StudioDestinationStamp,
    ) -> Result<bool, AppError>;
}
```

Detached work decodes both destination records, builds the destination projection and its at most
three recovery projections, reconstructs the draft, and runs `restore::plan`. The destination's
`doc_id` and `EpochPhase::Open` are asserted on the decoded destination record, and again at C3 and
C4 against freshly read bytes.

### 5.3 Store: export, the raw-evidence fallback and the archive payload

Export and the archive share one serializer and **do not require typed reconstruction**
(finding 5): they are built from the structurally decoded extension plus the ledger envelopes.

```rust
pub(crate) enum StudioInspectionPurpose {
    /// Existing behaviour: full reconstruction and a `StudioLocalDraft` projection.
    Draft,
    /// Structural decode plus ledger envelopes. Typed reconstruction is ATTEMPTED and its
    /// outcome labelled, never required.
    Archive,
    CopyPlan(Box<StudioOverlayCopyChoice>),
}
impl StudioInspectionCapture {
    pub(crate) fn rebuild_for(self, purpose: StudioInspectionPurpose)
        -> Result<(StudioInspectionStamp, StudioInspectedDraft), AppError>;
}
pub(crate) struct StudioInspectedDraft {
    pub(crate) target: StudioTarget,
    pub(crate) prepared: bool,
    pub(crate) draft: Option<StudioLocalDraft>,
    pub(crate) archive: Option<StudioDraftArchivePayload>,
    pub(crate) plan: Option<StudioOverlayCopyPlan>,
    pub(crate) branch: Option<[u8; 32]>,
    pub(crate) content: Option<[u8; 32]>,
    pub(crate) provenance: Option<StudioOverlayProvenance>,
    /// `Ok(())` when typed reconstruction succeeded, `Err(reason)` when it did not. An Err
    /// value still yields a complete archive and a complete export (finding 5).
    pub(crate) replayable: Result<(), String>,
}
```

**`catcoms-studio-draft-v1` payload**, the single format used by export and by the archive record:

```
u8   version = 1
u8   provenance (0 Closing, 1 Unconfirmed)
u8   replayable (0 no, 1 yes)
bytes document.server_id, u16 doc_type tag, bytes document.logical_key
u8   target kind, bytes channel, [bytes object]
bytes author(32), bytes basis(32), bytes branch(32), bytes content(32)
u64  branch_generation
bytes receipt (canonical Receipt::encode)
bytes seed
[provenance == 1: bytes provider(32), u64 observed_mls_epoch, u64 observed_at_ms]
u32  count
     per entry, in saved sequence: bytes id(32), u64 sequence, u64 ts,
                bytes author(32), bytes operation (DomainOp::encode)
```

**Size, derived rather than asserted (finding 3).** Revision 2 said the payload was "at most
`MAX_RECORD_BYTES` plus framing" and then bounded the record by `MAX_RECORD_BYTES`, which cannot
both hold. The archive is its own record kind (5.4), so it was never obliged to fit the intent
record's cap. Three constants are derived from the actual field bounds:

```rust
// Fixed per-entry cost: id(32) + sequence(8) + ts(8) + author(32) + this encoder's length framing.
const ARCHIVE_ENTRY_OVERHEAD_BYTES: usize = /* 80 + framing, computed from Encoder */;
// version, provenance, replayable, document (MAX_SERVER_ID_BYTES + tag + MAX_LOGICAL_KEY_BYTES),
// target, author/basis/branch/content (4 * 32), generation, optional provenance triple, framing.
const ARCHIVE_HEADER_BYTES: usize = /* computed from the same fields */;

pub const MAX_DRAFT_ARCHIVE_PAYLOAD_BYTES: usize = ARCHIVE_HEADER_BYTES
    + MAX_RECEIPT_BYTES
    + MAX_CHECKPOINT_BYTES                      // 2 MiB seed
    + MAX_INTENT_BYTES_PER_DOCUMENT             // 4 MiB of encoded operations
    + MAX_STUDIO_OVERLAY_OPS * ARCHIVE_ENTRY_OVERHEAD_BYTES;   // 256 entries
pub const MAX_DRAFT_ARCHIVE_RECORD_BYTES: usize =
    MAX_DRAFT_ARCHIVE_PAYLOAD_BYTES + MAX_ARCHIVE_SCOPE_BYTES + ARCHIVE_RECORD_FRAMING_BYTES;
pub(super) const MAX_DRAFT_ARCHIVE_SEALED_BYTES: usize = MAX_DRAFT_ARCHIVE_RECORD_BYTES + 40;
```

That is approximately 6 MiB plus 26 KiB, which is deliberately **larger** than
`MAX_RECORD_BYTES = 5 MiB + 1024`. The two 2 MiB and 4 MiB terms cannot both be saturated in a live
record, because the intent cap already constrains seed plus ledger, so the practically reachable
maximum is closer to `MAX_RECORD_BYTES` plus the entry table and header. The constant is
nevertheless derived from the field bounds and not from that coincidence, so it stays correct if the
intent cap moves. An implementation must carry a static assertion that the constant covers the
maximal encodable shape, and N19c exercises a maximal accepted branch and a one-byte-over refusal.

The three constants are threaded through: the archive reader's cap, the inventory family's
`sealed_cap()` and its per-record authentication rail, `EpochIntentBudget`'s accounting, the 16 MiB
archive sub-cap, and the native base64 bound, which stays inside `bounded_view`'s 32 MiB even at the
derived maximum.

The payload carries no signature and no key material, and there is no
decoder in the vault direction: it cannot produce a `StudioClosingOverlayBasis`, a `StudioOverlay`,
a `VerifiedReceipt` or any store record. `replayable == 0` is the raw-evidence case finding 5
requires: exact seed, ordered complete envelopes, timestamps, scope and provenance, with the
reconstruction failure labelled rather than concealed.

### 5.4 Store: the draft archive record and the disposal transaction

```rust
```rust
// crates/catcoms-app/src/store/epoch_recovery/inventory.rs
pub enum EpochRecordKind {
    Recovery, OwnerReceipts, Intents, Registry, Studio,
    /// Lossless disposed-draft evidence. A distinct PHYSICAL family with its own suffix,
    /// sealing domain, scope, canonical path, reader and sealed cap, and its own inventory key,
    /// but part of the INTENTS accounting class (finding 2, reviewer's 16.1 answer).
    DraftArchive,
}
```

`suffix()` returns `.draft-archive`; `domain()` returns the archive record domain; `scope()` returns
the archive scope function; `sealed_cap()` returns `MAX_DRAFT_ARCHIVE_SEALED_BYTES`. `storage_name`
gains the family in its iteration list, gated by `includes_intents()` so every coverage that scans
intents also scans archives and no narrower coverage silently misses one. `decode_record_scope`'s
existing domain and canonical re-derivation checks apply unchanged to the new domain, final and
temporary names are recognised by the existing `record_name`, `epoch_files_absent` sees the family
because it already scans at full coverage, and Agent 1's `epoch_mutation_guard` covers its writes.

```rust
impl ServerStore {
    /// One archive record per (server, logical document), in its own physical family.
    /// Accounted in `EpochIntentBudget` against `MAX_VAULT_INTENT_BYTES` and the archive
    /// ceiling of 6.5. Takes Agent 1's `epoch_mutation_guard` like every other family writer.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn write_studio_draft_archive_with_io(
        &mut self, server: u64, document: &LogicalDocument, payload: &StudioDraftArchivePayload,
        ts: u64, rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget, intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioDraftArchiveRecord, AppError>;

    pub(crate) fn read_studio_draft_archive(
        &self, server: u64, document: &LogicalDocument,
    ) -> Result<Option<StudioDraftArchiveRecord>, AppError>;

    /// Explicit, separately confirmed, and itself truthfully destructive.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn release_studio_draft_archive_with_io(/* .. */)
        -> Result<(), AppError>;

    /// ONE accounted atomic replacement of the intent record: the terminal manifest is written
    /// in the SAME sealed plaintext that removes the named annotated entries. Retires no
    /// ordinary intent, prunes no source, deletes no blob, writes no recovery record, and for
    /// `Preserved` performs no archive write of its own: the archive is already durable.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn dispose_studio_overlay_with_io(
        &mut self, server: u64, document: &LogicalDocument, target: StudioTarget,
        group: &ServerGroup, device: &MlsDevice, request: &StudioOverlayDisposalRequest,
        ts: u64, rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget, intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioOverlayDisposal, AppError>;
}

pub struct StudioOverlayDisposalRequest {
    /// Branch identity including its generation (finding 3).
    pub branch: [u8; 32],
    /// `branch_hash(active, ledger)` from the inspection the user actually saw.
    pub content: [u8; 32],
    pub accepted: usize,
    pub mode: StudioDisposalRequestMode,
}
pub enum StudioDisposalRequestMode {
    Preserve,
    /// Finding 9: the confirmation is a required field with a typed constructor.
    Discard(StudioDiscardConfirmation),
}
```

The lifecycle classification is unchanged from revision 1 apart from carrying `generation`,
`content` and the disposal mode:

```rust
pub enum StudioOverlayLifecycle {
    Absent,
    Draft { provenance: StudioOverlayProvenance, transfer: StudioOverlayTransfer,
            eligibility: StudioOverlayEligibility, basis: [u8; 32], branch: [u8; 32],
            content: [u8; 32], generation: u64, accepted: usize, archived: bool },
    Transferred(StudioHandoffOutcome),
    Disposed(StudioOverlayDisposal),
}
```

### 5.5 App and native surface

```rust
pub enum StudioControlAction {
    // ... existing ...
    OverlayLifecycle,
    ExportOverlay,
    FinishOverlayExport(Box<StudioPreparedInspection>),
    /// Produces the archive payload and writes the durable archive record.
    ArchiveOverlay,
    FinishOverlayArchive(Box<StudioPreparedInspection>),
    ReleaseOverlayArchive(Box<StudioArchiveReleaseRequest>),
    PrepareOverlayCopy(Box<StudioOverlayCopyChoice>),
    FinishOverlayCopyPreview(Box<StudioPreparedCopy>),
    ApplyOverlayCopy(Box<StudioOverlayCopyApply>),
    DisposeOverlay(Box<StudioOverlayDisposalRequest>),
}

pub struct StudioOverlayCopyChoice {
    pub destination: StudioTarget, pub item: StudioRecoveryItem, pub mode: StudioRecoveryMode,
}
/// `source_entry` of revision 1 is DELETED (finding 2). The plan reports what it consumed.
pub struct StudioOverlayCopyApply {
    pub destination: StudioTarget, pub item: StudioRecoveryItem, pub mode: StudioRecoveryMode,
    pub branch: [u8; 32], pub content: [u8; 32],
    pub epoch_id: u128, pub expected_projection: [u8; 32],
    pub nonce: [u8; 16], pub body: Vec<u8>,
}
```

Native commands registered by this design, none of which enables Save:

```ts
studio_overlay_read({ server, channel, object? })                  // extended, section 11
studio_overlay_lifecycle({ server, channel, object? })
studio_overlay_export({ server, channel, object? })
studio_overlay_archive({ server, channel, object? })
studio_overlay_archive_read({ server, channel, object? })
studio_overlay_archive_release({ server, channel, object?, archive, confirm })
studio_overlay_copy_preview({ server, channel, object?, destination, choice, mode })
studio_overlay_copy_apply({ server, channel, object?, destination, edit })
studio_overlay_dispose({ server, channel, object?, branch, content, accepted, mode, confirm? })
```

`confirm` is required exactly when `mode === "discard"` and must be the literal
`"destroy-local-draft"`; any other value, and its absence, refuse before any durable work
(finding 9). `studio_overlay_archive_release` takes the literal `"release-local-archive"`.

All use the existing `InvokeContext`: one UI session generation, one actor instance, one native
operation slot, one `ViewRequest` per `(state, server, target)` and one `RequestCancellation`
spanning every custody visit, exactly as `studio_overlay_read` does today. The two-visit commands
keep the existing 5-second `StudioInspectionDelivery` fence.

## 6. The manual operations

### 6.1 Inspect: unchanged

`studio_overlay_read` keeps its accepted contract; section 11 adds fields.

### 6.2 Export

| # | Stage | Custody | Work |
|---|---|---|---|
| X1 | Begin | yes | `begin_studio_inspection`, unchanged: channel known, current membership, `(group, device, owner, mls)`, one shared permit, one bounded authenticated read, stamp. |
| X2 | Rebuild | detached | `rebuild_for(Archive)`: **structural** decode, target and author checks, the section 5.3 payload from the structural entries and the ledger envelopes. Typed reconstruction is attempted and its outcome recorded in `replayable`; failure does not abort (finding 5). |
| X3 | Finish | yes | `finish_studio_inspection`'s existing rechecks, including the store stamp digest and physical size. |
| X4 | Deliver | native | Existing delivery guard, `bounded_view`, base64. |

- **E1.** Export writes no durable byte and authorizes nothing. Permitted while a transfer hold
  exists; refused, retryably, while a live hold exists.
- **E2.** Current membership is required through the unchanged `inspection_context`. A removed
  member acquires no offline export right.
- **E3.** Export is not `.pixa`, not settlement and not permission to delete the original. Disposal
  has its own request, fences and confirmation.
- **E4.** The payload names referenced PIX CIDs; it contains no pixel bytes and promotes, holds or
  fetches nothing.

### 6.3 Copy into current

Two-phase, mirroring the accepted recovery `Preview` -> `Apply` shape. Copy carries work forward and
is **never** a precondition for disposal (findings 1, 2).

**Destination scope**, unchanged from revision 1 and endorsed by the reviewer's 16.6:

- **Same-document copy** (primary): the branch's own logical document, now Open.
- **Cross-document copy** (the Prepared case): a different logical document of the same `doc_type`
  in the same channel, in practice Flipnote to Flipnote. Index is same-document only.
- **C-0.** The destination's identity is its complete `LogicalDocument`. A different channel label
  naming the same Flipnote object is the same destination and is refused while a transfer hold
  exists.

**Planner changes.**

```rust
pub(crate) fn plan(
    current: &StudioProjection, historical: &StudioProjection,
    history: &[&StudioProjection], item: StudioRecoveryItem,
    mode: StudioRecoveryMode, restorer: DeviceId, scope: PlanScope,
) -> Result<StudioRecoveryPlan, AppError>;
pub(crate) enum PlanScope { SameDocument, CrossDocument }

pub struct StudioRecoveryPlan {
    pub disposition: StudioRecoveryDisposition,
    pub body: Option<Vec<u8>>,
    pub original_author: Option<DeviceId>,
    /// NEW (finding 2): the operation ids in `historical` that this proposal consumes,
    /// derived by the planner from the item it actually resolved. Bounded and small.
    pub source_ops: Vec<[u8; 32]>,
}
```

`control::preview` passes `SameDocument` and maps its `&[StudioRecovery]` to projections; recovery
behaviour is otherwise unchanged. `CrossDocument` keeps every capacity, conflict, tombstone and
over-cap check and drops only the logical-key equality.

| # | Stage | Custody | Work |
|---|---|---|---|
| C1 | Begin | yes | `capture_studio_overlay_copy` (5.2): the accepted source capture plus the destination's two authenticated records and their stamps, one permit, one visit. |
| C2 | Plan | detached | Decode both destination records, build the destination and recovery projections, reconstruct the draft, run `restore::plan`. |
| C3 | Preview | yes | The existing source-stamp recheck plus `studio_destination_is_current`, destination channel known, destination still Open with the same `doc_id`, `expected_projection = recovery_fingerprint()`. Returns one bounded proposed body or an explicit hold. Saves nothing. |
| C4 | Apply | yes | Exact-retry shortcut first (`contains_exact_operation` on the destination), then re-plan from durable state and require `epoch_id`, `expected_projection`, `disposition == Ready` and a byte-identical `body`. Then the ordinary `StudioRequest::Apply` publication path. |

There is **no C5**: copy writes nothing to the branch's record (findings 1, 2, 8).

- **C1'.** Refused, retryably, while a live hold exists on either document, and while a transfer
  hold exists on the destination. Permitted while a transfer hold exists on the source, and it does
  not clear `Prepared`, retire the original envelopes, or count as evidence that the original
  handoff completed.
- **C2'.** A bulk copy is the user issuing C3/C4 per item. There is no batch command and no batch
  atomicity. Each item consumes ordinary admission, typed policy, capacity preflight, reference
  protection and content budget.
- **C-P (finding 2, stated plainly).** Copy is **projection-level**. It recovers the selected value
  of an element as a new operation authored by the copier. It does **not** preserve a branch entry
  that was superseded within the branch, a conflict alternative, the original authorship of an
  accepted envelope, or the accepted ordering. `source_ops` reports exactly which source operations
  the proposal resolved and nothing more. No count of copied items ever establishes that the branch
  was preserved; only the archive of 6.5 does that. The native result and the hooks row say so.

### 6.4 Disposition

One transaction, two truthful modes, a self-contained terminal manifest.

| # | Precondition, checked under one exclusive store visit before any write |
|---|---|
| D1 | Current membership, complete target and channel scope, and the requester is the branch's own author. |
| D2 | No live hold and **no transfer hold**: a Prepared branch refuses outright. |
| D3 | `request.branch` equals the durable `branch_id()` (basis fingerprint and generation), `request.content` equals `branch_hash(active, ledger)`, and `request.accepted` equals the entry count. A UI that has not re-inspected since the branch changed refuses with `BranchChanged`. |
| D4 | For `Preserve`: a durable archive record exists for this document, it decodes, its `content`, `branch` and `generation` equal the branch's, and its entry list equals the branch's entries by id, envelope, sequence and timestamp. This is full-envelope matching against **retained lossless evidence**, not against a re-authored projection edit (finding 2, 16.1). |
| D5 | For `Discard`: `StudioDiscardConfirmation` is present and exact, in addition to D3. No archive is required. Nothing else substitutes for it (finding 9). |
| D6 | Intent and storage preflight for the complete replacement peak, including at the vault cap. |

**The write.** `EpochIntentState` is rebuilt as the ledger with exactly the branch's annotated ids
removed and the extension with `active` cleared and `disposed` set, then encoded, preflighted,
sealed and atomically persisted in **one** replacement through the existing writer and sync barrier.
Only durable completion returns an acknowledgement. For `Preserve` the archive is already durable
before this transaction begins (6.5), so the ordering is archive -> flush -> single atomic intent
replacement, which is 16.2's requirement.

**What is not touched.** No ordinary intent is removed; the removal set is exactly
`disposed.entries`. The receipt-retirement path keeps its existing overlay filter and remains
incapable of removing an annotated id. No source, recovery, owner-journal, Registry or blob write
accompanies the transaction, and **disposal performs no unlink**.

**Reference consequence, corrected (reviewer's N19 point).** Disposal stops the branch from
contributing to the conservative reference set. For `Preserved`, the archive record's own reference
collection (6.5) continues to protect the branch's base-only, superseded, removed and pending CIDs,
so nothing the archive names becomes reclaimable. For `Discarded`, any CID with no other holder
becomes reclaimable after a complete scan, which is exactly what the user confirmed. Revision 1's
claim that the destination's copied operations retain the branch's references was wrong and is
withdrawn: a destination operation retains only the CIDs it names.

**Honest labelling.** `Preserved` satisfies recovery-before-removal: the complete envelopes, order,
timestamps, seed, scope and provenance are retained losslessly and their pixels stay protected.
`Discarded` is an explicit **waiver** of preservation, not a satisfaction of it, and every result
and hooks row says so.

### 6.5 The draft archive record

Placement, answering 16.1's requirement that any archive representation have its own bounded schema,
provenance and coordinated inventory and writer design.

- **Family, corrected (finding 2).** A distinct physical record kind
  `EpochRecordKind::DraftArchive` with suffix `.draft-archive`, sealing domain
  `b"catcoms/epoch-draft-archive-store/v1"`, its own scope function and canonical path
  `epoch_draft_archive_path(&scope)`, its own authenticated reader and sealed cap, its own inventory
  key, final and temporary recognition, `epoch_files_absent` participation and I-4 invalidation. It
  is **not** an independent authority or storage-budget family: coverage is gated by
  `includes_intents()` and every byte is charged to `EpochIntentBudget`. Revision 2's "one more
  record inside the existing Intents family" is withdrawn: A6 shows the family is chosen by filename
  before decryption and the authenticated scope domain must then equal that family's domain, so a
  second record under `EpochRecordKind::Intents` has no representable identity.
- **Schema.** Plaintext is the archive scope bytes, an archive header (version, sequence, `at`,
  `branch`, `content`, `generation`, `provenance`, `replayable`) and the section 5.3 payload,
  bounded by the derived `MAX_DRAFT_ARCHIVE_RECORD_BYTES` and sealed at
  `MAX_DRAFT_ARCHIVE_SEALED_BYTES` (5.3).
- **Cardinality and quotas.** At most **one archive per logical document**. A second preserving
  disposal on the same document requires the user to release the existing archive first, through
  the separate confirmed `studio_overlay_archive_release`. Vault-wide archives are capped at
  **16 MiB inside** the existing `MAX_VAULT_INTENT_BYTES` of 64 MiB, not additional to it. Refusals
  are `ArchiveCapacity` and retain all work.
- **Accounting.** Charged in `EpochIntentBudget.records`, `record_slots` and `bytes`, with the full
  replacement peak, temporary siblings and both generations invalidated on failed I/O, exactly as
  the intent record is.
- **Inventory.** The `DraftArchive` arm charges its bytes and its record slot into
  `EpochIntentBudget` alongside `Intents`. The **reference** arm decodes its bounded canonical
  payload and collects the seed projection's CIDs and every operation's CIDs, the same two sets
  `base_blob_cids()` and `hold_creative_operation` produce for a live branch, so an archived CID is
  part of the durable conservative set before the branch's own pins are allowed to disappear. A
  corrupt or unsupported archive fails closed for reclamation, as every other record does. This is
  the coordinated inventory work 16.1 demands and it must be agreed with Agent 1 (C-1, C-3, I-4)
  and Agent 3.
- **Ordering.** The archive is written and flushed as its own accounted replacement, taking Agent
  1's `epoch_mutation_guard` once that exists (see the sequencing note below), **before** the
  disposal transaction. A crash between them leaves the
  archive durable and the branch intact; the exact retry re-verifies D4 and proceeds. A crash during
  the archive write removes nothing.
- **Authority.** The archive is never `StudioRecovery`, never replay evidence, never a source,
  never importable, and never occupies a recovery slot or an eviction deadline. It is readable and
  exportable, and it is destroyed only by the explicit release action.

**The seam has landed, and it carries two guarantees this design inherits** (Agent 1, `705d44b`,
verified in source by Agent 2):

- **The coverage condition is structural, not conventional.** Revision 6 asked only that "reference
  scans run at full coverage" be preserved. `EpochStorageScan::collect_creative_references` in fact
  **refuses** unless `progress.visited_entries == 0` and
  `coverage() == RecoveryOwnerReceiptsIntentsRegistryAndStudio`, and
  `finish_creative_references` is the only path to `Protection::install`. A narrower or partly
  consumed scan therefore cannot install a protection set at all. The hazard that gating by
  `includes_intents()` might let a narrow scan install a set omitting archive CIDs is closed by
  construction rather than by discipline.
- **The reference arm fails closed until this design's collector replaces it.** A reference scan
  that meets an archive returns "draft archive reference collection is not implemented" rather than
  completing with a known set that omits the archive's CIDs. The refusal is scoped to
  `self.references.is_some()`, so ordinary accounting scans still count archives and only reference
  scans refuse. **Agent 2 accepts this and asked for it to stay.** Its value is that it makes it
  impossible to land `write_studio_draft_archive_with_io` before the collector: doing so breaks
  reclamation loudly instead of silently reclaiming archived pixels, and loud is the correct
  direction for the one failure that would destroy the preservation guarantee.

> **I-5 (new).** When this design's collector replaces that arm, the fail-closed behaviour is
> **narrowed, never removed**: the arm stops refusing every archive and starts refusing any archive
> whose bounded canonical payload it cannot decode. 6.5's rule that a corrupt or unsupported archive
> fails closed for reclamation is that same guard after narrowing, and it must be visible in the
> diff as a narrowing rather than a deletion.

Mutation succession, **corrected after implementation**. The original wording said M19 "retires
with the arm it guards". That was wrong, because the arm narrows rather than vanishes: an archive
the collector cannot read must still fail the scan closed, which is exactly what M19's vault
exercises. What retires is the **name and the message assertion**, not the coverage.

- **M19 is retired as a mutation.** Its test survives, reclassified as an **empty-vault
  corrupt-archive regression**: it reaches the refusal from a vault with no branch at all, which
  none of the collector-side tests do, and it asserts the refusal rather than its wording, because
  the narrowing necessarily changes the reason.
- **M28** replaces it as the archive reference guard, with a different assertion: removing the
  collector arm yields a set missing the archive's CIDs, failing N19.
- Two further mutations are required alongside M28, because M28's whole-arm deletion cannot
  distinguish them: **M28a** removes the payload-to-outer-scope document binding, which would
  otherwise install one group's references under another and leave the pin set looking complete;
  **M28b** swallows a reference-extraction failure into an empty set, which would turn uncertainty
  into an installed, known, incomplete pin set. Both are fail-closed paths that the whole-arm
  mutation passes straight through.

`write_draft_archive_for_test`, Agent 1's `cfg(test)`, `pub(in crate::store)` hand-sealer, was
originally specified here as deleted when `write_studio_draft_archive_with_io` lands. **That was
wrong, for the same reason the M19 retirement was wrong.** The fail-closed tests need to write
payloads that are deliberately *not* valid archives: an undecodable body, a corrupted seed, a
payload naming another document. A production writer will never produce any of those, so deleting
the helper would delete the only way to reach the guards that exist for them.

The helper therefore survives, narrowed to that purpose, and the tests that write a **valid**
archive move to the real writer. It bypasses nothing the seam validates, since the production
scope, sealing, framing and path are real and only the body is synthetic, which is exactly what
makes it the right tool for a deliberately malformed body.

**Sequencing against I-4.** Agent 1's `epoch_mutation_guard` does not exist yet; I-4 is last in its
sequence. The seam commit that adds the `DraftArchive` family deliberately contains **no writer**,
so there is nothing for it to guard and the seam is landable without I-4. Pulling I-4 forward for
this would reorder Agent 1's remaining work for no present benefit and is not requested. The
obligation instead attaches where it will be acted on: **`write_studio_draft_archive_with_io` and
`release_studio_draft_archive_with_io` are enumerated in I-4's participant list**, alongside the
recovery, owner, Registry, Studio source, intent and cleanup writers that also rotate nothing today.
Until I-4 lands, the archive writers are in exactly the same position as every other five-family
writer, which is the position I-4 exists to fix. A commit-message note alone would not be enough,
because nobody greps commit messages; the participant list is the checklist the sweep actually
follows.

### 6.6 Request identity and rollover (finding 3)

`minimum_new_basis_closed_epoch` is still not advanced by disposal, because a fresh Save on a
still-eligible basis after a disposal is a legitimate new decision. The rollover defence is instead
a namespace:

- `branch_generation` is monotonic per logical document, starts at 1, and is incremented exactly
  once when a branch is first accepted where none exists. It is never reset or reused, and
  exhaustion refuses new branches rather than wrapping.
- `branch_id = H("catcoms/studio-overlay-branch/v1", basis fingerprint, branch_generation)`.
- `studio_overlay_begin` returns the branch id that a new acceptance would create or extend: the
  current id when an active branch exists on that basis, otherwise the id for
  `branch_generation + 1`. It is derived, never reserved, so nothing durable changes and a `begin`
  result that another visit has overtaken simply stops matching. `studio_overlay_read` returns the
  current branch's id.
- Every Save, copy and disposal request carries that `branch`.

**Two-stage resolution (finding 1).** Revision 2 collapsed "unknown identity" into `Stale`, which
also refused the legitimate first Save of the next generation, so N17b could never reach its own
second step. `branch_id` is a hash and cannot be inverted, so the cheap structural stage genuinely
cannot tell the two apart; the stage that mints the basis can. The resolution therefore splits
across Agent 1's existing Flow S stages, adding no new stage and preserving AG1-001:

| Stage | Work | Answer |
|---|---|---|
| S1, structural, no basis | `classify_request` matches `branch` against the active branch, the `completed` manifest and the `disposed` manifest, in that order | `Active`, `Transferred`, `Disposed`, or `Unmatched` |
| S1a | terminal acknowledgement for `Transferred` and `Disposed` | releases |
| S1b, authorizing, basis minted here anyway | for `Unmatched` only: `admit_new_branch(target, branch, &fresh)` requires no active branch and `branch == H(domain, fresh.fingerprint(), branch_generation + 1)` | `New { generation }` or `Stale` |

So the full answer set is: exact current branch to `Active`; retained terminal generation to its
acknowledgement; **exact derived next generation under fresh live authority to `New`**; an older
generation, a skipped generation, an unrelated basis and any unknown identity to `Stale`. The
generation increment and the first accepted envelope become durable in the same sealed replacement
at S3, so there is no reserved-but-uncommitted generation and no second durable transition.

The reviewer's trigger now resolves safely end to end: accept G1 (generation 1 admitted as `New`),
dispose G1, `begin` offers generation 2, the first G2 Save is admitted as `New`, dispose G2
replacing the manifest, then deliver a delayed exact retry of a G1 request. G1's id names generation
1, which matches no live branch and no retained manifest, and at S1b it is not the derived
next generation either, so it returns `Stale`. G1's work is not resurrected, no unbounded history is
kept, and the legitimate G2 acceptance is no longer collateral damage. Retaining one manifest
preserves the terminal acknowledgement for the most recent disposal; forgetting older ones degrades
to refusal, which is the safe direction and which the reviewer's 16.3 accepts.

## 7. Stale, rewound and nonpristine bases

Unchanged from revision 1 except for S3. `studio_overlay_lifecycle` classifies from durable state
alone: `SourceMissing`, `SourceNotClosing`, `SourceReplaced`, `SourceRewound`, `CloseMissing`,
`ReceiptChanged`, `SuccessorNotPristine`, `SuccessorMissing`, `TenureUnknown`, `Fault`,
`NotCurrentAuthor`, and `Unconfirmed(..)`. Automatic transfer is refused for every one of them; the
manual path remains available; the branch, its envelopes, its order and its protected references are
retained across restart and any refusal. `check_basis_floor` remains the independent second fence
after a rewind. Agent 1's `Hold` outcomes map onto these reasons, satisfying its prerequisite P2.

- **S3, corrected (finding 5).** A branch that is authenticated, canonical and structurally
  consistent but fails typed reconstruction is classified `Manual(NotReplayable)`. Export, archiving
  and **`Preserved` disposal all remain available**, because none of them requires reconstruction
  (5.3, 6.5). Only the typed projection view and copy planning refuse, and the native result labels
  the reconstruction failure explicitly. Revision 1's position, that discard was the only remaining
  resolution, is withdrawn.

## 8. Durable local work on an awaiting-tenure preview: separate design review

### 8.1 Provenance and the seed problem (finding 6)

The reviewer confirmed the direction and identified that the current preview callback cannot supply
the original checkpoint bytes: `prepare()` consumes `raw`, `PreparedProvisionalStudioSeed` retains a
parsed `UnconfirmedStudioSeed`, and the tail subsequently advances its projection, so
`projection.checkpoint(..)` no longer reproduces the seed (A2).

Correction, in three parts:

1. **Retain the exact bytes.** `UnconfirmedStudioSeed` gains a private
   `seed_bytes: Zeroizing<Vec<u8>>` set in `parse` to the argument it has just proved equal to
   `projection.checkpoint(receipt.close_record_hash)?.bytes()`. It is immutable and unaffected by
   the tail.
2. **Expose it only through the existing scoped callback.**
   `ProvisionalStudioSeedUse` gains `seed_bytes: &'a [u8]`, so every current-scope check the
   accepted contract already performs (mount, numeric server, channel, copied watch, attempt
   generation, current membership, proven provider identity, unexpired hint) gates access to it.
   No public accessor and no `Clone` is added.
3. **Do not trust the retention.** The mint copies the bytes under custody, and the **detached**
   plan stage re-runs `UnconfirmedStudioSeed::parse(target, &receipt, &captured_bytes)` before the
   branch is built. That re-proves the receipt binding, the canonical compact encoding and the
   seed-to-projection identity from first principles. A mismatch refuses with no durable change.
   The same re-parse runs on every restart reconstruction of the branch.

**Memory and capacity accounting**, which the reviewer required to be explicit: a ready preview now
retains its parsed graph **and** its original seed bytes, at most 2 MiB each. That retention is
inside the existing retained-seed slot, not additional to it, and the design's own rails (8.3) count
it. With the accepted three preview-eligible slots the worst case adds up to 6 MiB of retained bytes
across the runtime, which must be measured (15.6) and reported honestly, not assumed.

```rust
/// Minted ONLY inside a live `with_provisional_studio_seed` callback. No public constructor and
/// no path from a caller-supplied receipt, epoch id or projection.
pub struct StudioUnconfirmedOverlayBasis(/* private */);
impl StudioUnconfirmedOverlayBasis { pub fn fingerprint(&self) -> [u8; 32]; }
```

It binds `target`, the local device as `author`, the candidate `Receipt` bytes, the exact seed
bytes, the provider `DeviceId`, the current MLS epoch and the receiver-clock observation time;
`source_id` and `source_version` are canonically zero. Additional mint conditions: `tail_complete()`
must be true; the target's logical document must have **no installed source**; the requester and the
provider must both be current members and the provider the proven endpoint identity of the hint's
peer. `fingerprint()` covers the provenance discriminant, so a `Closing` and an `Unconfirmed` basis
over the same receipt and seed cannot be interchanged.

### 8.2 What is persisted, and why the tail is not

The persisted base is the seed checkpoint only, at
`doc_id = epoch_id(doc_type, logical_key, closed_epoch + 1, close_record_hash)`. The signed tail is
not persisted: it is bounded at 20,000 operations and 4 MiB, which cannot coexist with a 2 MiB seed
in a 5 MiB record, and persisting other members' signed operations would require re-verifying
foreign signatures out of the vault on every restart. The reviewer's 16.4 accepts this.

The consequence is stated plainly and tested: **typed admission runs against the seed-only base**, so
an operation valid only against the tail is refused at acceptance with an explicit reason, before
any durable change, even though the live preview displays the merged content. Section 11's native
result distinguishes the merged preview view from the persisted draft base.

### 8.3 Local storage and quotas

Simultaneous, not additive, and all checked before acknowledgement. The reviewer's 16.5 endorses the
layered shape and correctly notes the values are unvalidated.

| Rail | Value |
|---|---|
| Unconfirmed branches per logical document | 1 |
| Unconfirmed branches per server | 3 |
| Accepted operations per unconfirmed branch | 64 |
| Seed bytes | 2 MiB (existing checkpoint limit) |
| Extension metadata | 64 KiB (unchanged) |
| Record total | `MAX_RECORD_BYTES` (unchanged) |
| Vault-wide unconfirmed persisted bytes | 8 MiB, **inside** `MAX_VAULT_INTENT_BYTES` |
| Retained original seed bytes per ready preview | 2 MiB, inside the existing retained-seed slot |

Per-document, per-server and vault-wide limits all apply together with the record and metadata
limits; a per-channel limit alone would allow aggregate growth as channels accumulate. The
per-server count and vault-wide byte total come from the inventory's Intents arm, which requires
Agent 1's structural decode to expose the provenance discriminant and the charged bytes (section
14). Finding 6's retained seed bytes are counted in the **memory** accounting as well as the
persisted-byte accounting. Refusals are `StorageRefused { reason }` and retain all existing work.

### 8.4 Expiry versus retained work

Unchanged from revision 1. Preview expiry, capacity eviction, replacement, unwatch and rewatch,
lock, mount or server replacement, membership change and restart remove the live preview and never
the durably accepted branch; a retained branch never revives a preview, extends a hint lifetime,
re-enters the ready cache or produces an `AwaitingTenureReceipt` result. A refused acceptance leaves
editor work unsaved and visible and reports no durable success.

### 8.5 What an unconfirmed branch can never do

Installed source; epoch gate; `VerifiedCheckpoint`; owner tenure; receipt issuance, verification or
publication; signing; Registry pointer publication; settlement; receipt-covered retirement;
`StudioRecovery` evidence; replay evidence; ordinary Apply; automatic handoff or any `Prepared`
state, which `validate` forbids outright. What it can do: the same four manual operations with the
same fences, plus the derived classification below.

### 8.6 Reconciliation, derived rather than persisted

Computed on read from durable state; nothing is written, so there is no reconciliation crash window.
`AwaitingSource` while no installed source exists. `BaseConfirmed` when the installed source's
`doc_id` equals the branch's base `doc_id` **and** its opening checkpoint's seed change hash equals
the branch's, in which case copy into that source becomes available when it is Open.
`BaseSuperseded` otherwise, with copy still offered against the actual current projection under an
honest label. `BaseConfirmed` is a statement that two hashes agree, never a promotion of preview
attribution, tenure or signing authority.

### 8.7 Acceptance path

Distinct control actions so neither path can be reached with the other's request:
`BeginUnconfirmedOverlaySave`, `PrepareUnconfirmedOverlaySave { branch, nonce, body }`,
`FinishUnconfirmedOverlaySave`. Staging, stamps, admission, permit ownership, PIX admission
placement, retry classification and commit ordering are Agent 1's Flow S unchanged, with three
substitutions: `studio_closing_basis` becomes the 8.1 mint; the S3 re-mint re-enters
`with_provisional_studio_seed` and requires the same fingerprint; and 8.3's rails are charged
alongside the ordinary ones. The detached stage re-parses the captured seed bytes (8.1 part 3). If
Agent 1's Flow S is not implemented, this path is not implemented either; it is not a second writer.

## 9. Repeated-owner tenure

### 9.1 What already works

`ReceiptBook` and `OwnerReceiptJournal` are tenure-keyed, and a continuously present member already
observes A -> B -> A correctly. The integration work is to prove this end to end through real
actors, real membership changes and real restart, and to prove the refusals.

### 9.2 The required cases

T1 A -> B -> A with a continuously present observer; T2 restart between each transition; T3 a member
joining between owner changes; T4 hidden higher old-tenure history; T5 a device that becomes owner
by its own join. Details as in revision 1; T5 is section 9.3.

### 9.3 The correction, and how L5 is closed (finding 7)

**Problem.** `new_joined` calls `OwnerTenure::unknown` unconditionally, so a device joining into a
recycled low leaf becomes the designated committer with `None` and can never issue a receipt, while
every witness knows the answer.

**Part 1: the self-join inference, rephrased.** Revision 1 justified it as "was not a member at any
earlier epoch", which the reviewer correctly says is false for a returning device. The accurate
statement is about **current continuous membership**:

> This device's current continuous membership in this group began at this epoch. A tenure is an
> uninterrupted run as designated committer, so this device's *current* tenure cannot have begun
> before its current membership did. If it is the committer now, its current tenure started here.

```rust
// crates/catcoms-sync/src/owner_tenure.rs
impl OwnerTenure {
    /// Deliberately NOT available to `unknown`, which also serves legacy snapshots where the
    /// device may have been committer for an unknown number of prior epochs.
    pub(super) fn joined(group: &ServerGroup, device: &MlsDevice) -> Self {
        let mut state = Self::unknown(group);
        if group.designated_committer() == Some(device.device_id()) {
            state.start = Some(state.position.epoch);
        }
        state
    }
}
```

`new_joined` calls `joined(&this.group, &this.device)`. `unknown`, `new`, the restore path,
`applied`'s existing arms, `start` and `decode`'s `start > epoch` rejection are untouched.

**Part 2: why the old safety argument is withdrawn.** Revision 1 claimed a wrong value produces
receipts nobody accepts. A4 shows that is false: `complete_checkpoint_head_scoped` refuses a
proof/observation mismatch only when local observation is `Some`, so an Unknown-tenure newcomer
accepts a fresh proof's claimed tenure and can hold a selection a continuously observing witness
refuses. Disagreement is therefore not self-correcting, and "until a later witnessed transition" is
not a progress guarantee. Agreement must be established structurally.

**Part 3: extending the observation rule so every participant sees the same discontinuity.** A
witness currently compares `Position { owner: Option<DeviceId>, epoch }`, which cannot distinguish a
same-commit remove-and-re-add of the committer from an ordinary same-owner commit. `Position` gains
the committer's leaf identity:

```rust
// catcoms-mls
impl ServerGroup {
    /// Leaf index and a digest over the designated committer's leaf identity:
    /// blake3(index, signature_key, credential bytes). The HPKE `encryption_key` is
    /// DELIBERATELY EXCLUDED so that an ordinary self-update, which rotates keys but keeps the
    /// credential, does not look like a discontinuity.
    pub fn designated_committer_leaf(&self) -> Option<(u32, [u8; 32])>;
}

// catcoms-sync
struct Position { owner: Option<DeviceId>, leaf: Option<(u32, [u8; 32])>, epoch: u64 }
```

`applied` gains one explicit arm: across a contiguous step, when `before.owner == after.owner` and
both are `Some` but `before.leaf != after.leaf`, the result is `Some(after.epoch)`, a new tenure.
Every other arm is unchanged, so same-owner commits and self-updates still preserve knowledge.

The credential is the correct discriminator because `group.rs:247-274` binds a joiner's KeyPackage
credential to `(this group, invite_nonce)`, so a genuine rejoin always presents a different
credential, while an update does not change it (A3). Under this rule the witness computes
`Some(after.epoch)` and the rejoining device computes `Some(join epoch)`, which is the same value.
L5's disagreement is closed.

**Part 4, corrected (SEC-PAIR-001 finding 4): the rule must bind every applier, not the builder.**
Revision 2 excluded the invisible case with two conditions, and the reviewer showed neither binds a
witness. The invite ledger is **local to the admitting party**: A7 records that
`process_incoming`'s own comment says single-use nonce enforcement stays with the admitting
committer's ledger, and that every other member can check only that an Add's credential names this
group and matches its leaf key. A malicious, modified or simply buggy committer can therefore build
a same-commit remove-and-re-add that reuses the signature key and credential bytes, and honest
witnesses will merge it, preserve the old tenure start, and diverge from the rejoining device. The
invite ledger is demoted to what it actually is: honest-join admission, not an authority invariant.

The prohibition moves to the **receive side**, into the pre-merge staged-commit inspection that
already exists at [group.rs:466-490](../crates/catcoms-mls/src/group.rs#L466):

> **M-1.** A single commit must not both remove the **pre-commit designated committer** and add the
> same `DeviceId`. Enforced on every staged commit before `merge_staged_commit`, and identically in
> the local commit builder.

Stated over `DeviceId`, not leaf index, so it does not depend on whether OpenMLS happens to recycle
the same leaf. Both inputs are available at that point: the pre-merge roster is still the pre-commit
roster, so `designated_committer()` and the removed leaves' device ids are both derivable, and the
added device ids come from the `add_proposals()` walk the method already performs for its credential
check. A violating commit is rejected before merge with the existing `InviteError`-style refusal
path, so no member ever reaches the ambiguous position.

What M-1 does not forbid: a device rotating to a new identity (remove A, add A', different
`DeviceId`), or a genuine rejoin in a later commit. Only the one ambiguous shape is excluded. If a
same-commit remove-and-re-add is ever needed, it requires an authenticated membership-incarnation
value every member can verify independently, which is not proposed here.

**Part 5, new: the v1 snapshot migration.** Revision 3 said a v1 snapshot could take `owner`,
`epoch` and `start` from its bytes and the leaf identity from the live group. That is unsound, and
the reviewer is right that it reintroduces exactly the ambiguity part 3 exists to remove: a v1
`start` was computed by the **old** `applied`, whose preserve branch cannot see a same-commit
remove-and-re-add, so a snapshot written after such a transition carries a stale `start`. Grafting
the current leaf digest onto it manufactures the continuity evidence v1 never recorded. M-1 cannot
repair it, because M-1 only prevents the transition being accepted *after* the upgrade, and N-T6c
establishes that an old build could have accepted one.

Worse, corroboration cannot repair it either: every pre-upgrade participant ran the same preserve
branch and therefore holds the same stale value, so a witnessed attestation would agree with the
wrong answer. There is no local or remote evidence that distinguishes the two cases from v1 bytes.

**Which v1 states are provably safe.** Exactly those with `start == Some(epoch)`. Proof from the old
`applied`: after any call, `self.position.epoch == after.epoch`. The genuine-change branch sets
`start = after.epoch`, so `start == epoch`. The preserve branch sets `start = self.start`, whose
value was fixed when `self.position.epoch` was `before.epoch == after.epoch - 1`, and `start` is
never raised while preserving, so `start <= after.epoch - 1 < epoch`. The `None` branches carry no
start. Therefore `start == Some(epoch)` implies the most recent applied step was a genuine
`DeviceId` owner change, which is visible under **both** the old and the new rule, so no hidden
discontinuity can lie at that step; an earlier one is irrelevant because the later genuine change
reset the tenure correctly. `OwnerTenure::new`'s founding `epoch == 0, start == Some(0)` is the same
case. Conversely `start < epoch` guarantees at least one preserve step, which is exactly where the
invisible discontinuity hides.

**The migration rule, and why a bare downgrade is not acceptable either.** The sound rule is
therefore "promote only `start == epoch`, else `Unknown`". Taken alone it is too blunt: a founder at
epoch 0 that has applied any commit holds `start = Some(0)` with `epoch > 0`, so **every existing
server's owner would be downgraded to `Unknown`, lose the ability to issue receipts and rotate, and
for a single-owner server never regain it**, since only a genuine owner change re-establishes a
start. That is a worse outcome than the risk it removes. The correction is therefore to separate the
two consumers of the value, which have opposite failure directions:

```rust
// catcoms-sync
pub enum ObservedOwnerTenure {
    /// Fully observed under the leaf-aware rule, or migrated from a provably safe v1 state.
    Observed(u64),
    /// A v1 `start < epoch`, with no leaf-continuity evidence. Sound for VERIFICATION and
    /// fail-closed for AUTHORING. Never laundered into `Observed` by a save/reload cycle.
    Imported(u64),
    Unknown,
}
impl<T, R> ChannelSync<T, R> {
    pub fn observed_owner_tenure(&self) -> ObservedOwnerTenure;
    /// Verification callers: `Observed` and `Imported` both yield `Some`. Keeping `Imported`
    /// visible here is strictly safer than `Unknown`, because `complete_checkpoint_head_scoped`'s
    /// `is_some_and` accepts a proof's claimed tenure when the local value is absent, so this can
    /// only add refusals, never an acceptance `Unknown` would have rejected.
    pub fn verification_owner_tenure_start(&self) -> Option<u64>;
    /// Authoring callers: only `Observed` yields `Some`.
    pub fn authoring_owner_tenure_start(&self) -> Option<u64>;
}
```

**`observed_owner_tenure_start` is removed, not repointed** (revision-4 hardening note, adopted).
Its doc comment promises an "independently observed" tenure, which would become false for
`Imported`, and leaving the old name in place would let a future call site inherit the wrong
semantics by default. Deleting it forces every existing site to declare which it wants, and the
compiler enumerates them. The intended mapping, from the audit:

| Call site | Accessor |
|---|---|
| `receipt_head.rs:214`, `prepare_receipt_head_snapshot` | authoring |
| `receipt_head.rs:237`, `head_snapshot_is_current` | authoring. A permit cannot exist for `Imported`, so either would be correct today; authoring is the semantics that stays correct if permit minting ever moves. |
| `receipt_head.rs:526`, the proof-signing check in `serve_receipt_head_with_handoff` | authoring. Signing a proof as the owner is authoring, not verification. |
| `detached.rs:175`, `complete_checkpoint_head_scoped` | verification |
| `studio/overlay.rs`, all three of `handoff_studio_overlay`, `prepare_studio_closing_overlay`, `save_studio_closing_overlay` | authoring accessor, **value passed through as `Option<u64>`; the refusal stays in the store.** See the rule below |
| `owner_tenure/tests.rs`, `studio_exchange/tests/*` | whichever the case asserts; several existing assertions become `verification_*` and must be read individually rather than renamed mechanically |

> **A-1 (revision-5 finding 1). The accessor choice and the refusal site are separate decisions.**
> A **store-delegating lifecycle wrapper** reads `authoring_owner_tenure_start()` and passes the
> resulting `Option<u64>` through unchanged. It does **not** call `require_observed_owner_tenure()`
> on the way in. The store owns every refusal, at the stage that actually needs a tenure.
>
> **Scope (revision-6 refinement).** A-1 is a rule about *deferring to a store stage that already
> classifies durable state*, not a general licence to defer. An app-level operation that performs
> irreversible authoring of its own before reaching such a stage must check its authority where that
> authoring happens; it cannot skip the check merely because A-1 exists. No operation in this design
> is of that shape, and the reviewer judged the single-stage exception below sufficient without
> raising it, but the scope is stated because this document is now implementation instructions and
> the revision-5 finding showed how brittle a whole-function reading is.

Revision 5 classified all three wrappers as "authoring, through `require_observed_owner_tenure`",
which would have moved the refusal above work that deliberately precedes it. The committed store
ordering is:

| Function | Order |
|---|---|
| `epoch_studio/handoff.rs` `handoff_studio_overlay_with_io` | `completed_branch` (:86), then `resolve_studio_handoff_with_io` (:109), then `completed_branch` again (:121), and **only then** `tenure.ok_or_else` (:128) |
| `epoch_studio/overlay.rs` `save_studio_closing_overlay_with_io` | After Agent 1's FS-002 the sequence is: decode and bound the request, enter budget, structural state read, `completed_retry`, `exact_retry`, ordinary collision, **then** `tenure.ok_or_else`, then current Closing source, fresh basis, basis comparison, media admission, capture, detach, commit. Every acknowledgement and exact-retry branch still returns or refuses strictly before any tenure is required. Media admission moved to **after** tenure, which is FS-002's own correction and is orthogonal to A-1 |
| `epoch_studio/overlay.rs` `prepare_studio_closing_overlay` | `tenure.ok_or_else` at :53, immediately. This one really is pure authoring, and A-1 still holds for it: the store refuses, the wrapper does not |

**A-1's precondition is structural, not merely ordering.** Agent 1's reverification against its
in-flight tree established something stronger than the ordering this design asked it to confirm:
`resolve_studio_handoff_with_io` **takes no tenure parameter at all**. The Prepared-resolution path
therefore cannot acquire a tenure requirement by someone moving a statement; it would take someone
adding an argument to a signature that has no use for one. The L11 limbo case that A-1 exists to
prevent cannot arise from ordinary drift. N-T7b is retained anyway, as defence in depth and as the
end-to-end reachability proof, but it is no longer the only thing standing between a refactor and
permanent data limbo. The Save path has no equivalent structural guarantee, because its tenure is a
parameter, so there the ordering and N-T7b remain the guard.

Hoisting the check would therefore have broken two accepted properties: idempotent acknowledgement
surviving the loss of authoring authority, which is Agent 1's AG1-001 rule that acknowledgement
needs no fresh basis or tenure; and restart recovery of an **already durable** `Prepared` handoff,
which needs no authority to *start* a new one. The second is the worse of the two under L11, because
an `Imported` single-owner server stays `Imported` indefinitely by design, so a crash leaving a
Prepared overlay would have become permanently unresolvable.

A-1 is stated as one rule rather than a per-wrapper "pure or mixed" classification on purpose: a
classification invites the next person to judge a new wrapper, and judging it wrong is exactly this
finding. Under A-1 there is nothing to judge, and `require_observed_owner_tenure()` is reserved for
call sites that are a single authoring stage with no acknowledgement or recovery branch at all,
which today means Agent 1's S1b basis mint and Agent 3's repair issuance.

Line numbers are deliberately absent for Save: they moved twice in two days. What is invariant is
the order and, for handoff, the signature.

**The reverification was load-bearing, not ceremonial.** Between this design's base and Agent 1's
FS-002 at `5a024a7`, `tenure.ok_or_else` did move earlier in `save_studio_closing_overlay_with_io`.
A-1 survived because it moved to just after the collision check rather than above the retry
branches, but that is the exact drift A-1 exists to catch, and it happened immediately. Agent 1 has
since bracketed the ordering with assertions on both sides,
`studio_overlay_exact_retry_is_acknowledged_without_media_admission` and
`studio_overlay_stale_basis_is_refused_before_any_media_admission`, so the Save path now has
executable guards of its own. N-T7b remains Agent 2's, and remains the only end-to-end proof that an
`Imported` server can still acknowledge and still resolve a Prepared handoff.

`prepare_receipt_head_snapshot` switches to `authoring_owner_tenure_start`, which is the change that
closes the safety hole at the sync layer: a device carrying a stale v1 start can no longer mint the
durable owner permit, and every later step derives its tenure from that permit rather than minting
authority independently. Verification behaviour is unchanged in both directions.

`Imported(s)` becomes `Observed(s')` on the first leaf-aware transition that sets a start, which is
a genuine owner change or part 3's leaf-discontinuity arm. It is never promoted by time, by a
restart, by a proof or by a peer's agreement.

**Snapshot format.** `encode`/`decode` gain a versioned tail carrying the committer leaf digest
**and a one-byte flag distinguishing `Observed` from `Imported`**; the 57-byte cap becomes 98.
Without that flag a save-and-reload would launder `Imported` into `Observed`, which is the same
defect one step removed. A v1 snapshot decodes to `Observed` when `start == Some(epoch)`, to
`Imported(start)` when `start < epoch`, and to `Unknown` when `start` is absent, taking `owner` and
`epoch` from its bytes and the leaf identity from the live group under the existing position
equality check, which still refuses a stale position, and keeping the `start > epoch` rejection.

**What this does not fix.** A legacy snapshot whose owner has no saved tenure bytes stays Unknown,
correctly. A device that becomes committer across an epoch gap it did not observe stays Unknown.
Section 9.5 is not implemented.

### 9.4 The live-tenure seam for Agents 1 and 3

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioOwnerTenure {
    Known(u64),
    /// A v1 snapshot's `start < epoch`, with no leaf-continuity evidence (9.3 part 5).
    /// Usable for verification, fail-closed for authoring.
    Imported(u64),
    Unknown,
}
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    pub fn observed_owner_tenure(&self) -> StudioOwnerTenure;
    /// Returns `Ok` only for `Known`.
    pub(crate) fn require_observed_owner_tenure(&self) -> Result<u64, AppError>;
}
```

- **V1.** `Unknown` **and `Imported`** are both fail-closed for **new authoring**, refused at the
  stage that needs the tenure and not at the wrapper that reads it (A-1): minting a Closing overlay
  basis, first local acceptance,
  handoff preparation, every signing turn, the commit, receipt issuance, rotation, Registry pointer
  publication, and Agent 3's live v2 repair issuance and application.
- **V2.** A returning owner in a new tenure observes a strictly different value from its earlier
  tenure, now including the 9.3 part 3 case.
- **V3.** The value is read at every custody visit that needs it and never cached across one.
- **V4.** A verified-reading tenure from a fresh owner proof travels only inside the existing
  private `HeadSelection` / `RegistrySeedFetch` / `CheckpointSeedSelectionUse` types. It is never
  written into `owner_tenure`, never returned by `observed_owner_tenure`, and never usable as this
  device's own authoring tenure. A4 is why this separation matters.
- **V5.** Agent 3 takes `require_observed_owner_tenure()` for issuance and holds, never substitutes,
  on `Unknown` and on `Imported`.
- **V6 (9.3 part 5).** `Imported` is never promoted by a restart, a save/reload, elapsed time, a
  fresh owner proof or a peer's agreement, and it is never returned by
  `authoring_owner_tenure_start`. It is visible to verification precisely because hiding it would
  make the device accept a proof's claimed tenure instead of comparing against what it holds.
- **V7 (new, revision-4 finding 1).** The app-level conversion is total and lossy in one direction
  only: `Observed(s)` becomes `Known(s)`, `Imported(s)` becomes `Imported(s)`, `Unknown` becomes
  `Unknown`. No app path may collapse `Imported` into `Known`, and
  `require_observed_owner_tenure()` succeeds for `Known` alone. The two halves of that statement are
  separately anchored mutations, M22g and M24b, because an implementation can satisfy the sync-layer
  tests while still laundering the value at the app boundary.
- **V8 (new, revision-5 finding 1).** Fail-closed means **new authoring is refused**, not that every
  path is refused. Under `Imported` and under `Unknown`, all of these stay reachable and must keep
  working: an exact accepted Save retry returning its saved entry, a completed-handoff
  acknowledgement, and resolution of an already durable `Prepared` handoff. V1's refusals and V8's
  reachability are complementary halves of the same contract, tested as N-T7 and N-T7b, so that
  "fail closed" can never degrade into "refuse everything whenever the tenure is unusable".

### 9.5 Not proposed for implementation

The witnessed-transition attestation protocol is recorded as the considered and rejected
alternative for legacy-snapshot and unobserved-gap owners. It is a new authority protocol whose
guarantee is a quorum-of-witnesses property rather than a cryptographic proof, and the reviewer's
16.3 agrees it is not a substitute for the 9.3 integration.

## 10. References, admission and budgets

- **R1.** Retained branches keep their conservative protection unchanged: the inventory's reference
  path enumerates `base_blob_cids()` plus every pending operation's CIDs.
- **R2.** Copy adds ordinary references through the destination's own operations, under
  `hold_creative_operation` and the ordinary admission and possession checks.
- **R3, corrected.** Disposal performs no unlink. For `Preserved`, the archive record's reference
  collection (6.5) keeps every CID the branch named protected. For `Discarded`, a CID with no other
  holder becomes reclaimable after a complete scan. A destination copy retains only the CIDs it
  names and is not a preservation mechanism (reviewer's N19 point).
- **R4.** The disposal and archive writes obey the same budget discipline as every other intent
  write: full replacement peak preflight, both generations invalidated on failed I/O, refusal at the
  vault cap without spending deletion credit, and an exact sync-only retry needing no replacement
  headroom.
- **R5.** Unconfirmed branches charge the ordinary per-server content budget in addition to 8.3's
  rails, consume no settlement or protocol reserve and create no unscanned cache.

## 11. Native results, events and the proposed UI-hooks update

```ts
type OverlayInspection =
  | { v: 1; kind: "absent"; channel: Decimal; object: Hex32 | null }
  | { v: 1; kind: "local-draft"; channel: Decimal; object: Hex32 | null;
      basis: Hex64; branch: Hex64; content: Hex64; generation: Decimal; accepted: number;
      transferState: "active" | "prepared";
      provenance: "closing" | "unconfirmed";
      eligibility: "transferable" | "manual";
      manualReason: OverlayManualReason | null;
      unconfirmedState: "awaitingSource" | "baseConfirmed" | "baseSuperseded" | null;
      replayable: boolean; archived: boolean;
      readOnly: true; content_: StudioContent | null }
  | { v: 1; kind: "disposed"; channel: Decimal; object: Hex32 | null;
      basis: Hex64; branch: Hex64; generation: Decimal; accepted: number;
      mode: "preserved" | "discarded"; archive: Hex64 | null };

type OverlayManualReason =
  | "sourceMissing" | "sourceNotClosing" | "sourceReplaced" | "sourceRewound"
  | "successorNotPristine" | "successorMissing" | "receiptChanged" | "closeMissing"
  | "tenureUnknown" | "fault" | "notCurrentAuthor" | "unconfirmed" | "notReplayable";
```

`replayable: false` carries a null typed projection with every other field present, which is
finding 5's user-visible shape.

**Write outcomes, corrected (finding 8).** Every command that writes reports exactly one of three
states, and they are never conflated:

| Outcome | Meaning | Correct client action |
|---|---|---|
| `refused` | The refusal happened before any durable write. Nothing changed. | Fix the named condition; a new attempt is safe. |
| `uncertain` | A write may have landed. | Retry the **exact** request; do not mint a new nonce or a new confirmation. |
| `partial` | A prior durable step of a multi-step sequence succeeded and a later one did not. Today the only such sequence is `archive durable, disposal pending`. | Retry the exact disposal; the archive is not rewritten. |

Revision 1's blanket rule that a storage refusal keeps the work unsaved applies only to `refused`.

Proposed rows for `FLIPNOTE-UI-HOOKS.md`, which Agent 4 applies:

| UI action | Native command | Result |
|---|---|---|
| Read a retained local draft | `studio_overlay_read` | Extended `OverlayInspection`; read-only, never an ordinary view |
| Show the lifecycle row | `studio_overlay_lifecycle` | Cheap structural state and reason; no content |
| Back up a draft to a file | `studio_overlay_export` | Bounded `{format:"catcoms-studio-draft-v1", basis, branch, accepted, provenance, replayable, bytes, bytesB64}`; changes nothing and authorizes no deletion |
| Keep a lossless in-app archive | `studio_overlay_archive` | Durable archive record; required before a preserving disposal |
| Read or release the archive | `studio_overlay_archive_read`, `studio_overlay_archive_release` | Release is separately confirmed and destroys the archive |
| Preview copying one item | `studio_overlay_copy_preview` | One proposed domain edit or an explicit hold; saves nothing; reports the source operations it resolved |
| Apply that exact copy | `studio_overlay_copy_apply` | Ordinary provisional content Save into the destination |
| Dispose of a draft | `studio_overlay_dispose` | `mode:"preserve"` requires a matching durable archive; `mode:"discard"` requires the exact confirmation token and destroys the bodies |

Truthfulness rules asserted by tests:

- `local-draft` is local only, with or without an archive or any number of copied items.
- **A copy count is never a preservation claim** (C-P). Only `archived: true` plus a matching
  `content` establishes that the branch is losslessly retained.
- `mode:"discarded"` states plainly that the bodies are gone; it is a waiver, not a preservation.
- `provenance:"unconfirmed"` is unconfirmed history; `unconfirmedState:"baseConfirmed"` means two
  hashes agree, not that the provider was ever owner. The merged preview view and the persisted
  draft base are labelled separately (8.2).
- `manualReason:"tenureUnknown"` means this device cannot presently prove the current owner's
  tenure, not that anything is wrong with the work.

Events reuse the existing bounded `SettlementNotices` rail and the `settlement-changed` channel:
`LocalDraftManual` and `LocalDraftDisposed`. Neither is a delivery, settlement or finality claim.
Agent 1's `LocalDraftRetained` and `LocalDraftHandedOff` are separate.

## 12. Crash, interruption and recovery ordering

| Interruption | Result |
|---|---|
| Any detached rebuild, including cancellation | No durable byte changed; branch, ledger, protection and permit intact until the worker drops them; the source and destination stamp rechecks refuse a stale result. |
| During the archive write, before rename | Nothing archived, nothing removed; the exact retry re-archives. |
| After the archive rename, before its flush | The exact retry reloads the authenticated archive, verifies D4 and flushes; no second archive and no second sequence. |
| Archive durable, disposal not yet attempted or failed | `partial`. The branch is intact and the exact disposal retry proceeds without rewriting the archive. |
| During the disposal write, before rename | Nothing removed, nothing recorded; the request is retryable verbatim. |
| After the disposal rename, before its flush | The exact retry reloads the record, sees `disposed`, and performs the sync-only flush `retire_included_with_io` already implements for `removed == 0`. |
| Copy interrupted at any point | The ordinary Save retry contract applies unchanged; there is no bookkeeping write to be inconsistent with (findings 1, 8). |
| Disposal requested while `Prepared` | Refused by D2. |
| Restart with a retained unconfirmed branch and no preview | Reconstructs by re-parsing its own persisted seed bytes against its receipt (8.1 part 3); `AwaitingSource` until an installed source exists. |
| Restart mid-copy with the destination rotated | The destination stamp, `epoch_id` and fingerprint refuse; re-preview against the new Open epoch. |

## 13. Prerequisites this design supplies to Agent 1

| Agent 1 requirement | Supplied by | State |
|---|---|---|
| P1 reviewed manual lifecycle | 6.1-6.6, 12 | Designed, unimplemented, unreviewed |
| P2 every `StudioOverlayHold` variant mapped to an actionable state | 7, 11 | Designed |
| P3 truthful native results, events and hooks rows | 11, including the corrected three-state write outcome | Designed |
| P4 live-tenure contract | 9.4 V1-V7, with 9.3's progress path and migration rule | Designed |
| P5 explicit statement that P1-P4 are implemented and reviewed | Status note | **No** |

Agent 1 must not register `studio_overlay_save` on the strength of this document.

## 14. Dependencies and integration changes for Agent 4

| File | Change | Note |
|---|---|---|
| `catcoms-replication/src/studio/overlay.rs` | provenance on `BasisData` and its fingerprint | Core; own verdict line |
| `.../studio/overlay/handoff.rs`, new `overlay/disposal.rs` | v3 encoding, `branch_generation`, `branch_id`, `classify_request`, the terminal `disposed` arm, extended `validate`, and the provenance guard on `prepare_handoff*` | Core; shared with Agent 1's C-1 |
| `.../studio/provisional.rs` | retained `seed_bytes` and the scoped accessor (finding 6) | Core; boundary (b) |
| `catcoms-sync/src/registry_seed/provisional/seed.rs` | `ProvisionalStudioSeedUse.seed_bytes` | Sync; boundary (b) |
| `catcoms-mls/src/group.rs` | `designated_committer_leaf()`; **M-1 enforced in `process_incoming`'s existing pre-merge staged-commit inspection** and identically in the local commit builder | **Authority-bearing; boundary (c); every member's receive path** |
| `catcoms-sync/src/owner_tenure.rs`, `lib.rs` | `Position.leaf`, the `applied` discontinuity arm, `joined`, the versioned snapshot tail **with its `Observed`/`Imported` flag**, the v1 migration rule, `ObservedOwnerTenure`, the new `authoring_owner_tenure_start` accessor and the `prepare_receipt_head_snapshot` call-site switch, and the `new_joined` call site | **Authority-bearing; boundary (c); the accessor split is the single change that closes the migration hole** |
| `catcoms-app/src/store/epoch_intents.rs` | `StudioOverlayLifecycle`; the Intents-arm archive record kind, provenance and unconfirmed byte counters | Shared with Agent 1 (C-1, C-3, I-4) and Agent 3 |
| `.../store/epoch_intents/retirement.rs` | Unchanged; the overlay filter stays | Shared with Agent 3 |
| new `.../store/epoch_intents/{disposal,archive}.rs` | The disposal transaction and the archive record | Agent 2 leaves |
| `.../store/epoch_intents/inspection.rs` | `StudioInspectionPurpose`, `rebuild_for`, the composite copy capture and destination stamp | Shared with Agent 1 |
| `.../store/epoch_studio.rs` | wider visibility for `read_studio_record` | Shared with Agent 3 |
| `.../store/epoch_recovery/inventory.rs` | **`EpochRecordKind::DraftArchive`**: suffix, domain, scope, sealed cap, `storage_name` gating by `includes_intents()`, inventory key, temporary recognition, `epoch_files_absent`, and its reference collection | **Shared with Agent 1 and Agent 3; highest-risk item after I-4. A new physical family variant touches every `match` on `EpochRecordKind`** |
| `catcoms-app/src/studio/restore.rs` | `history: &[&StudioProjection]`, `PlanScope`, `source_ops` | Agent 2 |
| `catcoms-app/src/studio/{control,dispatch}.rs` | New actions and responses | **Central enum edit** |
| `catcoms-app/src/studio/{inspection,settlement}.rs` | `rebuild_for` plumbing; two new settlement variants | Shared with Agent 1 |
| new `catcoms-app/src/studio/{lifecycle.rs, overlay/copy.rs}` | Classification and copy driver | Agent 2 leaves |
| `apps/desktop/src-tauri/*` | Nine commands, registration, security and capability rows | Agent 4 registers |
| `docs/*` shared | Section 11's rows | **Agent 4 owns; not edited here** |
| `.github/workflows/studio-overlay.yml`, `.github/scripts/` | A `lifecycle` job and a mutation script | Agent 4 owns |

**Handed to Agent 1:** structural decode must expose the provenance discriminant and charged bytes;
the Save classification must call `classify_request` rather than a bare `completed_retry`; Flow S's
basis mint is parameterized by provenance; the archive writer takes `epoch_mutation_guard` and the
inventory cursor must cover the archive record kind. **Handed to Agent 3:** the 9.4 seam with V1 and
V5; repair must resolve an interrupted Prepared overlay through the existing fence and must never
remove an annotated id outside the 6.4 transaction; an `Unconfirmed` branch is not repairable
history.

## 15. Limits, costs and measurements

Nothing here is measured. Required, for Index and Flipnote: (1) export and archive at 1, 64 and 256
operations and at the maximal record shape, separating the two custody visits from the detached
stage; (2) copy preview at the same shapes, separating destination decode, projection build and
`restore::plan`; (3) the disposal transaction at 256 entries with both manifests present, plus the
sync-only exact retry at the vault cap; (4) the archive record's effect on a five-family inventory
and on its reference arm; (5) `studio_overlay_lifecycle` on a vault with several large retained
branches; (6) the retained original seed bytes of finding 6 across three ready previews, as actual
retained memory rather than an assumed bound.

- **L1.** Copy planning and the typed projection view require reconstruction; export, archiving and
  `Preserved` disposal do not (corrected by finding 5).
- **L2.** Copy is per item, with no batch command and no batch atomicity.
- **L3.** Two full 256-entry manifests plus headers occupy roughly 43 KiB of the 64 KiB metadata
  ceiling; a document that has completed a 256-entry transfer and then accumulates a second
  256-entry branch can refuse disposal with `MetadataFull`, retaining the branch. Must be measured.
- **L4.** `Discarded` destroys the operation bodies; only the bounded manifest survives.
- **L3b (new, finding 3).** `MAX_DRAFT_ARCHIVE_PAYLOAD_BYTES` is approximately 6 MiB plus 26 KiB by
  derivation, so the 16 MiB archive sub-cap admits **two** archives at the derived maximum and about
  three at the practically reachable maximum. A third refuses with `ArchiveCapacity` and the branch
  stays retained. The sub-cap is a policy number, not a measurement, and belongs in 15.4.
- **L5, superseded twice.** Revision 1's tenure residual is closed by 9.3 part 3, and revision 2's
  reliance on the invite ledger and the local commit builder is closed by part 4's receive-side
  M-1. What remains is a coverage obligation rather than a trust assumption: M-1 must be enforced on
  every applier's staged commit before merge, and N-T6c proves it with an adversarial commit that
  bypasses the local builder.
- **L11 (new, and the most consequential limit in this design).** Every existing vault whose v1
  tenure snapshot carries `start < epoch`, which is every server the founder has applied any commit
  to, migrates to `Imported` and therefore **cannot issue receipts or rotate** until a genuine owner
  change occurs. For a single-owner server that is never. Verification and reading are unaffected,
  and no work is lost or at risk, but this is a real capability regression at upgrade for the
  installed base, and it is the price of not promoting unprovable continuity. **Decided as option
  (a) in 16.1: accepted with no operator-adoption override.** There is deliberately no technical
  escape: section 9.3 part 5 shows corroboration cannot help, since every pre-upgrade participant
  holds the same stale value. What such a server does **not** lose, by A-1 and V8, is
  acknowledgement of work it already accepted and resolution of a handoff it had already prepared;
  without that the indefinite `Imported` state would turn a crash during Prepared into permanent
  data limbo, which is why N-T7b exists.
- **L6.** A legacy-snapshot owner and an unobserved-gap owner remain Unknown and cannot rotate.
- **L7.** The persisted unconfirmed base is the seed checkpoint, not the previewed tail.
- **L8.** There is no import path for an export or an archive.
- **L9 (new).** One archive per logical document. A second preserving disposal requires an explicit
  release first, and the vault-wide archive ceiling of 16 MiB can refuse an archive on a vault that
  is otherwise within its intent budget.
- **L10 (new).** A ready preview now retains up to 2 MiB of original seed bytes in addition to its
  parsed graph; the worst case across three slots is 6 MiB of retained memory, unmeasured.

## 16. Open questions for the re-review

Revision 1's six questions and revision 2's five were answered, and every answer is adopted:
the archive belongs to the Intents accounting class with a distinct physical kind (16.1); one
archive per document plus explicit release (16.2); `Stale` for acknowledgements older than the
retained manifest is acceptable (16.3); excluding the HPKE key from the leaf digest is correct and
the membership rule is necessary but had to become receive-side (16.4); exact seed retention plus a
detached re-parse is preferred (16.5). Remaining:

Every question is now answered and adopted. Revision 3's four: keep `Unmatched -> {New, Stale}` at
S1b rather than a durable reservation; land `EpochRecordKind::DraftArchive` as an isolated Agent 4
integration commit ahead of the Agent 2 implementation, with Agents 1, 2 and 3 rebasing onto that
seam; keep the 16 MiB sub-cap as an explicit product policy to revisit after measurement; keep
M-1's hard pre-merge refusal rather than merging into `Unknown`.

**16.1, the v1 tenure migration policy (L11): decided as (a).** The reviewer chose option (a) over
my recommendation of (b), and the reasoning is better than mine: (b) would create a new
authority-bearing escape hatch whose whole purpose is to turn a value the design has just proved
untrustworthy for authoring into trusted authoring evidence. That needs its own server scoping,
confirmation, capability exposure, audit record, replay and restart semantics and tests, and it
deliberately defeats V6's otherwise clean rule that `Imported` becomes `Observed` only through a
real leaf-aware transition. For an alpha installed base already undergoing manual migration, the
simpler security model is worth more than the recovered capability. **`Imported` ships fail-closed
with no operator-adoption override**, and L11's regression is accepted as the price. Option (c) is
rejected outright: it silently restores the revision-3 vulnerability.

One refinement to section 9.3 part 5's "no fourth option" claim, from the same answer, is adopted:
there is no fourth way to **reconstruct** the historical tenure from v1 evidence, which is what that
argument asserts, but there is a separate future *migration class* that deliberately establishes a
**fresh** tenure rather than recovering the old one, through a designed identity or owner transition
or server recreation. That does not prove the old value and so is not a counterexample; it is
future migration work and is explicitly not part of Gate 4.

No open questions remain for this design.

## 17. Test and mutation plan

### 17.1 Normal regressions

Revision 1's N1-N5, N20-N21, N23-N28, N30-N31, N-T1 to N-T5 and N-T8 are retained with the schema
changes of section 5. **N-T7 is no longer retained unchanged: it is corrected below**, because
revision 4's `Imported` state added a second fail-closed value that its `Unknown`-only matrix could
not see. Added, changed or corrected:

| # | Level | Case | Independent observation |
|---|---|---|---|
| N6 | actor | Same-document copy after rotation, per item | Each item routes through the ordinary Save path, is authored by the copier with a fresh nonce, and appears in the destination projection; **no byte of the branch's record changes at any point** (findings 1, 2, 8). |
| N7 | actor | Copy exact retry after a lost response | `already_saved` true, no second destination operation; still no branch-record write. |
| N8 | actor | Stale `expected_projection` or `epoch_id` | Refused; nothing saved; re-preview succeeds. |
| N8b | store | **Finding 4 destination currency.** Change the destination's source record, then its recovery record, between C1 and C3, and again between C3 and C4, each with an authenticated same-size replacement | Each refuses at `studio_destination_is_current` with its own digest or size comparison; each fixture passes the source stamp check first. |
| N9 | store | Copy admission failures: `FLIPNOTE_MAX_FRAMES`, `FLIPNOTE_FRAME_BYTES`, `MAX_INDEX_OBJECTS`, over-cap, tombstoned target, missing PIX | Each yields its specific disposition or refusal; no partial destination write; branch and references intact. |
| N9b | store | **Finding 2 derived source ids.** A branch with one accepted `InsertFrame` referencing CID X and a base title. Request a Title copy | `source_ops` names the title's source operation and **not** the insertion; no request field can name a different entry, because `source_entry` does not exist; the native result reports the derived ids. Then assert that no copy count or `source_ops` value permits a `Preserve` disposal. |
| N10 | store | Cross-document copy while `Prepared` | Permitted into a genuinely distinct Flipnote in the same channel; refused when the destination is the branch's own document reached through another channel label; `prepared` never cleared. |
| N11 | store | **Finding 1 positive terminal case, `Preserved`.** Archive, then dispose, then reopen the vault | The archive is durable and decodes; exactly the annotated ids are gone; every ordinary intent remains; the record re-encodes canonically with `active == None`, `prepared == None` and `disposed` present; a second reopen is byte-identical. |
| N12 | store | `Preserve` without a matching archive, and with an archive whose `content`, `branch`, `generation` or entry list differs | Each refuses at D4 with the branch fully retained. |
| N13 | store | **Finding 1 positive terminal case, `Discarded`.** Dispose with the exact confirmation, then reopen | Manifest present with `mode:"discarded"`, entries gone, ordinary entries intact, canonical re-encode and reopen both succeed. |
| N14 | store | **Split by layer (finding 5).** Wrong `branch`, wrong `content`, wrong `accepted`, wrong author, wrong channel, and a `Preserve` request where the destructive branch is required | Each refuses at its own check with the branch intact; each fixture passes every earlier check first. The typed `StudioDiscardConfirmation` is present throughout, because at this layer it cannot be absent. |
| N14n | native | **Finding 5, the layer where the literal exists.** `studio_overlay_dispose` with `mode:"discard"` and: no `confirm` field, an empty string, a near-miss literal, the release token, and a correct token with wrong case | Each refuses in the adapter **before any custody or store mutation**; the correct token reaches the typed request; the branch is unchanged in every case. |
| N15 | store | Disposal under a transfer hold and under a live hold | Refused; read-only export and archiving still succeed under the transfer hold. |
| N16 | store | Interrupt the archive write and the disposal write at each barrier | Reproduces section 12's table, including the `partial` state and the post-rename sync-only retry with no second manifest and no second sequence. |
| N17 | store | Delayed Save retry for the most recently disposed branch | `classify_request` returns `Disposed` before any basis mint, tenure read, source lookup or media work; terminal acknowledgement; no new branch. |
| N17a | store | **SEC-PAIR-001 finding 1, admission.** Dispose G1, call `begin`, then submit the first G2 Save | Admitted exactly once as `New { generation: 2 }`; the generation increment and the first envelope land in one sealed replacement; an immediate exact retry classifies as `Active`, not as a second `New`. Then submit, on the same record: G1's old id, a random id, the id for generation 4, and an id derived from a different basis at generation 2. **All four return Stale, and none creates a branch, an entry or a generation increment.** |
| N17b | store | **Finding 3 rollover, now reachable.** Dispose G1 on basis B, accept G2 on B (through N17a's admission), dispose G2, restart, then deliver delayed exact retries of both G1 and G2 requests | G2's retry is acknowledged from the retained manifest; **G1's retry returns Stale and creates no branch, no entry and no second envelope**; `branch_generation` is monotonic across both disposals and across restart. |
| N18 | store | Retirement naming both ordinary and disposed ids | Ordinary ids retire; disposed ids are already absent; no annotated id of a live branch is ever removed by that path. |
| N19 | store | **Corrected reference lifecycle.** Base-only, superseded, removed-frame and pending CIDs, through: branch retained, archived, `Preserved` disposal, and `Discarded` disposal | Retained and archived: all four survive cleanup and reopen, the archived case proved by removing the branch and keeping only the archive. After `Preserved` disposal: still all four, held by the archive's reference collection. After `Discarded` disposal: a CID with no other holder becomes reclaimable and a CID still named by another holder does not. |
| N19b | store | **Archive physical identity (finding 2).** An archive and an ordinary intent record for the same document, coexisting | `storage_name` assigns each to its own family; the inventory keys them separately with no collision; `decode_record_scope` accepts each only under its own domain and rejects a record whose scope names the other family; `epoch_files_absent` reports the directory non-empty for an archive alone; a temporary archive sibling is recognised; the archive charges its bytes and slot into `EpochIntentBudget`; the one-per-document rule and the 16 MiB sub-cap each refuse before any write; a corrupt archive fails closed for reclamation rather than releasing its references; and a narrower coverage that excludes intents also excludes archives. |
| N19c | store | **Archive size (finding 3).** A maximal accepted branch, archived, and a shape one byte over | The maximal branch archives successfully within `MAX_DRAFT_ARCHIVE_RECORD_BYTES`; the over-size shape refuses with no partial record and no budget spend; a static assertion covers the derived constant against the maximal encodable shape; the sealed record stays within the family's `sealed_cap()` and the native base64 form within `bounded_view`. |
| N22 | store | **Finding 5 non-replayable branch** | Classification is `Manual(NotReplayable)`; export and archiving succeed and contain the exact seed, ordered envelopes, timestamps, scope and provenance with `replayable == 0`; `Preserved` disposal succeeds; only the typed projection and copy planning refuse; metadata readers do not fail. |
| N25b | actor | **Finding 6 seed extraction.** Accept unconfirmed work from a preview whose tail is non-empty | The captured bytes equal the originally fetched seed, not a checkpoint of the tail-advanced projection; the detached re-parse against the candidate receipt succeeds; a mutated captured byte fails the re-parse with no durable change; after restart the branch reconstructs from its persisted bytes with the same projection. |
| N-T6 | actor | Remove then rejoin in separate commits, the rejoining device becoming owner | The rejoining owner and every witness observe the same start; its receipts verify. |
| N-T6b | actor | **Finding 7 discriminator.** A committer leaf identity change across a contiguous step, with four observers: the rejoining owner, a known-tenure witness, an **Unknown-tenure newcomer** that requests a fresh owner proof, and a restarted copy of the witness | With 9.3 part 3 the witness and the rejoining owner agree, so the newcomer's proof-derived selection agrees too, and the restarted witness agrees after `decode`. Separately assert that an ordinary committer self-update, which rotates the HPKE key but keeps the credential, does **not** reset the observed start. |
| N-T7 | actor | **Corrected (revision-4 finding 1). The V1 authoring-refusal matrix, run for BOTH fail-closed values.** Two fixtures reaching the app layer: one with `Unknown`, and one built by migrating a genuine ambiguous v1 snapshot as in N-T6d so the value is a real `Imported(S)` rather than an injected one | For each fixture: `Server::observed_owner_tenure()` reports the expected variant, and for the migrated one it reports **`StudioOwnerTenure::Imported(S)`, not `Known(S)`**; `require_observed_owner_tenure()` refuses; and every V1 authority entry point refuses at its own check, each fixture passing the earlier checks first: Closing basis minting, first local acceptance, handoff preparation, a signing turn, the commit, receipt issuance, rotation, Registry pointer publication, and Agent 3's repair issuance and application. The `Imported` fixture additionally asserts that verification is **unaffected** in the same run, so the test cannot pass by making `Imported` behave as `Unknown` everywhere. |
| N-T7b | actor | **New (revision-5 finding 1). The positive complement to N-T7.** Using the same genuinely migrated `Imported(S)` fixture, and repeated for `Unknown` | Each of these still works with no authoring tenure available: an exact accepted Save retry returns its saved entry with no new envelope, no new sequence and no generation increment; a completed-handoff request returns its terminal acknowledgement; and an already durable `Prepared` handoff runs its existing resolution path to Active, Complete or Hold according to the actual evidence. In the same run, a genuinely new Save and a fresh handoff preparation both still refuse. The `Prepared` case is the one that matters most under L11, because an `Imported` single-owner server never regains authoring, so a permanent hold here would be permanent data limbo. **Diagnostics (revision-6 refinement):** the `Prepared` fixture also asserts `is_prepared()` and that its initial `completed_branch(target, author, current_basis)` is `None`. These are assertions, not the argument: the reason a completed-handoff short-circuit cannot mask the resolution path is that completion clears `active` and `prepared`, stores the `Completed` manifest and advances `minimum_new_basis_closed_epoch`, while `completed_branch` also matches on basis, so a live `Prepared` branch cannot legitimately carry a completed handoff's superseded basis. The assertions make a later regression say so out loud. |
| N-T6d | sync | **Revision-3 finding 1, the v1 migration.** Build old-format state with `start = Some(S)`, `S < epoch`; under the **old** observation rule apply the formerly invisible same-owner discontinuity and show `S` is preserved; serialize genuine v1 bytes; decode under the new implementation against the post-transition group | The result is **`Imported(S)`, not `Observed(S)`**: `verification_owner_tenure_start()` returns `Some(S)` so verification is unchanged and a mismatched proof is still refused, while `authoring_owner_tenure_start()` returns `None` and `prepare_receipt_head_snapshot` refuses. Save and reload the new snapshot and require it is still `Imported`. Separately: a v1 state with `start == Some(epoch)`, and the founding `epoch == 0, start == Some(0)`, both decode to `Observed`; a v1 state with no start decodes to `Unknown`; a `start > epoch` v1 state is still rejected. Finally, apply a genuine owner change and require the promotion to `Observed` at the new epoch. |
| N-T6c | mls, actor | **SEC-PAIR-001 finding 4, adversarial.** Construct the forbidden same-commit remove-and-re-add of the designated committer, reusing the same signature key and credential bytes, **bypassing the local commit builder**, and deliver the staged commit to an uninvolved witness | The witness refuses it **before `merge_staged_commit`**, so its epoch and its observed tenure are unchanged and no member reaches the ambiguous position. Assert separately that the local builder refuses to construct it, that a remove-and-re-add of a **different** `DeviceId` is still accepted, and that a genuine rejoin in a **later** commit is still accepted and produces agreement. The invite ledger is not consulted by the witness in any of these. |

### 17.2 Isolated mutations

Revision 1's M2-M8 (renumbered where the guard moved), M11-M14, M16-M21 and M23-M26 are retained.
Changed, added or corrected:

| # | Guard removed | Test | Assertion |
|---|---|---|---|
| M1 | The `validate` rule that no id occurs in both `completed.entries` and `disposed.entries` | N23 | "an id was both transferred and disposed": the record decoded and re-encoded successfully. |
| M1b | The `disposed`-is-self-contained property: reintroduce a `disposed` field that refers to a live field | N11 | "a terminal record failed to reopen": the post-disposal reopen fails, which is finding 1's defect made executable. |
| M3 | D3's `content` equality | N14 | "a stale request disposed a branch it had not seen". |
| M3b | D3's `branch` equality (generation ignored) | N17b | "an old generation's request matched the current branch". |
| M4 | D4's archive entry-list comparison, then separately its `content` comparison | N12 | "a branch was disposed against a non-matching archive": two mutations. |
| M5 | **Store layer only (finding 5).** D5's requirement that the destructive branch take a `StudioDiscardConfirmation` at all | N14 | "a disposal destroyed a branch through the preserving branch's checks". |
| M5n | **Native layer (finding 5).** The exact-literal comparison in the adapter's `confirm` parser | N14n | "any string confirmed a discard": a near-miss literal reached the typed request. |
| M9 | **Corrected.** Split the single sealed replacement into two writes, manifest first then ledger removal, leaving the intermediate state reachable | N16 | The fixture must reach the interruption **between the two writes** and observe "entries removed with no durable manifest" or "a manifest with the entries still present". A refusal caused by an invalid replacement does **not** count, so the mutant must produce two individually valid records. |
| M10b | **Corrected (finding 1).** `admit_new_branch`'s equality against the **derived expected-next** identity, weakened to "any identity the record does not hold" | N17a | "an arbitrary unknown identity was admitted as a new branch": G1's old id, or the generation-4 id, creates a branch. Mutating all-unknown-alike in the other direction, so that nothing is admitted, is a separate mutation failing N17a's first assertion. |
| M10c | The `Unmatched` arm reaching S1b at all (treat it as terminal `Stale` in `classify_request`, restoring revision 2) | N17a | "a legitimate first acceptance of the derived next generation was refused". |
| M15 | C4's `contains_exact_operation` retry shortcut | N7 | "a copy retry created a second destination operation". |
| M22 | 9.3's `designated_committer == device` condition in `joined` | N-T2 | "a joiner that is not the committer invented the current owner's tenure". |
| M22b | The `before.leaf != after.leaf` arm in `applied` | N-T6b | "a witness preserved a stale tenure across a real membership discontinuity": the witness and the rejoining owner disagree, and the Unknown-tenure newcomer accepts the value the witness refuses. |
| M22c | The exclusion of `encryption_key` from the leaf digest | N-T6b's self-update case | "an ordinary committer self-update reset the observed tenure". |
| M22d | **M-1's receive-side check in `process_incoming`**, leaving only the local commit builder's refusal | N-T6c | "a witness merged a forbidden same-commit remove-and-re-add": the commit is applied and the witness's observed tenure diverges from the rejoining owner's. A separate mutation removes only the builder's refusal and asserts N-T6c's builder case fails while the receive-side case still passes, so the two are proved independent. |
| M22e | **The v1 migration's `start == epoch` condition**, restoring revision 3's behaviour of copying any v1 `start` into the leaf-aware state | N-T6d | "an unprovable v1 tenure was promoted to authoring evidence": `authoring_owner_tenure_start()` returns `Some(S)` and `prepare_receipt_head_snapshot` mints a permit for a stale start. |
| M22f | The `Observed`/`Imported` flag in the snapshot tail (encode both as `Observed`) | N-T6d's save-and-reload case | "a restart laundered an imported tenure into an observed one". |
| M22g | **New (revision-4 finding 1).** The `ObservedOwnerTenure::Imported` arm of the app-level conversion in `Server::observed_owner_tenure`, mapped to `StudioOwnerTenure::Known(S)` | N-T7's migrated fixture | "an imported tenure crossed the app boundary as observed knowledge": the app reports `Known(S)` for a value the sync layer holds as `Imported(S)`. |
| M24 | **Retained, scope narrowed.** V1's fail-closed `Unknown` in `require_observed_owner_tenure` (substitute the current group epoch) | N-T7's `Unknown` fixture | "an Unknown-tenure device authored, signed or issued". |
| M24b | **New (revision-4 finding 1).** The `Imported` arm of `require_observed_owner_tenure`, returning `Ok(S)` | N-T7's migrated fixture | "an imported tenure became authoring evidence": a Closing basis is minted, or a receipt is issued, under a value with no leaf-continuity proof. M22g and M24b must be separately anchored, so that fixing one cannot mask the other. |
| M24c | **New (revision-5 finding 1).** A-1 itself: hoist `require_observed_owner_tenure()` to the top of `save_studio_closing_overlay` and, separately, of `handoff_studio_overlay`, in place of passing the `Option` through | N-T7b | "acknowledgement or recovery was refused for want of authoring authority": the accepted-retry case loses its saved entry, and the `Prepared` case cannot resolve. Two mutations, one per wrapper, because the two failure modes differ and the handoff one is the permanent-limbo case. N-T7 must still pass under both mutants, which is what proves M24c is testing A-1 and not V1. |
| M28 | The `DraftArchive` arm of the reference collection | N19 | "archived pixels became reclaimable after a preserving disposal". |
| M26 | **Corrected fixture.** The `Unconfirmed`-forbids-`prepared` rule in `validate` | N23 | The fixture must be a record that passes **every other** structural guard, including provenance encoding, zero source identifiers, entry ordering and canonical re-encode, so the failure isolates this prohibition. |
| M27 | 8.1 part 3's detached re-parse of the captured seed bytes | N25b | "a mutated captured seed became a durable base". |

Reconciliation mutations M21 must use a fixture where document identity and seed-hash matching
differ independently, so removing either half of the predicate fails on the intended comparison and
not on an earlier validation. Every entry requires a unique anchor, one executed failing test, the
intended assertion, byte-exact restoration and a passing restored regression. No mutation result
exists yet.

### 17.3 Harness and workflow

A new `.github/scripts/check-studio-overlay-lifecycle-mutations.py` following
`check-studio-overlay-mutations.py`, with logs under `logs/gate4-overlay-lifecycle-*.log`, and a
requested `lifecycle` job in `.github/workflows/studio-overlay.yml` added to a required workflow.
Local execution stays serial: `-j 1`, the existing per-package test debug override, no concurrent
Cargo work, no blanket cleanup.

## 18. Re-review request

Fill `[FULL_HEAD_SHA]` with the commit that adds this revision before sending. Do not send a
placeholder.

```text
Review type: design re-review after the revision-5 re-review. Boundary (b) PASSED at revision 4 and
is not resubmitted; (a) and (c) had one Medium finding.
Base: f2257b018d396a835529742c40d4b282bbc127d9. Head: [FULL_HEAD_SHA].
Compare: https://github.com/Thalpy/Mewtual/compare/f2257b018d396a835529742c40d4b282bbc127d9...[FULL_HEAD_SHA]
Scope/evidence: docs/GATE4-AGENT-2-DESIGN.md revision 6 and docs/GATE4-AGENT-2-STATUS.md.
Design only: no production code, no test and no measurement exists. No Cargo command was run.
Dependencies unchanged. Native Save stays unregistered; Agent 1's P5 is false.

This revision is narrow. Please re-examine (a) and (c) and confirm (b) is undisturbed.

You were right, and I verified the ordering you described at the committed base rather than taking
it from the prose: handoff_studio_overlay_with_io runs completed_branch (:86), then
resolve_studio_handoff_with_io (:109), then completed_branch again (:121), and only then
tenure.ok_or_else (:127); save_studio_closing_overlay_with_io runs completed_retry (:107), then
exact_retry (:129), and only then tenure.ok_or_else (:185), with its own comment at :145 saying a
later fault or Unknown tenure must not turn a saved exact request into a new append. Only
prepare_studio_closing_overlay requires immediately, at :53.

The correction is invariant A-1, and I generalised it rather than adopting your per-wrapper table.
An app wrapper reads authoring_owner_tenure_start() and passes the Option<u64> through unchanged;
the store owns every refusal at the stage that needs it. I chose one rule over a "pure or mixed"
classification because a classification invites the next person to judge a new wrapper, and judging
it wrong is precisely this finding; under A-1 there is nothing to judge, and
require_observed_owner_tenure() is reserved for call sites that are a single authoring stage with no
acknowledgement or recovery branch, which today means Agent 1's S1b basis mint and Agent 3's repair
issuance. Please say if you think the per-wrapper table is safer despite that, or if A-1 is too
blunt for a wrapper that might later need an app-level refusal of its own.

V1 now says "new authoring" explicitly, and V8 states the complementary reachability so that fail
closed cannot degrade into refuse everything: under both Imported and Unknown, an exact accepted
Save retry, a completed-handoff acknowledgement and resolution of an already durable Prepared
handoff all stay reachable. N-T7b is the positive complement to N-T7 on the same migrated fixture,
and M24c hoists require_observed_owner_tenure() to the top of each mixed wrapper, one mutation each,
with N-T7 required to keep passing under both mutants so M24c is demonstrably testing A-1 and not
V1. Attack whether N-T7b can pass for the wrong reason, in particular whether its Prepared case
genuinely exercises resolve_studio_handoff_with_io rather than an earlier completed_branch
short-circuit.

One honesty note: the line numbers above are the committed ordering at this design's base, and
Agent 1 has in-flight uncommitted changes to epoch_studio/overlay.rs. A-1's precondition must be
reverified at integration rather than assumed, which is what N-T7b makes executable.

Section 16 records no open questions and your 16.1 decision stands as (a).

Return PASS for (a) and (c), or numbered findings with severity, file/line, trigger, impact,
evidence and required correction. A PASS accepts design only: no implementation, no measurement and
no native Save exposure is claimed, signed repair and combined runtime integration are separate, and
full Gate 4 acceptance remains with Agent 4.
```

## 18b. Superseded revision-5 request (retained as the scope record)

```text
Review type: design re-review after the revision-4 re-review. Boundary (b) PASSED there and is not
resubmitted for reconsideration; (a) and (c) had one Medium test and mutation gap.
Base: 37fa87753d32a2c4f5d1172cc865910a93d90fa1. Head: f2257b018d396a835529742c40d4b282bbc127d9.
Compare: https://github.com/Thalpy/Mewtual/compare/37fa87753d32a2c4f5d1172cc865910a93d90fa1...f2257b018d396a835529742c40d4b282bbc127d9
Scope/evidence: docs/GATE4-AGENT-2-DESIGN.md revision 5 and docs/GATE4-AGENT-2-STATUS.md.
Design only: no production code, no test and no measurement exists. No Cargo command was run.
Dependencies unchanged from revision 4. Native Save stays unregistered; Agent 1's P5 is false.

This revision is narrow. Please re-examine (a) and (c) and confirm (b) is undisturbed.

The finding is closed by making the Imported authoring refusal executable rather than asserted.
N-T7 leaves the retained set and runs the full V1 authoring-refusal matrix for BOTH fail-closed
values. Its Imported fixture is built by migrating a genuine ambiguous v1 snapshot as in N-T6d, not
by injecting the variant, so the test exercises the real conversion path; it asserts
Server::observed_owner_tenure reports Imported(S) and not Known(S), that
require_observed_owner_tenure refuses, and that every V1 entry point refuses at its own check with
earlier checks passing first. It also asserts in the same run that verification is unaffected, so
the test cannot be satisfied by making Imported behave as Unknown everywhere, which would quietly
undo the reason Imported exists.

The two halves of the app-boundary invariant are separately anchored, because you showed an
implementation could satisfy N-T6d, M22e and M22f while still laundering the value one layer up:
M22g mutates the Imported arm of the app conversion, M24b mutates the Imported arm of
require_observed_owner_tenure, and M24's scope narrows to Unknown. Confirm the two anchors are
genuinely independent, so that fixing one cannot mask the other, and that V7 states the invariant
completely.

Two hardening notes are adopted. observed_owner_tenure_start is REMOVED rather than repointed,
because its "independently observed" contract would become false for Imported and leaving the name
would let a future call site inherit the wrong semantics by default; deletion makes the compiler
enumerate every site, and section 9.3 part 5 lists the intended mapping, including that the
proof-signing check and head_snapshot_is_current are authoring rather than verification. Check that
mapping against the actual call graph and say if any site is classified wrongly, particularly the
existing test assertions, which must be read individually rather than renamed mechanically. P4 now
cites V1-V7.

Your 16.1 decision is adopted as (a) over my recommendation of (b), with your reasoning recorded:
(b) would create an authority-bearing escape hatch whose purpose is to turn a value proved
untrustworthy for authoring into trusted authoring evidence, and it defeats V6. Imported ships
fail-closed with no operator override, and L11's regression is accepted. Your refinement to the "no
fourth option" claim is also adopted: no fourth way to RECONSTRUCT the historical tenure exists,
while deliberately establishing a FRESH tenure is a separate future migration class and not a
counterexample. Section 16 now records no open questions; say if you disagree that it is closed.

Return PASS for (a) and (c), or numbered findings with severity, file/line, trigger, impact,
evidence and required correction. A PASS accepts design only: no implementation, no measurement and
no native Save exposure is claimed, signed repair and combined runtime integration are separate, and
full Gate 4 acceptance remains with Agent 4.
```

## 18c. Superseded revision-4 request (retained as the scope record)

```text
Review type: design re-review after the revision-3 re-review (CHANGES REQUIRED on all three
boundaries, two findings).
Base: 909720739b6c0a455d37761e8149d7eec21cb6f4. Head: 37fa87753d32a2c4f5d1172cc865910a93d90fa1.
Compare: https://github.com/Thalpy/Mewtual/compare/909720739b6c0a455d37761e8149d7eec21cb6f4...37fa87753d32a2c4f5d1172cc865910a93d90fa1
Scope/evidence: docs/GATE4-AGENT-2-DESIGN.md revision 4 and docs/GATE4-AGENT-2-STATUS.md.
Design only: no production code, no test and no measurement exists. No Cargo command was run.
Dependencies: e65bfd8 is still unreviewed; Agent 1's runtime design has since passed user review as
design only, and this document now records that instead of calling it unaccepted; Agent 3's design
is not consumed except through the tenure seam; native Save stays unregistered and out of
FLIPNOTE-UI-HOOKS; GATE4-AGENT-2-STATUS still states that Agent 1's P5 is false. Your five
SEC-PAIR-001 corrections and your revision-3 acceptance of them are not reopened.

Please return the three separable verdicts again: (a) manual lifecycle, stale bases and
repeated-tenure integration; (b) the preview-local-work extension; (c) the locally observed tenure
correction.

Finding 1, the v1 snapshot migration, is the substantive change and the only one worth attacking
hard. Three parts:

First, the promotable set is now proved rather than guessed. Section 9.3 part 5 argues that
start == Some(epoch) is exactly the safe set, because the old applied's preserve branch fixes start
while position.epoch is before.epoch and never raises it, so preserve can only yield
start <= after.epoch - 1, while the genuine-change branch always yields start == after.epoch.
Therefore start == epoch implies the last applied step was a visible DeviceId owner change and no
hidden discontinuity can lie at that step. Check that argument against the actual applied, including
the None branches and OwnerTenure::new's founding case, and say if any path can produce
start == epoch through preserve.

Second, a bare downgrade of everything else is rejected, and I want that judged rather than assumed.
A founder at epoch 0 that has applied any commit holds start = Some(0) with epoch > 0, so the strict
rule strips every existing server's owner of receipt issuance and rotation, permanently for a
single-owner server. The correction splits the value by consumer: a new Imported(u64) state that
observed_owner_tenure_start still reports as Some, so verification is unchanged and strictly safer
than Unknown given complete_checkpoint_head_scoped's is_some_and, and a new
authoring_owner_tenure_start that reports None, with prepare_receipt_head_snapshot switched to it.
Confirm that this single call-site switch really is sufficient: head_snapshot_is_current and the
proof-signing path should be unreachable without a permit that prepare refused to mint. Attack
N-T6d and M22e.

Third, the snapshot tail carries an Observed/Imported flag, because without it a save and reload
would launder Imported into Observed. Attack M22f.

I also want the residual judged as a product decision, not silently absorbed. Section 9.3 part 5
argues that corroboration cannot repair a v1 start, because every pre-upgrade participant ran the
same preserve branch and holds the same stale value, so a witnessed attestation would agree with the
wrong answer. If that is right, there is no technical escape and L11's regression is the honest
price. Question 16.1 puts three options to the user: ship Imported and accept the regression, ship
it with a one-time explicitly confirmed operator adoption per server, or promote automatically with
a documented residual. I recommend the second. Say whether you agree that no fourth option exists.

Finding 2 is documentation consistency. O2 has been rewritten so the design holds exactly one
archive placement, and the two stale cross-references you named are corrected: Agent 1's design is
recorded as user PASS, and the pre-Unmatched shorthand is gone from the status note. Confirm the
design-of-record no longer contains a second, contradictory storage architecture anywhere.

Return PASS for each of (a), (b) and (c) separately, or numbered findings with severity, file/line,
trigger, impact, evidence and required correction, stating which boundary each belongs to and which
earlier findings remain open. A PASS accepts design only: no implementation, no measurement and no
native Save exposure is claimed, signed repair and combined runtime integration are separate, and
full Gate 4 acceptance remains with Agent 4.
```

## 18d. Superseded revision-3 request (retained as the scope record)

```text
Review type: design re-review after SEC-PAIR-001 (CHANGES REQUIRED on all three boundaries).
Base: 21ca8fa93c07b8bb65a00bc0bbc555518f8a7132. Head: 909720739b6c0a455d37761e8149d7eec21cb6f4.
Compare: https://github.com/Thalpy/Mewtual/compare/21ca8fa93c07b8bb65a00bc0bbc555518f8a7132...909720739b6c0a455d37761e8149d7eec21cb6f4
Scope/evidence: docs/GATE4-AGENT-2-DESIGN.md revision 3 and docs/GATE4-AGENT-2-STATUS.md.
Design only: no production code, no test and no measurement exists. No Cargo command was run.
Dependencies unchanged: e65bfd8 is still unreviewed; Agent 1's runtime design is unaccepted and is
consumed by name only; Agent 3's design is not consumed except through the tenure seam; native Save
stays unregistered and out of FLIPNOTE-UI-HOOKS; GATE4-AGENT-2-STATUS still states that Agent 1's
P5 is false. Revision-1 findings 1, 2, 4, 5, 6, 8 and 9 were closed by SEC-PAIR-001 and are not
reopened here.

Please return three separable verdicts again: (a) manual lifecycle, stale bases and repeated-tenure
integration; (b) the preview-local-work extension; (c) the locally observed tenure correction,
which touches catcoms-mls on both the build and the receive path.

Section 0 maps each of the five SEC-PAIR-001 findings to its correction. The four to attack:

Finding 1 is corrected by splitting the verdict across two stages that already exist. classify_request
is now structural and basis-free and returns Active, Transferred, Disposed or Unmatched; Unmatched is
explicitly NOT a verdict, and admit_new_branch resolves it into New or Stale at Agent 1's S1b, which
mints the basis anyway and already compares a request identifier against it. Confirm this preserves
AG1-001, that no blob read, promotion, hold, possession check, tenure read or source lookup can
precede a terminal acknowledgement, and that there is no reserved-but-uncommitted generation: the
increment and the first envelope land in one sealed replacement at S3. Attack N17a, which now admits
exactly one New and then requires Stale for G1's old id, a random id, a generation-4 id AND an id
derived from a different basis at the same generation. M10b mutates the derived-expected-next
equality rather than treating all unknown identities alike, and M10c restores revision 2's behaviour
to prove the refusal it caused was real. Then check N17b is actually reachable now.

Finding 2 is corrected by adopting your 16.1 answer literally: EpochRecordKind::DraftArchive, a
distinct physical family with its own suffix, domain, scope, canonical path, reader, sealed cap,
inventory key, temporary recognition and epoch_files_absent participation, gated in storage_name by
includes_intents() and charged entirely to EpochIntentBudget. Revision 2's "one more record inside
the existing Intents family" is withdrawn as unrepresentable; audit fact A6 records why. Verify the
gating choice in particular: a coverage that excludes intents must also exclude archives, or a
narrow scan would miss a record whose references it is responsible for. Attack N19b, which now
requires no inventory key collision, per-family domain rejection, temporary-sibling recognition and
correct coverage gating, and M28.

Finding 3 is corrected by deriving three constants from the actual field bounds rather than reusing
the intent cap, which the archive never had to obey once it is its own record kind. The derived
payload maximum is about 6 MiB plus 26 KiB, larger than MAX_RECORD_BYTES; the two large terms cannot
both be saturated in a live record, but the constant does not depend on that coincidence. Check the
derivation covers the entry table, the receipt, the header and this encoder's framing, that a static
assertion is required rather than assumed, and that the constants are threaded through the reader
cap, the family's sealed_cap, the per-record authentication rail, EpochIntentBudget, the 16 MiB
sub-cap and the native base64 bound. Attack N19c, and judge L3b: 16 MiB admits only two archives at
the derived maximum (question 16.3).

Finding 4 is the one to attack hardest. The invite ledger is demoted to honest-join admission and is
no longer load-bearing: audit fact A7 quotes process_incoming's own comment saying single-use nonce
enforcement stays with the admitting committer. M-1 now binds every applier: a single commit must not
both remove the pre-commit designated committer and add the same DeviceId, enforced in the existing
pre-merge staged-commit inspection before merge_staged_commit, and identically in the local builder.
It is stated over DeviceId, not leaf index. Verify both inputs really are available pre-merge, that
the add_proposals walk the method already performs supplies the added ids, and that the rule does not
forbid a legitimate identity rotation or a genuine later rejoin. Attack N-T6c, which constructs the
forbidden commit while bypassing the local builder and requires an uninvolved witness to refuse
before merge, and M22d, which proves the receive-side and builder-side checks are independent. Then
answer question 16.4: a hard pre-merge refusal stops that member's epoch from advancing at all while
the commit stands. Is that the right trade against merging and falling back to Unknown tenure?

Finding 5 is corrected by splitting the confirmation tests by layer: the native adapter tests
absence and every wrong literal before any custody or store mutation (N14n, M5n), and the store tests
that the destructive branch requires the typed confirmation at all (N14, M5). Confirm neither layer's
test can pass for the other's reason.

Answer the four questions in section 16. Question 16.2 is a process question you are better placed
to judge than I am: adding a sixth EpochRecordKind variant touches every match over that enum,
including code Agents 1 and 3 are changing right now, so it may belong in an isolated Agent 4
integration commit ahead of the Agent 2 implementation.

Return PASS for each of (a), (b) and (c) separately, or numbered findings with severity, file/line,
trigger, impact, evidence and required correction, stating which boundary each belongs to and which
earlier findings remain open. A PASS accepts design only: no implementation, no measurement and no
native Save exposure is claimed, signed repair and combined runtime integration are separate, and
full Gate 4 acceptance remains with Agent 4.
```

## 18e. Superseded revision-2 request (retained as the scope record)

```text
Review type: design re-review after CHANGES REQUIRED on all three boundaries.
Base: a901f6b0f64df2b4ea9cc0221b64ac98276f582d. Head: 21ca8fa93c07b8bb65a00bc0bbc555518f8a7132.
Compare: https://github.com/Thalpy/Mewtual/compare/a901f6b0f64df2b4ea9cc0221b64ac98276f582d...21ca8fa93c07b8bb65a00bc0bbc555518f8a7132
Scope/evidence: docs/GATE4-AGENT-2-DESIGN.md revision 2 and docs/GATE4-AGENT-2-STATUS.md.
Design only: no production code, no test and no measurement exists. No Cargo command was run.
Dependencies unchanged: e65bfd8 is still unreviewed; Agent 1's runtime design is unaccepted and is
consumed by name only; Agent 3's design is not consumed except through the tenure seam; native Save
stays unregistered and out of FLIPNOTE-UI-HOOKS; GATE4-AGENT-2-STATUS still states that Agent 1's
P5 is false.

Please return three separable verdicts again: (a) manual lifecycle, stale bases and repeated-tenure
integration; (b) the preview-local-work extension; (c) the locally observed tenure correction,
which now also touches catcoms-mls.

Section 0 maps each of the nine findings to its correction. Verify each against the code, not the
prose. The four structural changes to attack first:

Finding 1 and 2 are corrected by DELETING copy bookkeeping from the durable record. There is no
`copy` arm, no C5 stage and no `source_entry` request field. Disposal's preserving mode now rests on
a durable draft archive record, and the terminal `disposed` manifest is self-contained: confirm from
the proposed `validate` that no rule of `disposed` refers to `active`, `prepared` or any live field,
so a record with `active == None` re-encodes canonically and reopens. Attack N11, N13 and M1b, which
is the finding-1 defect made executable. Then attack the honesty of C-P in 6.3: copy is stated to be
projection-level and lossy for superseded operations, conflicts, ordering and original provenance,
and no copy count may permit a preserving disposal (N9b).

Finding 3 is corrected by a branch-generation namespace rather than acknowledgement history.
`branch_id = H(basis fingerprint, branch_generation)` and `classify_request` returns Stale for any
id the record does not know. Run the reviewer's own trigger as N17b: dispose G1, accept G2 on the
same basis, dispose G2, restart, then retry both. Require G1's retry to return Stale with no branch,
no entry and no second envelope, and G2's to return the terminal acknowledgement. Judge whether
degrading older acknowledgements to refusal is acceptable (question 16.3) and attack M3b and M10b.

Finding 5's correction changes the archive's cost model: export and the archive are built from the
STRUCTURAL record plus ledger envelopes, so typed reconstruction is attempted and labelled, never
required. That is what makes a preserving disposal available for a non-replayable branch. Confirm
this does not weaken the accepted typed inspection view, and attack N22.

Finding 6 is corrected by retaining the exact verified seed bytes in UnconfirmedStudioSeed, exposing
them only through the accepted scoped callback, and NOT trusting that retention: the detached stage
re-runs UnconfirmedStudioSeed::parse against the candidate receipt before the branch is built, and
again on every restart reconstruction. Verify from provisional.rs that parse's existing
`projection.checkpoint(..).bytes() == bytes` equality is what makes the retained value provable, that
the tail's advance is why recomputation is impossible, and that the memory cost is accounted in 8.3
and L10 rather than assumed. Attack N25b and M27.

Finding 7 is the one whose correction you should attack hardest, because revision 1's argument was
wrong. The false claim that a wrong value produces receipts nobody accepts is withdrawn, on the basis
of `complete_checkpoint_head_scoped`'s `is_some_and`: an Unknown-tenure reader accepts a proof's
claimed tenure. Agreement is now structural. Position gains the committer's leaf identity,
blake3(index, signature_key, credential), and `applied` gains one arm treating a same-owner
contiguous step with a changed leaf identity as a new tenure at after.epoch, which is exactly what
the rejoining device's self-join inference computes. The HPKE encryption_key is deliberately EXCLUDED
so an ordinary committer self-update preserves knowledge; verify that choice against
openmls::group::Member and against group.rs:247-274, which binds a joiner's credential to
(group, invite_nonce). Judge the residual of the residual in 9.3 part 4, a re-add reusing both the
same signature key and the same credential at the same leaf, and whether the commit-builder
membership rule is necessary or the invite ledger's fresh nonce already excludes it (question 16.4).
Attack N-T6b, which now includes a known-tenure witness, an Unknown-tenure newcomer requesting a
fresh proof, a restarted witness and an ordinary self-update, plus M22b and M22c. Also confirm the
inference is now phrased in terms of current continuous membership and is still confined to
ChannelSync::new_joined, never OwnerTenure::unknown and never the restore path, and that the
versioned snapshot tail keeps decode's existing position equality and start > epoch rejections.

Findings 4, 8 and 9 are smaller. For 4, confirm the composite capture takes both destination records
under the SAME single preparation permit in the SAME visit, materializes no projection under custody,
and rechecks both digests and physical sizes at preview completion and at application (N8b). For 8,
confirm the three-state write outcome, refused / uncertain / partial, and that `partial` names the
only multi-step sequence that exists, archive durable with disposal pending. For 9, confirm the
confirmation token is a required field with a typed constructor in both the Rust request and the
native argument list, and that N14 and M5 can actually exercise its absence and a wrong literal.

Also judge the new surface this revision adds, which did not exist at revision 1: the draft archive
record as a second record kind inside the existing Intents family, with its own sealing domain,
budget participation, one-per-document cardinality, 16 MiB vault ceiling inside the existing 64 MiB,
Intents-arm accounting and, critically, its REFERENCE collection. Section 10's R3 now claims that a
preserving disposal keeps every CID the branch named protected through the archive, and that a
destination copy retains only the CIDs it names; revision 1's contrary claim is withdrawn. Verify
R3 against creative_references.rs and the cleanup path, and attack N19 and N19b. Question 16.1 asks
whether this placement is right or whether the archive should be its own inventoried family despite
colliding with Agent 1's I-4 and Agent 3's writers.

Answer the five questions in section 16. Confirm the corrected test plan: M9 must reach the
interruption between two individually valid records rather than a different refusal, M26's fixture
must pass every other structural guard, and the reconciliation mutations must separate document
identity from seed-hash matching. Return PASS for each of (a), (b) and (c) separately, or numbered
findings with severity, file/line, trigger, impact, evidence and required correction, stating which
boundary each belongs to and which revision-1 findings remain open. A PASS accepts design only: no
implementation, no measurement and no native Save exposure is claimed, signed repair and combined
runtime integration are separate, and full Gate 4 acceptance remains with Agent 4.
```
