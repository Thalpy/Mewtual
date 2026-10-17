# Gate 4: adversarial review preambles

Use these with the [four implementation handoffs](GATE4-AGENT-HANDOFFS.md). The implementer
fills in the immutable full base/head SHAs, compare URL, scope note, CI run/checkout evidence
and prior verdicts before sending. Do not send placeholders as a ready review request.
Review agents should read the common contract below and the relevant scope. Design approval,
implementation approval, regression-finding closure and full-gate acceptance are separate verdicts.

## Review needed now: core handoff signing split

This is the outstanding review for work already implemented, not one of the four future
runtime acceptance reviews. INSPECTION-TEST-001 is closed and the previous bounded handoff
production PASS stands. The new core split at `e65bfd8` has not received adversarial acceptance.
The integration branch also contains an independently merged jukebox fixture fix; keep the
core review scoped to the exact pair below and read evidence documentation at the pushed head.

```text
Please adversarially review e65bfd89acecd4e660edb0560410e1d02cec5e21 against
8190dc46b885f6b61efe9d84c8a1675fb041f823 on Thalpy/Mewtual PR #26:
https://github.com/Thalpy/Mewtual/compare/8190dc46b885f6b61efe9d84c8a1675fb041f823...e65bfd89acecd4e660edb0560410e1d02cec5e21

Read docs/GATE4-HANDOFF-SIGNING-REVIEW.md and the latest HANDOVER/P1-PERFORMANCE evidence.
Challenge opaque authority and batch provenance, actual source/branch binding, complete
envelope/order/timestamp preservation, pre-sign typed/aggregate admission, exactly one signing
operation per call, live MLS/membership/key/owner/observed-tenure checks, private signed output
and complete final assembly. Check the existing store uses this same staged algorithm without
weakening Prepared -> whole Source -> Completed, source-required metadata or reference fences.

Local evidence: five core and 25 app handoff tests pass; two opt-in profiles are ignored.
Strict replication/app Clippy passes. MLS-epoch, accounting-owner and relocated acceptance-order
mutants fail at their intended executed assertions, restore exact bytes and pass restored tests.
Inspect the ordinary-edit independent oracle and both real 256-operation fixtures. The measured
single signing call is a work-count observation, not maximal-shape latency or heap qualification.
Verify any new CI run against its actual checkout; earlier native/inspection runs are older code.

Return PASS for this bounded core implementation or numbered findings with concrete failure
paths and necessary corrections. Actor/native Save, runtime permit/stamp/commit scheduling,
manual/provisional lifecycle, repeated tenure, signed repair and full Gate 4 remain separate.
```

## Common contract for the four subsequent reviews

```text
You are the independent adversarial reviewer for the Gate 4 scope below. Read
docs/GATE4-AGENT-HANDOFFS.md, your corresponding implementation-agent status note,
docs/BACKEND-IMPLEMENTATION.md, docs/design-creative-suite.md, docs/design-epoch-close.md,
docs/INTERFACES.md and docs/FLIPNOTE-UI-HOOKS.md, plus the relevant accepted design/review notes.
Inspect actual code and tests in the pinned diff; documentation and implementer claims are
evidence pointers, not proof. Account for dependency commits and any merge-resolution changes.

First state the full resolved baseline/head, actual reviewed scope, accepted dependencies,
and whether you executed tests or only inspected source/logs. Distinguish the branch code SHA
from a PR merge checkout and a later documentation-only SHA. Do not reuse an earlier workflow
PASS for changed code without checking its relevance.

Try concrete failure paths across authentication, authority changes, target/channel identity,
concurrency, cancellation, restart, resource admission, durable ordering and publication.
Test assertions must establish that crafted inputs reach the intended guard. Equivalent display,
successful decryption alone, a broad is_err(), synthetic preview or state enum is insufficient
when a different failure can mask removal of that guard. Require an independent positive oracle.
For claimed mutation evidence verify the unique guard removed, executed test count, exact intended
assertion, byte-exact source restoration and restored pass. Build errors and zero tests do not count.

Preserve accepted finding closures unless you demonstrate a new regression. Separate design,
production and test-coverage findings. Return a scoped verdict and numbered findings with severity,
file/line, trigger, impact, evidence and required correction. List unexecuted or missing acceptance
evidence explicitly. A bounded PASS does not imply complete Gate 4 acceptance; Gate 5 stays closed.
Do not edit production code or silently fix the implementation during this review.
```

## Review 1: actor/native local Save and automatic handoff

Send the common contract and this scope together after filling the bracketed fields.

```text
Review type: [design / bounded implementation / finding re-review].
Base: [FULL_BASE_SHA]. Head: [FULL_HEAD_SHA]. Compare: [IMMUTABLE_COMPARE_URL].
Scope/evidence: [AGENT_1_STATUS_AND_REVIEW_NOTE], [CI_RUNS_AND_ACTUAL_CHECKOUTS].
Dependencies: [CORE_SIGNING_VERDICT], [MANUAL_LIFECYCLE_VERDICT_OR_EXPLICITLY_DISABLED_NATIVE_SAVE].

Challenge the complete capture -> detached validation -> finite signing -> detached assembly ->
durable commit -> native delivery path, not only the new sign_next call. Identify every expensive
decode, graph restore, inventory traversal and final conversion still holding Server/vault/native
custody. One blocking thread or one signature per call alone does not prove actor responsiveness.

Verify the original four-slot shared preparation reservation remains owned through queue, worker,
ready result, signing, commit and delivery; cancellation never frees a live owner's slot. Check
actual numeric server, group/type/logical/channel, device/membership/key, MLS epoch, observed tenure,
actor/sync incarnation, mount, full source/intent wrapper digest+size and original native request/
session at the necessary visits. A core snapshot hash or inspection result is not write authority.

Try same-size different authenticated wrappers, same display with extra history, stale session
after successful conversion, mid-signing membership/MLS change and an interrupted Prepared record.
First local acceptance must derive from actual Closing source+saved close+observed tenure. Exact
saved retry must not require a now-absent Closing basis or create a second envelope. Ordinary
failed Apply must not become accepted local work.

Require whole Prepared -> Source -> Completed with full signed evidence and retained pending
ledger; challenge every write/flush interruption, actual replacement peaks, required-metadata
links, common source-write/reference inventory fences and publication hold. No signed prefix
may escape. Completed publication must use ordinary paging and preserve the initial Save limit.

Inspect real Index/Flipnote Save/restart/automatic handoff/catch-up, actual cleanup, maximum
accepted shapes and combined authoritative/other-server progress while a real job is paused.
Native Save must remain unexposed until its manual lifecycle is implemented and reviewed.
Return a verdict for this runtime boundary only; other Gate 4 work remains separately reviewed.
```

## Review 2: manual/provisional overlay lifecycle and repeated tenure

The preview-local-work extension and any new tenure authority need a concrete design review
before implementation acceptance. Use separate checkpoints/verdicts if their changes are split.

```text
Review type: [design / bounded implementation / finding re-review].
Base: [FULL_BASE_SHA]. Head: [FULL_HEAD_SHA]. Compare: [IMMUTABLE_COMPARE_URL].
Scope/evidence: [AGENT_2_STATUS_AND_REVIEW_NOTE], [CI_RUNS_AND_ACTUAL_CHECKOUTS].
Dependencies: [RUNTIME_SEAMS_AND_ACCEPTED_DESIGNS]; native Save exposure: [ACTUAL_STATE].

Challenge lossless inspection/export/copy/disposition for branches that cannot auto-handoff.
Test mixed ordinary and annotated intents, full-envelope matching, explicit user action,
copy admission failures, recovery-before-removal, exact retries and restart at each transition.
Export, equal projections, markers, eviction acknowledgement and a failed copy cannot discard
accepted work or label it settled. Removed membership cannot acquire a new offline export right.
Verify actual PIX cleanup protects base-only, superseded, removed and pending references.

Try stale/rewound/nonpristine sources and preview expiry/replacement/remount while local work
exists. The old Closing-overlay basis must still reject an unconfirmed preview. Inspect the
separately reviewed preview-local-work evidence format, lifetime, quotas, complete scope binding
and reconciliation. A preview is never an installed source, tenure, receipt or signing capability;
its expiry must not erase a durably accepted local draft. A permanently disabled placeholder
does not satisfy the requested preview-local-work requirement.

Exercise actual A -> B -> A succession/rejoining and a newcomer with Unknown tenure. Same key,
Welcome, current group epoch or a carried historical receipt must not invent the tenure start.
Verify independent authority acquisition, first-receipt inheritance, exact retry with a returning
owner, stale same-key signature refusal, hidden higher old-tenure history and durable recovery.
Fail-closed Unknown without any implemented legitimate progress path is incomplete integration.

Check native session/final-delivery fences and truthful local-only/awaiting/manual/recovery
labels/events. Require real actor/store/native evidence for both Index and Flipnote, complete
projection/envelope comparisons, isolated guard mutations and lossless crash/reopen behavior.
State whether all manual-lifecycle conditions for enabling native Save are satisfied. This PASS
does not independently accept signed repair, combined runtime integration or full Gate 4.
```

## Review 3: signed fault repair runtime

```text
Review type: [design / bounded implementation / finding re-review].
Base: [FULL_BASE_SHA]. Head: [FULL_HEAD_SHA]. Compare: [IMMUTABLE_COMPARE_URL].
Scope/evidence: [AGENT_3_STATUS_AND_REVIEW_NOTE], [CI_RUNS_AND_ACTUAL_CHECKOUTS].
Dependencies: [REPAIR_DESIGN_VERDICT], [TENURE_SEAM], [SOURCE_PREPARED_FENCE_INTEGRATION].

Review issuance, durable application, serving/receiving and runtime exit from Fault, not merely
ReceiptRepair encode/decode or ReceiptBook unit tests. Identify the explicit winning-receipt
decision and its authorization. Restore/Copy, projection equality or receipt hints cannot repair
a fault. Only the actual current owner with independently verified issuer tenure may authorize
a live v2 repair, including exact retries and A -> B -> A; historical v1 evidence is insufficient.

Verify both full conflicting receipts, selected hash, sequence and complete target binding.
Try a different named pair, earlier same-key tenure, old repaired loser/descendants, newer valid
progress and a third conflicting baseline. A retry must not erase newer progress or clear an
unrelated active fault. Inspect bounded full evidence retained through restart and replacement.

Trace the actual persisted owner decision, losing typed recovery, source replacement, intent
retirement and publication barriers. Crash at each write/flush and reopen from sealed bytes.
No recovery capacity or uncertain IO must mean a visible hold with accepted work retained.
Respect Prepared overlays, source-required metadata and complete reference inventory on every
common writer; check the real cleanup path cannot reclaim still-required pixels.

Verify bounded authenticated repair/head/record/seed transport, current authority and exact seed
admission, save-before-serve, duplicate/replay pacing, job ownership and other-server progress.
Require two real peers entering Fault and converging after repair, then a newcomer after repair,
with actual installed state, losing recovery, receipt evidence and native Fault/Repairing/current
events. Include Registry dependencies needed by Index and Flipnote. Isolate current-tenure and
recovery-before-replacement guard mutations. Return a bounded repair verdict, not full Gate 4.
```

## Review 4: integrated Gate 4 acceptance

Request this after the implementation scopes and required checks have concrete evidence.
Agent 4 can request earlier bounded reviews of tests/CI fixes, clearly labelled as such.

```text
Review type: full Gate 4 acceptance.
Baseline: [FULL_INTEGRATION_BASE_SHA]. Head: [FULL_FINAL_HEAD_SHA].
Compare: [IMMUTABLE_COMPARE_URL]. Matrix: docs/GATE4-ACCEPTANCE.md at [DOC_SHA].
Bounded design/implementation verdicts and findings: [EXACT_CHECKPOINTS_AND_DISPOSITIONS].
Required suite evidence: [RUN_JOB_URLS_AND_ACTUAL_MERGE_CHECKOUT_SHAS].

Audit every remaining Gate 4 requirement against production behavior, a meaningful integrated
test and review disposition. Do not infer full acceptance from the three bounded PASS verdicts,
new native command names or older scheduling/inspection acceptance. Inspect integration conflict
resolutions, common dispatch/registration, source/reference fences and native contracts as code.

Require create/save/share -> rotation -> local-only Save -> restart -> verified successor ->
automatic whole handoff -> peer catch-up -> settlement/recovery; stale/provisional manual
lifecycle; real A -> B -> A/newcomer authority; fault -> signed repair -> losing recovery ->
reopen/joiner. Include both Index/Flipnote and the Registry paths they depend on. Check original
envelope identity, complete projection, actual durable bytes and blob cleanup at transitions.

Challenge combined interference: paused large preparations, queued retries and ready results
while authoritative discovery/receive and another server progress; UI spam, session replacement,
lock/remount and crashes during Prepared or repair. Require real permit owners and actual native
conversion. A synthetic ready object, fixed sleep or injected authoritative state cannot establish
these boundaries. Check maximal accepted input/output shapes, quota/refusal and publication limits.

Verify required workspace/platform, native, frontend, formatting, strict Clippy, ambient-dependency,
supply-chain, focused mutation/restoration and two-client checks from their job logs on applicable
code. List ignored and unexecuted tests and unresolved failures. Confirm previously failing baseline
checks were actually reverified; no warning/advisory suppression or weakened assertion counts as
resolution. Distinguish real actor/native/CLI evidence from packaged desktop UI acceptance.

Check HANDOVER, BACKEND-IMPLEMENTATION, INTERFACES and FLIPNOTE-UI-HOOKS agree with actual
callable commands, flags, events, local/shared/receipted states and remaining limits. Require a
specific explanation for any supposedly deferred Gate 4 row; other product gates cannot conceal
missing Gate 4 behavior. Gate 5-7 features themselves are outside this review.

Return PASS for full Gate 4 only if all required behavior, suites and review findings support it;
otherwise return REQUEST CHANGES with concrete numbered findings and unmet acceptance items.
State exactly what ran independently and what was assessed from supplied evidence.
```
