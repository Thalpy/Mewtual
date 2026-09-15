# 1
Design the remaining Gate 4 local Save and automatic handoff runtime.

Read docs/GATE4-AGENT-HANDOFFS.md, including the common instructions and Agent 1 assignment, and docs/GATE4-REVIEW-PREAMBLES.md. Follow its links to the current design, implementation and review evidence.

Design capture → detached preparation → individual signing turns → detached assembly → durable commit → native delivery. Specify concrete APIs, ownership, cancellation, authority/version checks, shared preparation permits, scheduling fairness and crash recovery.

Preserve the accepted Prepared → whole Source → Completed transaction and reference protections. Check the current review status of the core signing split; do not assume acceptance.

Native Save exposure depends on Agent 2’s reviewed manual lifecycle. Agree the interface between your scopes.

Write your proposal and test/mutation plan in docs/GATE4-AGENT-1-DESIGN.md, and maintain docs/GATE4-AGENT-1-STATUS.md. Identify dependencies and integration changes for Agent 4.

Do not implement production code or edit shared contract documents yet. Finish with a copyable adversarial review request.

# 2
Design the remaining Gate 4 overlay lifecycle and repeated-owner tenure integration.

Read docs/GATE4-AGENT-HANDOFFS.md, including the common instructions and Agent 2 assignment, and docs/GATE4-REVIEW-PREAMBLES.md. Inspect the existing implementation and accepted designs before proposing changes.

Cover bounded manual export, copy and explicit disposition of retained drafts; stale/rewound bases; durable local work based on awaiting-tenure previews; and actual A → B → A succession/rejoining.

Reuse accepted read-only inspection. Preserve complete accepted work and PIX references. Define how provisional local work survives preview expiry without acquiring installed-source or owner authority. Never infer tenure from a reused key, Welcome or historical receipt.

Specify concrete APIs, evidence/storage changes, authorization, recovery ordering, failure behavior and tests/mutations. Identify the manual-lifecycle prerequisites Agent 1 needs before enabling native Save and the tenure interface Agent 3 needs for repair.

Write docs/GATE4-AGENT-2-DESIGN.md and maintain docs/GATE4-AGENT-2-STATUS.md.

Do not implement production code or edit shared contract documents yet. Finish with a copyable adversarial review request.


#3
Design the remaining Gate 4 signed fault repair runtime.

Read docs/GATE4-AGENT-HANDOFFS.md, including the common instructions and Agent 3 assignment, and docs/GATE4-REVIEW-PREAMBLES.md. Inspect the existing ReceiptRepair v2, ReceiptBook, owner journals, recovery storage and authenticated transport.

Design explicit repair selection, current-owner/issuer-tenure authorization, durable issuance, application, distribution and visible exit from Fault.

Specify exact conflicting-receipt evidence, retry behavior, recovery-before-replacement, crash/reopen handling, capacity admission and publication ordering. Preserve interrupted overlay transactions and all common source/reference fences. Include Registry dependencies required by Index and Flipnote.

Coordinate live tenure with Agent 2 and source/commit custody with Agent 1. Distinguish existing codec/book support from missing runtime behavior.

Write concrete APIs, state transitions, storage/wire changes, tests/mutations and unresolved decisions in docs/GATE4-AGENT-3-DESIGN.md. Maintain docs/GATE4-AGENT-3-STATUS.md.

Do not implement production code or edit shared contract documents yet. Finish with a copyable adversarial review request.


# 4
Design the integration and acceptance plan that will finish Gate 4.

Read docs/GATE4-AGENT-HANDOFFS.md, including the common instructions and Agent 4 assignment, docs/GATE4-REVIEW-PREAMBLES.md, and the current backend/design/UI-hook documents.

Create docs/GATE4-ACCEPTANCE.md mapping every outstanding Gate 4 requirement to its implementation owner, dependencies, concrete production-path scenario, required tests, evidence and review status.

Plan integration of Agents 1–3, shared API/registration changes, and updates to the UI hooks and other contract documents. Identify dependency cycles and resolve the integration order without weakening requirements.

Include combined rotation/local Save/handoff, manual/provisional work, repeated succession, signed repair, concurrent progress, teardown, reference cleanup and crash recovery. Audit current CI evidence; distinguish historical failures, current failures and unexecuted checks.

Write docs/GATE4-AGENT-4-DESIGN.md and maintain docs/GATE4-AGENT-4-STATUS.md. Do not implement production changes or mark the gate complete during this design assignment.

Finish with a copyable adversarial review request. Final Gate 4 acceptance requires integrated evidence and a separate user-provided review; Gate 5 must wait.