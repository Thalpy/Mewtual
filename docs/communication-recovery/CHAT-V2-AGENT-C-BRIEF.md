# Agent C: joined-member text sources

Primary-coordinator brief, 12 November. Work on a separate branch/worktree based
on `e0a807e2a55e6aa6aa0f08a7898798b7b1dd2490` from
`origin/chat-v2-member-outbox-production`. Preserve the primary's and B's changes.
The primary owns native integration and broader validation; C owns this source
vertical. Namespace5's focused acceptance is complete, but the overall PR is not
fully green and Chat V2 remains disabled.

After C's source survey, the primary recorded explicit
[coordination decisions](CHAT-V2-JOINED-SOURCE-COORDINATION.md). Apply them to this
brief: production invite/pin, founder governance/archive and transport are
primary-owned prerequisites; a warm real-MLS/Core source milestone is partial
evidence. In particular, do not use a phase-1 epoch snapshot as final authority or
discard removed A1 originals to unblock A2.

## Required outcome

A genuinely joined ordinary member can author and durably save its own Create,
reply, Edit and Delete while the founder is offline. Use B's actual member outbox;
do not require a founder certification or founder Append for each message.

The existing signer vertical supports only the founder and the empty generation-1
promotion. It must keep refusing later prefixes until a genuine source exists.
The Sync seams already exist on `ChannelSync`: `ordinary_member_signer_data`,
`draw_ordinary_draft_inputs` and `sign_ordinary_admitted`.

Implement the actual joined-member owner/source using the original live
`ChannelSync`, its independently trusted V2 pin and a replayed governing prefix.
Preserve whole ownership; do not clone an actor SDK or manufacture authority from
decoded IDs, a caller-selected pin/image, a boolean, a historical Admission or a
generic callback. Governance4 currently provides metadata, not this operational
owner. C implements the joined source factory; the primary owns the founder's
production governance4 owner. Neither may treat the metadata wrapper as one.

## Boundaries and coordination

Read AGENTS.md and the relevant architecture, interfaces, threat model, handover
and MESSAGE-FLOW sections first. Also read:

- `CHAT-V2-ORDINARY-MEMBER-SIGNER.md`
- `CHAT-V2-GENERAL-OPEN-ISSUER.md`
- `CHAT-V2-MEMBER-OUTBOX.md`
- `CHAT-V2-MEMBER-SAVE-HANDOFF.md`
- `CHAT-V2-NATIVE-MEMBER-SAVE-NEXT.md`

Write the source/factory interface and ownership proposal to
`scratchpad/agent-c/interface-proposal.md`, including real producers, consumers,
current-authority proofs and owning refusals. Obtain adversarial design review.
Coordinate shared `operational/owner.rs` and `host/member.rs` changes with the
primary; it owns the native key-admission cut and native lifecycle work. A separate
Agent A session is not required. Continue independent source/replay work while
shared integration details are agreed.

Keep joined-source implementation in its own modules where practical. Do not edit
native Tauri/browser code, voice/media, namespace5 recovery, B's outbox engine or
unrelated UI. Any necessary B-boundary change needs an explicit narrow proposal.
Do not introduce unused forwarding APIs as the delivered outcome.

## Acceptance

Exercise real App/Sync ownership and the production source/save paths, not only
Core fixtures or mocked authorization. Cover:

- a genuine join, a nonempty governing prefix and founder-offline ordinary saves;
- exact retries retaining the same signature/original, including uncertain saves;
- current membership/source loss before signing and before save admission;
- Remove-versus-sign ordering and removal/rejoin A1/A2: preserve historical
  receipts, but do not revive A1 authority or publication debt as A2;
- stale/foreign pin, group, device, source and prefix rejection;
- epoch changes, delayed/duplicate governing updates and source replacement;
- agreement across multiple genuine members and adversarial delivery order.

Do not use empty placeholder channel ledgers as evidence of integrated projection
or target lookup. Identify that dependency explicitly and agree its real consumer
where needed. Keep the additive ledger and existing removed-member approval policy
unchanged; unresolved authority must not become new signing permission.

Clock/RNG must use the repository seams. No local Cargo commands: use CI and
coordinate Rust execution with the primary. Run relevant focused checks during
implementation; the primary owns broader gates. Keep the named slow
10,000-operation catch-up case deferred until final acceptance.

Obtain a read-only adversarial diff review, fix every blocker/high and account for
medium/low findings, then push reviewable commits and update the handover. Report
implemented behavior, actual executed results and remaining limitations separately.
Do not merge or activate Chat V2.
