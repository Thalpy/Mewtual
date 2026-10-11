# Gate 4 acceptance matrix

Current checkpoint: 2026-10-12. Gate 4 is **not accepted** and Gate 5 remains closed.

This is the current completion ledger. Historical implementation notes remain in the four agent
status documents and in Git history; they do not override this matrix.

## Exact integration state

| Item | Exact state |
|---|---|
| Shared baseline | `gate4-agent1-runtime` at `bdcae4a30a0870443db92a14f2d80ff8e4947818` (PR #38) |
| Isolated completion candidate | `fix/six-client-recovery-gate4`, assembled on committed parent `700d1daba4069af69fea4ff76b105c3008e6825c`; local implementation, full suites and Review 4 are complete, while the publication commit SHA and exact-head hosted CI are pending |
| Suite integration PR | PR #27 targets `Create-suite-2` at `5a899c22e7e26a7853c34f8adac4c11246846204` from exact head `2ab84645...`; it is open and mergeable, but that is not Gate 4 acceptance |
| Flow S integration | PR #35 is merged as `a62a7f80913866c402de297b6121ef9e074ebf6b`; it preserved Agent 3's reviewed ancestry, Agent 2's rail implementation and the separately identifiable full-envelope parked-request correction |
| Archived-tenure integration | PR #34 is merged; its archived Observed-tenure consumer and reviewed Registry B0-before-Fault ordering are in the shared baseline |
| Detached repair integration | PR #36 is merged and incorporated in the baseline; PR #37's S3 ordering, Registry memoization and Prepared-bucket resume corrections are merged through `acc43bcf...`; PR #39 shards the unchanged runtime mutation set in CI and is merged as `2ab84645...` |
| Verified repair history | Historical PR #32 checkpoint `87629d6b72992254911a8e44f698d535bb5d7904`; incorporated through merged PR #33 |
| Documentation history | Historical PR #31 checkpoint `f0af61c9b1247fa955300ac50545074e18e9b302`; incorporated through merged PR #33 |
| Preserved Agent 3 head | `15b715a10704a8dafc2cccef65854d4d45ad55ca`, second parent of merge `6a2f89792eecbf6ef2e65d683e6b515bbc65181d` |

PR #33 preserves Agent 3's ancestry and keeps the Agent 4 corrections separately identifiable.
PRs #31, #32 and #34 remain historical source/integration checkpoints, not additional merge
instructions.

## Dependency order

The specialist chain has been assembled on the isolated candidate. What remains is one acceptance
sequence rather than four independent agent queues:

1. **Code completion:** generalized Flow S/R, C-3's adoption decision, structured Save refusal
   mapping, lifecycle/repeated-tenure evidence and detached repair are incorporated. The fresh-key
   join-born owner path is pinned through the real actor/receiver; the same-key Welcome case is an
   intentional product refusal because a provider cannot consume a Welcome for a GroupId it owns.
2. **Bounded exposure:** the candidate promotes P5 only for the reviewed lifecycle and ticketed
   Unconfirmed Save commands. It keeps direct Closing Save and every unfinished repair command
   unavailable. Typed parsers, command ACLs and the Studio consumer land together.
3. **Acceptance evidence:** the final focused handoff mutation, complete root/native/frontend gates,
   flow check, strict Clippy, dependency policy and an explicit Bash ambient check pass locally.
   Exact-head hosted Linux ambient and the complete store/runtime mutation workflows remain required;
   the final startup probe must also be rerun because its latest local attempt hung before `setup`.
4. **Review 4:** the independent complete-diff review and correction re-reviews report no unresolved
   finding at any severity. Publish that reviewed tree without adding unreviewed code.

The shared baseline stays unchanged until this sequence passes. Gate 5 remains closed.

## Requirement matrix

| ID | Requirement | Current state | Precise next action | Acceptance evidence still required |
|---|---|---|---|---|
| G4-A1-CORE | Detached typed handoff preparation and finite signing | **Implemented; SHA-pinned production review and both test-finding re-reviews are complete.** The 2026-10-06 review returned a bounded PASS for `8190dc4...e65bfd8`. SIGN-TEST-002 and SIGN-TEST-001's editor-cap/aggregate halves closed in the first re-review. The 2026-10-10 short re-review closes **SIGN-TEST-001b**: real Index and Flipnote member-authored handoffs succeed, while the per-device cap charges a non-owner and exempts the owner, pinned by three isolated mutants. Verdict: `GATE4-HANDOFF-SIGNING-REVIEW.md`. | Preserve the accepted production boundary and closed regressions through final exact-head CI and Review 4. | The bounded PASS does not absorb later runtime work. Residual non-blockers remain explicit: no isolated preflight/framing mutant and the product mapping for valid-but-untransferable member work. |
| G4-A1-S | Durable local Save through Flow S | **Implemented.** Closing remains an internal automatic path; Unconfirmed is exposed only through the ticketed Save pair. The actor is a production caller of the generalized store stages. Exact sealed-file bytes, live provenance, aggregate rails, full-envelope correlation, retained commit authority and stamped refusals are preserved. | Preserve the direct-Closing prohibition through exact-head CI. | Local combined suites and Review 4 pass. Exact-head hosted mutation evidence remains; a direct cross-provenance parked-plan regression is a recorded LOW coverage gap. |
| G4-A1-C3 | Resumable bounded inventory | **Implemented with measured synchronous debt.** The storage cursor is implemented and reviewed, and replay's manual evidence move uses the shared cross-visit inventory job. All-family writer warms, bounded per-key restart credit and the section 7 release profile are complete. A real foreign writer overtaking a cold job now has a regression proving the writer-warmed restart mints on the next turn. H5, catch-up, receive, H1 and repair keep their existing single-visit scans as explicitly documented debt rather than acquiring unsafe second permits or new unreviewed retained stages. | Preserve the shared-job invariants and the synchronous-debt statement through exact-head CI and Review 4. A future latency gate may split the retained owners; Gate 4 does not claim their cold/remount paths fit `INVENTORY_SLICE_MS`. | Cursor invalidation, bounded progress, retained-input ownership, real-writer restart progress and the 60 s replay fallback; cold automatic work retains its fail-closed rails. |
| G4-A1-R | Resolve interrupted Prepared state | **Implemented and reviewed.** Flow R resolves Prepared records in three scheduler stages with the source restore detached; its barriers and byte-for-byte equivalence are covered. Four background rails skip a Prepared document rather than pausing unrelated receive. | Preserve the reviewed boundary through exact-head CI and combined scenarios. Keep the documented repair-claim collision cost and temporarily occupied overlay slot explicit. | Combined restart/repair/concurrency evidence on the final head; no claim that the residual `Busy` window or wasted stale rebuild is eliminated. |
| G4-A1-MAP | Structured eligibility/manual reasons | **Implemented and locally reviewed on the candidate.** Store refusals carry `StudioOverlaySaveRefusal`; the actor preserves that type; native maps it exhaustively; TypeScript rejects unknown variants. Closing and Unconfirmed plans retain their own authority and stamped refusal without repeating detached work. The reachable `EpochBound` and `IntentConflict` paths map to typed refusals; invariant failures remain errors. | Preserve exhaustive matches through exact-head hosted mutations. | Local Review 4 and full suites pass; exact-head store/runtime mutation evidence remains. |
| G4-A2-P1 | Inspect/export/copy/dispose lifecycle | **Implemented, bounded-review PASS and exposed on the isolated candidate.** Every recorded P1 finding is closed. The UI presents local-only provenance, uses two-step copy, requires destructive confirmations and retains exact uncertain edits. | Preserve the reviewed boundary through exact-head CI. | Complete local native/frontend and combined evidence pass. |
| G4-A2-PREVIEW | Durable work from awaiting-tenure preview | **Implemented and exposed on the isolated candidate.** Custody admission, S3 re-entry, reconciliation, restart, rails and detached-result correlation remain intact. The UI freezes the exact ticket/basis/branch/nonce/body, follows typed retry actions and displays the re-read overlay separately from its non-authoritative base. | Preserve the local full-suite evidence through exact-head CI. | No handoff, signing, receipt, source or publication authority may be inferred from the overlay. |
| G4-A2-TENURE | Real repeated owner/rejoin/newcomer | **Implemented at the supported product boundary.** Actor succession/receipts cover distinct same-key tenures; hidden higher history crosses discovery; a fresh-key join-born owner starts a fresh product history while a continuously present receiver retains the former history in recovery. A same-GroupId provider is refused rather than pretending it can consume its own Welcome. | Preserve that explicit product decision through exact-head CI. | The final local suite and review pass; there is no claim of impossible same-key self-Welcome continuity. |
| G4-A2-P5 | Permission to expose native Save | **TRUE on the isolated candidate for the ticketed Unconfirmed Save and reviewed lifecycle family.** P1-P4 code/evidence are assembled and P3 now includes command policy, exhaustive native/frontend conversion and a truthful UI. Complete local native/frontend/security evidence and Review 4 pass. This is not permission for direct Closing Save or repair commands. | Obtain exact-head hosted CI before merging the candidate. | Publication and the exact-head required check set. |
| G4-A3-CURRENT | Signed current-tenure Studio/Registry repair | **Core/store and automatic execution are integrated through the detached job.** The fail-closed PR #33 corrections, prepared-state classifier and fresh-small-source scheduling remain preserved; PR #36 re-enabled execution only with shared-pool/job ownership, and PR #37 corrected S3 install-before-budget plus Registry resume. | Preserve the merged ordering, durability retries and fail-closed authority checks through exact-head CI and combined review. | Final-head interrupted-B2/B3, full-pool, cancellation/result-holding, unrelated-actor and real-peer replacement evidence without broadening the bounded verdict. |
| G4-A3-HIST | Historical report admission with archived Observed witness | **Implemented, independently accepted and merged through PR #34.** The durable head-service context carries CORE-005's private, non-cloneable witness to shared Studio/Registry admission. Exact retained attestations survive archive lookup loss; new historical evidence must match the full archived tuple and is never treated as live overflow/source authority. Studio and both Registry head adapters authenticate/account their exact source before B0, propagate failed or uncertain B0, and only then retain Fault as a service refusal. Real A -> B -> C and restart regressions cover both document families, absent/wrong authority before capacity is occupied, explicit/prepared adapters, no prepared reconstruction, exact retained retry and unchanged Fault state. | Preserve the merged behavior through exact-head CI and the final combined review. | Historical pair accepted only for the archived Observed tenure; Imported/Unknown/wrong/older tenure and malformed evidence refuse without writes. |
| G4-A3-BOUND | Detached bounded repair runtime | **Implemented, merged and bounded-review clean through PR #37; not by itself a full Gate 4 verdict.** S1-S4 use shared-pool and per-target ownership, detached rebuild, exact S3 revalidation and token-routed completion. Real-peer Flow D, restart/newcomer coverage, per-target fairness and 36 runtime mutants are present. PR #37's external review and short re-review closed its one MEDIUM and one LOW; no blocker/high/medium remains in that bounded delta. PR #39 changes only CI execution, sharding the same mutation set after the monolithic Windows job timed out. | Run the unchanged complete runtime mutation workflow on the published candidate head. | Local full combined suites and Review 4 pass. Recorded residuals include unpaced peer repaired-seed refetch, no runtime crash injection between B1/B2 or B2/B3, and no large cold bucket through the whole job. |
| G4-I-NATIVE | Shared registrations and truthful interfaces | **Implemented and locally verified on the isolated candidate.** Eleven lifecycle/Save additions are registered and permissioned; read/write classification and module-scoped session gates are tested. Direct Closing Save and repair stay absent. | Publish and run exact-head native/frontend/security checks. | The local native suite (336 library plus 5 ACL tests), 1,300 frontend tests, check/build and flow gate pass. |
| G4-I-E2E | Combined Gate 4 scenarios | **Code and local evidence are assembled.** Focused store/actor/native/frontend scenarios are present, including fresh-key tenure, local-only Save and lifecycle/copy; complete root, native and frontend suites pass. | Publish the candidate and obtain the exact-head hosted ambient/mutation checks plus a fresh startup result. | Final exact-head results without weakening assertions. |
| G4-I-REVIEW | Full Review 4 | **Complete for the assembled implementation.** The reviewer inspected the actual combined diff and surrounding paths, carried prior accepted findings forward, and found no unresolved issue at any severity. The startup-harness diagnostic LOWs were fixed and have focused coverage. | Keep the publication commit code-identical to the reviewed tree and record the exact SHA in PR #41. | Exact-head hosted evidence remains a separate merge prerequisite; earlier bounded reviews remain inputs, not substitutes. |

## Current exact-head checkpoint

- The isolated candidate's committed parent is `700d1daba4069af69fea4ff76b105c3008e6825c`.
  The locally reviewed completion delta contains the structured refusal carrier, fresh-key product
  regression, lifecycle/Unconfirmed native registration, Studio consumer and the final harness/test
  corrections. One publication commit and its exact SHA are still pending.
- The two PR #40 review MEDIUMs are already closed by
  `55ff3d93744cd1f965ab083e910d9645f1f6ac26`: a held Studio page reacquires the permit released by
  evicting its retained Registry provider before service may compete, and the
  `handoff-priority-owed` mutant targets a dedicated running-preparation regression.
- A child checkpoint of PR #40 fixes the previously baseline-observed Windows six-client reunion
  failure. The saved routes were present, but listener-port reuse made rapid reconnect sockets fail
  with `WSAEADDRINUSE`; Windows dials now allocate a fresh outbound port while Unix retains the
  NAT-useful default. Both focused native six-client scenarios and the complete `catcoms-net`
  suite pass. Historical failed-suite entries below remain evidence about their exact older SHAs,
  not evidence against this correction. The final combined native suite now passes locally.
- The final local candidate passes `cargo test --all --all-features`; the desktop/Tauri suite
  (336 library plus 5 command-ACL tests); both strict root and desktop Clippy commands; desktop
  `cargo check`; root formatting; all 1,300 frontend tests; Svelte check; production build;
  `cargo deny`; `test:flows`; and the focused `handoff-priority-owed` mutation/restoration check.
  `scripts/check-no-ambient.sh` passes under the installed Scoop/Git Bash. The WSL launcher still
  fails locally with `E_ACCESSDENIED`, which is neither a code failure nor a second pass.
- The startup gate passed on the assembled application after its marker was made observable under
  inherited `RUST_LOG=warn`. Review found that the first harness correction hid default diagnostics
  when `RUST_LOG` was absent; the helper now preserves the unset default, appends only to a nonempty
  filter, and is covered by six cross-platform tests. Later local startup reruns hang before `setup` and create no
  application log even with a 60 s limit, so they are recorded as an unresolved local execution
  result rather than a pass. A fresh exact-head startup run is required.
- PR #41 is the draft publication vehicle. It must target `gate4-agent1-runtime` at
  `bdcae4a30a0870443db92a14f2d80ff8e4947818`, name the final candidate SHA and preserve the PR #28
  ancestry relationship. Its exact-head Linux ambient, complete store/runtime mutations and other
  required hosted checks are the remaining acceptance authority.
- A repository ruleset object contains PR and signed-commit rules, but its branch condition matches
  no ref. The branch-rules endpoint returns no active rules for `Create-suite-2`, and classic
  branch protection is absent. Therefore PRs, signatures and status checks are **not automatic
  merge fences**. The renamed `lifecycle-mutations (1..3)` checks need no ruleset-name migration,
  but Gate 4 must require the complete named check set manually unless protection is configured.
- This candidate promotes P5 only for the bounded lifecycle/ticketed-Unconfirmed family described
  above. It does not expose direct Closing Save or repair commands, imply Gate 5 acceptance, or make
  a hosted check pass before that exact candidate SHA actually runs it.

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
- These bounded reviews accept the work they inspected; they do not substitute for the final
  combined evidence or Review 4.
- P5 is promoted only on the isolated completion candidate and only for the listed lifecycle and
  Unconfirmed Save commands. Gate 4 remains open until publication, the remaining exact-head hosted
  evidence and user acceptance; Gate 5 remains closed.
