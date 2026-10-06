# Gate 4 acceptance matrix

Current checkpoint: 2026-10-05. Gate 4 is **not accepted** and Gate 5 remains closed.

This is the current completion ledger. Historical implementation notes remain in the four agent
status documents and in Git history; they do not override this matrix.

## Exact integration state

| Item | Exact state |
|---|---|
| Shared baseline | `gate4-agent1-runtime` at merged PR #33 commit `9d2f3e341d9fa503e2917de61fab348dc29850d0`; unchanged by this candidate |
| Finalization candidate | PR #34 / `gate4-finalization`, based exactly on `9d2f3e34...`; adds the archived Observed-tenure consumer and the reviewed Registry B0-before-Fault ordering correction |
| Verified repair history | Historical PR #32 checkpoint `87629d6b72992254911a8e44f698d535bb5d7904`; incorporated through merged PR #33 |
| Documentation history | Historical PR #31 checkpoint `f0af61c9b1247fa955300ac50545074e18e9b302`; incorporated through merged PR #33 |
| Preserved Agent 3 head | `15b715a10704a8dafc2cccef65854d4d45ad55ca`, second parent of merge `6a2f89792eecbf6ef2e65d683e6b515bbc65181d` |

PR #33 preserves Agent 3's ancestry and keeps the Agent 4 corrections separately identifiable.
PRs #31 and #32 remain historical source checkpoints, not additional merge instructions.

## Dependency order

The remaining work is one ordered chain rather than four independent agent queues:

1. **Agent 1 runtime foundation:** the structural inventory now exposes live overlay provenance plus
   authenticated physical charged bytes. Next, parameterize Flow S over the accepted
   Closing/Unconfirmed bases, adopt the C-3 cursor at the six runtime scan owners, and implement
   Flow R.
2. **Agent 2 product paths:** use those seams for awaiting-tenure local work, reconciliation and
   restart reconstruction; drive the real actor/Studio A -> B -> A' and newcomer path; obtain a
   whole-boundary lifecycle review.
3. **Agent 3 completion:** the CORE-005 archived Observed-tenure consumer is implemented and its
   Registry integration correction is independently accepted; finish the bounded detached
   runtime/evidence gaps and obtain the bounded repair verdict.
4. **Agent 4 exposure and acceptance:** only after P1-P4 pass, change P5, register the approved
   native commands, update UI/interface truth, run the combined scenarios and request Review 4.

Agent 1's first two seams and Agent 3's historical consumer are independent and can be developed in
either order. Native registration is downstream of both specialist acceptance and P5.

## Requirement matrix

| ID | Requirement | Current state | Precise next action | Acceptance evidence still required |
|---|---|---|---|---|
| G4-A1-CORE | Detached typed handoff preparation and finite signing | **Implemented; dedicated independent acceptance still outstanding.** `e65bfd89...` is in the baseline. | Run the SHA-pinned core signing review from `GATE4-REVIEW-PREAMBLES.md`; preserve its prior focused evidence. | Bounded PASS for `8190dc4...e65bfd8`; do not infer it from later broad CI. |
| G4-A1-S | Durable local Save through Flow S | **Closing path implemented; structural inventory seam implemented and reviewed; provenance-general Flow S absent; native unavailable.** Every authenticated Intents row now reports exact sealed-file bytes and live-branch provenance without turning terminal historic metadata into a live branch. | Parameterize capture/prepare/commit over `StudioOverlayBasis` without changing Closing semantics, then consume the inventory facts in Agent 2's capacity rails. | Closing compatibility plus real Unconfirmed first append, rails, expiry/replacement and exact retry tests. |
| G4-A1-C3 | Resumable bounded inventory | **Storage cursor implemented and reviewed; six runtime owners still use direct structural loads/full scans.** | Convert the six production owners with identity, mount, generation, cancellation, restart limit and backoff checks. Coordinate repair writers already integrated from Agent 3. | Cursor invalidation, bounded progress, retained-input ownership and another-server progress. |
| G4-A1-R | Resolve interrupted Prepared state | **Not implemented.** | Build Flow R on the adopted cursor and existing source/reference fences. | Restart at each Prepared/Source/Completed barrier; reads/copy remain available and destructive actions remain refused. |
| G4-A1-MAP | Structured eligibility/manual reasons | **Agent 2 types exist; Agent 1 runtime still returns string refusals in relevant paths.** | Map runtime refusals to `StudioOverlayEligibility` / `StudioOverlayManualReason` without collapsing distinct cases. | Store -> actor -> native conversion coverage for every reason, including stale final delivery. |
| G4-A2-P1 | Inspect/export/copy/dispose lifecycle | **Implemented with unresolved behavior/test findings; unregistered; whole-boundary review not passed.** Windows preservation is fixed by `5f141994...`; `ed8ab0a8` closes M2; and `0335262e` closes the historical D4/D1/object-probe/M-1 evidence items and supplies a successful-apply control. It does not close the remaining list in Agent 2 status. | Before re-review, close or explicitly disposition the missing exact-retry half of M3, the C1'/C4 transfer-hold mismatch (M4), the vacuous provenance test (L2), exact-retry kind misreport (L4), same-document copy coverage (L5), the wrong-object-channel diagnostic Low, and P1 copy-across-restart evidence. Then request bounded lifecycle review. | Both document kinds, exact copy retry, same-document copy, restart/refusal, exact envelopes, wrong authority/scope/session, quota and crash barriers. |
| G4-A2-PREVIEW | Durable work from awaiting-tenure preview | **Replication/sync basis exists; app path absent.** | After G4-A1-S, implement custody admission, rails, S3 re-entry, save, reconciliation, restart reconstruction, native results and non-Closing selector exclusion. | Real preview Save for Index/Flipnote; expiry/replacement/restart; no handoff/signing authority. |
| G4-A2-TENURE | Real repeated owner/rejoin/newcomer | **Sync/receipt fixture passes; production actor and Studio rotation absent.** | Drive real MLS A -> B -> A' through actor, Studio, restart and newcomer. | Distinct observed tenures, first receipt, old key/receipt refusal, hidden higher history and convergence. |
| G4-A2-P5 | Permission to expose native Save | **FALSE.** | Change only after P1-P4 have implementation PASSes recorded by Agent 2. | Security allow/deny negatives while false; positive native evidence only after promotion. |
| G4-A3-CURRENT | Signed current-tenure Studio/Registry repair | **Core/store implementation integrated; automatic runtime execution now fail-closed pending its detached boundary.** PR #33 review found that repair-only service skipped the durability retry and that active repair ran synchronously outside shared admission. Commit `3fcde979...` makes repair carriage repeat exact source and authenticated owner-record durability, including uncertain B2/B3 writes, and disables automatic apply, owner resume and repaired-seed installation rather than pretending source preparation covers repair execution. A later re-review retained one MEDIUM residual because the disabled Registry router still rebuilt the full source before its gate; `f7c74cb2...` classifies only from exact-current prepared state. Review then found a fresh-small-source liveness loop; `164a94d7...` schedules that detached classification independently of size and represents exact checked absence separately from cold/unknown state. | Preserve the store/core behavior, build the capture/detach/revalidate/commit runtime, then re-enable automatic execution only with shared-pool ownership through result handling. | Interrupted-B2/B3 Studio/Registry service, warm full-pool deferral/no mutation, cancellation/result holding, unrelated-actor progress and real two-peer Fault -> decision -> replacement -> restart/newcomer. |
| G4-A3-HIST | Historical report admission with archived Observed witness | **Implemented and independently accepted on `gate4-finalization`; exact-head CI pending.** The durable head-service context carries CORE-005's private, non-cloneable witness to shared Studio/Registry admission. Exact retained attestations survive archive lookup loss; new historical evidence must match the full archived tuple and is never treated as live overflow/source authority. Studio and both Registry head adapters authenticate/account their exact source before B0, propagate failed or uncertain B0, and only then retain Fault as a service refusal. Real A -> B -> C and restart regressions cover both document families, absent/wrong authority before capacity is occupied, explicit/prepared adapters, no prepared reconstruction, exact retained retry and unchanged Fault state. | Require exact-head Linux ambient and repair-mutation CI before merging this bounded candidate. | Historical pair accepted only for the archived Observed tenure; Imported/Unknown/wrong/older tenure and malformed evidence refuse without writes. |
| G4-A3-BOUND | Detached bounded repair runtime | **Studio job implemented on `gate4-agent3-repair` (Agent 3, 2026-10-06); Registry still fail-closed; not yet independently accepted.** S1 reserves a shared-pool slot and a per-target live claim before any body read and captures bounded plaintext. S2 rebuilds detached. S3 installs the rebuild only if context and digest match disk, then runs the unchanged transaction. S4 drops ownership after the attempt. Completions are token-routed; authority changes abandon the job; cancelled waiters keep their worker's ownership. Explicit decisions, Flow D, owner resume and the owed replacement schedule it. See `GATE4-AGENT-3-STATUS.md`. | Registry job, two-peer scenario, S3 cost measurement, harness mutants, then a bounded repair verdict. | Full-pool warm/cold deferral, cancellation/result holding, unrelated-actor progress, large repair fairness, retry/teardown and full actor/native delivery. |
| G4-I-NATIVE | Shared registrations and truthful interfaces | **Correctly fail-closed.** Save and unfinished repair commands are unregistered. | Register only reviewed commands after P5; update command ACL, invoke handler, interfaces and UI hooks together. | Command-security allow/deny tests, session/final-delivery tests and production conversion for each command. |
| G4-I-E2E | Combined Gate 4 scenarios | **Not executed because prerequisite product paths remain absent.** | After bounded specialist verdicts, implement the seven scenarios in the handoff document. | Rotate/save/restart/handoff/catch-up; stale/unconfirmed lifecycle; succession; repair; concurrency; references/quotas; earlier regressions. |
| G4-I-REVIEW | Full Review 4 | **Not ready.** | Pin the final integrated SHA after all implementation and suites, then send the filled Review 4 preamble. | Independent full-gate PASS. |

## Verified candidate evidence

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
  core-signing verdict, Agent 1/2/3 completion verdicts or Review 4.
- P5 remains false, Gate 4 remains open, and Gate 5 remains closed.
