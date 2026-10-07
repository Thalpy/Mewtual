# Gate 4 Agent 4 status: integration and completion coordination

Current checkpoint: 2026-10-07. PR #35 is being reconciled on
`gate4-runtime-completion` against `gate4-agent1-runtime` at
`9c63bd6eaa4bbf4f22b1f3bcd6b0fdee20ae45aa`. Gate 4 remains incomplete and Gate 5 remains closed.

The detailed live ledger is [GATE4-ACCEPTANCE](GATE4-ACCEPTANCE.md). This document records the
integration history, current ownership boundaries and the single remaining execution sequence.

## Current state

Agent 4 has assembled the bounded repair integration checkpoint, with its automatic execution
currently isolated pending the detached runtime boundary:

- Agent 3 history is preserved by merge `6a2f89792eecbf6ef2e65d683e6b515bbc65181d`.
- The Windows directory durability gap is fixed at `5f141994a1cd6e54bf67ac6bf34f47296eeff6f3`.
- Repair high-water is issuer-tenure scoped, with checked exhaustion, at
  `43c1d8bee6d418a0dab6330723c32b8b2f76f6ab`.
- Store repair mutations run in serial Linux/Windows CI and desktop strict Clippy is enforced at
  `2b6f716ef05fdf99bdc04da531eb0c0194682e65`.
- The mutation timeout is sufficient for the observed Windows duration without weakening a test.
- The suite-only Registry preparation test uses a private fixture pool at
  `87629d6b72992254911a8e44f698d535bb5d7904`; production capacity is unchanged.
- The Agent 1 structural inventory seam now returns authenticated physical Intents bytes and only
  the provenance of a live overlay branch. Terminal disposed/transferred metadata retains its
  diagnostic provenance internally but cannot consume a live-branch capacity slot.
- All 19 exact-head PR #32 checks pass. A later whole-PR review of #33 found two HIGH integration
  findings outside the inventory seam: repair-only durability and unadmitted synchronous runtime
  execution. The durability gate is corrected; automatic execution is now fail-closed pending the
  required detached job. Commit `3fcde979...` carries the correction and its uncertain-B2/B3 and
  full-pool regressions. Re-review closed those defects but retained one MEDIUM residual because
  the disabled Registry router still reconstructed the full source before the gate. Correction
  `f7c74cb2...` reuses only exact-current prepared state, treats missing/cold/stale state as unknown,
  and defers unknown or pending repair without mutation. Its first rereview found that a fresh
  receiver with a small source never scheduled that preparation; `164a94d7...` makes classification
  preparation size-independent and preserves exact checked absence separately from cold state.
  Final bounded re-review found no remaining BLOCKER, HIGH, MEDIUM or LOW finding.
- PR #34 review found one later MEDIUM integration defect: both Registry head adapters asked a
  valid Fault source for a head before persisting the independently authorized report, and one
  negative could be capacity-masked. The candidate now authenticates/accounts the exact source,
  attempts B0 with full failed/uncertain writer propagation, and only then preserves Fault as a
  hard service refusal. Real explicit/prepared, empty-capacity authority, post-write/reopen and
  no-reconstruction regressions pass. Final re-review has no finding at any severity.
- In PR #35's preserved ancestry, `eef69729ae4b4200a6060b9f91719d44538094e5` implemented the next bounded runtime dependency: Flow S now
  carries typed Closing or Unconfirmed bases, and the crate-private awaiting-tenure consumer proves
  source absence, re-enters the current complete preview at S1b/S3, plans detached, enforces the
  3/server-group, 8 MiB/vault and 64-op rails, reconstructs those counters from fresh inventory, releases
  them after durable disposal, and excludes Unconfirmed history from automatic handoff. Real
  Flipnote preview, expiry/retry, operation-limit, inventory and disposal regressions pass.
- Review-response commit `6aee00046e35410ba42c9290c0d1f52fd41dd86b` binds each parked
  plan to target, provenance, basis, branch and a domain-separated digest of the full canonical
  operation. It also makes the central ordinary intent writer and the independent retirement writer
  fail closed when a replacement would preserve live Unconfirmed work outside the sole accounted
  Flow S path. Exact sync retry and terminal disposal remain available. Independent re-review closes
  every BLOCKER/HIGH/MEDIUM finding; the only residual LOW is that receipt-covered retirement is not
  separately exercised with a live Unconfirmed branch, although it uses the same guarded writer as
  the real manual-recovery regression.

The newer shared baseline independently contains the generalized Flow S, actor Save, reconciliation,
restart coverage and handoff exclusion. PR #35's merge resolution therefore keeps those newer
implementations and adds only the missing 3/server-group and 8 MiB/vault accounting rails plus its
reviewed request-correlation and off-path-writer corrections. The code is still intentionally
unavailable to the renderer. P5 is false; native Save and repair commands remain unregistered.
The reconciled exact head still requires CI and a bounded integration review before merge.

## Exact branch state

| Item | State |
|---|---|
| Shared integration baseline | `gate4-agent1-runtime` at `9c63bd6eaa4bbf4f22b1f3bcd6b0fdee20ae45aa` |
| Repair candidate | Historical PR #32 checkpoint `87629d6b72992254911a8e44f698d535bb5d7904`; its reviewed ancestry is in merged PR #33 |
| Documentation checkpoint | Historical PR #31 checkpoint `f0af61c9b1247fa955300ac50545074e18e9b302`; its documentation ancestry is in merged PR #33 |
| Archived-tenure branch | PR #34 merged; `gate4-finalization` is historical at `7f80815e...` |
| Runtime completion branch | PR #35, `gate4-runtime-completion`; original implementation `eef69729...` plus separately identifiable review response `6aee00046...`, now reconciled with exact base `9c63bd6e...` |
| Agent 3 source | `gate4-agent3-repair` at `15b715a10704a8dafc2cccef65854d4d45ad55ca`; preserved, not rewritten |

PR #28 remains the Agent 3 source record. PR #32 was its first integrated successor; merged PR #33
preserves the same ancestry and adds the structural seam plus the bounded review response. Merged
PR #34 adds the historical consumer without rewriting that history. PRs #28, #31 and #32 remain
historical source checkpoints, not additional merge instructions.

## Historical ownership checkpoint

The agent-by-agent lists below record PR #35's original 2026-10-06 checkpoint. Later Agent 1 and
Agent 1/2 commits already on `9c63bd6e...` supersede several “still required” entries; their own
status documents and `GATE4-ACCEPTANCE.md` are authoritative for current ownership. They are kept
here as integration history, not as a live gap list.

### Agent 1

Completed: structural decode, transient reference protection, I-4 generation enforcement, the
storage half of C-3, scheduled Closing Flow S, typed Closing/Unconfirmed capture-plan-commit, and
automatic Closing Flow H with non-Closing exclusion.

Still required:

1. map structured eligibility/manual reasons instead of returning only strings;
2. adopt `EpochStorageCursor` at the six runtime scan owners;
3. implement Flow R after cursor adoption;
4. finish the required maximum-shape/custody measurements; and
5. obtain the dedicated core-signing and final coherent runtime reviews. The bounded generalized
   Flow S / awaiting-tenure review is complete through `6aee00046...`, but it does not substitute
   for those wider verdicts.

These are internal prerequisites and are not blocked by P5. P5 blocks exposure, not implementation.

### Agent 2

Completed: manual lifecycle implementation, structural eligibility, archive/reference plumbing,
the sync/receipt preview basis, live-tenure substrate, CORE-005 witness and real MLS returning-owner
fixture. Agent 4 supplied the missing Windows directory barrier.

Still required:

1. close or explicitly disposition the current lifecycle/copy findings before re-review: M3's
   missing exact-retry half, M4's C1'/C4 transfer-hold mismatch, L2's vacuous provenance test,
   L4's exact-retry kind misreport, L5's same-document copy gap, the wrong-object-channel
   diagnostic Low, and P1's copy-across-restart evidence; `ed8ab0a8` already closes M2, while
   `0335262e` closes the older D4/D1/object-probe/M-1 evidence items and supplies M3's
   successful-apply control, so none of those closures should be reopened or credited twice;
2. add real Index evidence for the now-implemented internal preview custody, rails, S3, Save,
   fresh-inventory reconstruction and handoff-exclusion slice;
3. implement 8.6 reconciliation, actual reopen/restart reconstruction and actor scheduling;
4. drive the returning owner through the real app actor and Studio rotation;
5. complete native result contracts without registering commands; and
6. obtain a whole-boundary lifecycle/repeated-tenure review.

P5 remains **FALSE** until those P1-P4 requirements have implementation PASSes.

### Agent 3

Completed and integrated in the candidate: signed current-tenure Studio/Registry repair core/store
transactions, owner records, durable transitions, durability-gated serving, native conversion
types, core/store mutation harnesses and the issuer-tenure sequence correction. Automatic catch-up
application, owner resume and repaired-seed installation are deliberately disabled until the
detached admitted runtime exists; ancestry alone does not make those paths safe to activate.

Still required:

1. require exact-head Linux ambient and repair-store mutation CI for the archived Observed-tenure consumer;
2. build the detached S1-S4 custody split and C-3/source-fence integration, then re-enable the
   currently fail-closed automatic repair paths;
3. add fetched-seed, positive owed-Registry, real two-peer/newcomer and fairness evidence; and
4. obtain the bounded Review 3 verdict for the completed boundary.

### Agent 4

Completed: preserved-history integration, response classification, Windows durability, repair
sequence correction, two-platform mutation CI, desktop Clippy, the Registry pre-gate reconstruction
and fresh-receiver liveness corrections, truthful unavailable registration state, and the bounded
internal generalized Flow S / awaiting-tenure candidate and its bounded review response. Pushed-head
Linux ambient and mutation CI remain required before merge readiness is claimed.

Still required: integrate the specialist completions, maintain this ledger, register only approved
commands after P5, implement/run the seven combined scenarios, update interface/UI truth and request
Review 4. Agent 4 must not turn dependency ancestry into an implementation claim.

## Historical completion sequence at PR #35's original base

The next execution order is:

1. Obtain exact-head Linux CI for the independently reviewed generalized Flow S / internal preview
   candidate.
2. Adopt the C-3 cursor at runtime call sites, then implement Flow R.
3. Close Agent 2's remaining lifecycle/copy findings and copy-restart evidence, then complete Index
   preview evidence, reconciliation, actor/native result wiring and the real actor/Studio
   returning-owner path.
4. Complete Agent 3's detached/runtime evidence, then obtain bounded reviews for Agents 1-3.
5. If and only if Agent 2 records P5 true, add native registrations, ACLs, interface rows and UI
   hooks in one reviewable checkpoint.
6. Run the combined Index/Flipnote/Registry scenarios and all required suites, then request Review 4.

## Verification for the current code candidate

On `gate4-runtime-completion` through correction
`6aee00046e35410ba42c9290c0d1f52fd41dd86b`, the definitive complete root suite passes, as do all
1,282 frontend tests, root and desktop strict all-target/all-feature Clippy, desktop `cargo check`,
root formatting, Svelte check, production build and `cargo deny`. The first root run had one
order-sensitive `studio_actors_new_member_after_checkpoint_discovers_installs_and_receives_tail`
failure; the exact test passed alone and the complete rerun passed. The desktop/Tauri run passes 324
of 325 library tests and reproduces only
`six_client_recovery::six_client_native_restart_and_partition_recovery`; a focused candidate rerun
and a focused run on exact base `c6f7fea0...` fail with the same partition-boundary convergence
shape, so this is recorded as an unresolved baseline defect, not a candidate pass. No assertion was
weakened; the five command-ACL integration tests pass separately. Startup/flow remain inapplicable
because setup, process, renderer and command-registration
paths are unchanged. Local `bash scripts/check-no-ambient.sh` could not start WSL (`E_ACCESSDENIED`),
so Linux ambient and both repair-store mutation jobs remain exact-head PR checks.

The independent bounded review initially found two MEDIUM defects: parked-plan correlation omitted
provenance/operation identity, and ordinary replacements could change live-Unconfirmed physical
bytes without updating the outer counters. Re-review found two residual MEDIUMs: `DomainOp::id`
did not bind the body, and the separate retirement writer bypassed the central guard. Commit
`6aee00046...` fixes all four with the full canonical-operation digest, sole accounted Flow S seam,
and shared fail-closed retirement rule. Final re-review reports no remaining BLOCKER/HIGH/MEDIUM;
the receipt-covered-retirement LOW test gap is recorded above. This verdict accepts only this
bounded candidate, not the remaining Agent 1-3 work or full Gate 4.

At `87629d6b72992254911a8e44f698d535bb5d7904`:

- all 19 GitHub checks pass;
- complete root suites pass on Ubuntu and Windows;
- Linux ambient-dependency checking passes in CI;
- root and desktop strict Clippy, native, frontend test/check/build and cargo-deny pass;
- repair-core and repair-store mutations pass on Ubuntu and Windows;
- lifecycle, lifecycle mutations, handoff, inspection, overlay, prepared signing, NAT and
  two-process workflows pass; and
- the independent bounded review has no unresolved BLOCKER/HIGH/MEDIUM finding.

`test:startup` and `test:flows` were not run because this candidate changes no setup, process,
renderer or command-registration path. They become applicable when later work reaches those paths.

On the completion line after the inventory seam, the complete root suite, all 1,282 frontend tests,
frontend check and frontend production build pass locally. The native suite is 324 pass / 1 fail:
`six_client_recovery::six_client_native_restart_and_partition_recovery` fails identically when run
alone on pinned baseline `bcc88941...`; its reverse-order companion passes. At correction commit
`3fcde979...`, root and desktop strict Clippy, desktop check, cargo-deny, the two final uncertain-B3
regressions and the full frontend test/check/build gates pass. Re-review closed those findings but
identified the Registry pre-gate reconstruction residual. At `f7c74cb2...`, focused prepared-source,
ordinary Registry-pass routing and repair tests plus strict affected-crate Clippy pass. The first
rereview then found the fresh-small-source liveness loop; `164a94d7...` adds the requester-bound
fresh-receiver and exact-absence regressions, which pass with the focused repair set and strict
affected-crate Clippy. On the final local candidate, the complete root suite, complete native suite
(325 library and 5 command-ACL tests), all 1,282 frontend tests, root and desktop strict
all-target/all-feature Clippy, desktop `cargo check`, root formatting, Svelte check, production build
and `cargo deny` pass. Final bounded rereview has no remaining finding. Startup/flow remain
inapplicable because no setup, process, renderer or command-registration path changed. Linux ambient
and repair-mutation evidence remain exact-head CI checks and are not claimed from Windows.

On `gate4-finalization`, the archived-tenure candidate passes the complete root suite, all 1,282
frontend tests, root/desktop strict Clippy, desktop `cargo check`, Svelte check, production build,
`cargo deny`, focused archive/doctest checks and the real A -> B -> C persist-reopen regression.
Two complete native runs each passed 324/325 library tests and all command-ACL tests, alternating
between the documented six-client convergence scenarios; a focused normal-order run passed on exact
baseline `9d2f3e34...` and failed on the candidate, while older pre-Gate-4 evidence records 2/5 on
PR #29 itself. This is an inherited timing flake, not a claimed native pass. Independent review's
one MEDIUM cloneable-witness finding was fixed; re-review reports no BLOCKER/HIGH/MEDIUM. Linux
ambient and repair-store mutation evidence remains exact-head CI-owned.

PR #34's Registry ordering correction additionally passes the focused current-tenure uncertain-B0
restart regression and the expanded real A -> B -> C explicit/prepared adapter regression. The
latter pins absent/wrong authority before capacity, B0 failure precedence, exact retained retry,
unchanged Fault state and zero prepared-path full loads. Final adversarial re-review has no remaining
finding at any severity. Merge readiness records final local gates on the correction bytes; Linux
ambient and repair-store mutations remain authoritative only on the pushed exact head.

Those final correction-byte gates now pass for the complete root suite, root/desktop strict Clippy,
desktop check, all 1,282 frontend tests, Svelte check, production build and `cargo deny`. The
complete native run passes 324/325; a focused retry reproduces the same normal-order six-client
final-convergence flake already observed on the pinned baseline, while the reverse-order companion
passes. Native is therefore not claimed green locally; exact-head CI remains the merge authority.

## Non-negotiable boundaries

- Native Save stays unavailable while P5 is false.
- Unfinished repair commands stay unregistered.
- Imported and Unknown tenure never become authoring authority.
- Preview-local work never becomes installed source, receipt, handoff or signing authority.
- Historical repair must bind the actual archived Observed witness; current tenure or ancestry is
  not a substitute.
- The shared preparation pool, source/reference fences, Prepared -> Source -> Completed order and
  exact retry identity remain intact.
- The shared baseline remains unchanged while `gate4-runtime-completion` is verified and reviewed.
- Gate 5 remains closed until full Gate 4 Review 4 passes and the user accepts it.
