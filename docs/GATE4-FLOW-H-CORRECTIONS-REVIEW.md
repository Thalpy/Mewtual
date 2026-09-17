# Gate 4: Flow H scheduled-runtime corrections

Status: **awaiting adversarial review.** This round exists because the previous adversarial
review of `808ef2f` returned **REQUEST CHANGES**, and because a material part of what it found
was that the implementer's own account of the preceding round was wrong. That is the context the
next reviewer should carry in, and it is the reason this request asks for the corrections to be
re-derived rather than confirmed.

## What the previous review found

It checked ten claimed fixes and reported: **six genuine, three partial, and one — the second P1 —
not covering its own named trigger.** It found two further P1s, both of which were the same defect
class the round had just claimed to close, reintroduced by the fixes themselves. It also judged the
status document's open-items list "honest but materially incomplete", naming five omissions.

Blocking set: NEW-1, NEW-2, NEW-3, NEW-4, with NEW-5 to land alongside NEW-2.

The two P1s are worth restating because they define what this review should be suspicious of:

- **NEW-1**: the job-abandonment check compared owner tenure alone, on the stated belief that
  `observed_owner_tenure_start` returns `None` whenever the epoch or owner moved. It does not. It
  reports when the current owner's tenure *began*, and `OwnerTenure::applied` preserves that across
  a same-owner commit (`crates/catcoms-sync/src/owner_tenure.rs:52-68`). The commonest way a job
  dies — a member joining or leaving — left it permanently unsignable while the check saw a live
  tenure. The status document asserted the opposite as fact.
- **NEW-2**: `HandoffRuntime::hold` read `self.job`, and three of its four call sites had already
  cleared the job. The preceding round had *found* this bug class, fixed one instance, and written
  it up as closed. Every H5 refusal therefore recorded no backoff at all.

## What changed since `808ef2f`

Two commits.

| | |
|---|---|
| NEW-1 | `HandoffJob` records the MLS epoch as well as the tenure; `handoff_check_authority` compares both |
| NEW-2 | `HandoffRuntime::hold` deleted entirely; only `hold_target(target, now)` remains |
| NEW-3 | `handoff_commit` builds the budget before taking the job, so a retryable failure no longer discards H1–H4 |
| NEW-4 | the probe backs off the target that actually failed, not the rail cursor |
| NEW-5 | all three `HandoffCompletion` arms are gated |
| NEW-8 | `pending()` driven by `runnable(now)` rather than `busy()` |
| NEW-9 | `release_if_stalled` abandons a non-detached job when the receiver pauses |
| NEW-10 | completions routed by a never-reused `HandoffJob::token`, not by target |

Plus two tests for H5 guards that previously had none, and a rewritten open-items list.

## Evidence, offered as pointers rather than proof

Full `catcoms-app` lib suite, `cargo fmt` and `cargo clippy --all-targets -D warnings` clean.

Four mutations were executed against the new guards. Each failed at a named assertion and each
mutated file was confirmed byte-identical to `HEAD` afterwards:

| Mutation | Effect |
|---|---|
| M32: compare tenure only, dropping the MLS conjunct | fails **only** the MLS test; the owner-change test still passes, reproducing NEW-1's blind spot exactly and only where the review said it was |
| M33: route completions on target instead of token | the live job is cleared by a superseded worker (`left: None, right: Some(2)`) |
| M34: delete the H5 index-reference call | the commit **succeeds**, durably writing an Index entry (`epoch: 1, accepted: 1`) pointing at a source that no longer exists |
| M35: drop `tenure != Some(stamp.tenure)` from `studio_handoff_is_current` | a batch signed under one tenure commits under the next |

M34 is the one to weigh: before this round, deleting that H5 call changed no test at all.

## Known open, disclosed rather than discovered

The previous reviewer had to tell the implementer that a three-item list should have had eight.
This list is offered in that spirit — it is what is known to be open, and it is **not** offered as
a guarantee of completeness:

1. H1 and H5 each drain a full epoch-storage inventory under custody, which design 6.1 does not put
   in H1 and which C-3's resumable cursor is meant to bound.
2. The H5 index check adds its own unbounded under-custody read loop, one `load_studio_epoch` per
   `PutObject`. Belongs with item 1.
3. `handoff_priority` omits `studio_has_page_request`, which `run` itself treats as authoritative.
4. A detached Flow H waiter inherits an unrelated request's cancellation, discarding multi-turn
   signing progress.
5. Design 7.3's `explicit_retry` relief is not wired to the handoff maps, though the code comment
   cites 7.3's pacing as satisfied.
6. `next_at`, `hold_ms` and `quiet` are never pruned against the current watch rail.
7. `next_token` uses `saturating_add`, so at `u64::MAX` tokens would repeat. Not reachable in any
   real deployment; recorded because the last round's omissions were also individually defensible.

Coverage still absent, by name: both `replay_ready()` gates; `handoff_complete`'s mis-targeted
`Prepared`/`Assembled` arms (only `Cancelled` is covered); `handoff_sign`'s `remaining() == Some(0)`
early return; `background_step`'s "a yield consumes no turn" rule, where the existing test calls
`handoff_sign` directly and bypasses the scheduler.

Flow R (R1/R2/R3), invariant I-4, change C-3 and design §13's eight measurements are not started.
Native exposure remains gated on Agent 2's P5, still false.

## The review request

```text
Review type: finding re-review, plus bounded implementation review of the new work.
Base: 808ef2fc6cd171d1fe4c4cfbf0c7e498db0a47c4.
Head: [FULL_HEAD_SHA]. Compare: [IMMUTABLE_COMPARE_URL].
Scope/evidence: docs/GATE4-FLOW-H-CORRECTIONS-REVIEW.md, docs/GATE4-AGENT-1-STATUS.md,
docs/GATE4-AGENT-1-DESIGN.md sections 6.1, 7.1, 7.2 and 7.3.
Dependencies: the prior REQUEST CHANGES verdict on 808ef2f and its blocking set
NEW-1..NEW-5; the core handoff-signing PASS; the A-001/B-001 and RT-001/RT-002 PASSes.

Read the common contract in docs/GATE4-REVIEW-PREAMBLES.md and Review 1's scope. This is a
re-review of corrections whose predecessor round was independently found to have misreported
itself: of ten claimed fixes, six were genuine, three partial and one did not cover its own
named trigger. Treat the implementer's fix table, mutation table and open-items list as claims
to be re-derived, not as evidence. The single most valuable finding last time was that a P1
was reported closed while its main trigger was still open; look for that shape again first.

Establish independently what facts a StudioHandoffAuthority actually binds, and whether tenure
and MLS epoch are together sufficient to decide that a captured job can still be signed and
committed. Device key rotation, actor or sync incarnation, registry mount identity, designated
committer change without an epoch advance, and an interrupted Prepared record from a previous
attempt are the candidates to try. If any fact can move without moving either of the two now
compared, the abandonment check is still blind in the same way it was blind before.

Attack HandoffJob::token. It is minted only in handoff_probe and compared only in
handoff_complete. Confirm no path mints a job without a fresh token, no path compares a
completion by any other key, and a token cannot recur across the actor's lifetime including
across a receiver rebuild or a store reopen. Establish whether a Flow S OverlayContext, which
carries token 0, can ever reach a handoff completion arm.

Re-derive the backoff invariant rather than reading it. The claim is that every path which
gives up on a job records a per-target hold, and that no path clears a job without one. Prove
or break it by enumerating every assignment to HandoffRuntime::job and every early return in
handoff_probe, handoff_sign, handoff_commit, handoff_complete, release_if_stalled and
handoff_check_authority. The previous round's second P1 was exactly a missed enumeration here.

Challenge release_if_stalled specifically. It abandons any non-detached job when the receiver
is paused. Show whether it can discard a Ready job whose H5 would have succeeded, whether a
pause that resolves within one turn causes avoidable loss of a fully signed batch, and whether
Detached being exempt can strand admission when its worker never returns.

Challenge pending() now being driven by runnable(). A job held by backoff or detached reports
not runnable. Demonstrate whether any state exists in which the job can make progress but no
other pending term keeps the driver awake, so the job stalls until unrelated work arrives.

For the two new H5 tests, verify that each crafted input actually reaches the intended guard and
that no different failure could mask the guard's removal. The index test asserts H1 succeeded
before the reference is removed; check that this is what makes it specific to the H5 call, and
that removing the file is a faithful stand-in for eviction, retirement or cleanup rather than a
shape the production paths cannot produce. The tenure test supplies a different tenure at H5
than at H1; check that this is what a real superseded tenure presents to that comparison.

Do not accept the disclosed open-items list as complete. It was incomplete by five items last
round. Report anything it omits, and say so explicitly if the omission pattern persists.

Return a verdict for this correction round and the new work only. Flow R, I-4, C-3, the design
section 13 measurements and native exposure are out of scope and not claimed.
```

## Where to look first

`crates/catcoms-app/src/studio/receiver/handoff.rs` is the whole scheduled runtime and carries
every correction in this round. `crates/catcoms-app/src/studio/receiver/catchup.rs` carries the
token threading and the completion enum. `crates/catcoms-app/src/store/epoch_studio/handoff.rs`
carries the two H5 guards, both call sites of `check_index_object_sources`, and
`crates/catcoms-app/src/store/epoch_studio/handoff_capture.rs` carries `studio_handoff_is_current`.
