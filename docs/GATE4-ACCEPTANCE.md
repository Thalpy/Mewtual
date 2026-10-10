# Gate 4 acceptance matrix

Current checkpoint: 2026-10-10. Gate 4 is **not accepted** and Gate 5 remains closed.

This is the current completion ledger. Historical implementation notes remain in the four agent
status documents and in Git history; they do not override this matrix.

## Exact integration state

| Item | Exact state |
|---|---|
| Shared baseline | `gate4-agent1-runtime` at `acc43bcf030197b0291c194adcb30fade4a736c3` |
| Suite integration PR | PR #27 targets `Create-suite-2` at `5a899c22e7e26a7853c34f8adac4c11246846204` from exact head `acc43bcf...`; it is open and mergeable, but that is not Gate 4 acceptance |
| Flow S integration | PR #35 is merged as `a62a7f80913866c402de297b6121ef9e074ebf6b`; it preserved Agent 3's reviewed ancestry, Agent 2's rail implementation and the separately identifiable full-envelope parked-request correction |
| Archived-tenure integration | PR #34 is merged; its archived Observed-tenure consumer and reviewed Registry B0-before-Fault ordering are in the shared baseline |
| Detached repair integration | PR #36 is merged and incorporated in the baseline; PR #37's S3 ordering, Registry memoization and Prepared-bucket resume corrections are merged as `acc43bcf...` |
| Verified repair history | Historical PR #32 checkpoint `87629d6b72992254911a8e44f698d535bb5d7904`; incorporated through merged PR #33 |
| Documentation history | Historical PR #31 checkpoint `f0af61c9b1247fa955300ac50545074e18e9b302`; incorporated through merged PR #33 |
| Preserved Agent 3 head | `15b715a10704a8dafc2cccef65854d4d45ad55ca`, second parent of merge `6a2f89792eecbf6ef2e65d683e6b515bbc65181d` |

PR #33 preserves Agent 3's ancestry and keeps the Agent 4 corrections separately identifiable.
PRs #31, #32 and #34 remain historical source/integration checkpoints, not additional merge
instructions.

## Dependency order

The remaining work is one ordered chain rather than four independent agent queues:

1. **Agent 1 runtime foundation:** generalized Closing/Unconfirmed Flow S, non-Closing handoff
   exclusion and Flow R are implemented. Complete the remaining C-3 runtime adoption and the
   structured refusal mapping recorded in Agent 1's current status.
2. **Agent 2 product paths:** preview Save through the actor, reconciliation, restart reconstruction
   and native result types are implemented but unregistered. Complete the remaining repeated-tenure
   product paths and whole-boundary lifecycle review.
3. **Agent 3 completion:** the CORE-005 archived Observed-tenure consumer and detached repair
   runtime are merged, including the PR #37 corrections. Finish exact-head hosted evidence and
   carry the bounded verdict into the combined acceptance review without broadening its claim.
4. **Agent 4 exposure and acceptance:** only after P1-P4 pass, change P5, register the approved
   native commands, update UI/interface truth, run the combined scenarios and request Review 4.

Agent 1's first two seams and Agent 3's historical consumer landed independently. Native
registration remains downstream of specialist acceptance and P5.

## Requirement matrix

| ID | Requirement | Current state | Precise next action | Acceptance evidence still required |
|---|---|---|---|---|
| G4-A1-CORE | Detached typed handoff preparation and finite signing | **Implemented; SHA-pinned production review and both test-finding re-reviews are complete.** The 2026-10-06 review returned a bounded PASS for `8190dc4...e65bfd8`. SIGN-TEST-002 and SIGN-TEST-001's editor-cap/aggregate halves closed in the first re-review. The 2026-10-10 short re-review closes **SIGN-TEST-001b**: real Index and Flipnote member-authored handoffs succeed, while the per-device cap charges a non-owner and exempts the owner, pinned by three isolated mutants. Verdict: `GATE4-HANDOFF-SIGNING-REVIEW.md`. | Preserve the accepted production boundary and closed regressions through final exact-head CI and Review 4. | The bounded PASS does not absorb later runtime work. Residual non-blockers remain explicit: no isolated preflight/framing mutant and the product mapping for valid-but-untransferable member work. |
| G4-A1-S | Durable local Save through Flow S | **Implemented internally for Closing and Unconfirmed; native unavailable.** The actor is a production caller of the generalized store stages. Every authenticated Intents row reports exact sealed-file bytes and live-branch provenance. The shared baseline owns the 3/numeric-server and 8 MiB/vault rails; merged PR #35 adds full-envelope parked-request correlation so one caller cannot claim another request's detached result. Its bounded correction review passed. | Keep the command unregistered while P5 is false and carry the merged behavior through exact-head CI and combined review. | The combined head still needs its final evidence. Residual LOW coverage: direct Closing and cross-provenance equivalents of the common fingerprint regression. |
| G4-A1-C3 | Resumable bounded inventory | **Partly implemented.** The storage cursor is implemented and reviewed, and replay's manual evidence move is the first production owner to use a shared cross-visit inventory job. H5's measured approximately 0.15 s single visit was accepted as technical debt rather than split. The remaining runtime-adoption steps are not complete. | Complete the remaining measured C-3 adoption in `GATE4-AGENT-1-C3-RUNTIME.md`; steps 4 and 5 still require section 7's measurements. Coordinate the already-merged repair writers. | Cursor invalidation, bounded progress, retained-input ownership and another-server progress on each adopted owner; preserve the documented 60 s replay fallback limits. |
| G4-A1-R | Resolve interrupted Prepared state | **Implemented and reviewed.** Flow R resolves Prepared records in three scheduler stages with the source restore detached; its barriers and byte-for-byte equivalence are covered. Four background rails skip a Prepared document rather than pausing unrelated receive. | Preserve the reviewed boundary through exact-head CI and combined scenarios. Keep the documented repair-claim collision cost and temporarily occupied overlay slot explicit. | Combined restart/repair/concurrency evidence on the final head; no claim that the residual `Busy` window or wasted stale rebuild is eliminated. |
| G4-A1-MAP | Structured eligibility/manual reasons | **Agent 2 types exist; Agent 1 runtime still returns string refusals in relevant paths.** | Map runtime refusals to `StudioOverlayEligibility` / `StudioOverlayManualReason` without collapsing distinct cases. | Store -> actor -> native conversion coverage for every reason, including stale final delivery. |
| G4-A2-P1 | Inspect/export/copy/dispose lifecycle | **Implemented and bounded-review PASS; not natively exposed.** Every recorded P1 finding is closed, including exact retry/restart, transfer-hold, provenance, result-kind, same/cross-document copy and wrong-object-channel coverage. This bounded PASS is one P5 input, not full Gate 4 acceptance. | Preserve the reviewed boundary while completing the remaining P2-P4/product work; register only after P5. | Full integrated native/UI and combined-scenario evidence remains downstream of P5. |
| G4-A2-PREVIEW | Durable work from awaiting-tenure preview | **Implemented internally, not natively registered.** Custody admission, S3 re-entry, actor Save, 8.6 reconciliation, actor/store restart, native result conversion, aggregate rails, non-Closing handoff exclusion and merged PR #35's detached-result correlation are in the shared baseline. P5 remains false. | Retain the internal boundary until the remaining P1-P4/product evidence permits P5. | Final exact-head lifecycle evidence, then native/UI evidence only after promotion; no handoff/signing authority is granted by a preview. |
| G4-A2-TENURE | Real repeated owner/rejoin/newcomer | **Bounded PASS on the narrowed reviewed claim, not on the whole product path.** Actor succession and first receipts cover both same-key tenures; hidden higher history crosses the real discovery wire; the product's fresh-key A', same-key refusal through actor discovery and N-T5 through the receiver scheduler remain missing. | Complete those three honestly named product/actor paths and settle the fresh-key owner's document-continuation decision. | Distinct observed tenures, old key/receipt refusal and convergence through the production actor/scheduler boundaries, not only hand-ticked servers or bare checks. |
| G4-A2-P5 | Permission to expose native Save | **FALSE.** P1 is a bounded PASS and P2/P4 exist, but P3/native truth is partial and the full product-path evidence above is not complete. | Change only after Agent 2 records implementation/review PASS for the required P1-P4 boundary. | Security allow/deny negatives while false; positive native evidence only after promotion. |
| G4-A3-CURRENT | Signed current-tenure Studio/Registry repair | **Core/store and automatic execution are integrated through the detached job.** The fail-closed PR #33 corrections, prepared-state classifier and fresh-small-source scheduling remain preserved; PR #36 re-enabled execution only with shared-pool/job ownership, and PR #37 corrected S3 install-before-budget plus Registry resume. | Preserve the merged ordering, durability retries and fail-closed authority checks through exact-head CI and combined review. | Final-head interrupted-B2/B3, full-pool, cancellation/result-holding, unrelated-actor and real-peer replacement evidence without broadening the bounded verdict. |
| G4-A3-HIST | Historical report admission with archived Observed witness | **Implemented, independently accepted and merged through PR #34.** The durable head-service context carries CORE-005's private, non-cloneable witness to shared Studio/Registry admission. Exact retained attestations survive archive lookup loss; new historical evidence must match the full archived tuple and is never treated as live overflow/source authority. Studio and both Registry head adapters authenticate/account their exact source before B0, propagate failed or uncertain B0, and only then retain Fault as a service refusal. Real A -> B -> C and restart regressions cover both document families, absent/wrong authority before capacity is occupied, explicit/prepared adapters, no prepared reconstruction, exact retained retry and unchanged Fault state. | Preserve the merged behavior through exact-head CI and the final combined review. | Historical pair accepted only for the archived Observed tenure; Imported/Unknown/wrong/older tenure and malformed evidence refuse without writes. |
| G4-A3-BOUND | Detached bounded repair runtime | **Implemented, merged and bounded-review clean through PR #37; not a full Gate 4 verdict.** S1-S4 use shared-pool and per-target ownership, detached rebuild, exact S3 revalidation and token-routed completion. Real-peer Flow D, restart/newcomer coverage, per-target fairness and 36 runtime mutants are present. PR #37's external review and short re-review closed its one MEDIUM and one LOW; no blocker/high/medium remains in that bounded delta. | Finish the exact `acc43bcf...` hosted run on PR #27 and carry the precise bounded verdict into Review 4. | Full combined actor/native delivery remains downstream. Recorded residuals include unpaced peer repaired-seed refetch, no runtime crash injection between B1/B2 or B2/B3, and no large cold bucket through the whole job. |
| G4-I-NATIVE | Shared registrations and truthful interfaces | **Correctly fail-closed.** Save and unfinished repair commands are unregistered. | Register only reviewed commands after P5; update command ACL, invoke handler, interfaces and UI hooks together. | Command-security allow/deny tests, session/final-delivery tests and production conversion for each command. |
| G4-I-E2E | Combined Gate 4 scenarios | **Not yet complete because C-3/MAP, P5/product-tenure and native exposure prerequisites remain open.** | After specialist completion and P5, run the seven scenarios in the handoff document. | Rotate/save/restart/handoff/catch-up; stale/unconfirmed lifecycle; succession; repair; concurrency; references/quotas; earlier regressions. |
| G4-I-REVIEW | Full Review 4 | **Not ready.** | Pin the final integrated SHA after all implementation and suites, then send the filled Review 4 preamble. | Independent full-gate PASS. |

## Current exact-head checkpoint

- PR #27 is open from `gate4-agent1-runtime` at
  `acc43bcf030197b0291c194adcb30fade4a736c3` into `Create-suite-2` at
  `5a899c22e7e26a7853c34f8adac4c11246846204`. It is mergeable, but mergeability is not an
  acceptance result.
- The exact-head hosted run started on 2026-10-10. At this checkpoint the long root, native,
  frontend/Tauri, repair and mutation jobs are still running; only completed checks may be
  recorded as evidence.
- A repository ruleset object contains PR and signed-commit rules, but its branch condition matches
  no ref. The branch-rules endpoint returns no active rules for `Create-suite-2`, and classic
  branch protection is absent. Therefore PRs, signatures and status checks are **not automatic
  merge fences**. The renamed `lifecycle-mutations (1..3)` checks need no ruleset-name migration,
  but Gate 4 must require the complete named check set manually unless protection is configured.
- This document-only refresh does not promote P5, register a command, or imply Review 4.

## Historical verified candidate evidence

The following evidence is pinned to code head
`87629d6b72992254911a8e44f698d535bb5d7904` on PR #32. The documentation-only merge on the
completion line does not change it.

- All **19 GitHub checks passed**.
- Complete root build/test passed on Ubuntu (1h11m33s) and Windows (1h4m0s).
- Linux `scripts/check-no-ambient.sh` passed in CI.
- Linux frontend/Tauri checks and strict desktop all-target/all-feature Clippy passed.
- `cargo-deny`, studio-native, lifecycle, lifecycle mutations, overlay, inspection,
  prepared-signing, Linux NAT, two-process Linux/Windows and repair-core Linux/Windows passed.
- Store repair mutation/restoration passed on Ubuntu (37m16s) and Windows (1h1m18s).
- Local focused repair, Windows directory durability, issuer-tenure sequence and final harness
  regressions passed; root formatting, strict affected-crate Clippy and diff checks passed.
- `test:startup` and `test:flows` were not run because the candidate changes no setup, process,
  renderer behavior or native command registration.

Earlier local suite-only failures were investigated rather than hidden. The repeated unopened
Registry failure was a test sharing the production process-global preparation pool; `87629d6b`
injects the existing private test pool while preserving cold preparation, expiry, no-recapture and
fresh-request assertions. The exact-head Windows full suite proves the correction. A separate
native restart/partition failure reproduced on the pinned baseline; the exact candidate's required
`studio-native` CI job is green.

The completion line additionally passed the complete root suite locally after the inventory seam
was added (test debug symbols were disabled only to fit the workspace volume), plus all 1,282
frontend tests, frontend check and production build. The native suite passed 324 tests and repeated
the known `six_client_recovery::six_client_native_restart_and_partition_recovery` failure; the exact
test fails identically on baseline `bcc88941...`, while its reverse-order companion passes. This is
recorded as inherited evidence, not as a candidate pass. Linux CI remains the authority for
`check-no-ambient.sh`.

The PR #33 response at `3fcde979...` repeated the complete root suite successfully and passed both
final uncertain-B3 service regressions, root and desktop strict all-target/all-feature Clippy,
desktop `cargo check`, all 1,282 frontend tests, Svelte check, the production build and `cargo deny`
(advisories, bans, licences and sources). The native suite again passed 324 tests and reproduced only
the same baseline-identical six-client failure above. Startup/flow gates remain inapplicable because
the response changes no setup, process, renderer or command-registration path. Exact-head Linux
ambient and repair-mutation results remain CI-owned and must be recorded from the pushed draft head.

The Registry routing corrections at `f7c74cb2...` and `164a94d7...` pass the prepared-source
classifier, the fresh-requester transport-produced ordinary Registry-pass regression and all focused
repair tests. The tests pin detached preparation for a small healthy source, exact checked absence,
ordinary installation progress, fail-closed pending/unknown handling, no actor-thread full Registry
load and no durable repair mutation. On the final local candidate, the complete root suite, complete
native suite (325 library and 5 command-ACL tests), all 1,282 frontend tests, root and desktop strict
all-target/all-feature Clippy, desktop `cargo check`, root formatting, Svelte check, production build
and `cargo deny` all pass. Startup/flow remain inapplicable because no setup, process, renderer or
command-registration path changed. Exact-head Linux ambient and repair-mutation results remain
CI-owned and must pass on the pushed PR #33 head.

The `gate4-finalization` archived-tenure candidate passes the complete root suite, root and desktop
strict all-target/all-feature Clippy, desktop `cargo check`, `cargo deny`, all 1,282 frontend tests,
Svelte check and production build. Its focused sync archive/doctest and real A -> B -> C
Studio/Registry persist-reopen regression pass. Two complete native runs each passed 324 of 325
library tests and all command-ACL tests, alternating between the two known six-client final-
convergence scenarios; the normal-order test then failed twice alone while it passed once alone on
the exact `9d2f3e34...` baseline. Historical evidence predating Gate 4 records the same test at 2/5
on PR #29's own head, so this is recorded as an inherited timing flake, not a native-suite pass.
Linux ambient and repair-store mutation results remain exact-head CI checks. Startup/flow remain
inapplicable because this candidate changes no setup, process, renderer or command-registration path.

PR #34 review then found that both Registry adapters attempted `receipt_head()` on a valid Fault
source before B0, and that one wrong-archive negative ran only after the bounded slot was occupied.
The correction authenticates and accounts the explicit or prepared source, attempts B0 with its
full writer error semantics, and only then preserves Fault as a hard no-head refusal. The negative
matrix now runs before valid admission; a post-write uncertain B0 regression reopens and proves the
exact attestation was retained; and the prepared regression directly proves no full reconstruction.
The focused Registry and real A -> B -> C regressions, definitive complete root suite, root and
desktop strict Clippy, desktop check, all 1,282 frontend tests, Svelte check, production build and
`cargo deny` pass on the correction bytes. The complete native run passes 324/325 and its focused
retry reproduces only the already baseline-observed normal-order six-client final-convergence
flake; the reverse-order companion passes, so no green native-suite claim is made locally. Merge
readiness additionally requires Linux ambient/store-mutation and native evidence against the
pushed correction head.

## Review ledger

- Agent 4's independent full-candidate review found no remaining BLOCKER/HIGH/MEDIUM finding after
  the Windows durability and issuer-tenure sequence corrections. It retained one LOW coverage gap:
  no transaction-level MAX-issuance regression proves absence of signing, B1 and write side effects.
- Re-review of the CI timeout and deterministic test-pool correction found no findings.
- Independent review of the structural inventory seam initially found that terminal disposed
  metadata could be mistaken for a live branch and that a ledger-only control was absent. Both
  were fixed; re-review found no remaining finding.
- Review of PR #33 as a whole then found two HIGH repair-integration defects: repair-only replies
  could bypass the source/owner durability retry, and active repair transactions ran synchronously
  outside shared-pool admission. The candidate now repeats both service barriers and fail-closes
  automatic repair execution pending a real detached job. Focused B2/B3 and full-pool regressions
  pass. Independent re-review closed the durability and automatic-mutation bypasses but retained a
  MEDIUM residual: Registry routing performed a redundant full source reconstruction before the
  disabled gate. `f7c74cb2...` reuses exact-current prepared state and defers unknown state. Its
  first rereview closed that defect but found a fresh-small-source defer loop; `164a94d7...` schedules
  the detached classification and preserves a separately rechecked absence state. Final bounded
  re-review found no remaining BLOCKER, HIGH, MEDIUM or LOW finding.
- Independent review of the archived-tenure consumer found one MEDIUM capability-lifetime defect:
  the public archived witness was cloneable and could outlive the one-entry durable archive. The
  witness is now non-`Clone`/non-`Copy`, application code receives only a snapshot-bound borrow,
  internal snapshot duplication remains private, and a compile-fail doctest pins the boundary.
  Re-review found no remaining BLOCKER/HIGH/MEDIUM finding. PR #34 review subsequently found one
  MEDIUM Registry integration defect: valid Fault sources refused before B0, and the wrong-archive
  negative could be masked by occupied capacity. Both adapters now persist or fail B0 before the
  unchanged Fault refusal, and the negative runs against an empty owner record. Final re-review of
  the implementation plus the uncertain-write and no-reconstruction additions found no remaining
  BLOCKER, HIGH, MEDIUM or LOW finding.
- These bounded reviews accept the candidate work they inspected; they do not supply the missing
  Agent 1/2 completion verdicts, final combined evidence or Review 4.
- P5 remains false, Gate 4 remains open, and Gate 5 remains closed.
