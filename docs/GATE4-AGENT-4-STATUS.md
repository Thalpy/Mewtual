# Gate 4 Agent 4 status: integration and combined acceptance

Current checkpoint: 2026-10-03. Gate 4 is **incomplete**; Gate 5 remains closed.

## Integration-trial checkpoint

- A no-commit merge of `origin/gate4-agent3-repair` at `15b715a10704a8dafc2cccef65854d4d45ad55ca` into the Agent 1 baseline at `bcc88941a8677afa62957c258a127f35f328ff67` is retained only in the isolated `gate4-agent4-repair-trial` worktree. It is not an accepted shared-line merge.
- The merge was textually clean, but `cargo check -j 1 -p catcoms-app --lib --all-targets` exposed a semantic integration failure: Agent 3 added `StudioControlResponse::Fault` and `Repaired`, while Agent 2's exhaustive delivery-classification methods did not classify them.
- The trial now classifies both variants as synchronous, non-detached responses in `delivery()` and `begin_delivery()`. A focused regression uses real fault/repair outcomes and pins both methods for both variants.
- Focused verification passed: root formatting; the affected `catcoms-app` check; the corrected focused unit test (1 passed, 0 failed, 838 filtered); the separate Tauri workspace check; and the native repair-conversion test (1 passed, 0 failed, 324 filtered). An earlier `--exact` invocation selected zero tests and is not evidence.
- The required read-only adversarial review found no defect in this narrow reconciliation. It identified a future native-boundary test gap that becomes actionable only when native repair commands are registered and P5 is true. The reviewer could not independently reopen the worktree because the shared process helper failed before creation, so this remains a constrained trial review rather than acceptance of Agent 3.
- Reconciled-source inspection confirms the historical gap: `epoch_owner/repair.rs` admits only current-tenure pairs because it cannot access the archived Observed witness; `epoch_studio/repair.rs` rejects the historical branch as unavailable; and replication leaves historical authority to a future store adapter. CORE-005 is present but has no app-side consumer.
- Frontend baseline dependencies were installed with `npm ci`; the complete frontend unit suite then passed: 1,282 passed, 0 failed. The initial dependency-missing run selected no meaningful assertions and remains recorded as an environment failure, not a product failure. `npm ci` also reported five dependency-audit findings (one low, four high), which this checkpoint has not triaged or changed.
- Next unblocked Agent 4 task: inspect the CORE-005 historical-admission and overlay-provenance boundaries in the isolated merge, then run the broader repair paths. Do not register native Save/repair commands and do not claim Gate 4 completion.

Agent 4 owns integration, shared registrations/contracts, combined acceptance, required gates and
truthful status. Specialist implementation remains with Agents 1-3. The detailed requirement and
suite ledger is [GATE4-ACCEPTANCE](GATE4-ACCEPTANCE.md).

## Current summary

The current integration line contains substantial Agent 1 and Agent 2 work, but it is not ready
for native Save or a full-gate review. Agent 1 still owes Flow R, C-3 runtime adoption and a coherent
runtime review. Agent 2's manual lifecycle is implemented but has no whole-scope PASS, awaiting-
tenure work lacks its app path, repeated tenure lacks the real actor/Studio rotation, and P5 is
explicitly false. Agent 3's current-tenure repair is on a draft branch with unresolved integration,
historical-admission and bounded-runtime evidence; it does not include the newer CORE-005 witness.

The only registered overlay command on the integration baseline is the accepted read-only
`studio_overlay_read`. The lifecycle native functions exist but remain unregistered. There is no
native overlay Save or repair command. This is the correct fail-closed state while P5 and the
specialist reviews remain open.

## Exact baseline and branch state

| Item | Inspected state |
|---|---|
| Remote integration line | `origin/gate4-agent1-runtime` at `bcc88941a8677afa62957c258a127f35f328ff67` (verified with `git ls-remote`) |
| Agent 4 branch | `gate4-agent4-integration` at `bcc88941a8677afa62957c258a127f35f328ff67` |
| Agent 4 worktree | `M:\Git (local)\CatComs\target\gate4-agent4-integration` |
| Original shared checkout | `gate4-agent1-runtime` at `bcc88941...`, with unrelated untracked `.gitignore.bak`; untouched |
| Agent 3 branch | `origin/gate4-agent3-repair` at `15b715a10704a8dafc2cccef65854d4d45ad55ca` |
| Agent 3 merge base | `d1b05b37c568f5425d69f03259e6dfbbef22a502` |
| CORE-005 witness | `066a6533` is an ancestor of the integration line and is **not** an ancestor of Agent 3's inspected head |
| PR #28 | Open, draft, base `gate4-agent1-runtime`, head `gate4-agent3-repair`, GitHub reports mergeable/UNSTABLE, no submitted reviews |

GitHub's PR JSON reports `baseRefOid=d1b05b37...`, while the live base branch resolves to
`bcc88941...`; the former is the current merge base, not the live integration tip. Evidence and
future merge tests must name which SHA they use.

PR #28's latest reported check set is mixed. Linux repair-core, Linux root build/test,
studio-native, inspection, two-process Linux/Windows, both Linux NAT jobs and cargo-deny passed.
Windows root build/test, the handoff job and Linux frontend/Tauri failed; Windows repair-core and
the lifecycle job were cancelled. These are branch/PR results from 2026-10-01, not evidence for an
Agent 4 integration head.

## Evidence-backed agent state

### Agent 1

- C-1 structural decode, C-4 transient reference holds, Flow S and Flow H are integrated.
- The storage half of C-3 is implemented/reviewed; six runtime call sites still use the old
  whole-scan ownership and need cursor adoption.
- Flow R is not implemented.
- Flow S accepts a `StudioClosingOverlayBasis`; it has not been generalized to the accepted
  Closing/Unconfirmed provenance enum needed by Agent 2's preview-local work.
- `load_epoch_intents_structural` returns only `EpochIntentState`; the lower store read returns
  physical bytes, but no agreed inventory seam supplies Agent 2 with provenance plus charged bytes.
- The core signing split at `e65bfd89...` is present, while its own review note, Agent 1 status and
  the review preamble still record the independent acceptance as outstanding.
- Native Save must remain unavailable while Agent 2 P5 is false.

### Agent 2

- The authoritative completion matrix dated 2026-10-02 supersedes older implementation tables.
- Inspect/export/copy/disposition are implemented but unregistered; review `510d0b54` returned
  changes required, and later fixes have not received a whole-boundary PASS.
- Structured `StudioOverlayEligibility` / `StudioOverlayManualReason` classification is present;
  Agent 1 still returns string refusals and must map the structured result.
- Replication/sync contain the reviewed `StudioUnconfirmedOverlayBasis`, retained exact seed and
  gated mint. The app admission, quotas, S3 re-entry, Flow S use, reconciliation, restart rebuild,
  native result and non-Closing handoff-selection skip are absent.
- Repeated-tenure evidence reaches real MLS/sync/receipt behavior, and CORE-005 exists, but the
  production actor plus real Studio rotation scenario is absent.
- Windows preservation remains a real durability gap: both directory-sync helpers are no-ops on
  `cfg(not(unix))`.
- P5 is false.

### Agent 3

- `gate4-agent3-repair` implements current-tenure Studio/Registry repair, owner records, serving,
  distribution and unregistered native conversions.
- Its latest status reports each repair step as one custody visit, partial runtime catch-up tests,
  missing fetched-seed/positive Registry coverage, and historical admission blocked on CORE-005.
- CORE-005 is now available on the integration line at `066a6533`; Agent 3's branch predates it.
  That removes the dependency excuse but does not implement historical repair or its tests.
- A read-only merge analysis shows overlapping shared files in native Studio, app Studio/control,
  store Studio, replication Studio epoch and sync receipt-head. Integration must preserve both
  lifecycle/provenance and repair contracts; the draft is not merged merely to simplify history.
- PR #28 is draft and unstable and has no recorded Review 3 PASS.

## First Agent 4 changes

1. Created the isolated `gate4-agent4-integration` branch/worktree without switching, resetting or
   stashing another checkout.
2. Added [GATE4-ACCEPTANCE](GATE4-ACCEPTANCE.md), including specialist requirements, all seven
   combined scenarios, native integration, review status and a command-level evidence ledger.
3. Added this current-state record with exact refs, PR state, source-verified blockers and handoffs.
4. Kept all native write/repair commands unavailable. No security allowlist, invoke-handler,
   interface or UI-hook row was promoted.

The accepted integration branch still contains documentation-only changes. The response
classification patch and regression exist only in the explicitly unaccepted trial worktree.

## Verification at this checkpoint

| Command | Result |
|---|---|
| `git fetch --all --prune`; direct `git ls-remote` | Passed; exact remote refs recorded above. |
| `gh pr view 28 --json ...` | Passed; draft/open, mergeable/UNSTABLE and check state recorded above. |
| Read-only `git merge-tree` of current integration and Agent 3 | Completed; shared-file overlap identified. A later no-commit merge was performed only in the isolated trial worktree. |
| `cargo fmt --all -- --check` | Passed at `bcc88941...`. |
| `cargo deny check` | Passed advisory/ban/licence/source checks; duplicate dependency warnings only. |
| `bash scripts/check-no-ambient.sh` | Not executed: the host's WSL shim could not start `/bin/bash`. |
| `npm --prefix apps/desktop test` | Initial setup run failed before assertions because `node_modules` was absent. After `npm ci`, the configured rerun passed: 1,282 passed, 0 failed. |

The complete backend and native suites were not run for this documentation-only integration
checkpoint. They remain mandatory after any accepted code integration and at the final Gate 4
head. The first npm setup failure is retained separately from the configured passing run.

## Actionable specialist handoffs

There is no direct channel to the prior implementation agents in this session. The following are
messages for the user to forward; they have not been sent and no agreement is claimed.

### Agent 1

> Integration baseline is `bcc88941a8677afa62957c258a127f35f328ff67`; Agent 2's accepted
> preview basis/mint is present through `330c16ed`. Please implement two SHA-pinned seams without
> waiting for P5: (1) a structural inventory result that exposes the overlay provenance and the
> authenticated physical bytes charged to the Intents budget without reconstructing the branch,
> and (2) provenance-parameterized Flow S accepting the existing `StudioOverlayBasis` variants,
> with Closing behavior byte/semantics compatible. Preserve branch-generation classification,
> S1b media admission, S3 fresh preview re-entry/no-installed-source check, shared permit ownership
> and the receiver selector's non-Closing skip. Acceptance: focused Closing controls stay green;
> an Unconfirmed first append is detached; per-document/server/vault rails are charged; preview
> replacement/expiry cannot erase the durable branch; no native Save is registered. Then complete
> C-3 runtime adoption and Flow R, coordinating repair writers against Agent 3's `15b715a1...`.

### Agent 2

> Integration baseline is `bcc88941a8677afa62957c258a127f35f328ff67`; your current matrix at
> 2026-10-02 and P5=FALSE are authoritative. Once Agent 1 supplies the provenance/charged-byte and
> Flow S seams, complete section 8 app admission, 8.3 rails, S3 current-preview re-entry, 8.6
> reconciliation, restart reconstruction, native results and the non-Closing handoff-selector M1
> guard. Add the real actor/Studio A -> B -> A' plus newcomer scenario. Separately, provide a
> bounded lifecycle re-review request covering the fixes after `510d0b54`, including copy restart
> evidence. Do not change P5 or request registration until P1-P4 have implementation PASSes.

### Agent 3

> Your inspected head is `15b715a10704a8dafc2cccef65854d4d45ad55ca`; current integration is
> `bcc88941a8677afa62957c258a127f35f328ff67`, merge base `d1b05b37...`. Reconcile onto the current
> line and reassess CORE-005: archived Observed-tenure witness `066a6533` now exists on integration
> but is absent from your branch. Implement the app-side historical admission consumer and N17
> rather than merely clearing the dependency label. Preserve Agent 2's provenance/archive/tenure
> contracts and Agent 1's mutation-generation/Prepared/source fences. Also close the detached
> processing, fetched-seed/positive owed-Registry, full actor/native and fairness gaps, rerun the
> focused workflows on the actual reconciled SHA, and request Review 3. Keep native commands
> unregistered for Agent 4.

## Persistence design review needed before code

The Windows gap changes a persistence guarantee and therefore requires independent design review
before implementation. Proposed narrow contract:

1. A successful `EpochMutation` replacement that is relied on as preservation evidence must not
   report success until both file contents and the parent-directory namespace entry have reached
   the strongest supported durable barrier.
2. On Windows, use a directory handle opened with backup-semantics and call the platform flush
   primitive. If the platform/filesystem refuses directory flushing, return an error; never report
   a successful no-op.
3. Preserving disposal must execute this barrier before removing the original branch. A failure
   leaves the branch and its references intact and returns a retryable/uncertain result consistent
   with the existing transaction stage.
4. Exact retry may repair a committed-but-not-confirmed archive, but may not overwrite mismatched
   evidence. Unix behavior and persisted wire formats remain unchanged.
5. Tests inject directory-open/flush failure at the `WriteHooks` seam, assert no removal and exact
   retry, and run a real Windows replacement/reopen control in CI.

This proposal is not an implementation or a design PASS.

## Next unblocked Agent 4 task

Continue the isolated Agent 3 reconciliation without registering commands or claiming
integration: inspect the newer CORE-005 historical-admission and overlay-provenance boundaries for
semantic conflicts, then run the broader repair paths that exercise those boundaries.
The accepted branch remains documentation-only until Agent 3's dependency is reviewed and the
shared merge resolution is ready for the mandatory complete suites.
