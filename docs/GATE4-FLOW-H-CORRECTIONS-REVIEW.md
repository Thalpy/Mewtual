# Gate 4: Flow H scheduled-runtime corrections

Status: **REQUEST CHANGES received on `da7b8fa`; corrections implemented at `d711485` and
awaiting re-review.** Send the round-3 request at the end of this document. The compare URL
resolves only once `gate4-agent1-runtime` is pushed; the branch is ahead of `origin`. Later
documentation-only commits do not change the reviewed code SHA, which is `d711485`.

The reviewer confirmed NEW-1, NEW-2's original bug, NEW-4, NEW-5, NEW-10 and both new H5 guards as
genuine, and accepted M34 and M35 as strong isolated oracles. It then found **two further
capacity-stranding P1s and a load-bearing check the accepted design specifies**, which is the same
pattern the round before had shown: a correction introducing the defect class it was closing.

| Finding | Verified against source | Disposition |
|---|---|---|
| **FLOWH-001** P1: backoff neither enforced under load nor self-waking at idle | confirmed: `background_step` gated H5 on stage alone, and `pending` uses `runnable`, which is false while held | fixed, M36 + M37 |
| **FLOWH-002** P1: a worker completing after a pause re-strands the permit | confirmed: `complete` has no `paused` check and no release after | fixed, M38 |
| **FLOWH-003** P2: H3 performs no per-visit wrapper reauthentication | confirmed: design 6.1 line 549 specifies it and §13 names **M4** as its mutation; `handoff_sign` had no store access at all | fixed, M39 |
| **FLOWH-004** P2: H1 reads intent bodies before admission | confirmed: design §7.2 is *titled* "Reservation precedes every body read" | fixed, M40 |
| **FLOWH-TEST-001** P2: the H3 time bound has no discriminator | **refuted**, see below | no change |

FLOWH-TEST-001 does not hold. Case 3 of `studio_handoff_signs_in_bounded_slices` disables the
count limiter with `usize::MAX` and drives a 200 ms-per-read `SteppingClock` against the 250 ms
budget, asserting `signed() == 2` with `remaining() > 0`; case 2 disables the time limiter so only
the turn cap can stop it. **M5b** is recorded in the status document and fails at a *different*
assertion, "the slice budget did not bound the slice", 8 signed where 2 was required. Both landed
in `c676749`, outside the reviewed range — a diff-scoped read would not see them. The reviewer's
*derived* conclusion, that the open-items list was incomplete again, stands regardless: FLOWH-001
through -004 were all absent from it.

Two fixes went beyond the literal finding, and the reasoning should be checked rather than taken:

- **FLOWH-004's reservation is placed after an in-memory eligibility test**, not at the very top of
  the probe. Reserving unconditionally would take a process-wide permit on every background turn
  of a quiescent vault and could transiently refuse a concurrent Save — the opposite of what 7.2's
  memo exists for. The test uses only `intent_generation`, `is_quiet` and `held`, none of which
  read a body. If that reasoning is wrong, the fix is wrong.
- **A failed reservation now records no backoff.** That also closes the previously-disclosed item
  about capacity contention escalating like genuine ineligibility, but it is a behaviour change
  the finding did not ask for.

The compare URL below resolves only once `gate4-agent1-runtime` is pushed.

**The reviewed head was `da7b8fa`, which is not the branch tip.** Later commits on
`gate4-agent1-runtime` that only touch this document are documentation-only and carry no reviewed
code; the common contract asks reviewers to make exactly this distinction, and this round creates
the case.

## Background: why this thread exists

This round exists because the previous adversarial
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

Full `catcoms-app` lib suite at the head commit: 656 passed, 0 failed, 11 ignored, 1307 s.
`cargo fmt` and `cargo clippy --all-targets -D warnings` clean. Run locally at `-j 1` with
`--test-threads=4`; no CI run has been made against this head. The eleven ignored are the
pre-existing opt-in profiles, unchanged by this round.

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

### Round 3 request: FLOWH-001 to FLOWH-004

```text
Review type: finding re-review of FLOWH-001..FLOWH-004.
Base: da7b8fa8993547268c403d78158fcfb02e9ca6b6.
Head: d711485d0e366d3a2139ec9290f29c4664e5e412.
Compare: https://github.com/Thalpy/Mewtual/compare/da7b8fa8993547268c403d78158fcfb02e9ca6b6...d711485d0e366d3a2139ec9290f29c4664e5e412
Evidence: 660 passed, 0 failed, 11 ignored at this head; cargo fmt and clippy --all-targets
-D warnings clean. Local only, run at -j 1 with --test-threads=4; no CI run against this head.
One earlier run failed studio_exchange scheduling's cancelled-preview-transport deadline test,
which is recorded as contention-sensitive; it passes in 6.6 s alone and a clean re-run with no
competing Cargo process passed 660 with none. That was worth confirming rather than assuming,
because FLOWH-004 makes the H1 probe hold a process-wide permit and those tests use the real
four-slot pool, so genuine starvation there would look exactly like a flake.
Scope/evidence: docs/GATE4-FLOW-H-CORRECTIONS-REVIEW.md, docs/GATE4-AGENT-1-STATUS.md,
docs/GATE4-AGENT-1-DESIGN.md sections 6.1, 7.1, 7.2, 7.3 and 13.
Dependencies: your REQUEST CHANGES verdict on da7b8fa, and the corrections it confirmed as
genuine there (NEW-1, NEW-2's original bug, NEW-4, NEW-5, NEW-10, M34, M35).

This is the third consecutive round in which a correction introduced the defect class it was
closing. FLOWH-001 was created by NEW-8's own fix; FLOWH-002 sits on the other side of the
Detached transition NEW-9 handled. Assume the same has happened again and look for it before
anything else. Four specific candidates, in the shape the last three rounds took:

Backoff now gates execution through can_sign/can_commit and publishes a deadline through
wake_in, and pending still uses runnable. That is three predicates over one piece of state.
Establish whether they can disagree: a stage that is due but publishes no deadline, a deadline
that outlives the job, a job whose target changes, or progressed() clearing next_at while a
gate has already read it. The previous two P1s were both predicates disagreeing about one fact.

wake_in deliberately covers only a live job, on the reasoning that a job is what holds a
resource and an abandoned target holds nothing. Break that: find a state where something
scarce is held with no live job, or where the actor needs to wake and no other term will
wake it. The actor merges this into the delivery-throttle sleep; confirm a Studio-only
deadline actually causes a loop iteration that re-signals studio_pending, and that firing
recompute_due_delivery with an empty dirty set is genuinely inert.

handoff_complete now calls release_if_stalled when paused. Confirm this covers every arm that
can install ownership, that it cannot fire while a worker still owns a bundle, and that the
abandon it performs records backoff for the right target. Check whether an unpaused completion
can reach a stage that is equally unreachable for some other reason.

H3's new reauthentication runs once per visit, before the first sign_next. Establish that it
is genuinely before the first signature and not merely before the loop; that a read failure
answering "not current" cannot be turned into a denial of service by an unrelated transient;
and that abandoning on mismatch is right rather than parking. Design section 9.1 says a stamp
mismatch discards the candidate and never falls back - check the implementation agrees.

FLOWH-004's reservation is NOT at the top of the probe. It sits after an in-memory eligibility
test over intent_generation, is_quiet and held, because reserving unconditionally would take a
process-wide permit on every background turn of a quiescent vault and could transiently refuse
a concurrent Save. Verify that test reads no body, that no path reaches load_epoch_intents_
structural without the permit, and that a rail where every target is quiet or held still
cannot read anything. If the eligibility test can be wrong about a target, the fix is wrong.

Also re-derive, since a failed reservation now records no backoff at all: prove no target can
be starved by a permanently contended pool, and that removing the hold has not reintroduced a
hot loop somewhere the previous rounds closed one.

I dispute FLOWH-TEST-001 and have set out the citation above. If you still believe the H3 time
bound is unguarded after reading case 3 of studio_handoff_signs_in_bounded_slices and M5b in
the status document, say specifically what those do not establish.

Do not accept the open-items list as complete. It has been incomplete in all three rounds, by
five items, then by four. Report what it omits, and say plainly if the pattern persists.

Return a verdict for these corrections only. Flow R, I-4, C-3, the section 13 measurements and
native exposure are out of scope and not claimed.
```

### Round 2 request, as sent (verdict: REQUEST CHANGES)

```text
Review type: finding re-review, plus bounded implementation review of the new work.
Base: 808ef2fc6cd171d1fe4c4cfbf0c7e498db0a47c4.
Head: da7b8fa8993547268c403d78158fcfb02e9ca6b6.
Compare: https://github.com/Thalpy/Mewtual/compare/808ef2fc6cd171d1fe4c4cfbf0c7e498db0a47c4...da7b8fa8993547268c403d78158fcfb02e9ca6b6
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
