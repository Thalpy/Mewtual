# Adversarial review doctrine

Mewtual is a security-sensitive peer-to-peer application. Every code change receives an
adversarial review before final handoff. Documentation-only and repository-metadata-only changes
are exempt only when they cannot affect runtime, build, compatibility, security, privacy, or
user-facing truthfulness.

Run the review after the implementation and focused tests are coherent, but before declaring the
work complete. Protocol, identity, membership, authorization, persistence, networking,
cryptography, concurrency, and untrusted-input changes also require a design review before or
during implementation. The reviewer is read-only unless explicitly authorized to edit and must
inspect the actual current worktree and surrounding enforcement paths rather than relying on the
implementer's summary.

The review request must include:

- the concrete objective and user-visible outcome;
- changed files and the functions, types, or sections that matter;
- neighboring code that consumes, authenticates, persists, retries, expires, displays, or removes
  the affected state;
- the security, compatibility, ownership, boundedness, timing, and truthfulness invariants;
- the threat and failure model, including malicious peers, malformed input, crashes, restarts,
  reordered events, stale state, churn, and races;
- tests already added and verification already completed; and
- intentionally unsupported cases and honest limitations.

Findings must be prioritized as `BLOCKER`, `HIGH`, `MEDIUM`, or `LOW`. Each finding should include
exact file and line references, a concrete failure or attack sequence, why existing checks and
tests miss it, the smallest safe correction, and the regression that should pin the behavior.
Confirmed defects must be distinguished from design questions.

The primary implementation agent owns the complete test suites. To avoid duplicate work and Cargo
target-directory contention, the reviewer normally performs static inspection and focused checks
only unless the primary agent explicitly delegates a full suite. Every blocker and high-severity
finding must be resolved before handoff. Material corrections receive a short re-review of the
changed boundary.

## Prompt template

Replace every bracketed field before sending this prompt to the reviewer:

```text
Act as a read-only adversarial reviewer for Mewtual, a security-sensitive peer-to-peer desktop app.
Do not edit files. Inspect the actual current worktree and `git diff`; do not rely on this summary.
Read the repository instructions available in the checkout plus the relevant sections of
docs/ARCHITECTURE.md, docs/INTERFACES.md, docs/THREAT-MODEL.md, docs/HANDOVER.md, and
docs/ADVERSARIAL-REVIEW.md first.

Objective:
[What was built or is proposed, and the user-visible outcome.]

Primary files:
[Changed files with the functions/types/sections that matter.]

Neighboring enforcement and lifecycle paths:
[Files/functions that consume, authenticate, persist, retry, expire, display, or remove this state.]

Invariants that must hold:
[Security, compatibility, ownership, boundedness, timing, and truthfulness properties.]

Threat/failure model:
[Malicious peers/input, crashes, restarts, reordered events, stale state, network churn, races, etc.]

Tests and verification already completed:
[Focused regressions and commands. Do not duplicate full suites unless asked.]

Intentionally out of scope / honest limitations:
[Boundaries the change does not claim to solve.]

Review the implementation and design for:
1. authorization/authentication bypasses and confused-deputy behavior;
2. replay, downgrade, cross-group/cross-device binding, and wire/persistence compatibility;
3. races, cancellation, startup/shutdown ordering, stale success, expiry, replacement, and restart;
4. unbounded memory/work, amplification, scanning/SSRF, Sybil/rate-limit bypass, and privacy leaks;
5. contradictions between backend state, UI/diagnostics, docs, and actual present-time evidence;
6. failure paths that strand durable state or make recovery impossible;
7. missing unit, integration, adversarial, and compatibility regressions.

Return findings first, ordered BLOCKER/HIGH/MEDIUM/LOW. For every finding include exact file and
line references, the concrete failure/attack sequence, why current checks/tests miss it, the
smallest safe fix, and the regression test that should pin it. Distinguish confirmed defects from
design questions. If no blocker remains, say so explicitly and list residual risks/test gaps
separately.
```
