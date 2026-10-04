# Gate 4 Agent 4 status: integration and completion coordination

Current checkpoint: 2026-10-04. The verified repair candidate is ready for review but is not
merged into the shared baseline. Gate 4 remains incomplete and Gate 5 remains closed.

The detailed live ledger is [GATE4-ACCEPTANCE](GATE4-ACCEPTANCE.md). This document records the
integration history, current ownership boundaries and the single remaining execution sequence.

## Current state

Agent 4 has completed the bounded repair integration checkpoint:

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
- All 19 exact-head PR #32 checks pass and the independent bounded review has no remaining
  BLOCKER/HIGH/MEDIUM finding.

The code is still intentionally unavailable to the renderer. P5 is false; native Save and repair
commands remain unregistered. The current-tenure repair implementation does not make historical
repair complete, and merging ancestry does not supply the missing archived-tenure consumer.

## Exact branch state

| Item | State |
|---|---|
| Shared integration baseline | `gate4-agent1-runtime` at `bcc88941a8677afa62957c258a127f35f328ff67`; untouched |
| Repair candidate | `gate4-agent4-repair-candidate` at `87629d6b72992254911a8e44f698d535bb5d7904`; PR #32 open, ready, clean |
| Documentation checkpoint | `gate4-agent4-integration` at `f0af61c9b1247fa955300ac50545074e18e9b302`; PR #31 open |
| Completion branch | `gate4-completion-integration`, based on `87629d6b...` with the documentation history merged |
| Agent 3 source | `gate4-agent3-repair` at `15b715a10704a8dafc2cccef65854d4d45ad55ca`; preserved, not rewritten |

PR #28 remains the Agent 3 source record. PR #32 is its integrated successor and documents that
relationship. Neither PR #31 nor #32 has been merged into the shared baseline.

## What each agent actually leaves behind

### Agent 1

Completed: structural decode, transient reference protection, I-4 generation enforcement, the
storage half of C-3, scheduled Closing Flow S and automatic Flow H.

Still required:

1. parameterize Flow S over the accepted `StudioOverlayBasis` variants, using the now-complete
   structural provenance/charged-byte inventory result;
2. map structured eligibility/manual reasons instead of returning only strings;
3. adopt `EpochStorageCursor` at the six runtime scan owners;
4. implement Flow R after cursor adoption;
5. finish the required maximum-shape/custody measurements; and
6. obtain the dedicated core-signing and coherent runtime reviews.

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
2. consume Agent 1's completed inventory seam and the still-missing generalized Flow S seam in the app;
3. implement preview custody admission, rails, S3 re-entry, Save, reconciliation and restart rebuild;
4. exclude non-Closing branches from automatic handoff selection;
5. drive the returning owner through the real app actor and Studio rotation;
6. complete native result contracts without registering commands; and
7. obtain a whole-boundary lifecycle/repeated-tenure review.

P5 remains **FALSE** until those P1-P4 requirements have implementation PASSes.

### Agent 3

Completed and integrated in the candidate: signed current-tenure Studio/Registry repair, owner
records, durable repair transitions, serving/distribution, catch-up routing, native conversion
types, core/store mutation harnesses and the issuer-tenure sequence correction.

Still required:

1. consume CORE-005's archived Observed-tenure witness in app-side report admission;
2. prove N17 and the malformed/wrong/Imported/Unknown negative cases;
3. finish the detached S1-S4 custody split and C-3/source-fence integration;
4. add fetched-seed, positive owed-Registry, real two-peer/newcomer and fairness evidence; and
5. obtain the bounded Review 3 verdict for the completed boundary.

### Agent 4

Completed: preserved-history integration, response classification, Windows durability, repair
sequence correction, two-platform mutation CI, desktop Clippy, full exact-head suites, bounded
review and truthful unavailable registration state.

Still required: integrate the specialist completions, maintain this ledger, register only approved
commands after P5, implement/run the seven combined scenarios, update interface/UI truth and request
Review 4. Agent 4 must not turn dependency ancestry into an implementation claim.

## Unified completion sequence

The next execution order is:

1. Build the Agent 1 generalized Flow S seam on the completed structural
   provenance/charged-byte inventory result.
2. In parallel only conceptually, build Agent 3's archived-tenure admission consumer; it does not
   depend on native registration or Agent 2's preview app path.
3. Adopt the C-3 cursor at runtime call sites, then implement Flow R.
4. Close Agent 2's remaining lifecycle/copy findings and copy-restart evidence, then complete its
   preview-local app path and real actor/Studio returning-owner path.
5. Complete Agent 3's detached/runtime evidence, then obtain bounded reviews for Agents 1-3.
6. If and only if Agent 2 records P5 true, add native registrations, ACLs, interface rows and UI
   hooks in one reviewable checkpoint.
7. Run the combined Index/Flipnote/Registry scenarios and all required suites, then request Review 4.

## Verification for the current code candidate

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
alone on pinned baseline `bcc88941...`; its reverse-order companion passes. Strict lint, cargo-deny
and the new branch's Linux CI are recorded separately at publication time. The inventory seam's
adversarial re-review has no remaining finding.

## Non-negotiable boundaries

- Native Save stays unavailable while P5 is false.
- Unfinished repair commands stay unregistered.
- Imported and Unknown tenure never become authoring authority.
- Preview-local work never becomes installed source, receipt, handoff or signing authority.
- Historical repair must bind the actual archived Observed witness; current tenure or ancestry is
  not a substitute.
- The shared preparation pool, source/reference fences, Prepared -> Source -> Completed order and
  exact retry identity remain intact.
- PR #32 is not merged into `gate4-agent1-runtime` until the user chooses that integration action.
- Gate 5 remains closed until full Gate 4 Review 4 passes and the user accepts it.
