# Gate 4: four implementation handoffs

Prepared 2026-09-15 at integration base `50f1f99d08f586c9d97637a8f510ae222362a511`.
Start from the latest pushed `Create-suite-2` containing this document, record its full SHA,
and check for later changes before editing. The base includes the handoff signing code at
`e65bfd89acecd4e660edb0560410e1d02cec5e21` and the independently merged jukebox fixture fix
`d7ec5b9`. That fixture fix has not been independently retested in this documentation pass.

**Gate 4 is incomplete. Gate 5 must wait.** These four assignments divide the work that
remains now. They are not the older four blocks: the earlier combined scheduling block at
`6b71d96` is already accepted. The older broad overlay block is divided between Agents 1 and 2.
The assignments differ substantially in size and have dependencies; they are not percentages.

## Common instructions for all four agents

Read this whole document, your assignment, [HANDOVER](HANDOVER.md),
[BACKEND-IMPLEMENTATION](BACKEND-IMPLEMENTATION.md),
[design-creative-suite](design-creative-suite.md), [design-epoch-close](design-epoch-close.md),
[INTERFACES](INTERFACES.md) and [FLIPNOTE-UI-HOOKS](FLIPNOTE-UI-HOOKS.md).
Read the accepted overlay foundation/handoff and runtime review notes linked below before
changing their boundaries. Current status entries override older historical progress statements.

- Preserve user-accepted production boundaries and the SUC, TAIL, NATIVE, OVERLAY, HANDOFF and
  INSPECTION finding closures. Report a newly demonstrated regression explicitly; do not reopen
  them merely because integration remains. The new [core signing split](GATE4-HANDOFF-SIGNING-REVIEW.md)
  still needs the user's adversarial review. Passing tests or permission to continue is not that PASS.
- Work on a separate branch/worktree. Do not share a mutable checkout or run mutation harnesses
  against another agent's source. Preserve the UI agent's existing mockup, layout and adapter work.
  Continue routine reversible implementation, tests and fixes without repeated permission requests.
- Before a new authority, persistence or wire boundary, write the concrete proposed contract and
  request the user's adversarial design review. Existing accepted designs do not need fresh approval
  for routine implementation choices. Push reviewable checkpoints and include the applicable
  [review preamble](GATE4-REVIEW-PREAMBLES.md), exact base/head, tests and unresolved limits.
  Do not substitute your own review for the requested independent review.
- Each agent owns `docs/GATE4-AGENT-N-STATUS.md` for its number (create it). Record proposed API
  seams, invariants, touched files, test commands/results, CI URLs and actual checkout SHAs,
  review findings, and an exact proposed UI-hooks update. Agent 4 folds these into shared docs
  at every integrated checkpoint. Do not leave new callable contracts undocumented until the end.
- Prefer new leaf modules and focused tests. Coordinate central dispatch/export edits with Agent 4;
  keep any necessary branch-local wiring in an identifiable integration commit. Never enable an
  unfinished native command merely to resolve imports. Agree concrete Rust/JSON seams before
  consuming another agent's API; proposed names are not existing implementations.
- Use actual authenticated transport, actors, independently sealed stores and production native
  conversion for integration claims. A core fixture or injected ready-cache value is not proof
  of running-app behavior. Negative fixtures must pass earlier validation and isolate the guard.
- For high-risk guards, use targeted mutations: one executed intended assertion must fail,
  source must be restored byte-for-byte, then the restored regression must pass. Compiler failure,
  zero matched tests or an unrelated panic is not evidence. Avoid tests that just mirror a helper.
- Keep local checks serial on this Windows machine: previous concurrent Cargo work exhausted RAM
  and disk. Rust is at `C:/Users/phaso/.cargo/bin`; use `-j 1` and, where needed, the existing
  per-package test debug override. Do not install Visual Studio. Use GitHub for native/platform
  builds and the required suites; distinguish unrun checks from passes. No blanket cleanup.
- A bounded implementation PASS closes only its named scope. Agent 4 requests a separate final
  Gate 4 acceptance review after the combined evidence is complete. Do not start Gate 5.

## Ownership and integration order

| Agent | Owns | Can start immediately | Depends on |
|---|---|---|---|
| 1 | Local Save and automatic handoff runtime, store capture/commit seams, signing custody | Capture/commit design, actor scheduling, isolated runtime tests | Core signing review; Agent 2's reviewed lifecycle before native Save exposure |
| 2 | Manual overlay lifecycle, stale/provisional work, repeated-owner tenure and labels | Existing read-only inspection, manual lifecycle design, tenure audit/tests | Agent 1's write seams for final wiring; separate preview-local-work design review |
| 3 | Signed fault repair issuance, durable application and distribution | Existing repair codec/book audit, repair runtime design and fixtures | Agent 2's live tenure contract; Agent 1's common source/Prepared fences |
| 4 | Integration, shared docs/registrations, combined acceptance and required CI | Acceptance matrix, baseline CI diagnosis, fixture/integration plan | Reviewed Agents 1-3 and core split before final gate acceptance |

Agent 1 owns handoff preparation/signing and its new store/runtime leaves; Agent 2 owns new
overlay manual-action and tenure leaves; Agent 3 owns new repair leaves. Agent 4 integrates
overlapping changes in `actor.rs`, `studio.rs`, dispatch/receiver enums, native command and
security registration, `INTERFACES.md`, `BACKEND-IMPLEMENTATION.md`, `HANDOVER.md`,
`design-creative-suite.md` and `FLIPNOTE-UI-HOOKS.md`. Agent 4 owns common workflows/lockfiles;
other agents supply focused test/mutation scripts and the requested workflow patch.

Sequence: agree seams -> develop the three scopes in isolation while Agent 4 audits/tests ->
review and integrate bounded checkpoints -> enable native Save only after its runtime and
manual-lifecycle prerequisites pass -> exercise combined flows -> fix findings and run required
suites at the integrated head -> request full Gate 4 review. Agent 4 resolves conflicts by
preserving both contracts and rerunning affected checks, never by choosing one side wholesale.

## Agent 1: local Save and automatic handoff runtime

Copy this assignment, together with the common instructions, to the first implementation agent.

**Outcome:** a real actor can durably accept an explicit local overlay operation on an eligible
Closing document, retain and read it after restart, and automatically transfer the complete
branch when the verified eligible successor arrives. Unrelated work keeps progressing during
heavy preparation. Native exposure waits for Agent 2's reviewed manual lifecycle.

Start with [GATE4-OVERLAY-RUNTIME-REVIEW](GATE4-OVERLAY-RUNTIME-REVIEW.md),
[GATE4-HANDOFF-SIGNING-REVIEW](GATE4-HANDOFF-SIGNING-REVIEW.md),
[GATE4-OVERLAY-HANDOFF-REVIEW](GATE4-OVERLAY-HANDOFF-REVIEW.md) and its
[implementation review](GATE4-OVERLAY-HANDOFF-IMPLEMENTATION-REVIEW.md).
Relevant implementation:

- `crates/catcoms-replication/src/studio/{epoch,overlay}/handoff/preparation.rs`:
  `StudioHandoffAuthority`, `prepare_handoff_detached`, `StudioHandoffSigning::sign_next/finish`.
- `crates/catcoms-app/src/store/epoch_studio/{overlay,handoff,preparation,source}.rs`,
  `store/epoch_intents/{overlay,inspection}.rs`, `store/epoch_recovery/inventory.rs`.
- `crates/catcoms-app/src/studio/{overlay,inspection,dispatch}.rs`, `studio/receiver/`,
  `actor.rs`; `apps/desktop/src-tauri/src/studio.rs` and `studio/inspection.rs`.

Implement and prove:

1. Capture authenticated bounded intent/source bytes and public context under actual store custody,
   then release the sole Server/vault and native guards before expensive decode, reconstruction,
   typed admission, inventory work or final assembly. Audit existing helper calls: source capture
   and source-byte matching can invoke expensive intent-link validation. Calling the synchronous
   store handoff inside the existing `StudioDispatch` blocking worker still monopolizes custody.
2. Reserve from the existing four-slot `registry_catchup::preparation_pool()` before capture.
   Carry the original permit through queued/running/ready work, signing, commit and delivery.
   Cancellation cannot refund a slot while a worker/result still owns it. No new overlay pool,
   uncharged complete-source cache or worker-owned device/MLS secret.
3. Bind full numeric-server/group/type/logical/channel identity, actual device and membership,
   MLS epoch, independently observed tenure, actor/sync incarnation, mount and original native
   request/session. Capture and reauthenticate full actual source/intent plaintext digests and
   physical sizes, including required metadata links. Core snapshot hashes are not vault stamps.
4. Derive first local acceptance from the actual Closing source, its matching saved signed close
   and observed tenure. Accept an explicit local operation with stable full-envelope retry identity;
   ordinary failed Apply remains `NoEvidence`. Exact saved Active/Completed retry is checked before
   demanding a fresh Closing basis. Refusals retain existing work and do not sign a substitute.
5. Schedule one bounded operation per signing turn with fresh live authority and wrapper checks;
   heavy final assembly goes back to the retained worker. Coalesce target work, pace retries and
   preserve authoritative discovery/receive and other-server progress under repeated UI requests.
   Measure accepted maximal byte/seed/metadata/projection/roster shapes and retained input/output
   costs, not just the existing 256 small title-edit case. Report measurement limits honestly.
6. Commit Prepared -> whole Source -> Completed with actual evidence rechecks at every barrier,
   complete signed digests, exact full-ledger retention and original retry floor. Preserve
   HANDOFF-002's common source-write and reference-inventory fences, actual replacement peaks
   and recovery holds across restart. No durable signed prefix, partial retirement or bypass cache.
   Publish only after Completed using normal tail/pages, keeping the two-packet initial Save limit.
7. Provide native request/result and invalidation wiring with Agent 4, including generation changes
   so an already watched epoch notices new pending work. Keep local-only, shared-pending and
   receipted results distinct. Native conversion must recheck final delivery using the same original
   context across custody visits. Do not make Save callable before the manual path passes review.

**Acceptance:** real Index and Flipnote local Save/retry/reopen -> eligible receipt/successor ->
automatic whole handoff -> ordinary peer catch-up, with exact original envelopes/timestamps and
full projection. Interrupt every durable write/flush; change wrapper bytes without changing size
or display; change authority/tenure/channel/session/mount between stages. Verify no false success,
lost branch, premature publication or dropped references. Pause a real worker/signing sequence
while another document/server and an authoritative checkpoint complete. Retain the accepted
inspection/preview resource and final-delivery regressions. Push a bounded review checkpoint
using review preamble 1; hand over concrete APIs and lifecycle enablement conditions to Agents 2/4.

## Agent 2: overlay lifecycle, provisional local work and repeated tenure

Copy this assignment, together with the common instructions, to the second implementation agent.

**Outcome:** every retained local branch has a bounded, authorized way to be inspected, exported,
copied or explicitly disposed of when automatic handoff is unsafe. Repeated owner changes and
newcomers preserve the difference between unconfirmed history, local work and verified authority.

Start with [GATE4-CLOSING-OVERLAY-REVIEW](GATE4-CLOSING-OVERLAY-REVIEW.md), especially lifecycle
requirements, and the accepted handoff/runtime/inspection notes. Inspect
`crates/catcoms-app/src/studio/{inspection,replay,restore,settlement}.rs`,
`store/epoch_intents/`, `store/epoch_recovery/`, `studio_exchange/tests/succession/`,
`studio/receiver/catchup/preview.rs`, `crates/catcoms-sync/src/receipt_head/`, and the
owner-tenure observation/proof paths found by searching `observed_owner_tenure_start`.
Coordinate with the existing frontend `studio-native.ts`, `studio-session.ts` and
`studio-contract.ts`; preserve their architecture and the canonical mockup.

Implement and prove:

1. Reuse accepted read-only `studio_overlay_read`; do not duplicate its capture/delivery machinery.
   Define explicit bounded backup export, copy-into-current and disposition contracts for retained
   overlays. Export is not Gate 6 `.pixa`, settlement or permission to delete the original.
   Copy uses authorized current typed edits with explicit user intent; a bulk copy must respect
   admission/recovery/refinventory limits and the runtime custody rules agreed with Agent 1.
2. Preserve complete accepted envelopes, authorship claims, order/timestamps, conflicts/deletions
   and referenced PIX across restart and any refusal. Mixed ordinary/annotated ledgers must not
   lose either class. Disposition must be an explicit reviewed transition with durable evidence,
   full-envelope matching, exact retry and recovery-before-removal as required by P1. An equal
   projection, marker, export, eviction acknowledgement or membership change is not disposal.
   Keep existing membership authorization; do not invent an offline export path for removed users.
3. Stale/rewound/nonpristine bases refuse automatic rebase/handoff and retain visible recoverable
   work. Complete the manual path before requesting native Save enablement. Exercise actual blob
   cleanup: base-only, removed and superseded PIX stay protected until a legitimate transition.
4. Write and obtain a separate adversarial design review for durable local work based only on
   `AwaitingTenureReceipt` previews. The current Closing basis deliberately rejects this case.
   Specify provenance, local storage/quotas, expiry versus retained work, target/session identity,
   reference retention and eventual receipt reconciliation before implementing the extension.
   A preview can never mint installed-source, owner-tenure, receipt, signing or publication authority.
   The local draft must survive preview expiry/replacement without promoting the preview to trusted
   history. Disabling preview local work indefinitely does not complete this Gate 4 requirement.
5. Complete actual A -> B -> A tenure integration, including returning/rejoining owners, restart,
   a member joining between owner changes, hidden higher old-tenure history and first-receipt
   inheritance. Reusing a key, Welcome, a historical receipt or current MLS epoch is not proof of
   a new tenure start. Audit existing authenticated proof/observation mechanisms; if the required
   case needs a new authority protocol, propose it for review rather than guessing. Preserve
   Unknown until real evidence arrives and demonstrate eventual progress with legitimate evidence.
   Agree this live-tenure seam with Agent 3 before repair integration.
6. Supply truthful native results/events and UI-hooks rows for local-only, awaiting receipt,
   stale/manual action, recovery, storage refusal and repeated-tenure transitions. Preserve native
   final delivery/session fences and byte bounds; do not implement the UI layout or frame claims.

**Acceptance:** real Index/Flipnote draft inspection/export/copy/disposition with lossless records,
wrong-scope/author/session refusals, quota and crash/reopen evidence; stale-base work survives
actual cleanup. Real actor A -> B -> A and newcomer flows reject an earlier tenure's otherwise
valid same-key receipt and make progress only with independently valid new authority. The reviewed
preview local-work extension retains its draft through expiry and reconciles conservatively after
the actual first receipt. Provide isolated guard mutations where another failure could mask them.
Push design and implementation checkpoints with review preamble 2. Provide Agent 1 the exact
manual-lifecycle prerequisites that permit Save exposure, and Agent 4 the bridge contract changes.

## Agent 3: runtime signed fault repair

Copy this assignment, together with the common instructions, to the third implementation agent.

**Outcome:** conflicting owner receipts produce a visible persistent Fault; only an authorized
signed repair can select the winning receipt and resume the running app, preserving losing work
before any replacement. Peers and newcomers obtain and apply the same bounded repair evidence.

Start with `design-epoch-close.md` sections on Fault/repair, recovery and succession;
`crates/catcoms-replication/src/epoch.rs` (`ReceiptRepair`, `ReceiptBook::apply_repair`),
`epoch/repair_state.rs` and its tests; `crates/catcoms-sync/src/receipt_head/wire.rs`;
`crates/catcoms-app/src/store/epoch_owner.rs`, `store/epoch_registry/owner.rs`,
`store/epoch_studio/`, `store/epoch_recovery/`, `studio/settlement.rs`, and actual receipt-head,
checkpoint and owner-journal producer/consumer paths. The codec/book already exists; it is not
a runtime repair transaction. Restore/Copy is not signed fault repair.

Implement and prove:

1. Produce a concrete reviewed design before new issuance/application/persistence boundaries.
   Define how the actual current owner selects one of the exact conflicting receipts, how that
   decision is bound to the full target and independently observed issuer tenure, and where it
   is persisted before any publication. Do not invent a silent conflict-selection policy.
   Coordinate the explicit owner action/result with Agent 4 and live tenure with Agent 2.
2. Use ReceiptRepair v2 live authority even for exact retries; v1 is historical evidence only.
   Bind both full conflicting receipts, the selected receipt hash and sequence. Preserve repaired
   loser screening, newer progress on exact retry, distinct-fault holds and bounded evidence on
   restart. Same owner key in an earlier tenure cannot authorize a new live repair.
3. Persist the repair decision and required losing typed recovery before replacing/sealing sources
   or retiring covered intents. Integrate accounted owner journals, whole evidence, source-required
   metadata, shared inventory/ref protection and storage holds. Respect an interrupted Prepared
   overlay transaction on every common source path; resolve actual durable state, never overwrite
   it. Coordinate with Agent 1 rather than introducing a competing source writer or preparation pool.
4. Distribute repairs through the existing authenticated bounded receipt-head/record/seed routes
   where suitable. A wire field's existence is not service integration. Verify current authority,
   exact hashes/scope and expected seed before application. Save before serving, retain exact
   restart retries, pace bounded attempts and preserve other-server/authoritative progress.
5. Drive Fault -> Repairing -> applicable current state/recovery observations through actual
   actor/native events, including storage refusal and interrupted recovery. The UI must not infer
   repair from an ordinary Restore, an absent error or a displayed projection. Include required
   Registry repair paths for Index/Flipnote discovery without expanding to unrelated products.

**Acceptance:** fault on real peers from two valid conflicting receipts, current-owner repair,
losing work preserved, crash at every decision/recovery/source/publication boundary, reopen and
exact retry, joiner after repair, old-loser replay/descendants, a third conflict, mismatched named
pair, wrong issuer/scope/tenure/sequence/seed and A -> B -> A. Isolate v2 current-tenure and
recovery-before-replacement enforcement with mutations. Assert actual installed state, receipts,
recovery, publication and stored bytes, not just enum transitions. Supply bounded capacity and
fairness evidence. Push checkpoints with review preamble 3 and an integration contract for Agent 4.

## Agent 4: integration, combined acceptance and required suites

Copy this assignment, together with the common instructions, to the fourth implementation agent.

**Outcome:** one reviewed integrated commit satisfies the remaining Gate 4 behavior through
production adapters, with reproducible evidence and accurate UI contracts. This agent owns
closing the gate only after the user accepts the full-gate review; three bounded PASS verdicts
and a green focused workflow alone are insufficient.

Begin now by creating `docs/GATE4-ACCEPTANCE.md`: map every current Gate 4 requirement in the
backend board, the runtime/handoff lifecycle notes and `design-creative-suite.md` section 2.9
to an owner, concrete test/scenario, evidence SHA and review status. Mark missing behavior,
implemented-but-unreviewed behavior and unexecuted checks distinctly. Keep Agents 1-3's seams
and shared-file ownership synchronized; update FLIPNOTE-UI-HOOKS at each integration, including
unavailable controls until prerequisites pass. Request the outstanding core signing review now.

Own these combined scenarios, reusing existing real actor/native fixtures where appropriate:

| Scenario | Required visible and durable outcome |
|---|---|
| Rotate with local work | Create/save/share -> Closing -> local Save -> restart/read -> receipt -> automatic whole handoff -> peer catch-up -> later settlement/recovery; complete projections and original envelope identity throughout |
| Stale and unconfirmed work | Changed/rewound base and newcomer preview-local work retain an authorized manual path; expiry/refusal/cleanup cannot erase work or invent write/tenure authority |
| Repeated succession | A -> B -> A with real membership changes/rejoin and a newcomer; distinct actual tenures, first-receipt inheritance, stale same-key evidence refusal and preserved losing history |
| Fault and repair | Two valid conflicting receipts, visible Fault, signed current-owner repair, durable recovery before replacement, restart at barriers and a newcomer after repair |
| Concurrent progress and teardown | Large paused jobs and repeated UI/peer attempts coexist with authoritative Registry/Studio progress and another server; cancellation, lock, remount, server switch and final conversion preserve custody and reject stale output |
| References and admission | Actual PIX cleanup through source/intent/recovery changes; shared record/slot/byte limits, full-cap retries and crash replacement peaks; insufficient space retains recoverable work |
| Earlier Gate 4 regression | Solo repeated rotations/restarts, Registry pointer/tail paging, Create after Index rotation, replay/manual recovery and persisted eviction deadlines remain correct after all changes |

Exercise Index and Flipnote; include Registry prerequisites. Use normal adapter entry points,
not direct private state insertion, to substantiate automation. Existing deterministic actors,
native IPC conversion, two-process CLI smoke and packaged desktop UI are different evidence
levels: name which ran. Do not silently require unrelated Gate 5-7 features or claim a packaged
UI test from a core fixture. Publish remaining environment-dependent acceptance explicitly.

Reverify recorded CI failures on the integrated tree before fixing or reporting them. The
`e_unknown` jukebox fixture has already changed in merged `d7ec5b9`; do not blindly repeat the
old failure claim or redo the fix. Historical HANDOVER also records strict native unused-code
errors, a cargo-deny dependency advisory and ambient-dependency violations. Inspect current
job logs, the lockfile and authoritative advisory data before changes. Fix actual causes with
focused verification; do not suppress warnings/advisories or weaken assertions to obtain green.

Run and record the required suites on the final integrated code, using GitHub for platform builds:

- Root `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all --all-features` on the configured Linux/Windows matrix, and
  `bash scripts/check-no-ambient.sh` on Linux.
- Native `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` and `cargo check` with
  that manifest; preserve CI's strict warning policy. Frontend `npm --prefix apps/desktop test`,
  `npm --prefix apps/desktop run check` and `npm --prefix apps/desktop run build`.
- Required supply-chain checks in `.github/workflows/ci.yml`; focused `studio-overlay`,
  `studio-handoff`, `studio-inspection`, `studio-native` workflows and their mutation/restoration
  evidence; `.github/workflows/two-client.yml` platform smoke/topology checks. Add integration
  scenarios to the appropriate required workflow so they cannot silently remain opt-in.
- Scoped new tests/mutations and maximal-shape/custody evidence from Agents 1-3. Record exact
  executed counts, ignored cases, failure assertions, restored-source results, run/job URLs and
  actual PR merge checkout SHA. A workflow configured in YAML has not thereby executed.

After each integrated change run affected checks; after final functional changes run the complete
required gate. Keep baseline failures open until resolved or explicitly dispositioned by the user;
never quietly count them as passes. Put the final matrix and concise observable contracts into
HANDOVER, the backend gate board and UI hooks. Push and request review using preamble 4 with
immutable baseline/head and all bounded review dispositions. If anything remains, state the
specific unmet requirement instead of declaring Gate 4 complete or moving to Gate 5.
