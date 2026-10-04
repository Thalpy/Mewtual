# Gate 4 acceptance matrix

Current checkpoint: 2026-10-04. Gate 4 is **not accepted** and Gate 5 remains closed.

This is the current completion ledger. Historical implementation notes remain in the four agent
status documents and in Git history; they do not override this matrix.

## Exact integration state

| Item | Exact state |
|---|---|
| Shared baseline | `gate4-agent1-runtime` at `bcc88941a8677afa62957c258a127f35f328ff67`; unchanged |
| Verified repair candidate | `gate4-agent4-repair-candidate` at `87629d6b72992254911a8e44f698d535bb5d7904`; PR #32, open and ready for review |
| Completion line | `gate4-completion-integration`; starts from `87629d6b...`, merges the documentation checkpoint, and adds the reviewed structural inventory seam |
| Documentation checkpoint | `gate4-agent4-integration` at `f0af61c9b1247fa955300ac50545074e18e9b302`; PR #31 |
| Preserved Agent 3 head | `15b715a10704a8dafc2cccef65854d4d45ad55ca`, second parent of merge `6a2f89792eecbf6ef2e65d683e6b515bbc65181d` |

PR #32 has not been merged into the shared baseline. Its code history preserves Agent 3's ancestry
and keeps the Agent 4 corrections separately identifiable. The completion line is the only place
where the code candidate and documentation checkpoint are combined.

## Dependency order

The remaining work is one ordered chain rather than four independent agent queues:

1. **Agent 1 runtime foundation:** the structural inventory now exposes live overlay provenance plus
   authenticated physical charged bytes. Next, parameterize Flow S over the accepted
   Closing/Unconfirmed bases, adopt the C-3 cursor at the six runtime scan owners, and implement
   Flow R.
2. **Agent 2 product paths:** use those seams for awaiting-tenure local work, reconciliation and
   restart reconstruction; drive the real actor/Studio A -> B -> A' and newcomer path; obtain a
   whole-boundary lifecycle review.
3. **Agent 3 completion:** consume CORE-005's archived Observed-tenure witness for historical report
   admission, finish the bounded detached runtime/evidence gaps, and obtain the bounded repair
   verdict.
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
| G4-A3-CURRENT | Signed current-tenure Studio/Registry repair | **Implemented and integrated in PR #32.** Independent candidate review found no BLOCKER/HIGH/MEDIUM finding. | Preserve the accepted current-tenure behavior while completing the missing historical and bounded paths. | Real two-peer Fault -> decision -> recovery -> replacement -> restart/newcomer scenario. |
| G4-A3-HIST | Historical report admission with archived Observed witness | **Not implemented.** CORE-005 exists in `catcoms-sync`; app store admission still returns `historical owner authority is unavailable`. | Consume the durable archived witness under the existing owner snapshot and implement N17 plus negative substitutes. | Historical pair accepted only for the archived Observed tenure; Imported/Unknown/wrong/older tenure and malformed evidence refuse without writes. |
| G4-A3-BOUND | Detached bounded repair runtime | **Partial.** Repair core/store mutation coverage is strong, but each remaining repair step is still one custody visit and real routed evidence is incomplete. | Split the accepted S1-S4 work, integrate the C-3/source fences, and add fetched-seed/positive Registry/fairness/native evidence. | Large repair work coexisting with other targets/servers, retry/teardown, full actor/native delivery. |
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

## Review ledger

- Agent 4's independent full-candidate review found no remaining BLOCKER/HIGH/MEDIUM finding after
  the Windows durability and issuer-tenure sequence corrections. It retained one LOW coverage gap:
  no transaction-level MAX-issuance regression proves absence of signing, B1 and write side effects.
- Re-review of the CI timeout and deterministic test-pool correction found no findings.
- Independent review of the structural inventory seam initially found that terminal disposed
  metadata could be mistaken for a live branch and that a ledger-only control was absent. Both
  were fixed; re-review found no remaining finding.
- These bounded reviews accept the candidate work they inspected; they do not supply the missing
  core-signing verdict, Agent 1/2/3 completion verdicts or Review 4.
- P5 remains false, Gate 4 remains open, and Gate 5 remains closed.
