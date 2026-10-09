# Gate 4 Agent 3 status: runtime signed fault repair

Owner: Agent 3 ([assignment](GATE4-AGENT-HANDOFFS.md#agent-3-runtime-signed-fault-repair)).
Proposal: [GATE4-AGENT-3-DESIGN](GATE4-AGENT-3-DESIGN.md), revision 16 follow-up.
Review preamble: 3. Current entries override older ones.

## Re-review of the fairness fixes: dispositions, 2026-10-09

The short re-review of the entry below found no BLOCKER and no HIGH, and confirmed each claimed
fix against the code. It raised two new MEDIUM findings and five LOW. A second short re-review of
those fixes found no BLOCKER or HIGH and one more MEDIUM, in the LOW-1 fix; it agreed the peer
refetch (MEDIUM-2) is a tracked follow-up, not a blocker. Its residual: an exact-retry Acknowledge
succeeds again, so whatever can send Acknowledge can clear a document's backoff repeatedly. That
is no new authority, since an explicit decision already bypasses the backoff.

| Finding | Disposition |
|---|---|
| MEDIUM-1: after the person resolves a hold, the resume could wait up to 15 min (the doubled deferral; 60 s before). | **Fixed.** A successful Acknowledge of the document's recovery warning clears its deferral and hold, and its bucket's (`repair_user_resolved`). An ordinary Read deliberately does not: the renderer reads again after every refresh notice, so a repair outcome would clear its own backoff. Test: `acknowledging_a_documents_warning_ends_its_repair_backoff` (streak 3 at 480 s, Acknowledge through the receiver's control path, then the next 5 s visit fetches). A storage refusal has no such action and still waits out its window. |
| MEDIUM-2: THREAT-MODEL and the entry below claimed pacing beyond the owner's resume. A repaired-seed fetch started from an answer carrying a repair, from the router or after a job is paced only by the checkpoint slot and a hold; a peer owing a seed nobody serves refetches on every reporting discovery. | **Wording fixed; follow-up open.** Both now scope the pacing to the owner's resume visits and name the peer refetch as pre-existing and unpaced. The real fix, a per-target doubling refetch deferral in `await_repaired_seed` set when a repaired pass fails, touches the peer seed path every Flow D liveness test depends on, so it is left as a follow-up rather than added late in this round. |
| LOW-1: landed-install recovery still needed some peer to serve the seed. | **Fixed.** A cold B3 visit to a target that has been deferred before resumes instead of fetching again; the job classifies exactly at S3. A seed still owed is then fetched from the job's finish. New case `cold, seed never served` in the crash test. A second re-review found that this fallback could rerun a whole job on every visit (next row). |
| MEDIUM (second re-review): the cold fallback scheduled a Resume without deferring the visit. On a busy owner the job's finish often cannot start its fetch (another pass out, a discovery pending), and ordinary catch-up evicts the source again before the next visit, so every visit reran capture, rebuild and a B2 flush, holding the single job slot. | **Fixed.** A scheduled resume now defers the target's next visit like a started fetch does, so the owner runs at most one resume job per doubling window per target. Test: `a_cold_owed_source_whose_fetch_cannot_start_reruns_its_job_ever_more_rarely` (an owed cold source, evicted before every visit, each finish unable to fetch: three jobs in ten minutes, at 60, 180 and 420 s). |
| LOW-2: `resume_landed_install` skipped the durable-snapshot gate. | **Fixed.** Test: `the_router_resumes_a_landed_install_only_under_a_current_snapshot`. |
| LOW-3: a deferred held bucket's Registry turn still prepared its Studio target and bucket. | **Fixed.** `work_registry` asks `registry_decision_waiting` before preparing and spends the turn. Test: `a_backing_off_held_bucket_spends_its_turn_without_preparing_anything`. |
| LOW-4: Registry terminal reset unpinned. | **Fixed.** Asserted in `an_owed_bucket_replacement_is_fetched_then_installed_by_a_job`, with its own mutant. |
| LOW-5: "never runs under the decision" omitted the stale-snapshot and unreadable-record cases. | **Wording fixed** in the code comment and the entry below. |

**Harness.** Six new runtime mutants (26 in all): `RUNTIME-acknowledge-ends-backoff`,
`RUNTIME-cold-b3-resumes-after-a-deferral`, `RUNTIME-landed-install-needs-snapshot`,
`RUNTIME-held-bucket-turn-prepares-nothing`, `RUNTIME-registry-terminal-ends-backoff` and
`RUNTIME-scheduled-resume-defers`. `RUNTIME-router-resumes-landed-install` now matches the crash
test's renamed case. Subset runs of these seven detected each at its intended assertion and
passed each restored control. The scheduled-resume deferral applies to the Registry resume too.

## Review of the fairness round: dispositions, 2026-10-09

The adversarial review of the entry below found no BLOCKER and no HIGH: two MEDIUM, eight LOW.
Every fix has a regression, and each regression was broken to confirm it fails (as a runtime
harness mutant, below). The entry below is corrected in place where the review found it
overclaimed.

| Finding | Disposition |
|---|---|
| MEDIUM-1: per-target deferral removed the only cap on total automatic work. K held targets owing seeds no peer serves started K fetches a minute, each holding the single checkpoint slot ordinary catch-up waits on; K targets needing the user reran K whole jobs a minute. | **Fixed.** A target's visit deferral now doubles each time it is deferred again after the last one ran out: 60, 120, 240, 480 s, then a 15 min cap. Every persistent hold (`hold_repair`: a recovery warning, a storage refusal, a failed capture or rebuild) now also defers the visit, so job reruns back off too. A deferral that arrives while one is running extends it but never doubles it. A terminal outcome or a new explicit decision clears it; one quiet for a whole cap after it ran out is dropped, which bounds the map. Tests: `a_seed_that_never_arrives_is_refetched_ever_more_rarely_until_the_repair_ends` (six fetch starts in 40 minutes, not 41, then the install clears the streak) and `repeated_holds_back_a_target_off_further_and_a_new_decision_starts_it_afresh`. A shared floor between owner starts, the review's alternative, was tried first and dropped: it couples every target's first start to whichever target started last, the cross-target dependency the fairness fix had just removed. |
| MEDIUM-2: Studio cold crash recovery relied on the single warm cache surviving until the target's next visit; the test assumed it did. | **Fixed.** In the router's deferred branch, an owner whose prepared source owes nothing while its record holds the decision with B3 set resumes at once (`resume_landed_install`); the resume answers `AlreadyRepaired` and recycles. The cold test now runs the real router on the pass (unfetched: a sealed pass reaches the router only once fetched, but the deferred branch reads no seed bytes), then evicts the source to show the resume no longer needs it. That recovery still needs some peer to serve the seed; the re-review entry above removes that dependency. |
| LOW-1: stale `finish_registry_repair` doc. | **Fixed.** |
| LOW-2: `finish_studio_repair` deferred even when no fetch started. | **Fixed.** It defers only when this target's own fetch started or it is backing off; the Registry `AwaitingSeed` finish now does the same. |
| LOW-3: Registry Flow D's route overstated. | **Wording fixed** here and in the test's doc: the repair arrives through the bucket leg of either query. |
| LOW-4: an S3 that writes, with a page pending, could pause catch-up (suspected). | **Confirmed and fixed** (row in the table below). |
| LOW-5: held-replay test comment contradicted the code. | **Fixed.** It is a different repair with the same sequence number, after the seed fetch was dropped. |
| LOW-6: unrelated-actor test thin on A's own progress; pool limit unstated. | **Documented** (table below). |
| LOW-7: "Interrupted B2/B3" row pointed at post-B6 crash tests. | **Reworded** (table below). |
| LOW-8: Registry held-read errors dropped silently. | **Fixed.** Reported and deferred like Studio's. |

**One more behaviour change.** While a bucket's visit is deferred, a held decision still owns its
Registry turn: the turn reads the owner record and answers "owned", so ordinary bucket work does
not run under the decision. That costs one bounded owner-record read per deferred turn. Before,
any deferral (the old shared cadence included) let the ordinary turn run. The exceptions: with no
current owner snapshot, or an unreadable record, the ordinary turn runs as it always did.

**Harness.** Six new runtime mutants, and `RUNTIME-per-target-visit` re-anchored (20 in all):
`RUNTIME-visit-deferral-doubles`, `RUNTIME-terminal-ends-backoff`,
`RUNTIME-decision-starts-afresh`, `RUNTIME-deferred-bucket-turn-owned`,
`RUNTIME-router-resumes-landed-install` and `RUNTIME-s3-drops-stale-page`. That run exposed a
harness defect: a test name containing `::` skips the core harness's prefix, so the two two-peer
mutants (`RUNTIME-held-offer-holds-repair-only`, added in the entry below and never run before,
and `RUNTIME-s3-drops-stale-page`) ran zero tests and were rejected. They now use a `TWO_PEER`
prefix; the other two harnesses already name full paths. A subset run of these eight then
detected each at its intended assertion and passed each restored control.

**Residuals.**
- The pacing covers the owner's resume visits only (re-review MEDIUM-2): see the entry above.
- Not a global cap: K newly held targets still start K fetches in their first minute; the
  owner's resume work for them then decays to about K per 15 minutes.
- `repair_visits` lives in memory, so a restart starts every target afresh.
- A cancelled or authority-abandoned job holds nothing and may rerun on the next visit.
- The round-robin index can skip or repeat a target when the watch list is reordered.

## Real-peer Registry Flow D, plan D remainder, fairness fix, 2026-10-09

The entry below claimed to answer plan D's remaining items; it did not. These close them.

**Plan D, item by item.** Every new test was broken to confirm it fails.

| Plan D item | Evidence |
|---|---|
| Full-pool deferral with no mutation | `a_repair_job_reserves_a_slot_before_reading_and_waits_flat_when_the_pool_is_full` (earlier). |
| Cancellation and result holding | `a_cancelled_waiter_never_releases_the_slot_or_claim_its_worker_still_owns` (earlier). |
| Another actor progressing while a large S2 is paused | **New:** `an_unrelated_actor_progresses_while_a_repair_rebuild_is_paused`. Two servers share a four-slot pool. A's detached rebuild never finishes, yet B's whole job gets a slot, commits and releases it. A's worker keeps exactly one slot and its claim until it ends, then A commits. Over-reserving the pool fails it. Limits: A's own turn runs with no watches, so A's own catch-up progress while paused is not shown; and four paused S2 workers, from any actors, exhaust the process pool, after which every new job and preparation answers `Full` until one ends. |
| S3 within bounded turns: Selected pass carrying a repair | HIGH-1 liveness test (earlier), and Carol's actor run in the two-peer tests. |
| S3 within bounded turns: `PageReady` pass on the claimed target | **New:** `a_pending_page_on_the_claimed_target_never_delays_s3`. Bob's page reaches Carol through the real fetch adapters while her job is parked; one turn commits S3, then persists the page. Restoring the `replay_ready` gate on S3 fails it. That S3 writes nothing (the offer is held as unverifiable). An S3 that writes is the next row. |
| An S3 that writes, with a page pending on its target | **New, with a fix** (review LOW-4 on the fairness round): `a_repair_that_retargets_a_source_with_a_pending_page_never_pauses_catch_up`. Carol sits healthy on R1's successor with Bob's page `PageReady`; Bob's repair choosing R2 retargets her source. S3 returns before the page step, so the next turn saved the page against the retargeted source, was refused with the pass already `Paused`, and paused all catch-up. `finish_studio_repair` now drops the target's pending page. |
| S3 within bounded turns: faulted target under reporting discovery | The two-peer Studio and Registry actor runs. |
| Stale rebuild refused with no write | Studio and Registry stale-rebuild tests (earlier). |
| Fairness across held targets | **New, with a fix** (below): `a_held_target_never_delays_another_targets_resume` and `a_held_bucket_never_delays_another_buckets_resume`. |
| Interrupted repair, recovered through the job | **New:** `an_owner_that_crashed_between_install_and_recycle_resumes_a_studio_source`, warm and cold, alongside the Registry one (earlier). These crash after the replacement install, before the owner record's recycle. No runtime test crashes between B1 and B2 or between B2 and B3: those interruptions are covered at store level, where S3 runs the same transactions unchanged, and the runtime's part is that the owner resume schedules a job for any held decision. The cold case now goes through the real router (review MEDIUM-2 on the fairness round). Classifying from B3 alone fails it, and so does removing the router's own resume. |
| Two-peer Fault, decision, replacement, restart and newcomer through the actor | The two-peer Studio run (earlier). |

**Registry Flow D on a real peer**
(`a_peer_applies_a_bucket_repair_from_a_real_answer_and_installs_its_replacement`, through spawned
actors):
- Bob installed and published his bucket's R1, then met a rival R2; Carol holds both receipts and
  no seed.
- Bob's bucket decision is a job.
- Bob's real Registry discovery answer carries the repair to Carol. It comes through the bucket
  leg of a query: either her fault-reporting Registry turn or an ordinary Studio discovery, which
  always asks for the bucket first, and the head answer carries an applied repair whether or not
  a report was sent. The test pins that the repair comes from a real answer, not which leg.
- Her job applies it, and she fetches R1's bucket seed from Bob through a repaired pass. The
  router hands the seed to a Replace job, which installs it.
- Both buckets end on R1's successor.
- Refusing the offer fails the test, and so does blocking the router's bucket Replace step.

**MEDIUM-1, held half, on a real peer**
(`a_held_replay_on_a_real_peer_holds_only_itself_and_the_owed_seed_is_still_fetched`):
- Carol, with an observed tenure, applies Bob's repair and owes its replacement.
- A second current-tenure repair of the same pair with the same sequence number, choosing the
  other receipt, arrives. The transaction holds it with `SequenceNotNewer`. (A byte-identical
  replay never reaches a job: the owed filter sends it to the seed fetch.)
- Only the replay is held. Carol's seed fetch still mints, and the replay costs no further job.
- Charging the hold to the document fails the test.

**Fairness fix (runtime change).** Both owner-resume round-robins had one shared cadence. Any
held target bumped it to 60 s: a hold, a failed read, a started seed fetch, an `AwaitingSeed`
finish. So N held targets could hold a healthy one off for N minutes; reviews had flagged this
twice as a LOW residual.
- Each target, Studio target or Registry bucket alike, now carries its own visit deferral
  (`repair_visits`), bounded by expiry. It doubles while it repeats (see the entry above).
- The round-robin keeps its 5 s cadence. A deferral blocks nothing but that target's visit:
  offered repairs and fetched seeds still run.
- A started seed fetch defers only its own target; another target's pass is just a queue.
- `registry_repair_next_at` is gone.
- Contract change: the M1 test pinned the shared 60 s bump; it now pins this target's deferral
  and an undelayed cadence.

**Harness.** The runtime harness gains `RUNTIME-per-target-visit` and
`RUNTIME-held-offer-holds-repair-only` (14 mutants; 20 after the entry above).

## Runtime evidence: harness, two-peer run, S3 cost, 2026-10-06

(Corrected by the entry above: this entry did not cover every remaining plan D item.)

**Scripted mutants.** `scripts/check-agent3-runtime-mutations.py` reuses the core harness, and a
new workflow `agent3-repair-runtime.yml` runs it on Linux and Windows. It has 12 mutants:
- **The plan's four:**
  - live claim consult (M18);
  - restoring the `replay_ready` gate on S2;
  - restoring it on S3;
  - admission before read.
- **Stale-rebuild checks:** the Studio stale-rebuild check, the Registry stale-rebuild check, and
  the Registry record digest.
- **Guards from this session:**
  - the S3 custody restore;
  - the owner crash-between-install-and-recycle resume;
  - the per-offer hold;
  - the claimed bucket's skipped turn;
  - queued-preparation stranding.

Every replacement compiles without warnings, as CI's `-D warnings` requires. A full local run on
`53d167a0` detected all 12 at their intended assertions, restored every source byte for byte, and
passed each restored control (an earlier run had stopped at the Studio stale mutant, whose
assertion lacked a message; it now has one). The hosted workflow run is still to come.

**Two-peer run through the actors**
(`catchup/tests/repair/two_peer.rs`, `a_fault_is_decided_replaced_and_survives_restart_and_a_newcomer_through_the_actors`).
Nothing in it calls a repair transaction or runtime method: every step is a receive turn, a Read
or a control request on spawned actors.
- **Ownership change:** Alice founds, Bob, Carol and Dave join, and Bob removes Alice, so Carol
  observed Bob's tenure begin.
- **Fault:** Bob installed and published R1, then met a rival R2 from his own tenure. Carol holds
  both receipts and no seed.
- **Decision:** Bob's explicit decision is a job; `Busy` is retried as the visit model says.
- **Peer replacement:** Carol takes the repair from Bob's answer through her actor, owes R1's
  seed, fetches it from Bob and installs the replacement in a Replace job. Both sit on the same
  epoch and projection.
- **Restart:** actors and mounts come back from disk and snapshot. Bob's source has left Fault
  and his owner record is ordinary; no job reruns on anyone.
- **Newcomer:** Dave, a member whose vault never held the document, installs the repaired version
  through an ordinary owner proof.

**What the run found** (fixed here):
- **HIGH: re-staged reports blocked newcomers after any repair.** A faulted peer reports its
  frozen pair on every discovery until it applies the repair. Reports reaching the owner after
  its own decision finished were staged again: proof of the selected receipt stayed suppressed
  (no newcomer could install the repaired document), and the fault view offered the decided pair
  for a second decision with `may_decide: true`.
  - *Fix:* `admit_fault_report` now takes the repair the caller's exact source carries, and
    declines a report of exactly that pair while no decision is held, the source is servable and
    the repair verifies under the current tenure. Those are the conditions under which the head
    service carries it in the same answer. Any other case stages as before. Studio reads the
    carried repair through a warm, byte-verified probe that cannot fail or delay B0, and keeps B0
    ahead of its authoritative source read (see review AG3-IMP-003 below).
  - *Regressions:* the store test
    `a_report_of_a_pair_the_owner_already_repaired_is_answered_not_restaged` (an exact pair is
    answered, a different pair still stages), plus the two-peer run. Removing the decline fails
    both.
- **Protocol limit, not changed:** after A to B, leaf 0 is vacant, so no fresh MLS member can
  join under B's tenure. A pinned group refuses (`AdmissionAuthorityUnavailable`) until a
  succession proof exists, and a policy-less group would make the joiner the owner. The run
  therefore uses a document newcomer. An MLS newcomer's refusal of an offered repair (N16) stays
  pinned by the runtime test.

**Test barrier.** `wait_studio_preparation` (test-only) now also waits for a detached repair S2, so
actor fixtures await it without spending network deadlines.

**S3 cost** (`profile_repair_job_stages`, release; the smoke variant runs in the suite). The
fixture asked for 20,000 operations and the saved Studio source holds 6,939, at 4.9 MB physical:
the byte cap stops it first. One release run, while other agents' builds shared the machine;
read these as orders of magnitude:

| Stage | Decision | Replacement |
|---|---|---|
| S1 capture (custody) | 9 ms | 11 ms |
| S2 rebuild (detached) | 181 s | 153 s |
| S3 install (custody) | 14 ms | 11 ms |
| S3 transaction (custody) | 211 ms | 264 ms |
| **Total custody** | **234 ms** | **286 ms** |

For comparison, a cold restore of the same source takes 136 s, and the synchronous repair did
that inside custody before the job existed. At this size the job moves about two and a half
minutes of verification off the actor and keeps under a third of a second in custody.

**Load isolation.** Parallel full-suite runs exposed a load-sensitive Registry runtime test,
`studio_held_registry_page_is_discarded_after_fault_or_checkpoint_replacement`. Its receiver
drew from the process-wide preparation pool, and parallel actor tests can hold all four slots.
It now keeps a private pool, as the `unopened` fixtures already do; no assertion changed. The
two-peer test's actors also get private pools, through a test-only hook
(`actor::studio_pools_for_next_spawn`, consumed by the next `spawn` on that thread).

**Still open:** Registry Flow D on a real peer; the CI run of the new harness workflow; a bounded
repair verdict.

### Review of `02280999`: REQUEST CHANGES, both findings fixed

| Finding | Disposition |
|---|---|
| **AG3-IMP-003 (P1):** to find the carried repair, the Studio head adapter moved its whole source read before B0. That read propagates `receipt_head()`'s `ReceiptConflict` for a Faulted source, so a valid report of another pair was dropped with the refused answer, against design 6.5/U-7. The same read also refuses a cold source and invalidates the budget. | **Fixed.** B0 is back ahead of the authoritative source read, exactly as before `02280999`; against that commit the adapter is now purely additive. The carried repair comes from a probe of the warm source after its byte-for-byte check against disk. The probe cannot fail, delay B0 or touch the budget; a cold, changed or Faulted source gives `None`, and the report stages as it always did. A carried repair is also passed only from a servable source, Studio and both Registry adapters, so the decline stays exact: a Faulted source answers nothing. |
| **AG3-TEST-016 (P2):** no regression exercised admission while the owner's own source is Faulted. | **Added** `a_faulted_owner_source_retains_another_reported_pair_before_refusing_service`: warm and cold, with B independent of A or sharing one receipt. It asserts that service is refused, B is retained in the reserved slot, the source is byte-identical (no winner chosen), B survives a restart, A keeps priority with B waiting, and B is decidable once A is resolved. The store harness gains `REPAIR-b0-before-source-refusal`, which reinstates a refusing source read before B0. |

## Registry repair job: implemented, 2026-10-06

**The Registry gate is gone.** `registry_repair_execution_ready()` is deleted: every Registry repair
path now runs through the same detached job as a Studio source, scoped to the bucket
(`CheckpointTarget::Registry`). Nothing Registry-side runs a repair transaction synchronously any
more, and nothing restores a bucket under custody to decide whether to repair it.

**Core and store.**
- `RegistryEpoch::prepare_vault_source` is the detached restore. It takes the group's public facts
  as values, mirroring `StudioEpoch::prepare_vault_source`.
- `store/epoch_registry/repair_source.rs` holds S1 and S2:
  - `capture_registry_repair_source` is a bounded authenticated read under custody (S1);
  - `RegistryRepairCapture::rebuild` runs detached (S2);
  - the result is a `PreparedRegistryRepair`, bound to mount, server, group, bucket, actor,
    owner, MLS epoch, plaintext digest and physical size.
- The issue and apply transactions gain `*_prepared` entry points. They share the existing inner
  bodies, so every outcome, hold, CORE-007 refusal and recycle is unchanged.
- `check_prepared_registry` rechecks the context, re-reads the record and verifies it against the
  live budget before the rebuild is used.
- The writer (`update_registry_prepared_with_io`) uses the prepared unit only if the re-read bytes
  still match its digest and size; otherwise it refuses with "prepared Registry source changed".
- The Server wrappers (`issue_registry_fault_repair_prepared`,
  `resume_registry_bucket_repair_prepared`, `apply_registry_bucket_repair_prepared`) repeat their
  custody siblings' V5, channel, snapshot and owner checks exactly.

**Runtime.**
- **Job types.** `RepairRebuild::Registry` and `RepairRebuilt::Registry`.
- **S3.** `repair_execute` does a staleness pre-check (`registry_repair_source_is_current`): stale
  is an ordinary 5 s rerun, while any other refusal is a 60 s hold. Then `execute_registry` runs
  every input, and `finish_registry_repair` follows each outcome.
- **Entry points now scheduling the job:**
  - `repair_registry_fault`: V5 and the snapshot first, then the visit model, answering
    `RepairStarted { scope: RegistryBucket }`;
  - `offer_registry_repair`: Flow D from both discovery arms, with the same refusal memo,
    per-repair unverifiable hold and owner/tenure pre-check as Studio;
  - `resume_registry_repair`: B3 is read from the owner record, and a bucket that owes only its
    seed fetches it instead of rerunning the job;
  - the router's owed-bucket branch: one Replace path for both kinds.
- **Owed-repair facts.** These come from the retained prepared provider
  (`prepared_registry_owed_repair`; unknown defers, as Agent 4's gate did), never from a restore
  under custody. `owed_registry_repair` and `registry_repair_evidence` are now test-only for that
  reason.
- **Claims.** Consulted by `work_registry` (skip and advance, before any pointer refresh, owner
  maintenance or page pass) and `persist_registry_page` (drop the page), in addition to the
  router and `advance_checkpoint`.
- **Provider after a bucket write.** The provider is dropped only when no Registry preparation
  is queued or running. Otherwise that preparation's attachment would find no provider and fail
  the visit; the stamp check on every use refuses superseded bytes instead.
- **Removed.** The synchronous `install_repaired_registry_seed`, and the separate Registry repair
  memo (`registry_repairs_seen`, now in `repairs_seen` by target).

**Tests** (`catchup/tests/repair/registry.rs`: 7 here, plus 2 from the review below). The
full-load counter now also counts
`checked_registry_unit`, the custody restore the transactions use. Each guard was broken to
confirm its test fails:
- **The decision as a job:** the custody counter stays unchanged through S1 and S3. Passing no
  rebuild to the transaction fails it.
- **A stale rebuild writes nothing** and is reported "decide again". Accepting any bytes as
  matching fails it (the budget's inventory check still refused the write, as a second line).
- **The owed replacement:** `AwaitingSeed`, then a minted repaired pass, then the Replace job,
  then `Installed`, with the owner record recycled.
- **An owner owing only its seed** fetches it with no job. Ignoring B3 fails it.
- **An unverifiable offered repair** holds only itself, never the bucket. Holding the bucket
  instead fails it.
- **A claimed bucket gets no Registry turn.** Removing the consult fails it.
- **A bucket commit does not strand a queued Registry preparation.** Dropping the provider
  unconditionally fails it.
- **Contract change.** Agent 4's `unopened` test of the owed-bucket router keeps its assertions:
  defer, discard the pass, no custody load, no write. Its message no longer says repair is
  disabled.

**Residuals for review.**
- Registry Flow D on a real peer is not exercised end to end: this fixture's only peer has no
  store. The S3 owner refusal and the evidence branches are covered on the founder.
- The router's Replace start from a network-fetched bucket seed is covered only through its
  parts.
- On a peer, a cold provider is the steady state rather than an edge case. Every bucket job
  invalidates the provider, while Studio keeps its source warm. So repeated offers of a repair
  already applied there, while its seed is pending, each run a full job: rebuild, inventory scan
  and flush. The cost is bounded by this device's own discovery cadence, with no remote
  amplification. Keeping the post-S3 bucket warm would remove it.

### Adversarial review of the Registry job (uncommitted diff on `3b58be24`): no BLOCKER

| Finding | Disposition |
|---|---|
| **HIGH-1:** a crash between the owner's replacement install and its record recycle wedged the bucket. B3 alone fetched a seed that the held decision then deferred forever. | **Fixed.** `resume_registry_repair` classifies from the warm provider: owed means fetch the seed, owes-nothing means schedule Resume (which recycles), and B3 is used only while the provider is unknown. Regression: roll the owner record back after an install, then resume answers `AlreadyRepaired` and the record is recycled. Forcing the B3 fallback fails it. |
| **MEDIUM-1:** an offered repair's `Held` outcome or failed S3 held the whole target, so one replayed older repair a minute could block the owed seed. Studio had the same flaw. | **Fixed for both.** `hold_offer` holds that repair only, both from the finish functions and from the `repair_commit` error path. Regression for the error path: an owner's offered repair fails at S3, holds only itself, and the owed seed still mints. Restoring the target hold fails it. The held-outcome half needs a real peer and is still untested. |
| **LOW-1:** `finish_registry_repair` lengthened the shared owner resume cadence. | **Fixed.** The bump is removed; a hold now waits on its bucket backoff. The test that pinned the bump now pins "resume runs no job while the seed is pending" instead. |
| **LOW-2:** `RefreshRequired` was noted for outcomes that write nothing. | **Fixed.** It is noted just before each transaction call. |
| **LOW-3:** a stale graph kept its pool slot while any preparation ran. | **Fixed.** A provider that holds a graph is always dropped (no preparation can be pending against one), and the provider is also invalidated on the S3 error path. |
| **LOW-4:** the cold-provider cost was understated. | **Documented** above. |
| **LOW-5:** the router's owner Replace skipped the snapshot pre-check. | **Fixed.** Restored before the pass is consumed, for both kinds. No dedicated test. |
| **LOW-6:** the writer trusted its caller's context check. | **Fixed.** `context_matches` is checked inside `update_registry_prepared_with_io`. The earlier checks mask it, so no test is possible. |
| **LOW-7:** unused synchronous custody entry points remained. | **Fixed.** The five Server functions and four store wrappers are now `#[cfg(test)]`. |
| **LOW-8:** doc wording and a vacuous assertion. | **Fixed.** THREAT-MODEL and INTERFACES now say held decisions come from the owner record, and the claimed-bucket test asserts that no preparation started instead of an unchanged faulted file. |
| **Q6:** the test-only copy of the owed mapping did not pin the production one. | **Fixed.** Both paths now use `registry_owed_replacement`. |

## Detached S1-S4 repair runtime: progress, 2026-10-06

**Studio job implemented** (C below). `catchup/repair_job.rs` holds the job: claims, inputs,
stages, S1 `start_repair`, S2 `repair_detach` plus `RepairRebuild::run`, token-routed
`repair_complete`, the per-turn `repair_check_authority`, and S3/S4 `repair_commit`. The S3
handlers are `execute_decision`, `execute_resume`, `execute_offered` and `execute_replace` in
`catchup/repair.rs`; each calls the existing Server transaction on the installed rebuild.

**Entry points now scheduling the job:**
- the explicit `RepairFault` (visit model, `RepairStarted`);
- Flow D in the discovery arms (`offer_repair`, with a terminal-repair memo; the pass is dropped
  when the job takes it);
- `repair_owner`;
- the owed-replacement install, with the seed extracted at S1 under the authoring check.

The synchronous Studio install helper is removed.

**Claim consulted by:** the router, page receive, target selection, owner rotation, and
foreground `Apply` and `ApplyOverlayCopy`. The fault view gains a `Scheduled` blocker and a
bounded `last_attempt`, encoded natively; INTERFACES documents the contract.

**Registry stays fail-closed** behind `registry_repair_execution_ready()` until its job exists.

**Tests** (`catchup/tests/repair.rs`). Each guard was verified by breaking it:
- admission before any read: a capture before the reservation fails it;
- flat full-pool retry and same-work `Scheduled`;
- the owed replacement as a full job, with slot and claim released after S3;
- a stale rebuild writes nothing;
- a cancelled waiter keeps its worker's slot and claim;
- an MLS epoch change abandons the job: removing the check fails it;
- explicit decision, then fault view, then `last_attempt`;
- the live claim where no durable claim exists: removing the consult fails it (the durable-claim
  case alone could not show this);
- HIGH-1: S2 detaches past in-flight work and an undue discovery, and S3 commits with a checkpoint
  pass and discovery pending. Restoring either `replay_ready` gate fails it.

### Adversarial review of `54c79846`: no BLOCKER, findings fixed

| Finding | Disposition |
|---|---|
| H1 a rebuild finishing after a pause held its slot and claim indefinitely | Released in `complete` when paused, as `handoff_complete` does. The test pauses during S2; removing the release fails it. |
| M1 owed replacement reran the whole job every few seconds | `repair_owner` and `offer_repair` fetch the owed seed when the warm source owes exactly this repair, and an owner `AwaitingSeed` backs the resume off 60 s. Removing the owner check fails the test. |
| M2 unverified offers cost a capture and held the target | `offer_repair` requires authoring tenure and `verify_current_owner` before reserving anything. The unverifiable hold is per repair, not per target. The newcomer (N16) test: removing the pre-check fails it. |
| M3 untruthful outcome reporting | Every ending is reported: stale, abandoned, paused, cancelled, capture, S2 and budget failure. A new decision clears the old report. A decision that cannot start returns its error, not `Busy`. Two tests; removing the clear fails one. |
| M4 security docs said repair was off | THREAT-MODEL, HANDOVER and ACCEPTANCE updated. |
| L1 S3 reset other targets' catch-up state | Resets are scoped to the job's target, and no seed pass is minted while a page pass exists. |
| L2 rival preparation of a claimed source | `prepare_for` defers while claimed. Gossip, replay and Flow S/H writes are listed residuals. |
| L3 fetched seed discarded on `Busy` | Kept, with a 1 s recheck. |
| L4 one busy target paused resume for all | Only a hold slows the round-robin. |
| L5 spurious error for a peer with no source | Automatic work holds silently; an explicit decision still reports. |
| L6 view and claim disagreed | The blocker follows the claim; authority is checked before the view is annotated. |
| L7 split doc comment | Restored. |

**Re-review of `4bc753a6`: no BLOCKER or HIGH; fixes below.**
- **MEDIUM-1 (truthful reports):** an explicit decision abandoned before S3 now reads "decide
  again" instead of "it will rerun". A decision while paused is refused. The quiet endings
  (already applied, unverifiable, nothing held) are reported.
- **LOW-1:** a fetched seed is kept only while another repair job is busy, not while the pool is
  full.
- **LOW-2:** a cold owed owner classifies from the B3 flag in its owner record.
- **LOW-3:** the claim is checked before preparation in `advance_checkpoint`.
- **LOW-4:** the owner backs off only when a seed fetch started.
- **LOW-5:** doc drift fixed.
- **Tests:** paused refusal, unverifiable report with a per-repair hold, preparation refusal and
  pass drop for a claimed target, the cannot-start error, and the cold durable owner. The paused,
  pre-preparation and durable guards were each broken to confirm their tests fail.

Test gaps the review listed and that remain open: Flow D end to end through the job, the peer
branch of `execute_replace`, mutant coverage of the remaining claim consult sites (page receive,
selection, rotation, Apply), and a forged offer on a peer with observed tenure (this fixture's
only peer is a newcomer).

**Remaining:**
- the Registry job;
- the two-peer Fault, decision, replacement, restart and newcomer run;
- the in-custody S3 cost measurement;
- the harness mutants;
- removal of the Registry gate.

## Detached S1-S4 repair runtime: implementation plan, 2026-10-06

Answers G4-A3-BOUND in [GATE4-ACCEPTANCE](GATE4-ACCEPTANCE.md). Today every repair runtime entry is
switched off by `automatic_repair_execution_ready() == false` (`3fcde979`): explicit decisions,
offered-repair application, owner and Registry resume, and repaired-seed installation. This plan
builds design 10.3's job and only then turns that gate on. Base `c6f7fea0`.

**A. Captured authority (core).** Every group use on the repair and adoption paths reads four public
facts: group id, epoch, designated committer, and that committer's signing key (`verify_current_owner`
for receipts and repairs, `from_checkpoint`, the book and journal resolvers). A trait `OwnerAuthority`
is implemented by `ServerGroup` and by `CapturedOwnerAuthority`, a value snapshot of those facts
taken from the live group under custody. The repair and adoption core functions become generic over
it, so existing callers compile unchanged and S2 can run the existing logic with no MLS state. The
captured view answers `member_signature_key` only for the committer, so any other query fails
closed. Equivalence tests compare verdicts against the live group for each accept and refuse case.
Implemented in `81a771bd`. After the design review (below) S2 does not use it yet: it is the seam for
moving the adoption half (seed verification, `Repair` snapshot, successor build) into S2 if the
in-custody measurements require that.

**B. Store (revised after review).** The transactions stay as they are; S3 runs them unchanged on a
source S2 rebuilt. Studio needs no store change: `install_prepared_studio_source` installs the
rebuild after rechecking actor, owner, MLS epoch, group, plaintext digest and physical size, and
the transaction takes that warm unit through its existing digest-checked warm path. Registry
transactions restore the bucket under custody today, so they gain a variant that takes the
detached rebuild and rechecks its stamp against disk in the same way. Every outcome, hold,
CORE-007 refusal and recycle stays exactly as today, and so does the cold-source refusal.

**C. Runtime job.** Modelled on Flow H (`HandoffJob`):
- **S1:** one per-actor repair slot. One pool permit is taken before any body read; a refusal sets
  only a flat capacity retry and charges no target. A per-target live claim is taken (Studio target,
  or the Registry bucket) using the `OverlayAdmission` `Weak` reap. Then the bounded plaintext
  capture of the existing preparation, plus the job inputs: an explicit decision, a resume, an
  offered repair and its receipt, or seed bytes extracted under the authoring-tenure check.
- **S2:** the existing detached rebuild, holding the job's own permit. It never takes a second one.
- **S3:** in the actor turn, not gated on `replay_ready()`. Budget first, then install the rebuild,
  run the transaction, and drop the ownership. A stale rebuild holds the target and retries.
- Token-routed completion, with RT-001 drop-in-worker and cancelled-carries-nothing. Detached at
  the top of the `detach` chain beside `Prepare`, so pending discovery never parks it. Authority
  (tenure, MLS epoch) is checked every turn and abandons the job; per-target holds and wakes; the
  parked result counts in `result_parked()`.
- **Entry points that enqueue instead of executing:**
  - explicit `RepairFault`/`RepairRegistryFault`: `Scheduled`, `Busy`, or `Scheduled` again for
    the same decision while it is live;
  - offered repairs from the discovery arms, which drop the pass when a job is scheduled or busy;
  - `repair_owner` and `resume_registry_repair`;
  - the owed-replacement install.
- **Live claim consulted by:** ordinary install and `route_checkpoint_install`, page receive, owner
  rotation (the sticky target advances), Registry maintenance, and foreground `Apply` and
  `ApplyOverlayCopy`. Each drops the pass, or refuses, without rescheduling discovery for that
  target.
- **Outcome reporting:** a bounded per-target last-decision outcome and a scheduled blocker in the
  fault view, replacing the single overwritten failure slot as the only channel.

**D. Evidence and gate.** The gate is removed only after:
- full-pool deferral with no mutation;
- cancellation and result holding;
- an unrelated actor progressing while a large S2 is paused;
- S3 completing within bounded turns for a Selected pass carrying a repair, a `PageReady` pass on
  the claimed target, and a faulted target under reporting discovery;
- a stale rebuild refused with no write;
- fairness across held targets;
- interrupted B2/B3 Studio and Registry;
- a two-peer Fault, decision, replacement, restart and newcomer run through the actor.

In-custody S3 cost is measured at a maximal source.

Mutants: claim removal (M18), restoring the `replay_ready` gate, admission before read, and the
stale-rebuild check.

Not in scope: native registration (Agent 4, after P5), the C-3 cursor consumers (Agent 1), CORE-007
cancellation, and claims on gossip ingest and own-operation replay. A source changed by those
during S2 makes the rebuild stale and the job retries; this is a documented residual.

### Design review of `d23452c9`: no BLOCKER, plan revised

| Finding | Disposition |
|---|---|
| HIGH-1 job starves behind its own claim (`replay_ready`, discovery gate) | S2 detaches at the top of the chain and S3 is ungated; claimed-target deferral drops the pass without rescheduling discovery; three liveness regressions plus a gate mutant. |
| HIGH-2 S1 cannot sign or assemble evidence without the restored graph; preparing inside a reserved job is hold-and-wait | Signing and evidence happen at S3 inside the unchanged transaction on the rebuild; the job's S2 uses its own permit. |
| MEDIUM-3 partial S2 results change outcomes | Moot: S3 runs the unchanged transaction. A `StorageRefused` test is added. |
| MEDIUM-4 precomputed-snapshot writers | Moot: existing writers. If the adoption half moves to S2 later, pairing is by type and the capability is consumed inside the writer. |
| MEDIUM-5 missing consult points; Busy offered repair falls through | Busy, scheduled and live cases drop the pass. Consult points are listed in C; gossip ingest and replay are a residual. Owner-record re-saves no longer matter, because S3 reads the record fresh. |
| MEDIUM-6 stamp contents | The stamp is the existing prepared-source check (plaintext digest and physical size, actor, owner, MLS, group, mount). Authority, durable snapshot, channel and CORE-007 checks are re-run by the transaction at S3. |
| MEDIUM-7 trait boundary | Sealed, private constructor from a live group only, no codec (`81a771bd`). The `same_tenure` equivalence test is added when S2 first uses the view. |
| MEDIUM-8 visit-model reporting | Scheduled/Busy plus a per-target last outcome and blocker; native converter variant and INTERFACES update. Design 10.3 amendment: no native session is captured. |
| MEDIUM-9 overlay admission | Dropped: a pool permit plus a per-actor repair slot, as design 10.3 specifies. Save and handoff are unaffected. |
| LOW-10 B1 copy | Moot; `ReceiptRepairPlan` is no longer `Clone`. |
| LOW-11 admission across detach | Moot: minted at S3 by the transaction. |
| LOW-12 synchronous composites | The cold-source refusal is unchanged. |
| LOW-13 mutants and tests | Listed in D. |
| LOW-14 budget per path | Kept per job kind: the install uses `budget()`, the rest `inventory_budget()`. |

## Archived Observed-tenure integration candidate, 2026-10-05

On `gate4-finalization`, the shared Studio/Registry report admission consumes CORE-005's private
archived witness from the same still-current durable owner snapshot used by head service. It checks
retained exact-pair attestations before archive lookup, otherwise requires the complete
owner-key/start/tenure tuple to match current or archived Observed authority. Historical pairs are
stored with origin/retirement attestation but never become a live source seal or current-tenure
overflow hold. A real A -> B -> C MLS, persist/reopen regression covers both document families,
Unknown/no-archive/wrong-archive refusal, exact retained retry and a current-C screening repair.
The complete root and frontend suites, strict root/desktop Clippy, desktop check/build and
`cargo deny` pass. Independent review's one MEDIUM cloneable-capability finding was fixed by making
the witness non-`Clone`/non-`Copy`, exposing only a snapshot-bound borrow to application code and
pinning that boundary with a compile-fail doctest; re-review has no remaining BLOCKER/HIGH/MEDIUM.
Exact-head Linux ambient and repair-store mutations remain CI-owned. The detached S1-S4 runtime
remains unavailable, P5 remains false and commands remain unregistered.

## Current-tenure runtime repair, Studio and Registry, 2026-10-01

Implemented on `gate4-agent3-repair` (merge of base `9bfb7c79`/`94af29df` at `14dccacf`). Historical
report admission and N17 remain blocked: `94af29df` is a rejoin *test*, not the CORE-005 archived
witness, so only current-tenure (origin 0) evidence can be admitted.

**Integration correction (2026-10-05, PR #33):** a whole-candidate review found that repair-only
head responses could carry a readable source without repeating an uncertain B2 durability barrier,
and that Flow D/owner resume/repaired-seed installation still ran synchronously outside the shared
preparation pool. The candidate now flushes the exact source and re-saves the contextual owner
record before carrying any repair. Automatic repair execution is fail-closed until the designed
capture/detach/revalidate/commit job actually exists. The correction is commit `3fcde979...`;
independent re-review found no remaining finding or automatic-mutation bypass. The core/store
implementation and mutation harness remain integrated, but the active-runtime claims below are
historical and must not be read as current availability.

- **Owner record (5.2).** Tag 3 is a parsed canonical model; decode also requires
  `encode(decode(bytes)) == bytes`. Retained attestations are usable only after contextual restore
  against the local device and the durable snapshot epoch. New evidence enters only through
  non-`Clone` `ValidatedFaultAdmission`, minted fresh for a current-tenure pair after live
  current-owner checks. B1 binds repair, pair and the joint plan's journal candidate in one write;
  B3 marks application; terminal recycling removes the bound pair and omits tag 3 when empty.
- **Studio and Registry transactions (5.3, 5.4).** Issuance derives the decidable pair (shared
  derivation), refuses stale echoes, persists B1, then runs Flow A on the same source. Application
  reads the committed `repair_state()` first, saves B2, and replaces through the shared adoption
  install half, whose successor write consumes a recovery capability minted only after the
  `Repair` stage returned. Ordinary adoption refuses while the owner record holds a decision, and
  a repair-pending source continues through `prepare_repair_adoption`.
- **Serving and distribution (5.5, 5.6).** Head answers carry a repair only when the saved source
  applied it (B2) and it verifies under the durable owner tenure. A held decision never proves and
  gives no hint before B2. Proofs follow the journal's effective choice; a publication guard admits
  repaired reconciliation but refuses any tag 3. Peers apply delivered repairs (Flow D) from evidence
  they hold plus the answered receipt. Legacy rotation answers "not pending" for repair-bearing
  records instead of aborting the catch-up step.
- **App and native (5.7, 5.8).** V5 then durable snapshot, refusing on tenure disagreement; peer
  application holds on Imported/Unknown. `ReadFault`/`RepairFault` and Registry-scoped equivalents,
  `Repairing`/`StorageRefused` with native mapping, a native response encoder (commands unregistered,
  Agent 4), and a catch-up step that resumes held decisions for sources and buckets.
- **Agent 4 integration correction (2026-10-04).** Numeric repair high-water is now scoped to the
  v2 record's signed and independently authenticated issuer tenure rather than carried globally
  across owner succession. Same-tenure gaps and retry ordering are unchanged; unfinished journal
  provenance still blocks turnover. Checked allocation returns `RepairSequenceExhausted` at MAX
  before signing/B1 mutation, while a verified successor tenure begins at one. Wire and persistence
  encodings remain byte-compatible.
- **Agent 4 CI integration (2026-10-04; verified).** Store mutations now run in their own
  90-minute serial Linux/Windows matrix with Rust 1.89.0, Python 3.11, cache, strict warnings and
  always-uploaded logs. They were not appended to the 30-minute core job. The main Linux desktop
  job now runs strict Clippy across every target/feature of the separate Tauri workspace under the
  root `clippy.toml`. Existing harness fixes are inherited rather than duplicated. Exact candidate
  `2b6f716ef05fdf99bdc04da531eb0c0194682e65` passes both store-mutation jobs, both root jobs,
  desktop Clippy and Linux `check-no-ambient.sh` on PR #32. P5 remains false; Save/repair commands
  stay unregistered; at that checkpoint C-3 runtime adoption and the archived Observed-tenure
  consumer remained incomplete.

- **Report path (W-1, 6.5, 6.6), current tenure only.** Scoped head query v2 (`2 | v1 fields |
  count(0|2) | receipts`); v1 bytes unchanged. The report is captured opaque at queue time and
  decoded only after both request rails. A faulted peer attaches its pair (warm source, or a
  known-faulted bucket); a Fault phase now schedules discovery like Closing. The owner admits only a
  pair whose receipts verify under its durable current tenure, staging it in the reserved slot (B0)
  or, if occupied, as an overflow fingerprint (stale hold replaced, current hold accumulates; a
  fingerprint is released when its pair is stored or resolved). A failed stage refuses the answer.
  The durable proof gate suppresses proofs while reserved/overflow evidence is live and never
  proves or hints a retained pair member.

Deviations to review: the owner does **not** seal a reported current-tenure pair into its own source
(no materialisability dry run); it decides through the reserved binding, and a rolled-back owner on
the loser converges by case 6c's retarget. A non-live reserved pair is not migrated into an external
slot, so a new live pair waits in overflow until it is decided. The legacy ordinary guard still
refuses any tag 3, so ordinary rotation for that document waits for the decision (stricter than 5.2).

Not implemented at that source checkpoint: historical admission (CORE-005), the detached S1-S4
split (every step is one custody visit), and Imported coverage at the app boundary (no migrated-v1 Server fixture; the seam's
own `require` anchor covers it). Runtime-level tests of the catch-up repair step are partial: the
install router has one (an owed source with an unfetched pass, and the hold backoff); Flow D and
the resume steps are covered only through their store predicates, transactions and the sync
selection.

### Adversarial review of `90dca2c2`: REQUEST CHANGES, all findings addressed

| Finding | Disposition |
|---|---|
| P1 a repair hold surfaced as an installer error, pausing all Studio catch-up | Fixed. The runtime checks one shared rule (`repair_defers_install`) before Studio and Registry seed installs and defers that target only. Adoption under a held decision is allowed solely for its own selected receipt into a source owing it (a single post-load check). |
| Consequence: no runtime supplied the selected seed, so install cases stalled group-wide | Fixed. `select_repaired_checkpoint` mints a seed pass from a locally verified repair (authoring tenure; repair and selected receipt both current-owner verified); every `AwaitingSeed` enqueues it, adoption installs through `prepare_repair_adoption`, and the owner's next resume recycles. |
| P2 Flow D cold restores on the actor | Fixed. Studio prepares through the detached pool and reads evidence warm-only; Registry consults its source rail and detached inventory first. |
| P2 recovery capability was a tautology | Fixed. It binds the durable predecessor digest read before staging and refuses unless disk still matches before the successor write; a test swaps the on-disk source during the recovery write and the successor is refused. |
| P2 guards without failing tests | Added tests and mutants: contextual observer/epoch, publication guard, pre-B2 hint, rotation fence, Registry claim and defer, durable predecessor; plus the rolled-back-owner 6c retarget end to end. |
| P3 Held labelled Repairing | Fixed: a hold reports the saved phase. |
| P3 Registry resume without backoff | Fixed: 60 s backoff on hold or failure. |
| P3 rotation churn on a claimed target | Fixed: Studio rotation and Registry maintenance skip non-ordinary records. |
| P3 peer path ignored the device's own earlier-tenure claim | Fixed: refused (CORE-007). |
| P3 doc overclaimed re-validation | Fixed by making it true: the owner encoder re-decodes tag 3. |
| P3 `repair_failure` never read | Folded into the surfaced `owner_failure` slot. |
| P3 Registry `waiting` count | Fixed to exclude the decidable pair. |

Mutations: `scripts/check-agent3-store-mutations.py` now carries twenty guards (six original); all
twenty were detected at `f59eb4ed` under `RUSTFLAGS=-D warnings`.

### Adversarial re-review of `f59eb4ed`: REQUEST CHANGES, findings addressed

| Finding | Disposition |
|---|---|
| P1 a kept proof pass for the selected receipt was never fetched: the router installed it seedless, failed and dropped it, and every rotation repeated that | Fixed. An owed source never installs from an unfetched pass. Whenever an offered repair leaves the source owing, and whenever the router sees an unfetched pass for an owed target, the repaired pass is minted instead. The new runtime test `catchup::tests::repair` drives an unfetched pass into the router; restoring the old condition fails it with "no verified seed". |
| P2 the repaired install took its tenure from the pass, which for a proof pass is the proof's own claim | Fixed. `install_repaired_*` extracts the verified seed only if the pass was selected under this device's observed (authoring) tenure, then delegates. The owner goes through `resume_*` with its durable snapshot, and a peer through `apply_*`, which refuses the owner. |
| P2 persistent holds refetched the seed every 5 s | Fixed. `RecoveryPending`, `StorageRefused`, a hold, an install error and a failed mint put that target in a 60 s backoff, during which no repaired seed is fetched for it. The rest of the rotation keeps the 5 s cadence. The test covers it, and removing the check fails it. |
| P3 Registry tail not cleared after a routed install | Fixed. A shared `reset_registry_tail` is used by both installers. |
| P3 wrong or missing target for a bucket pass | Fixed. A minted pass records the Studio target it reports to. A bucket pass with none is still routed, and its diagnostics are only held. |
| P3 the owner installed outside its durable snapshot | Fixed. The router requires a current owner snapshot, otherwise it holds; see P2. |
| P3 an explicit owner decision could mint a pass a pending discovery then dropped | Fixed. No pass is minted while a discovery is pending or in flight; `repair_owner` re-mints on its next turn. |
| P3 the fetched seed was lost on a budget error | Fixed. The pass is taken after the budget call. |
| P3 test gaps | The router test, plus Registry offered evidence (Pair, Unverifiable, Terminal) in the peer bucket test. Still missing: a fetched-seed install end to end (it needs a two-node seed transfer), and the positive `owed_registry_repair`. |

`Unverifiable` evidence is still not memoised, by choice: it can change once the device faults on
the pair.

### Adversarial re-review of `7943ee04`: REQUEST CHANGES, findings addressed

One change resolves most of these: a source that **owes** a repair (B2 crossed, replacement
pending) installs through the repair transaction itself, using the fetched seed. A single router,
`route_checkpoint_install`, runs before both seed installers.

| Finding | Disposition |
|---|---|
| P1 livelock: a fresh proof pass overwrote the repaired seed pass | Fixed. The proof pass is dropped and the repaired pass minted instead (since `f59eb4ed`'s review, even when the proof names the selected receipt). Discovery cannot relaunch while a pass is set. |
| P2 no re-mint after B2 | Fixed. `owed_studio_repair`/`owed_registry_repair` read the repair and full pair from the warm source's committed evidence, so any pass for that target re-mints the selected seed without the owner resending anything. |
| P2 deferral pushed the global `next_at` out 60 s | Fixed. Deferral and failure use the ordinary 5 s `retry_discovery`, which rotates to the next target. |
| P2 `StorageRefused` unreachable through adoption | Fixed. The owed install goes through `install_repaired_*_seed` and then `apply_*_repair`, giving typed `Installed`/`RecoveryPending`/`StorageRefused` and owner recycling in the same step. |
| P2 no tests for the repaired selection | Added `registry_seed::tests::repaired`, covering newcomer `Unknown` (N16), `Imported` (N42), wrong target, unselected sibling, member-signed repair or receipt, other issuer tenure, then success, seed fetch and supersession. Verified by breaking the code: swapping in the verification accessor and dropping the selected-hash check each fail it. The store test now asserts the owed repair and pair after B2, and none after install. |
| P3 defer errors were silent | Fixed. Surfaced through `owner_failure`, and the target defers rather than installs. |
| P3 repeated Registry restores for the same repair | Fixed. A terminal memo per (bucket, repair), plus a tri-state `OfferedRepairEvidence` (Terminal, Unverifiable, Pair). |
| P3 seed always requested from one peer | Fixed. Rotates over the current page peers. |

Owner change while a source owes a previous owner's repair (not flagged by the review): the
repaired pass no longer verifies under the current owner, so the target holds. This is CORE-007's
accepted fail-closed limitation. The runtime surfaces the failure and backs that target off for
60 s while the rotation continues. It does not install the new owner's checkpoint over the owed
replacement.

## Base re-merge and the V5 tenure seam, 2026-10-01

Merges: `b0b73453` (base `b15ce314`) then `d35b1836` (base `1f8a11d9`), both clean with no file
overlap. Agent 2's rustfmt break (`d4ba216c`) and `release-identity` mutant/`RUSTFLAGS` harness fix
are on base, so nothing of another agent's was taken locally. Local at `d35b1836`: `cargo fmt --all
-- --check`, `bash scripts/check-no-ambient.sh` and strict workspace Clippy (`-j 1 --all-targets
--all-features -D warnings`, zero warnings) PASS. At `b0b73453` all six store mutations failed at
their named assertions and restored byte-exact. No test suite was run at `d35b1836`; CI is the
evidence for that head. At `b0b73453`, Windows `repair-core` again passed its library tests and then
hit the 30-minute job limit during mutations, so Windows mutation evidence is still missing.

**V5 is implemented** (`23465a17`, `catcoms-app/src/studio/tenure.rs`):
`Server::require_observed_owner_tenure()` returns the start for `Known` alone and refuses
`Imported` and `Unknown` with different messages; `observed_owner_tenure()` keeps `Imported(S)`
visible for verification. This is the T1/T2 contract of design 13.2 at the app boundary. It is
`#[expect(dead_code)]` until issuance consumes it, so the first consumer must remove that attribute.
The existing `with_durable_owner_snapshot` permit is already minted and rechecked through
`authoring_owner_tenure_start()`, so it refuses both fail-closed values too, but as one
undifferentiated `Unauthorized`. Planned issuance shape (not implemented): take V5 for the
diagnostic, then require its start to equal the permit's tenure inside the same custody visit and
refuse on any difference, so the two sources cannot silently disagree.

**Still blocked:** N17, the gating case, needs CORE-005's archived Observed witness, which V5 does not
supply. Only current-tenure (origin 0) issuance is now constructible. M-1 (Agent 2, `22758646`) is
relied on only indirectly: repair compares derived `tenure_id`, never `DeviceId`, and M-1's
pre-merge refusal keeps members agreeing on the start that feeds it. Its missing test is Agent 2's.

## Empty fault-section correction, 2026-09-28

Code: `ddedbba31709ea43607bfb3f7af25320b3fd8526`. AG3-IMP-002 rejects an entirely empty tag 3;
final recycling must omit the section. AG3-TEST-015 now pins rejection and absent-tag compatibility
through sealed reopen, ordinary preparation/publication and another reopen, with and without tag 2.
The negative failed before the fix; all 29 focused owner tests and all six store mutations/restored
controls pass. Adversarial review caught a LOW test-isolation gap; canonicality negatives now start
from valid nonempty records, and re-review has no remaining findings. A real terminal-repair cleanup
integration test remains required when the contextual recycler exists; this change does not add it.
Required local verification (logs: `logs/empty-fault-*`):

- `cargo test -j 1 --config profile.test.package.catcoms-app.debug=0 --all --all-features --no-fail-fast -- --test-threads=4`: **1,857 passed / 1 failed / 13 ignored**. The failure is `studio_exchange::tests::unopened::studio_registry_preparation_outliving_head_needs_a_fresh_request` at `unopened.rs:33` (expected cold preparation). An exact isolated retry using the same compiled test binary passes. Read-only triage confirms this setup cannot reach tag 3; shared four-permit preparation-pool contention is plausible but not proven by the log. Follow-up: isolate test capacity without weakening its lifetime assertions. All three owner-return scheduling cases pass.
- `cargo test -j 1 --config profile.test.package.catcoms-app.debug=0 --manifest-path apps/desktop/src-tauri/Cargo.toml`: PASS, **261 unit + 5 ACL**. `npm.cmd --prefix apps/desktop test`: PASS, **1,229**.
- `cargo fmt --all -- --check` and `cargo clippy -j 1 --all-targets --all-features -- -D warnings`: PASS.
- `bash scripts/check-no-ambient.sh`: FAIL, the same seven untouched calls. `cargo deny check`: FAIL, local rustls 0.23.40 / RUSTSEC-2026-0285. Startup/flow gates do not apply to this store-only correction.

[Core CI 36468854692](https://github.com/Thalpy/Mewtual/actions/runs/36468854692): Linux PASS;
Windows exceeded its 30-minute job limit during mutations, so the run is not a pass.
[Full CI 36468854991](https://github.com/Thalpy/Mewtual/actions/runs/36468854991): Linux frontend/Tauri
and cargo-deny PASS; root Linux/Windows jobs still running at handoff. CI uses merge checkout
`6b5eda10c7837b5b8ddae04de7e57d99e9ec8369` with Agent 1's newer base `918ffb9` and rustls 0.23.45;
the isolated local base remains `48f9069`. No merge or change to another agent's branch was made.

## Owner fault-record checkpoint, 2026-09-25

Code: `2488a097a52681deb5edb3a701513a65a5ed4029`; verification recorded 2026-09-28.
Implemented strict inert tag-3 decoding in `store/epoch_owner/fault_record.rs`: canonical bounded
pairs, local attestation framing, overflow, exact repair bindings and historical signatures.
Inventory/reopen accepts and accounts these bytes under the shared 27,904-byte plaintext /
27,944-byte physical cap. Legacy live owner reads and all ordinary writers refuse tag 3 or retained
journal reconciliation/proof before mutation; cleaned v2 journals remain compatible. Active close
binding now follows the effective journal choice, including reconciliation. Legacy bytes are unchanged.

This is persistence groundwork, not report admission: no production constructor, B1 write, observer/
durable-snapshot/custody capability or runtime repair endpoint is enabled. Agent 2's archived Observed
witness remains absent. No native/UI-hook change is proposed. Shared integration remains Agent 4-owned.
Read-only design and actual-diff implementation reviews found no remaining findings; 27 focused owner
tests pass, including maximal combined records above the old cap and refusal without byte/budget/RNG
changes. All five store mutations fail at their intended assertions, restore exact bytes, and pass
their restored controls (`python scripts/check-agent3-store-mutations.py`). Script/test re-review
also has no findings. An initial app test rebuild exhausted memory; the passing focused, mutation
and full Cargo test runs use `profile.test.package.catcoms-app.debug=0` without changing assertions.

Required local verification at the code SHA is complete:

- `cargo test -j 1 --config profile.test.package.catcoms-app.debug=0 --all --all-features --no-fail-fast -- --test-threads=2`: PASS, **1,856 passed / 13 existing ignored**, including all 699 app tests and all owner-return scheduling cases.
- `cargo test -j 1 --config profile.test.package.catcoms-app.debug=0 --manifest-path apps/desktop/src-tauri/Cargo.toml`: PASS, **261 unit + 5 ACL**.
- `npm.cmd --prefix apps/desktop test`: PASS, **1,229**.
- `cargo fmt --all -- --check` and `cargo clippy -j 1 --all-targets --all-features -- -D warnings`: PASS.
- `bash scripts/check-no-ambient.sh`: FAIL, the same seven untouched calls listed below. `cargo deny check`: FAIL, unchanged rustls 0.23.40 / RUSTSEC-2026-0285; bans, licences and sources pass. Startup/flow gates do not apply to this store/core-only slice.

[Core CI 36127096674](https://github.com/Thalpy/Mewtual/actions/runs/36127096674): PASS, **295 library tests + all 25 core mutations/restored controls on each of Linux and Windows**.
Actual merge checkout: `5b19998204540ae9b3ad91a15b266c34b532fa7c` (Agent 1 base `48f9069`).
[Full CI 36127096549](https://github.com/Thalpy/Mewtual/actions/runs/36127096549): all **10 new store tests pass on both platforms**, but the broader run FAILS in untouched owner-return scheduling at `studio_exchange/tests/scheduling.rs:567` (Linux: cancelled transport; Windows: cancelled transport/parser and three retained previews), native unused-code checks, and cargo-deny. No integrated Gate 4 PASS is claimed.
Agent 4 CI wiring: run `python scripts/check-agent3-store-mutations.py` serially in isolated Linux/Windows jobs; shared workflows remain unchanged.

## CORE-006 implementation checkpoint, 2026-09-24

Code: `35492b116a94abeaedb6e06d47934b7df086f89c`. Added immutable joint repair plans for Registry,
Studio Index and Flipnote. Source/journal effects are independently derived; covered live choices,
unfinalized earlier journal repairs (including NoChange), stale versions and owner turnover refuse.
Preparation preserves both inputs. An original-journal comparison supports B1 preflight; application
rechecks the exact candidate journal, whole source and final locked gate stamp, including exact retry.
These remain core APIs: historical admission, custody, global sequence and the durable transaction
claim/B1-B6 writes are still integration work. A plan does not certify a store write or grant custody.

Read-only design/implementation review and final re-review: no BLOCKER/HIGH/MEDIUM remains; the
LOW negative-test isolation gaps were fixed. All 22 focused joint tests pass. All five new mutation
guards fail at their intended assertions, restore byte-exactly and pass their restored controls.
[Core CI 36056471336](https://github.com/Thalpy/Mewtual/actions/runs/36056471336) passes on Linux and
Windows: **295 library tests and all 25 mutation/restoration/control checks on each platform**.
Actual CI merge checkout: `580c1b0a1aea93502506cd42b5eb4b217ac6ebaf` (Agent 1 base `48f9069`).

Required local validation is complete:
`cargo test -j 1 --all --all-features --no-fail-fast -- --test-threads=2` PASS
(1,846 passed, 13 existing ignored; includes replication 295 + 47 integration);
`cargo test -j 1 --manifest-path apps/desktop/src-tauri/Cargo.toml` PASS (261 unit + 5 ACL);
`npm.cmd --prefix apps/desktop test` PASS (1,229). `cargo fmt --all -- --check`, explicit rustfmt checks
for both included test files, and `cargo clippy -j 1 --all-targets --all-features -- -D warnings` PASS.
`bash scripts/check-no-ambient.sh` still fails the seven untouched calls listed below;
`cargo deny check` still fails unchanged rustls 0.23.40 / RUSTSEC-2026-0285. Broader CI `36056471088` also has
completed failures in native unused-code checks and cargo-deny. No integrated Gate 4 PASS is claimed.
Startup/flow gates do not apply to this core-only change. Shared integration remains Agent 4-owned.

## C-1/C-2/C-5/C-6 implementation checkpoint, 2026-09-24

Implemented the shared atomic gate/book repair transition and typed Studio/Registry adapters,
strict v3 action provenance, successor propagation, and whole-source Repair adoption/recovery.
Accepted operations remain intact; only an Open repair clears rejected quarantine. Current
authority precedes retry, screening preserves unrelated faults, and cross-tenure choices never
authorize installing the old owner's seed. Ordinary adoption cannot erase a pending continuation.
No runtime/store caller uses these new APIs yet; historical admission, custody, joint journal/source
compatibility and the B1-through-recycling durable claim remain integration work.

Implementation review found two HIGH issues (covered live roles accepted by v3 restore and
ordinary adoption bypassing pending repair) and one MEDIUM (39-byte Registry protocol undercount).
All are fixed with regressions; the two HIGH tests failed before the fixes. A LOW coverage gap is
closed by re-admitting the same real quarantined envelope after Open repair and restart.
Final bounded re-review found no remaining BLOCKER/HIGH/MEDIUM. Code checkpoint: `27bab16`.
Core CI `36005323406`: **273 library tests and all 20 mutations, exact restorations and restored
controls PASS on Linux and Windows**. Local focused repair tests: 50 unit + 1 integration PASS.
Full native suite: 261 unit + 5 ACL PASS; frontend: 1,229 PASS; formatting and strict workspace
Clippy PASS. Full root tests were run, then retried outside the sandbox with two test threads and
`--no-fail-fast`: app and replication unit processes exited abnormally (`0xffffffff`) without an
assertion report; remaining targets completed, including all 47 replication integration tests.
A separate full replication run also exited abnormally; an isolated app diagnostic passed.
The local exit cause is unresolved; independent complete core CI is green, not a full-workspace
PASS. Full CI `36005323438` failed the unchanged owner-return scheduling test on both platforms,
native unused-code diagnostics under `-D warnings`, and cargo-deny. Existing ambient failures (seven untouched calls) and cargo-deny
rustls 0.23.40 / RUSTSEC-2026-0285 remain. Startup/flow gates are not applicable to this core change.

The C-2 table and C-5 predicate now state the tested edge cases precisely: covered opening takes
precedence over ordinary settlement; a retained seal must match inheritance and not be covered;
Screened never owns installation; healthy cross-tenure sources preserve their roles. Shared
architecture/interface/threat/handover integration remains Agent 4-owned, as below.

Prior C-7 verification is now complete: native 261 unit + 5 ACL tests PASS; strict workspace
Clippy PASS; core CI run `35859493103` passed on Linux and Windows (243 tests and all 14 then-current
mutations). Full CI `35859493193` failed unchanged Studio exchange scheduling tests, native unused
code under `-D warnings`, and the rustls advisory. It was not a full-suite PASS.

## C-7 implementation checkpoint, 2026-09-23

The user's independent revision-16 verdict is **PASS** at `2b741e7`: CORE-006/007 and the
TEST-014 design plan are closed; C-7 implementation is authorized. Earlier entries below are
historical. The accepted bounded limitations remain; integrated TEST-014 execution is pending.

Implemented `epoch/owner_journal.rs`: distinct publication/pending/reconciled roles, full retired
receipt/close retention, bounded provenance, repeated repair, both finalization orders, strict
v2 restore and exact v1 compatibility. No store/runtime caller uses the new repair APIs yet.
Historical admission, global sequence, B1-B6/source compatibility and custody remain integration work.

Read-only adversarial implementation review and re-review found and closed one HIGH: an active
pending receipt could equal historical high-water after two repairs and never complete. Its test
failed before the fix and passes afterward in both finalization orders. No blocker/high/medium
remains in this leaf review. The mutation harness now pins that defect plus four other C-7 guards.

Verification: 15 focused journal tests PASS; the full replication library passed 242 tests before
that final regression/fix; frontend 1,229 tests PASS; replication strict Clippy PASS. All **14
mutations, byte-exact restorations and restored controls PASS**, including five C-7 guards.
The required ambient check fails on the same seven untouched calls recorded below; cargo-deny
fails on unchanged rustls 0.23.40 / RUSTSEC-2026-0285 (bans/licenses/sources pass). Native tests
and final full CI remain in progress; no full-suite or integrated repair PASS is claimed.

Agent 4 integration note (shared documents are left to their owner): the journal cap is now
**12,288 bytes**, propagating to **17,448 sealed owner-record bytes**. Update THREAT-MODEL's
8,488-byte figure and no-journal-rebase statement, and INTERFACES' no-rebase-API statement when
integrating this core. The leaf exists; durable runtime repair does not. Startup/flow gates are
not applicable to this core-only change.

## Independent review response, 2026-09-23: revision 16

Latest user-supplied review base: **`a7513697fb482abc235cadb8414d491f7851b6ce`**.
The [finding ledger](GATE4-AGENT-3-CORE-REVIEW.md) records the split disposition:

- **IMP-001 PASS/CLOSED.** The implementation and independent v4/v5 regressions are accepted;
  final core CI is green on Linux and Windows. No further codec change is made here.
- **CORE-005 bounded design PASS**, including the finite-history, late-reporter and device-local
  authority limits. Universal peer convergence is not a claim. Agent 2 must still agree/implement
  the archived Observed witness and matching durable capability; N49/N50 production-consumer
  tests and Agent 4's snapshot integration remain required.
- **CORE-001/002/003 remain accepted.** No new approval is requested for them.
- **CORE-004 REQUEST CHANGES:** CORE-006 (P1) replaces effect equality with joint compatibility;
  CORE-007 (P1) moves the earliest possible owner-turnover hold to B1, before B2.
- **TEST-014 (P2):** N3b now includes source Transitioned/journal NoChange with H as published
  and pending; N51 adds the reverse Screened/Replace and negative authority/evidence cases;
  N52 covers observed owner turnover after B1 before B2 and separately after B2 before B6,
  including a journal NoChange. These integrated tests are planned, not implemented or executed.

The [revised journal contract](GATE4-AGENT-3-AUTHORITY-FOLLOWUP.md#core-006-compatible-independent-source-and-journal-effects)
checks independently computed source and journal candidates. No proven losing decision may
remain usable for proof, settlement or installation; guarded losing recovery/history is retained.
Different non-losing heads are compatible without weakening the existing three-way proof check.
A journal NoChange still persists the owner repair and its target claim.

The proposed turnover limitation covers **any time after B1 until durable terminal/recycling**.
Pre-B2, the source is unchanged but the owner record may already have reconciled the journal or
retired pending evidence. Post-B2, committed source/recovery work may also be stranded. Every
ordinary prepare/reset/proof/publication path must respect the persisted repair claim, even
without journal provenance. Old issuer authority is never renewed by restart. No cancellation is
introduced; same-transaction authorized progress and already-terminal cleanup retain their rules.

**Scope:** four Agent 3 documentation files only. No code, schema, native registration, shared
workflow/lockfile, Agent 1/2 files or refs changed. C-7 remains unimplemented pending independent
re-review of these corrections. There is still no runtime report-admission/no-write regression;
the C-4 member-forgery primitive test does not substitute for N49.

**Internal review complete:** the read-only reviewer inspected the actual four-document diff
against `a751369` and neighboring enforcement paths. No BLOCKER/HIGH/MEDIUM remains preventing
independent re-review. One LOW accepted/proposed label mismatch was corrected and re-reviewed.
The review confirms the independent effects, B1/NoChange fence and test matrix; no Cargo was run.
It cannot substitute for the user's verdict. Only CORE-006/007 and TEST-014 are the new
independent review request; accepted CORE-005 is preserved.

### Verification

No backend/frontend suites are rerun for this documentation-only revision: it changes no runtime
or build behavior. Diff whitespace and contract-reference checks pass; read-only adversarial
design review/re-review completed as above.
The previous implementation's final execution evidence is now complete for the focused boundary:

- Local `cargo test --locked -j 1 -p catcoms-replication --lib epoch::repair_state::tests::`:
  **20/20 PASS** at `27e1b9998022c1dcaa9e3c411909467d4fbeb383`.
- [Final core/mutation CI](https://github.com/Thalpy/Mewtual/actions/runs/35811047096):
  **Linux and Windows each PASS 228/228 tests and all nine intended mutations, exact restorations
  and nine restored controls.** Both actual merge checkouts are
  `0651da0ac579168938005b0cbbe161e3ca86b1c2`, into base
  `28bb73d2e29ee03841ebe053afad203602df872e`. Both logs were inspected; ignored local copies
  are `logs/agent3-core-27e-linux.log` and `logs/agent3-core-27e-windows.log`.
- [Full required CI](https://github.com/Thalpy/Mewtual/actions/runs/35811047134) at that code:
  frontend **1,229/1,229**, frontend check/build and Linux/Windows formatting/strict Clippy pass.
  Native full tests fail during compilation on unchanged unused-code diagnostics under
  `-D warnings`; the later native check is not reached. Cargo-deny fails on unchanged
  rustls 0.23.40 / RUSTSEC-2026-0285 (bans/licenses/sources pass).
- Root full suites remain **in progress**, so their subsequent ambient gates have not run.
  The local ambient gate was run and failed on the seven untouched calls recorded below.
  No full-suite, integration or Gate 4 PASS is claimed. Startup/flow gates do not apply to this
  documentation revision or the unchanged core decoder/test scope.

## Implementation restart, 2026-09-23

Agent 3 now works in **`gate4-agent3-repair`**, at
`M:\Git (local)\CatComs\target\gate4-agent3-repair`, isolated from the mutable Agent 1/2
checkout. Base: **`918ffb9b3034c43ed33574225e04f1be2e87090d`**. No shared branch is switched,
reset, rebased or force-pushed. Commits use explicit Agent 3 pathspecs; mutation scripts run
only on this worktree. The older shared-checkout description below is historical.

**First implementation checkpoint: C-3, C-4, C-8 and the C-5 sequence accessor only.**
This is not the complete core repair transition, a store transaction, runtime repair, native
exposure, or Gate 4 acceptance. No current source can leave Fault through this checkpoint.

- C-4: `conflicting_receipt_pair` checks bounded canonical full receipts, historical signatures,
  full logical scope and genuine same-tenure conflict, without returning authority.
  `ReceiptRepair::check_evidence` adds the exact pair, selection, fault tenure, v2 and sequence
  bindings. `ResolvedRepair::verify` reuses it while retaining signature, role, enclosing-document
  and stored-sequence checks. The live authority guard and `apply_repair` retry ordering remain.
- C-3: verified losing receipts remain preserved evidence but cease to be adoption anchors,
  both during admission and restart. Unrepaired newer anchors and genuine third baselines refuse.
- C-8: only repair-bearing book versions 4/5 can derive identity from resolved evidence when
  there is no current head. Latest/tenure consistency, retained scope and legacy framing remain.
- Eight new core regressions exercise these leaves. `scripts/check-agent3-core-mutations.py`
  covers eight mutations: M5; admission and both restart-anchor checks; exact-hash-only
  narrowing of each of those three checks; and headless identity. Each mutation requires
  exact restoration and a passing control. Descendants on the provably losing inheritance
  baseline are exercised as both retained anchor kinds; unknown same-baseline ancestry
  still cannot authorize rollback.

**Verification is recorded below.** Local compiled checks wait for the other agents' Cargo
commands to finish; no separate large target directory or competing build is created.

**Dependency refresh against the base:** Agent 1's `EpochMutation` capability exists in
`store.rs`; all future repair writes must use it, including retry/sync and failed-I/O paths.
C-3's storage scanner exists; runtime adoption remains Agent 1's checkpoint. Agent 2's
`verification_owner_tenure_start` / `authoring_owner_tenure_start` split still does not exist.
Agent 3 accepts T1-T5 as written and will not substitute the single current accessor, a carried
receipt field or the current MLS epoch for authoring evidence. This leaf checkpoint consumes
neither runtime dependency. Agent 4 continues to own shared registrations, workflows and docs.

**Read-only preimplementation review found four confirmed design/source mismatches** in
the remaining C-1/C-2/C-7 work: Open quarantine restoration, screening while retaining a
different Fault, consecutive unpublished reconciliations, and differing-baseline journal
publication evidence. They are recorded in
[the core follow-up review](GATE4-AGENT-3-CORE-REVIEW.md). Those transitions are not implemented
by this checkpoint; the revision-14 PASS is not claimed to settle these newly demonstrated paths.
The second design audit confirmed CORE-001/002 and the direct canonical-head correction in
CORE-003, but found three remaining journal decisions in CORE-004. That document labels them
explicit open questions, not an accepted or total contract.

The read-only implementation review found no BLOCKER/HIGH/MEDIUM defect in the leaf checkpoint.
Its one LOW gap (retained losing-baseline descendant tests) was fixed and statically re-reviewed,
including the three added isolating mutations. The review executed no Cargo commands and does
not replace the user's independent review or execution evidence.

Proposed shared documentation/UI update for Agent 4: document the evidence-checking APIs and
headless book restore as core prerequisites; keep the current unavailable-native-repair row
unchanged. In particular, do not claim `Repairing`, an owner choice, recovery-before-replacement,
or peer convergence from these unit tests.

### Current checkpoint and follow-up verification

Implementation: `66933c96a27ab76bac48f9bf0f540a8317ea9158`.
Reviewed regression expansion: `703c84d86dab41c3dc4e885835ad3eb1300bffd5`.
Separate branch-local verification wiring: `1a2ec4dfd7d848bf3a59d6fff6e12e24ee3bdcdc`.
Only tests/docs/mutation controls changed after the first implementation commit; the wiring
commit changes only its new workflow and this status. No existing common workflow was edited.

[Expanded-regression CI on the original base](https://github.com/Thalpy/Mewtual/actions/runs/35803402782)
uses head `703c84d` and actual merge checkout **`182b48212768a52aa0e73c3c61f5e508bc622d85`**,
confirmed in its desktop checkout log. Frontend tests (1,229/1,229), check and build passed again;
native full tests again stopped during compilation on the same untouched unused-code diagnostics.
Formatting and strict Clippy also passed on both platforms with the expanded regressions.
Root full suites are still running at this entry. The branch sources differ from `1a2ec4d`
only in its new test workflow/status, but their **PR merge bases differ**: Agent 1 advanced
`gate4-agent1-runtime` from `918ffb9` to `397f6895166ccabb95056396ff373199f0076f87` while
verification was in flight. The later full run `35804059956` was initially cancelled as a
duplicate; the checkout audit caught this distinction and it was **restarted as attempt 2**:
[newer-base full CI](https://github.com/Thalpy/Mewtual/actions/runs/35804059956/attempts/2).
It is running, not a PASS; record its actual checkout and results on completion. The older
`66933c9` full run `35802458116` stays cancelled after retaining its partial logs. A cancellation
is not verification evidence. No local rebase/merge or Agent 1/2 ref mutation was performed.

[Dedicated repair-core run](https://github.com/Thalpy/Mewtual/actions/runs/35804060155)
at `1a2ec4d` **PASSED on Windows and Linux**, each with **225 passed, zero failed/ignored**
in the complete replication library. Both checkout logs confirm actual PR merge
**`dda15bdcefb0cf43299a8f106ac0a360c489715d`**, merging `1a2ec4d` into the newer `397f689` base.
On each platform all eight mutants failed at the intended named assertion, every source was
restored byte-for-byte, and each of the eight restored exact controls passed. Both sets of raw
logs were downloaded and checked: eight mutant logs and eight passing controls per platform.
Artifacts: [Linux](https://github.com/Thalpy/Mewtual/actions/runs/35804060155/artifacts/10727522307),
[Windows](https://github.com/Thalpy/Mewtual/actions/runs/35804060155/artifacts/10728035646).
Local copies are under ignored `logs/agent3-core-ci-linux/` and `logs/agent3-core-ci-windows/`.

The new workflow received a read-only static review with no findings. It has read-only permissions,
serial locked builds and always-attempted evidence upload. The review did not execute its commands.
No root-workspace, native, runtime, transport or full-Gate-4 PASS is inferred from this library run.

The user has been asked for the independent CORE-001--004 design review required by the common
handoff before those new gate/journal boundaries are implemented. No verdict is inferred from
silence, routine implementation authorization, the internal static review, or these leaf tests.

### Verification evidence, initial leaf checkpoint

Draft PR: [#28](https://github.com/Thalpy/Mewtual/pull/28), base `gate4-agent1-runtime`,
head `66933c96a27ab76bac48f9bf0f540a8317ea9158`. No merges or shared-ref changes were made.
[Initial required CI](https://github.com/Thalpy/Mewtual/actions/runs/35802458116)
checked out PR merge **`2e9721fa2d6072e2f42c8d69dec4ec39907d4d7c`** (desktop job checkout log).
The subsequent test-only descendant expansion does not change production code, but still
needs execution at its own head. Current observations:

| Check | Result |
|---|---|
| Local `cargo fmt --all -- --check` and `git diff --check` | PASS, including descendant expansion |
| Python script parse and unique anchors for all eight mutations | PASS; this is not mutation execution |
| CI root formatting and strict Clippy, Linux and Windows | PASS on initial leaf checkpoint |
| CI `cargo test --all --all-features`, Linux and Windows | Running at this entry; not yet a PASS |
| CI `npm --prefix apps/desktop test` | PASS: 1,229 passed, zero failed |
| CI frontend `check` and `build` | PASS: zero check errors/warnings; Vite build completed |
| CI native `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | Attempted; compilation blocked by existing unused-code diagnostics under `RUSTFLAGS=-D warnings`, before tests |
| CI native `cargo check` | Not reached after native compilation failure |
| CI ambient-dependency gate | Pending behind root suite at this entry |
| CI `cargo deny check` | FAIL: existing `rustls 0.23.40`, `RUSTSEC-2026-0285`; bans/licenses/sources passed |
| Agent 3 focused mutations | Prepared; not yet executed |

This table is the initial-run record; the later dedicated mutation PASS above supersedes its last
row. The final evidence update changes documentation only, so it does not rerun or queue another
copy of the expensive suites. Existing complete-suite runs continue and remain explicitly pending.

Native diagnostics are `SourceFormat::as_mime` (`src/media_decode.rs:109`) and unused
security-intent APIs/fields (`src/security_intent.rs`, including `PendingApproval` fields at
line 155). Those files and both lockfiles are unchanged by Agent 3. The supply-chain log
recommends rustls >=0.23.45. Agent 4 owns the dependency/native integration disposition;
this checkpoint adds no advisory ignore, warning suppression or unfinished native registration.
Failure logs: [native/frontend](https://github.com/Thalpy/Mewtual/actions/runs/35802458116/job/106995653398),
[supply chain](https://github.com/Thalpy/Mewtual/actions/runs/35802458116/job/106995653449).

No startup, frontend-flow or visual gate is claimed: these leaves change no setup, process,
UI, send/friend flow or native command. The separate
`.github/workflows/agent3-repair-core.yml` is branch-local verification wiring, added in its own
identifiable commit so the complete replication suite and eight mutations can execute without
waiting on the shared local build target. It changes no existing common workflow. Agent 4 owns
integration and required-check configuration; the exact patch is also supplied as
[GATE4-AGENT-3-CI.patch](GATE4-AGENT-3-CI.patch). Its presence is not execution evidence.

**Local focused execution at `703c84d86dab41c3dc4e885835ad3eb1300bffd5`:**
`cargo test --locked -j 1 -p catcoms-replication --lib epoch::repair_state::tests::` passed:
17 passed, zero failed/ignored (includes the eight new regressions). It started only after
the other agents' Cargo work became idle and reused the existing target directory.
The eight-mutation local runner waited up to 15 minutes for their next full app suite to finish,
then exited `NO_LOCAL_SLOT` without invoking Cargo or touching source. No local mutation evidence
is claimed; its execution moved to the dedicated hosted workflow. No local Agent 3 process remains.

The local ambient-dependency script was also executed and **failed** on seven existing calls:
`Instant::now` in native `media_decode.rs` lines 333/426/444/504, and `tokio::time::sleep` in
`studio/receiver/catchup/tests.rs:1848`, `studio_exchange/tests/scheduling.rs:150`, and
`tests/support/studio_preview.rs:329` under `catcoms-app`. All four files are byte-identical
to the base; Agent 1's existing status already records the ambient gate as red. This failure
is retained explicitly for Agent 4 and is not suppressed by the new focused workflow.

## Checkpoints

| Date | Checkpoint | Base | Head | Kind | Verdict |
|---|---|---|---|---|---|
| 2026-09-15 | Design revision 1 | `1bcb1bca204d721b848b17c0835faf931ae930e3` | `7efc9c2aba0a37d9aec57e268d9ff63edaca1b8a` | design, docs only | **REQUEST CHANGES**: AG3-DES-001 to AG3-DES-008, AG3-TEST-001; U-1 to U-6 decided |
| 2026-09-15 | Design revision 2, findings answered | `7efc9c2aba0a37d9aec57e268d9ff63edaca1b8a` | `63a11e1a6451c7ed373c90b0e81b59d8a748a72a` | design, docs only | **REQUEST CHANGES**: AG3-DES-003, 005, 007, 008 corrections **accepted**; new AG3-DES-009 to AG3-DES-012; M2 and M12 non-isolating; U-7 and U-8 decided |
| 2026-09-16 | Design revision 3, second-round findings answered | `63a11e1a6451c7ed373c90b0e81b59d8a748a72a` | `a62178b94f20cd60a5363e6a3d6d6216edb9e516` | design, docs only | **REQUEST CHANGES**: historical-pair direction, 2a/2b split, v2 framing, U-7, U-8 and M12 **accepted**; new AG3-DES-013 to AG3-DES-016; M2 still masked; U-9 and U-10 decided |
| 2026-09-16 | Design revision 4, third-round findings answered | `a62178b94f20cd60a5363e6a3d6d6216edb9e516` | `ad023d2e5b514a8f9598b1fe433fd2b080ecad6c` | design, docs only | **REQUEST CHANGES**: C-8 epoch-zero fix, reconciled lifecycle, healthy-source clobber fix and M2 **accepted**; new AG3-DES-017 to AG3-DES-021; AG3-TEST-003 on M12/N24/N5d; U-11 agreed |
| 2026-09-16 | Design revision 5, fourth-round findings answered | `ad023d2e5b514a8f9598b1fe433fd2b080ecad6c` | `3737f1d4fff6f2c08302b0ecb1b024008818a1cf` | design, docs only | **REQUEST CHANGES**: AG3-DES-021 **closed**; case 6c/N32, the sequence concept and M2 accepted; new AG3-DES-022 to AG3-DES-025; AG3-TEST-004 on N5c/N18/M12 |
| 2026-09-16 | Design revision 6, fifth-round findings answered | `3737f1d4fff6f2c08302b0ecb1b024008818a1cf` | `135766ca9f290ab96d3133b771bc42da74fe7825` | design, docs only | **REQUEST CHANGES**: AG3-DES-023 and AG3-DES-024 **closed**; fresh-path AG3-DES-025 and the AG3-TEST-004 corrections accepted; new AG3-DES-026 to AG3-DES-029; AG3-TEST-005 |
| 2026-09-16 | Design revision 7, sixth-round findings answered | `135766ca9f290ab96d3133b771bc42da74fe7825` | `6d498c2e901e5071104a2533e0a632dc5676b2a7` | design, docs only | **REQUEST CHANGES**: AG3-DES-029 **closed**; source-bound encoding shape and M14 accepted; new AG3-DES-030 to AG3-DES-033; AG3-TEST-006 |
| 2026-09-16 | Design revision 8, seventh-round findings answered | `6d498c2e901e5071104a2533e0a632dc5676b2a7` | `b8bb5f3a3db6d0b8e450c82119f95e2cb929bce6` | design, docs only | re-review requested |
| 2026-09-16 | Revision 8.1, `DraftArchive` seam consumed | `b8bb5f3a3db6d0b8e450c82119f95e2cb929bce6` | `76b854494ff8f30eb0d5844e22a783d6927ba182` | design, docs only | no rebase needed: `705d44b` is already an ancestor |
| 2026-09-16 | Design revision 9, eighth-round findings answered | `76b854494ff8f30eb0d5844e22a783d6927ba182` | `3b6a4b40462ae83a341f8f6741c93edff55b5ef7` | design, docs only | **REQUEST CHANGES**: AG3-DES-038 **closed**, owner-side AG3-DES-034 accepted; new AG3-DES-039 to AG3-DES-044; AG3-TEST-008 |
| 2026-09-16 | Design revision 10, ninth-round findings answered | `3b6a4b40462ae83a341f8f6741c93edff55b5ef7` | `11ce6f1b58288e44d6ff14dc2a98f42f7cc5e13b` design body, `f5ac522eff264eeddca30c2c4176cacfd723a158` status | design, docs only | **REQUEST CHANGES**: AG3-DES-040 (6.3/13.2) and AG3-DES-043 **closed**, AG3-DES-044 write order accepted; new AG3-DES-045 to AG3-DES-049; AG3-TEST-009 |
| 2026-09-17 | Design revision 11, tenth-round findings answered | `11ce6f1b58288e44d6ff14dc2a98f42f7cc5e13b` | `333924318a65210375fa992bc49006c2fac3c236` | design, docs only | **REQUEST CHANGES**: AG3-DES-045, 048 and 049 **closed**; new AG3-DES-050 to AG3-DES-053; AG3-TEST-010 |
| 2026-09-17 | Design revision 12, eleventh-round findings answered | `333924318a65210375fa992bc49006c2fac3c236` | `04b27f7dc59f917e556c5a30d1a609f6b211ab32` | design, docs only | **REQUEST CHANGES**: AG3-DES-050, 051, 052 and 053 **closed**; one new finding, AG3-DES-054, plus AG3-TEST-011 |
| 2026-09-17 | Design revision 13, twelfth-round finding answered | `04b27f7dc59f917e556c5a30d1a609f6b211ab32` | `df1a8fe8828b6e7abfeaf0bbed42eed75ef3f9a6` | design, docs only | **REQUEST CHANGES**: AG3-DES-054's mechanism accepted; one new finding, AG3-DES-055, plus AG3-TEST-012 and two editorials |
| 2026-09-17 | Design revision 14, thirteenth-round finding answered | `df1a8fe8828b6e7abfeaf0bbed42eed75ef3f9a6` | `d48280012cc653b458b1e3ce00b49f6ae2f23c0e` | design, docs only | **PASS**: AG3-DES-055 and AG3-TEST-012 closed; bounded repair design accepted |
| 2026-09-17 | Editorial follow-up to the PASS | `d48280012cc653b458b1e3ce00b49f6ae2f23c0e` | this commit | docs only | the reviewer's remaining nit; no design change |

Working checkout: `M:\Git (local)\CatComs`, shared with the parallel Agent 1 and Agent 2 sessions,
which are now doing implementation and design work respectively. Agent 1 has the checkout on its
own branch, so this pass commits there rather than switching branch, touching only the two Agent 3
documents with explicit pathspecs. `Create-suite-2` was fast-forwarded once, at revision 2, to keep
these documents off Agent 1's branch alone; it has not been moved since. **Agent 3 implementation
must move to a separate branch or worktree before any code change**; no mutation harness may run
against another agent's source.

## Design verdict

**PASS at `d48280012cc653b458b1e3ce00b49f6ae2f23c0e`**, bounded to the Agent 3 repair **design**.
Fourteen revisions, fifty-five numbered findings and twelve test-plan findings, all closed.

What this PASS does **not** cover, and what therefore gates the next step:

| Not accepted | Owner | State |
|---|---|---|
| Any implementation of this design | Agent 3 | not started; must move to a separate branch or worktree first |
| Mutation execution and CI evidence | Agent 3 | **no Cargo, npm or script command has been run in any of the fourteen passes** |
| Live tenure seam T1 to T3 | Agent 2 | design PASSED, no implementation |
| I-4 and C-3 runtime seams | Agent 1 | not started |
| Core handoff signing split `e65bfd8` | core | still unreviewed |
| Integration and full Gate 4 acceptance | Agent 4 | separate review |

Gate 5 stays closed.

## Finding ledger, revision 13 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-055 | P1 | **Answered in revision 14.** The turnover assignment was unconditional, so read literally it also overwrote a hold already for the current tenure and kept only the newest fingerprint, reopening AG3-DES-048. The transition is now total: absent creates, stale replaces, same tenure accumulates. | Design 5.2, 15.1 N44(b) |
| AG3-TEST-012 | P2 | **Answered.** N44(b) asserts the exact durable transition after each same-tenure admission and that it never takes the replacement branch, so N44(b) and N46 fail under each other's behaviour. | Design 15.1 N44(b) |
| Editorials: stale `f5ac522` reference, and an earlier-revisions list skipping 10 to 12 | - | **Both corrected.** | Design 17 |

Closed in the revision-13 round: everything except AG3-DES-055; AG3-DES-054's mechanism accepted.

## Finding ledger, revision 12 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-054 | P1 | **Answered in revision 13.** A stale hold from a previous tenure still occupied the single slot, and nothing said what became of it when the next tenure overflowed; the "refuse, slot occupied" reading reopens AG3-DES-032 after a restart. Turnover is now an explicit atomic replacement in the same owner-record write, only under a positively observed authoring tenure, never a merge. | Design 5.2, 15.1 N46 |
| AG3-TEST-011 | P2 | **Answered.** N46 covers T1 overflow, owner change, T2 conflict at exhausted capacity, replacement rather than merge, crash and reopen, no proof for an unrelated T2 request, and no turnover under `Unknown` or `Imported`. | Design 15.1 N46 |
| Editorial, N43 named `live_overflow` | - | **Corrected** to the `OverflowHold` field. | Design 15.1 N43 |

Closed in the revision-12 round: **AG3-DES-050, 051, 052 and 053**. Still closed: AG3-DES-045, 048,
049, 040, 043, 044's write order, 038, 030, 023, 024 and 029's exact-pair dedupe rule. Accepted:
M1 to M19.

## Finding ledger, revision 11 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-050 | P1 | **Answered in revision 12.** `receipts_conflict` admits a pair differing only in `TenureSelection`, while `check_opening_receipt` also demands equal closed epochs, so the head-or-opening approximation classified an undrainable pair as materialisable and then barred it from direct issuance. Materialisability is now a dry run on a clone that must reproduce the pair byte-for-byte. | Design 5.2, 15.1 N45(e) |
| AG3-DES-051 | P1 | **Answered.** A reserved pair may share a receipt with an external while two externals may not, so a numerically free slot can be structurally inadmissible. "Room" now means an admissible slot, and an inadmissible pair stays reserved as kind 3. | Design 5.2, 15.1 N45(c) |
| AG3-DES-052 | P2 | **Answered.** `check_scope` enumerates all four bindings and their illegal combinations. | Design 5.2, 15.1 N45(f) |
| AG3-DES-053 / AG3-TEST-010 | P2 | **Answered.** The stale bare-start demotion prose is rewritten to the `OverflowHold` algorithm, canonical invariants added including the inert non-live shape, and N37(g) realigned with N45(c). | Design 5.2, 15.1 N37(g), N44(e) |

Closed in the revision-11 round: **AG3-DES-045, AG3-DES-048 and AG3-DES-049**. Still closed:
AG3-DES-040, 043, 044's write order, 038, 030, 023, 024 and 029's exact-pair dedupe rule. Accepted:
M1 to M19.

## Finding ledger, revision 10 round

All five were internal contradictions in revision 10 itself, and all five are confirmed.

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-045 | P1 | **Answered in revision 11.** The overflow hold was added to the struct and the proof gate but never to the codec, so the mechanism described as surviving restart had no bytes and N43 was impossible as written. It now has a canonical encoding, validation and a corruption rule. | Design 5.2, 15.1 N44(a) |
| AG3-DES-046 | P1 | **Answered.** "Must drain first" and "cannot be drained, repair directly" both applied to `{R1,R3}`, leaving no legal next step. `pair_is_materialisable`, computed without mutating the source, splits the two. | Design 5.2, 15.1 N45(a)(b) |
| AG3-DES-047 | P1 | **Answered.** Migration now precedes any B1 so the binding is deterministic, and recycling is binding-specific so a terminal kind-3 pair is cleared from `reserved` rather than stranded there. | Design 5.2, 15.1 N45(c)(d) |
| AG3-DES-048 | P1 | **Answered.** The hold keeps up to four individually cleared pair fingerprints plus a sticky `unknown` flag, so one reporter's successful retry can no longer forget another known conflict. | Design 5.2, 6.6, 15.1 N44(b)(c) |
| AG3-DES-049 | P1 | **Answered.** The stale `observed_tenure_id` pseudocode is gone; both predicates use one `tenure_id` derived from the authoring accessor, and the hold stores that id rather than a bare start epoch. | Design 5.2, 6.6, 15.1 N44(d) |
| AG3-TEST-009 | P2 | **Answered.** N44 and N45. | Design 15.1 |

Closed in the revision-10 round: **AG3-DES-040** in 6.3 and 13.2, and **AG3-DES-043**.
AG3-DES-044's write order accepted. Still closed: AG3-DES-038, 030, 023, 024 and 029's exact-pair
dedupe rule. Accepted: M1 to M19 where previously accepted.

## Finding ledger, revision 9 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-039 | P1 | **Answered in revision 10.** `repair_kind` had no value for the reserved slot, so a pair there could be selected as active work and then not be nameable at B1. A fourth kind binds directly to it. | Design 5.2, 15.1 N37(g) |
| AG3-DES-040 | P1 | **Answered, and a missed dependency change absorbed.** Agent 2's accepted design removes `observed_owner_tenure_start` for a verification/authoring split with a fail-closed `Imported`; 6.3 now has a per-use accessor table binding every mutation, drain and issuance to the authoring accessor, with identity compared as a derived `tenure_id`. T1 to T3 restated. | Design 3 R32, 6.3, 6.6, 13.2, 15.1 N42 |
| AG3-DES-041 | P1 | **Answered.** Demotion migrates into the history list when there is room; where there is not, a new live conflict sets a durable evidence-free hold carrying its tenure, which suppresses proof while the reporter retries. | Design 5.2, 6.6, 15.1 N43 |
| AG3-DES-042 | P1 | **Answered.** `is_repaired_loser` screens before conflict handling, so a reported pair cannot always be re-derived through the live seal. Admission keeps it owner-side under the proof gate, repairable through `repair_kind 3`. | Design 3 R33, 6.5, 15.1 N39 |
| AG3-DES-043 | P1 | **Answered.** A peer has no owner record and no `resolved_repair` before B2, so `target_is_claimed` adds a runtime claim acquired at S1 and owned through S4. | Design 10.3, 15.1 N37(h), M18, M19 |
| AG3-DES-044 | P1 | **Answered.** The drain is its own crash-safe transaction: source fault write and durability first, slot cleared second, duplicates idempotently cleaned. | Design 5.2, 15.1 N37(i) |
| AG3-TEST-008 | P2 | **Answered.** N39 exact-pair identity, N42 tenure states, N43 repeated-tenure capacity, N37(h)(i), M18 and M19. | Design 15.1, 15.2 |

Closed in the revision-9 round: **AG3-DES-038**, and the owner-side half of AG3-DES-034. Still
closed: AG3-DES-030, 023, 024 and 029's exact-pair dedupe rule. Accepted: M14 and M1 to M11.

## Finding ledger, revision 8 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-034 | P1 | **Answered in revision 9.** Section 10.3 still used `repair_install_pending()` as the claim predicate, the very one shown to start too late, while `advance_checkpoint` calls `install_studio_seed_step`, a real source-mutating install. `repair_transaction_nonterminal` is now the single target-claim fence for every source-mutating path. | Design 3 R31, 10.3, 15.1 N37(e), M17 |
| AG3-DES-035 | P1 | **Answered.** The reserved slot was on the wire and in the proof gate but absent from the struct and `check_scope`. It is now explicit with full codec and validation invariants, and its alias rule permits a shared receipt because the accepted three-receipt shape requires it. | Design 5.2, 15.1 N41 |
| AG3-DES-036 | P1 | **Answered.** A live reserved pair now outranks all historical work in the active-pair derivation, and the repair field cannot be reused for an external while it waits, closing the post-recycle crash hole. | Design 5.2, 15.1 N37(f) |
| AG3-DES-037 | P1 | **Answered by making liveness derived rather than stored.** Recomputing it from freshly observed tenure at every custody visit makes demotion free, removes the capacity question, and stops a stale pair either being sealed under the wrong owner or suppressing proof forever. | Design 5.2, 6.6, 15.1 N37(g) |
| AG3-DES-038 | P2 | **Answered.** "`adopted_successor` is unchanged" is deleted; every successor constructor clones the book, the disposition and its exact repair binding. | Design 5.1 C-6 |
| AG3-TEST-007 | P2 | **Answered.** N37(e)(f)(g), N41, M17, and N36(b)'s stale seven-receipt wording corrected to nine. | Design 15.1, 15.2 |

Closed in the revision-8 round: **AG3-DES-030** at the mechanism level. Still closed: AG3-DES-023,
AG3-DES-024, AG3-DES-029. Accepted and not reopened: M14 and M1 to M11.

## Finding ledger, revision 7 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-030 | P1 | **Answered in revision 8.** `apply_repair` clears `fault` on success, so "inline hashes must match the live fault" could only hold before B2 and stranded every resume afterwards. Binding is now two-phase, against `fault_evidence()` pre-B2 and against the committed `repair_state()` pair post-B2, with a missing or corrupt source refusing in either phase. | Design 3 R29, 5.2, 15.1 N36(f)(g) |
| AG3-DES-031 | P1 | **Answered.** `repair_install_pending()` starts after B2, so a B1-persisted repair was unfenced and a current-tenure report could degrade a pending case 6c replacement into a terminal 6d screening. One `repair_transaction_nonterminal` predicate spans B1 to recycling. | Design 6.5, 15.1 N37(b), M13, M15 |
| AG3-DES-032 | P1 | **Answered.** A reserved live-conflict slot closes the maximal-capacity state, and one durable `authoritative_proof_allowed` predicate is consumed on every head request rather than only on the admitting exchange. | Design 5.2, 6.6, 15.1 N37(c)(d), M16 |
| AG3-DES-033 | P1 | **Answered on all three points.** Disposition is defined by the operation performed with three values covering every C-2 case; snapshot v3 keeps the adoption bit explicit and binds the tag to the exact repair; and `RepairTransition` plus every successor path carries it, since `adopted_successor` copies only the book. | Design 3 R30, 5.1 C-2, C-5, 15.1 N38 |
| AG3-TEST-006 | P2 | **Answered.** N36 gains post-B2 resumes, N37 becomes four variants including pre-B2 and maximal capacity, N38 covers the full disposition lifecycle, M13 is widened and M15 and M16 added. | Design 15.1, 15.2 |

Closed in the revision-7 round: **AG3-DES-029**. Accepted and not reopened: the source-bound
encoding shape, M14, and M1 to M11.

## Finding ledger, revision 6 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-026 | P1 | **Answered in revision 7.** Revision 6's own headline fix could not cross B1: the codec required a repair to match a retained pair while the correction said the source's pair is never retained. The repair is now tagged, kind 2 being source-bound with its pair **inline**, which keeps the record self-validating given that `decode` sees only the logical document. Bound follows to seven receipt-sized values. | Design 3 R26, 5.2, 10.1, 15.1 N36 |
| AG3-DES-027 | P1 | **Answered.** `Closing -> Fault` is permitted and `repair_install_pending()` requires `Closing`, so a current-tenure report through the live seal would silently abandon an outstanding recovery and replacement. A nonterminal repair now fences report-induced source mutation; proof stays suppressed meanwhile. | Design 3 R27, 6.5, 15.1 N37, M13 |
| AG3-DES-028 | P2 | **Answered.** A disposition tag is persisted in the Studio restart unit's snapshot (version 3), not in core's tested `ResolvedRepair` codec, so an exact retry after a screening application returns `Screened` rather than the false `Repaired`. | Design 5.1 C-5, 5.3, 15.1 N38 |
| AG3-DES-029 | P1 | **Answered.** `is_repaired_loser` is an admission guard, not proof that a different frozen pair is resolved; the old rule would have stranded a peer frozen on `{R1,R3}` after `{R1,R2}` was repaired. The no-op is now the exact pair only. | Design 3 R28, 6.5, 15.1 N39, M14 |
| AG3-TEST-005 | P2 | **Answered.** N36 to N39 cover the four new boundaries; M13 and M14 mutate the two guards this revision adds. | Design 15.1, 15.2 |

Closed in the revision-6 round: **AG3-DES-023** and **AG3-DES-024**. Accepted and not reopened: the
fresh-path half of AG3-DES-025, the AG3-TEST-004 corrections, M2 and M1/M3-M11.

## Finding ledger, revision 5 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-022 | P1 | **Answered in revision 6.** `ReceiptBook.fault` is itself one durable slot, so the source's own pair never needed a record copy; requiring it to be "retained here" is what let two historical pairs crowd out the pair actually blocking the document. The record now holds external pairs only and the derivation falls back to the book. The missing terminal-pair recycling is specified as an explicit, idempotent, crash-safe removal. | Design 3 R25, 5.2, 15.1 N31b, N31c |
| AG3-DES-023 | P1 | **Answered.** Three different bounds coexisted: the new five-receipt formula, the superseded three-receipt paragraph in the same section, and 11.4 KiB in section 10.1. One canonical formula now, stated once, with 10.1 agreeing and the struct display corrected. | Design 5.2, 10.1 |
| AG3-DES-024 | P1 | **Answered.** Core verifies current owner before its retry shortcut on purpose; revision 5 inverted that inside `plan_repair`, an authority-bearing API. Authority and evidence now precede every retry, sequence and in-progress outcome, with the store check named as defence in depth rather than the safety property. | Design 5.1 C-2, 15.1 N35 |
| AG3-DES-025 | P2 | **Answered.** Case 6d leaves a different fault standing, so a distinct `Screened` outcome and pair-specific barrier and crash prose separate a terminal disposition from a usable document. | Design 5.3, 8, 15.1 N33 |
| AG3-TEST-004 | P2 | **Answered.** N5c narrowed to the same-baseline higher head; N18 split into malformed-refuses and valid-unrelated-screens; M12's fixture qualified as historical throughout; Flow D's stale "case 6b" wording replaced by deference to the classification. | Design 7, 15.1 N5c, N18, 15.2 M12 |

Closed in the revision-5 round: **AG3-DES-021**. Accepted and not reopened: case 6c with N32 for
AG3-DES-017, the monotone sequence concept subject to AG3-DES-024's ordering fix, M2, and
M1/M3-M11.

## Finding ledger, revision 4 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-017 | P1 | **Answered in revision 5.** "Not faulted" is not "safe to leave unchanged": neither `prepare_checkpoint_adoption` nor `prepare_settlement` screens the receipt it already holds, so a source sitting on the loser would install or settle it. New case 6c retargets any source whose own `latest`, opening or adoption target is covered; 6b now requires no covered anchor; the same-baseline higher-head limitation is asserted rather than papered over. | Design 3 R21, 5.1 C-2, 15.1 N32 |
| AG3-DES-018 | P1 | **Answered.** The extension is re-specified for two pairs plus one repair with canonical ordering and alias rejection, the bound raised to five receipt-sized values (the old one made the maximal state unwritable, since the reader caps before unsealing), and active status is **derived** on load rather than stored, so no atomic source-plus-record transition is implied. | Design 3 R22, 5.2, 15.1 N31 |
| AG3-DES-019 | P1 | **Answered.** A repair for a pair the source is not blocked on is screening-only (case 6d), not `PairMismatch`, so the historical repair completes and frees the slot. `Held(RepairInProgress)` protects the single `resolved_repair` slot while a replacement is outstanding, released on terminal state rather than on `applied`. | Design 3 R24, 5.1 C-2, 5.2, 15.1 N33 |
| AG3-DES-020 | P1 | **Answered.** Sequence and retry classification moved ahead of the fault and tenure tests and applies to every path; the non-faulted path had skipped `apply_repair` and its monotonicity guard while Flow D began applying repairs regardless of fault status. | Design 5.1 C-2, 15.1 N34 |
| AG3-DES-021 | P2 | **Answered.** Journal v2 derives document and tenure identity from the effective retained set including `reconciled`; the existing derivation from `high_water.or(in_flight)` would reject the shape C-7 permits and N7 requires. | Design 3 R23, 5.1 C-7 |
| AG3-TEST-003 | P2 | **Answered.** M2 accepted unchanged. M12 retargeted to both slots occupied plus a third report, since a second distinct pair now legitimately fills the free slot. N24's discard claim withdrawn; N5d's "stored bytes unchanged" replaced with an assertion that can actually hold. | Design 15.1 N5d, N24, 15.2 M12 |
| U-11 | decision | **Agreed by the reviewer and adopted**: only a faulted reporter may fill the second slot. | Design 5.2, 16 |

Accepted in the revision-4 round and not reopened: C-8's epoch-zero restore fix, the reconciled
publication lifecycle, the healthy-current-tenure half of AG3-DES-013, and M2.

## Finding ledger, revision 3 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-013 | P1 | **Answered in revision 4.** Case 5 matched "cross-tenure, any shape", so an owner applying a historical repair to its own healthy source would reopen a sealed epoch and lose newer progress. Fault status now classifies before tenure; a source not faulted on the named pair reaches only the screening case. Flow D no longer discards repairs for non-faulted documents, which is what made case 6 unreachable. | Design 5.1 C-2, 7 Flow D, 15.1 N5c, N5d |
| AG3-DES-014 | P1 | **Answered, and revision 3's claim was wrong.** `decode_mode` derives the document from `latest` alone and `ResolvedRepair::verify` requires equality, so the epoch-zero shape failed `ReceiptConflict`. C-8 widens that derivation for versions 4 and 5 only. | Design 3 R17, 5.1 C-8, 15.1 N2b |
| AG3-DES-015 | P1 | **Answered.** Revision 3 defined only the creation of `reconciled`. Promotion on completion, retirement on supersession, clearing on tenure change, inert stale retries and decode constraints are now specified; without promotion, `complete_studio_head` had no valid path for the reconciled winner at all. | Design 3 R18, 5.1 C-7, 15.1 N28 |
| AG3-DES-016 | P1 | **Answered.** A single active pair now spans the source fault and the owner record, with a source fault always active and one bounded deferred slot, so sequential repair works in both orderings. Ordinary receipt issuance is explicitly not blocked. | Design 5.2, 6.5, 15.1 N31 |
| AG3-TEST-002 | P2 | **Answered, and the masking is confirmed at the source**: `reserve` sets `ready = false` and a failed write never commits, so the successor `reserve` fails on `Reconcile` regardless. M2 now bypasses the recovery save before it reserves. | Design 3 R20, 15.2 M2 |
| U-9 | decision | **Reviewer's answer adopted**: `EpochFaultRecord` stays owner-only. | Design 5.2, 16 |
| U-10 | decision | **Reviewer's answer adopted**: no automatic expiry. | Design 5.2, 16 |

Accepted in the revision-3 round and not reopened: the historical-pair report direction, the 2a/2b
adoption split with the committed-state terminality oracle, the Studio and Registry v2 framing and
parse-order correction, U-7, U-8, and the retargeted M12.

## Finding ledger, revision 2 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-009 | P1 | **Answered in revision 3, and the diagnosis is confirmed at the source.** `ingest_verified`, `ingest_adoption` and `check_opening_receipt` all return `Fault` before considering the incoming receipt, so a repair is the only exit and revision 2's "the new owner's first receipt converges it" was false. Reports now carry the complete frozen pair, historical pairs are admissible and land in an owner-side evidence record rather than through the live seal, and cross-tenure repair is a distinct `Unblocked` transition that installs nothing. | Design 3 R14, 5.1 C-2 case 5, 5.2, 5.6 W-1, 6.3, 6.5, 15.1 N5b, N17 |
| AG3-DES-010 | P1 | **Answered.** Case 2 splits into ordinary-seal and adoption modes, and the committed `repair_state()` becomes the sole terminality oracle after B2, so the plan can no longer contradict `install_pending`. A cross-tenure source is never claimed for a forbidden install. | Design 5.1 C-2, C-5, 5.3 S-2 step 6, 15.1 N5 |
| AG3-DES-011 | P1 | **Answered.** Clearing a losing `in_flight` returns the owner to its older high water for both selection and adjacency, so the journal gains a distinct `reconciled` canonical decision and `canonical_head()`, deliberately not the publication bit. The size arithmetic is stated and the journal bound raised rather than relying on ~128 bytes of headroom. | Design 3 R15, 5.1 C-7, 6.7, 15.1 N7, N28 |
| AG3-DES-012 | P2 (P1 for Registry) | **Answered.** Studio and Registry v2 encodings both specified, tag is an unambiguous `2` with a counted report list of 0 or 2, and the parse and charge order corrected against the real seam by splitting the decode so the report stays opaque until after the per-requester rail. | Design 3 R16, 5.6 W-1, 12, 15.1 N26, N26b |
| M2 non-isolating | P2 | **Answered.** The mutant now weakens the "durable save returned successfully" predicate into `Ok(Some(capability))`; minting on an `Err` path leaked nothing. | Design 15.2 M2 |
| M12 non-isolating | P2 | **Answered.** Retargeted at the frozen-pair replacement refusal in the owner record, a guard core never sees, plus a report-boundary side-effect assertion. | Design 15.2 M12 |
| U-7 | decision | **Reviewer's answer adopted over my proposal**: admit before answering, no proof for a disputed receipt, fail closed on an uncertain write. | Design 6.5, 6.6 |
| U-8 | decision | **Confirmed**: no pre-B2 `Repairing`. | Design 5.7, 6.4 |

Accepted in the revision-2 round and not reopened: AG3-DES-003, AG3-DES-005, AG3-DES-007 and
AG3-DES-008.

## Finding ledger, revision 1 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-001 | P1 | **Answered in revision 2.** U-1 option (a) adopted with the complete contract the reviewer required: a repair-independent `conflicting_receipt_pair` checker (the repair does not exist yet when a report is validated), a report-only wire and store boundary that authenticates, bounds, validates and records through the common source fences, and an explicit admissibility rule limiting reports to a same-tenure pair in the current owner's tenure, which also removes the historical-versus-live authority confusion. Duplicate, failed-write, version and rate rules stated. | Design 5.1 C-4, 5.6 W-1, 6.5, 15.1 N26, M12 |
| AG3-DES-002 | P1 | **Answered.** Both failure paths reproduced from source. The transition is computed as a complete validated candidate on a clone, including the existing adoption mode, and committed with the gate under one lock; five explicit cases; every refusal leaves both unchanged. | Design 5.1 C-1/C-2, 15.1 N1 to N5, M1 |
| AG3-DES-003 | P1 | **Answered.** The `!receipts_conflict` predicate is withdrawn as unsound; a newer head survives only under a positive lineage justification, and the correction moved into shared core with explicit source context per U-2. | Design 5.1 C-2 case 1a, 15.1 N3b |
| AG3-DES-004 | P1 | **Answered.** Disposition, continuation and completion are now three distinct things read from the source, not from the ingest outcome; every retry row flushes; six barriers with mark-applied separated; the "Fault retained" claim after B2 withdrawn. | Design 5.1 C-5, 5.3 S-2, 6.4, 8, 15.1 N12, M8 |
| AG3-DES-005 | P1 | **Answered.** One eligibility predicate shared by serving and publication ordering, satisfied at B2/B3, never by a B1-only decision, per U-4. The deadlock case is an explicit test. | Design 5.5, 6.6, 10.2, 15.1 N27, M7 |
| AG3-DES-006 | P1 | **Answered, and the finding is stronger than stated.** `prepare_verified` makes `in_flight` irrevocable and demands exact hash equality at one closed epoch, so the journal was wedged, not merely mis-preferred. A repair-authorized `resolve_repair` inside B1 is the only replacement, with the losing bytes retained. | Design 5.1 C-7, 5.2, 6.7, 15.1 N7, N28 |
| AG3-DES-007 | P2 | **Answered.** The capability's predicate is derived by the common writer from the authenticated durable predecessor, binds storage scope and the predecessor wrapper digest, is minted only after the required save or flush returns, is reacquired after reopen, and covers the verified empty-recovery case. Existing Prepared, metadata-link and reference checks untouched. | Design 9, M2, M9 |
| AG3-DES-008 | P1 | **Answered.** Explicit capture, detached, revalidate and commit split; permit carried through queue, worker, ready result and delivery; weak admission bookkeeping; full coordinate rebinding; the mutation generation made mandatory over every repair write and possible-I/O path, not three headline writes. | Design 10.3, 11, 13.1, 15.1 N30 |
| AG3-TEST-001 | P2 | **Answered.** All ten mutants reworked at the boundaries they actually reach; M10's intended assertion was wrong and is replaced; M11 and M12 added; no useful redundant validation is deleted to manufacture a failure. | Design 15.2 |

## Audit claims corrected across revisions

| Claim | Correction |
|---|---|
| "No production path ingests a second receipt into a Studio source" (rev 1) | **Withdrawn as overstated.** `adopt_studio_checkpoint` ingests another receipt, classifies `ReceiptIngest::Fault` and crosses its own durable barrier (`store/epoch_studio/adoption.rs:89-119`). The accurate and still load-bearing statement is that its provenance is a current-owner discovery selection, so evidence flows owner-to-peer only and there is no reporter-to-owner direction. |
| "`seal_studio_epoch` has no non-test caller" (rev 1) | **Reduced to what was verified.** A grep over `crates/` and `apps/` at the base found call sites only under test modules. That is a search result, not an exhaustive reachability proof, and nothing in the design depends on it. |
| "The repair binds the full target" (rev 1, implied) | **Qualified.** The signature binds `LogicalDocument.server_id` against the MLS group id only. The local numeric server, the store scope and the Flipnote channel need the separate authenticated store and request-target checks. |
| "Preserve a newer head when it is outside the pair and does not conflict" (rev 1) | **Withdrawn as unsound.** `receipts_conflict` compares same-tenure epoch and inheritance conflicts and is not an ancestry proof; a head descending from the loser passes it and then blocks the adoption planner's `latest == receipt` precondition. |
| "A duplicate repair means the transaction is complete" (rev 1) | **Wrong.** `ReceiptRepairIngest::Duplicate` establishes only that the book's disposition happened. Recovery and installation can still be outstanding. |
| "The repair holds keep Fault after B2" (rev 1) | **Wrong.** B2 clears the fault. The truthful state is a read-only `Repairing` hold with the whole branch retained. |
| "A cross-tenure fault converges through the current owner's ordinary first receipt" (rev 2) | **Wrong, and the most serious error in revision 2.** Every admission path returns `Fault` before considering the incoming receipt, so no receipt from any owner or tenure can clear a fault. A repair is the only exit. |
| "A report naming an older tenure may be refused" (rev 2) | **Withdrawn.** Combined with the above, that rule would have made cross-tenure repair permanently unreachable, which is the case the v2 record format exists to serve. Historical pairs are admissible; the authority question lives with the repair, not the report. |
| "One reported receipt is enough" (rev 2) | **Wrong for the historical case.** A new owner may hold neither member, so the reporter, which is faulted and holds both, always sends both. |
| "The per-requester rail and pending cap precede the v2 decode" (rev 2) | **Wrong.** `queue_checkpoint_head` decodes the scoped query before charging the per-requester rail, so the decode is split instead. |
| "Clearing a losing `in_flight` reconciles the journal" (rev 2) | **Insufficient.** Both the head selector and `prepare_verified` key off the retained high water, so the owner falls back to an older receipt and reads its next one as a gap. |
| "Cross-tenure case 5 applies to any shape" (rev 3) | **Wrong.** It would let a historical repair reopen a healthy current-tenure source and drop its legitimate head. Fault status must classify first. |
| "Case 5's epoch-zero shape satisfies restart" (rev 3) | **Wrong.** `decode_mode` derives the document from `latest` only, so a resolved repair with no head fails `ReceiptConflict`. Asserted without checking the decoder. |
| "A repair for a document with no local fault is discarded" (rev 3) | **Contradicted its own case 6.** Distribution has to reach the screening path or the case is dead code. |
| "`reconciled` plus `canonical_head()` finishes the journal fix" (rev 3) | **Incomplete.** Creation without promotion, retirement, tenure clearing or decode constraints leaves the reconciled winner impossible to complete and eventually stale. |
| "M2 is isolated once it mints on a lie" (rev 3) | **Still masked.** The failed accounted write leaves the budget unready, so the successor write fails on `Reconcile` before the intended assertion can fail. |
| "A non-faulted source is safe to leave unchanged" (rev 4) | **Wrong.** Neither installer screens the receipt it already holds, so a source sitting on the loser installs or settles the repudiated branch. Not-faulted is not the same as not-on-the-loser. |
| "`active`/`deferred` is specified" (rev 4) | **Only in prose.** The persisted encoding stayed singular and the bound stayed sized for three receipt-sized values, so the maximal two-pair state could not be written or reopened. |
| "A source fault always becomes the active pair" (rev 4) | **Wrong once a repair is durable.** It strands a persisted B1 decision behind a later unrelated fault. A pending repair owns the target; a source fault waits its turn. |
| "A repair for a different pair is `PairMismatch`" (rev 4) | **Wrong.** It is simply not about this source's blocker. Screening-only application is what removes the deadlock. |
| "The non-faulted path only needs authority and evidence checks" (rev 4) | **Wrong.** It bypassed `apply_repair`'s `repair_sequence` monotonicity guard, so a delayed lower-sequence repair could overwrite a newer one. |
| "N5d asserts stored bytes are unchanged" (rev 4) | **Self-contradictory.** Case 6b durably adds repair evidence; the assertion had to be narrowed to non-repair content. |
| "A source fault's pair must be retained in the record to be selected" (rev 5) | **Wrong, and the record could not represent the result.** Two retained historical pairs could crowd out the pair actually blocking the document, and its repair had nowhere valid to bind. The source's pair is already durable in the book and needs no slot. |
| "The other retained pair becomes active when a repair terminates" (rev 5) | **Incomplete.** Nothing removed the pair that just terminated, so already-resolved evidence could become active again. |
| "MAX_RECORD_BYTES ... 3 * MAX_RECEIPT_BYTES" (rev 5, second statement) | **Stale duplicate.** Revision 5 added the correct five-receipt formula but left the old one later in the same section and 11.4 KiB in section 10.1. |
| "Sequence and retry classify before anything else" (rev 5) | **Unsafe ordering.** Core deliberately verifies current owner before its retry shortcut; inverting that inside an authority-bearing API reintroduces the M4 bug one layer up. |
| "B2 means the fault has durably ended" (rev 5, after case 6d) | **False for the screening cases.** A terminal disposition is not the same as a usable document. |
| "The source's pair is never stored, and the repair may still name it" (rev 6) | **Unrepresentable.** The codec required a repair to match a retained pair, so the source-bound case the correction existed for could not be encoded at all. A source-bound repair now carries its pair inline. |
| "A nonterminal repair owns the target" (rev 6) | **Only against discovery.** The report path could still seal a current-tenure pair, move the phase to `Fault` and collapse `repair_install_pending()`, abandoning an outstanding replacement. |
| "`Screened` separates terminal from usable" (rev 6) | **Only on the fresh path.** No provenance was persisted, so an exact retry reported `Repaired`. |
| "A report whose members are already screened is a no-op" (rev 6) | **Over-broad.** `is_repaired_loser` governs receipt admission, not whether a different frozen pair is resolved; it would strand a peer on `{R1,R3}` after `{R1,R2}` was repaired. |
| "The inline pair must match the live `ReceiptBook::fault`" (rev 7) | **Only true before B2.** `apply_repair` clears `fault` on success, so the rule necessarily failed exactly when a resume was needed. |
| "`repair_install_pending()` is the ownership fence" (rev 7) | **Starts too late.** A B1-persisted repair is already nonterminal and owns the target, so a pre-B2 report could degrade a required replacement into a terminal screening. |
| "Deferred live evidence keeps proofs suppressed" (rev 7) | **Not durably.** There was no capacity for it at the legal maximum and no persistent term in the proof refusal, so a restart resumed proving a disputed head. |
| "`Transitioned` and `Screened` cover the dispositions" (rev 7) | **Incomplete.** Cases 6a and 6c mutate a source that was never Faulted and fitted neither value; the tag also had no place in the snapshot layout and was dropped by every successor path. |
| "`repair_install_pending()` is the one-claim predicate" (rev 8) | **Wrong for the same reason the report fence was.** `advance_checkpoint` installs into the source and was unfenced for the whole B1-to-B2 interval. |
| "The live slot is specified" (rev 8) | **Only on the wire.** It was absent from the struct and from `check_scope`, so it could be lost on restart or filled with unvalidated evidence. |
| "The slot is drained once the owning repair terminates" (rev 8) | **Not ordered.** It was missing from the active-pair derivation, so a crash before the drain let historical evidence take the target. |
| "Liveness is a property of the stored pair" (rev 8) | **Wrong.** It is a judgement against the tenure current at admission; storing it means an owner change leaves the pair sealable under the wrong owner or suppressing proof forever. |
| "`adopted_successor` is unchanged" (rev 8, C-6) | **Contradicted C-5** in the same revision and, taken literally, reproduces the successor-provenance loss. |

## Facts established by the revision-9 audit

- `advance_checkpoint` calls `install_studio_seed_step`
  (`studio/receiver/catchup/discovery.rs:189`) and `install_registry_seed_for_studio` (`:159`), both
  reaching `adopt_studio_checkpoint`, so ordinary discovery mutates and can replace the source. A
  claim predicate that starts after B2 therefore leaves it unfenced for the whole B1 interval.

## The `DraftArchive` seam at `705d44b`

Verified in code, and the reason no rebase action was needed: `705d44b` is already an **ancestor**
of the branch head, landing two commits after Agent 3 revision 8, so the checkout already contains
the seam and there is nothing to replay.

- `EpochRecordKind` has six variants; `DraftArchive` is the sixth
  (`store/epoch_recovery/inventory.rs:48-62`). The enum is closed, so a match missing the arm is a
  compile error at this commit, not a silent default.
- `intent_class()` returns true for `Intents | DraftArchive`
  (`inventory.rs:67-69`): its own physical family, the Intents accounting class, charged against
  `MAX_VAULT_INTENT_BYTES`.
- Own suffix `.draft-archive`, own domain `catcoms/epoch-draft-archive-store/v1`, own scope, own
  inventory key, own sealed cap `MAX_DRAFT_ARCHIVE_SEALED_BYTES` of about 6 MiB plus 35 KiB, and a
  bounded authenticated reader in `store/epoch_draft_archive.rs` that performs **no typed decode**.
- **No writer exists.** `write_studio_draft_archive_with_io` and
  `release_studio_draft_archive_with_io` have zero matches in the tree, matching the module's own
  statement that nothing there writes, releases or decodes an archive. So I-4 has nothing to guard
  in this family today, and both names are already on I-4's audited participant list in
  `GATE4-AGENT-1-DESIGN.md` 9.2 for when Agent 2 lands them.

Agent 3 consequences are section 12.1 of the design: consume the variant, handle it in any match,
add no writer, and never treat an archive as repairable history (I-12, N40).

## Facts established by the revision-8 audit

New this revision, verified in code:

- `ReceiptBook::apply_repair` sets `self.fault = None` and installs `resolved_repair` in the same
  mutation (`epoch.rs:1765-1774`), so a persisted source-bound repair cannot be validated against a
  live fault after B2.
- `adopted_successor` builds a fresh unit, copies `self.receipts` and marks the latest installed
  (`studio/epoch/adoption.rs:140-150`). Nothing Studio-layer crosses, so a disposition tag held
  outside the book is dropped by that path unless it is copied explicitly.
- `StudioEpoch::snapshot`'s leading byte is `if self.adopting { 2 } else { 1 }`
  (`studio/epoch.rs:432`) and is the only value telling restore whether to call
  `ReceiptBook::decode` or `decode_adoption`.

## Facts established by the revision-7 audit

Verified in code and still relied on:

- `transition_verified_receipt` permits `Open | Closing | Fault -> Fault` and clears the receipt
  hash (`epoch.rs:2539-2549`), so a report driven through the live seal collapses
  `repair_install_pending()`, which requires `Closing`.
- `EpochOwnerReceiptState::decode` and `check_scope` take only `(bytes, scope, document)`
  (`store/epoch_owner.rs:66-124`): no source is available, so a source-bound repair must carry its
  evidence inline to stay self-validating.
- `StudioEpoch::snapshot` already carries a version byte distinguishing ordinary from adopting
  (`studio/epoch.rs:432`), so the disposition tag extends the restart unit rather than core's
  already-tested `ResolvedRepair` codec.

## Facts established by the revision-6 audit

Verified in code and still relied on:

- `ReceiptBook.fault` is a single `Option<(Receipt, Receipt)>` alongside the single
  `resolved_repair` (`epoch.rs:1571-1575`), so the source holds exactly one unresolved pair and
  holds it whether or not any owner record mentions it.
- `ReceiptBook::apply_repair` calls `verify_current_owner` **before** its exact-retry shortcut, with
  the comment "Authority must precede the retry shortcut: a returning key is not its old tenure"
  (`epoch.rs:1735-1741`).

## Facts established by the revision-5 audit

Verified in code and still relied on:

- `prepare_checkpoint_adoption` requires only `adopting`, `Closing`, not faulted and
  `latest == receipt` (`studio/epoch/adoption.rs:89-95`), and `prepare_settlement` requires not
  `adopting`, `Closing`, not faulted and then acts on `receipts.latest()`
  (`studio/epoch/settlement.rs:71-79`). **Neither screens the receipt it already holds** against a
  resolved repair; `is_repaired_loser` only screens receipts arriving at admission.
- `OwnerReceiptJournal::decode` derives its document from
  `high_water.as_ref().or(in_flight.as_ref())` and then requires document presence to agree with
  tenure presence (`epoch.rs:2084-2094`), so a reconciled-only journal fails to restore.
- `ReceiptBook.resolved_repair` is a single `Option` (`epoch.rs:1575`) that `apply_repair`
  overwrites, so a second repair can erase the evidence an in-flight replacement depends on.

## Facts established by the revision-4 audit

Verified in code and still relied on:

- `ReceiptBook::decode_mode` derives its document as
  `latest.as_ref().map(|receipt| receipt.document.clone())` (`epoch.rs:1877`) and then calls
  `resolved.verify(document.as_ref(), ..)`, whose first condition is
  `document != Some(&self.repair.document)` (`epoch/repair_state.rs:43`). A resolved repair with no
  `latest` therefore fails `ReceiptConflict`.
- `OwnerReceiptJournal::mark_published` returns early only on an exact `high_water` match and
  otherwise requires `in_flight` to be present and equal (`epoch.rs:2043-2058`). A receipt that is
  neither has no completion path.
- `EpochStorageBudget::reserve` sets `self.ready = false` before returning
  (`store/epoch_budget.rs:376`) and a writer error returns without `commit()`, so every later
  `reserve` fails with `BudgetError::Reconcile`.

## Facts established by the revision-3 audit

Verified in code and still relied on:

- `ingest_verified` returns `Ok(ReceiptIngest::Fault)` on `self.fault.is_some()` before the
  repaired-loser screen, the tenure comparison and the high-water logic (`epoch.rs:1615-1619`);
  `ingest_adoption` does the same before its anchor search (`epoch/adoption.rs:27-29`); and
  `check_opening_receipt` returns early too (`epoch.rs:1687-1689`). **A repair is the only exit
  from Fault. No receipt from any owner or tenure can clear one.**
- `prepare_verified` requires a same-tenure receipt to be exactly one epoch beyond `high_water`
  (`epoch.rs:1993-1998`), and the head selector's `own_choice` is `pending().or(published())`
  (`store/epoch_studio/discovery.rs:229`), so clearing a losing pending decision returns the owner
  to its older high water for both selection and adjacency.
- `OwnerReceiptJournal::encode` is version 1 with `high_water`, `in_flight` and `tenure`
  (`epoch.rs:2061-2068`), and its decoder enforces a `(high_water, in_flight)` adjacency invariant
  (`epoch.rs:2105-2116`). Adding a third retained receipt leaves roughly 128 bytes of headroom
  against `MAX_OWNER_RECEIPT_JOURNAL_BYTES = 3 * MAX_RECEIPT_BYTES + 256`, so the design raises the
  constant rather than relying on that margin.
- `encode_scoped_query` delegates `CheckpointTarget::Registry` to `encode_query`, a different
  framing from the Studio channel/object one (`receipt_head/wire.rs:70-97`).
- `queue_checkpoint_head` charges the global preauth rail and authenticates, then calls
  `decode_scoped_query`, and only afterwards charges the per-requester rail
  (`receipt_head.rs:378-415`).

## Facts established by the revision-2 audit

Verified in code at the base and still relied on:

- `begin_checkpoint_adoption` sets `self.adopting = true` whenever the outcome is not `Stale` and
  the source was not already faulted (`studio/epoch/adoption.rs:75-77`), so **a fault produced by
  adoption leaves the source in adoption mode**. Any Fault exit must decide that mode explicitly.
- `ingest_adoption` is reachable with `opening == None` on a source holding epoch-zero content, so
  an adoption fault can name receipts closing an epoch unrelated to the gate's.
- `checkpoint_bytes_by_hash` calls `receipt_head()` first, which errors while faulted
  (`studio/epoch.rs:199-222`), and serves only the installed opening's exact seed: **a faulted peer
  serves no seed at all.**
- The head selector prefers `journal.pending().or(published())` for an owner and proves only on
  three-way equality with the held source receipt (`store/epoch_studio/discovery.rs:229-242`).
- `OwnerReceiptJournal::prepare_verified` rejects any different `in_flight` within a tenure and, at
  an equal `closed_epoch`, demands exact hash equality with `high_water` (`epoch.rs:1984-2008`). No
  existing API can correct a journal whose decision is a repair's loser.
- `queue_checkpoint_head` bounds the request at `MAX_QUERY + 144`, charges the global preauth rail
  before authenticating, then the per-requester rail, then decodes (`receipt_head.rs:357-418`).

Carried forward from revision 1 and unchanged: the Fault-gate restart requirement
(`epoch.rs:2352`, `epoch/adoption.rs:115`), `apply_repair`'s unconditional head overwrite
(`epoch.rs:1765-1767`), the adoption anchor predicates (`epoch/adoption.rs:37-46`, `:109-113`),
the inert wire fields (`receipt_head.rs:541`, `receipt_head/detached.rs:223-225`), the absence of
any `RecoveryReason::Repair` constructor, the `pub(crate)`/private status of
`restore_verified_from_vault` and `receipts_conflict`, and the common source fences
(`rotation.rs:121`, `adoption.rs:80`, `epoch_intents/retirement.rs:159-161`).

## Proposed API seams

Full signatures are in [the design](GATE4-AGENT-3-DESIGN.md) section 5. Summary, revision 2:

- Core: `EpochGate::commit_repair` (C-1); `ReceiptBook::plan_repair` taking the two full receipts
  explicitly, with `RepairSource`, `RepairTransition`, `RepairPlan`, `RepairHold`, and
  `StudioEpoch`/`RegistryEpoch` `apply_receipt_repair` committing atomically (C-2); the
  repaired-loser-is-not-an-anchor predicates (C-3); `conflicting_receipt_pair` plus
  `ReceiptRepair::check_evidence` layered on it (C-4); `StudioRepairState`, `repair_state`,
  `repair_install_pending`, `fault_evidence`, `ReceiptBook::repair_sequence` (C-5);
  `prepare_repair_adoption` (C-6); `OwnerReceiptJournal::resolve_repair`, `canonical_head`, the
  version-2 journal with its `reconciled` slot and that slot's promotion, retirement and
  tenure-clearing lifecycle (C-7); the repair-bearing book document derivation (C-8).
- Store: `EpochFaultRecord` holding up to two canonically ordered **external** pairs plus at most
  one **tagged** repair, either external-indexed or source-bound with its pair inline, with active
  status **derived** on load (pending repair, else the source's own fault pair from the book, else a
  **live** reserved pair, else the lowest external pair, else the reserved pair as history) and an
  explicit terminal-pair recycling transition; the owner record's version-3 section carrying a
  `reserved: Option<FaultPair>` slot whose liveness is **derived from freshly observed tenure**,
  bounded for nine receipt-sized values, stated once; `prepare_epoch_repair`
  (which also reconciles the journal) and `mark_epoch_repair_applied`; `apply_studio_repair`,
  `issue_studio_repair`, `report_studio_fault`, `studio_fault_evidence`, `StudioRepairOutcome`,
  `StudioRepairHold`, `CheckedRepairRecovery`, `stage_studio_repair_recovery`, `servable_repair`;
  the Registry mirrors.
- Sync: `ReceiptHeadSelection.repair` served instead of `None`;
  `AuthenticatedCheckpointHint::repair`; `select_repaired_checkpoint`; the version-2 **Studio and
  Registry** scoped head queries, each with a counted report list of 0 or 2 receipts, a raised cap
  of `MAX_QUERY + 2 * MAX_RECEIPT_BYTES`, and a header-only decode that keeps the report opaque
  until after the per-requester rail. All new symbols are named `receipt_repair` or `fault_repair`
  to avoid the existing MLS delivery `repair_outbox` family.
- App: `StudioControlAction::{ReadFault, RepairFault}`,
  `StudioControlResponse::{Fault, Repaired}`, `StudioFaultView`, `StudioFaultCandidate`,
  `StudioFaultRepairRequest`, `StudioRepairStatus`, `StudioRepairBlocker`,
  `StudioSettlementState::{Repairing, StorageRefused}`, and a `catchup/repair.rs` runtime step.
- Native (designed, **not registered**): `studio_fault_read`, `studio_fault_repair`, plus
  `"repairing"` and `"storageRefused"` in the settlement mapping.

**None of these exist yet.** A proposed name is not an implementation.

## Invariants this scope must not weaken

1. **I-1** Book, gate and adoption mode change together or not at all, under the one gate lock.
2. **I-2** The losing branch is durably readable as recovery before any byte of the replacing
   source is written, enforced by a capability minted only by a returned durable save.
3. **I-3** Only the actual current designated committer, with an issuer tenure observed
   independently at this exact custody visit, authorizes a live repair. v1 and an earlier tenure of
   the same key fail closed.
4. **I-4** Both full conflicting receipts, the selected hash and the sequence match the locally held
   fault exactly; a different named pair holds.
5. **I-5** A repair retires no intent, discards no overlay branch and reduces no pending ledger.
6. **I-6** A repair is servable only after a durable local application barrier, never from a signed
   but unapplied decision; serving claims availability, never delivery.
7. **I-7** Durable disposition, unfinished continuation and terminal completion are distinct; a
   retry resumes outstanding work, preserves newer progress, reuses the same snapshot identity and
   deadline, and clears no unrelated fault.
8. **I-8** Every hold retains the complete branch and is labelled for the state actually persisted.
9. **I-9** A repaired loser is not a high-water anchor; a covered losing-baseline descendant stays
   stale without re-faulting; a genuine third baseline still faults.
10. **I-10** A report records evidence only: no winner, no adoption, no third receipt replacing a
    frozen pair, no inferred issuer tenure.
11. **I-11** An irrevocable owner decision is replaced only by a verified repair naming it as the
    loser, and its historical bytes are retained.
12. **I-12** A `DraftArchive` record is a preserved local draft, never repairable history: no repair
    path reads, decodes, replaces, retires or reclaims one, and none is ever evidence.

Inherited and not weakened: HANDOFF-002's common source-write and reference-inventory fences, the
Prepared source-replacement fence and publication hold, the four-slot shared preparation pool, the
two-retained-plus-staged recovery policy with its warning and seven-day deadline, the 48 MiB
settlement reserve and 16 MiB protocol allowance, the registry lineage ceiling, and the rule that
uncertain IO blocks the budget until full inventory reconciliation.

## Touched files

Both passes: `docs/GATE4-AGENT-3-DESIGN.md`, `docs/GATE4-AGENT-3-STATUS.md`. No production code, no
test, no shared contract document and no workflow has been changed. Planned files are in design
sections 5 and 13.3.

## Executed checks

**None, in any of the fourteen passes.** No Cargo, npm or script command has been run: these
checkpoints change no code, and the local machine keeps checks serial. Every number quoted in the design is a constant
read from source at the base or an explicitly labelled estimate. The maximal-shape replacement
cost, the custody time of the capture and commit stages, and the protocol-allowance arithmetic in
design section 10.1 are **unverified**.

## Dependencies

| Dependency | Owner | State | What happens without it |
|---|---|---|---|
| Design verdict on revision 2 | user / independent reviewer | requested | no implementation starts |
| Core handoff signing split `e65bfd8` | Agent 1 / core | unreviewed | unaffected: no repair path uses it |
| Live tenure contract T1 to T5 (design 13.2) | Agent 2 | **implemented on base**: sync split (`verification_`/`authoring_owner_tenure_start`) and the app seam `require_observed_owner_tenure()` / `observed_owner_tenure()` (`23465a17`, V5) | available; no Agent 3 consumer yet. Repair issuance, application and drain bind to the **authoring** side, where `Imported` and `Unknown` are holds |
| CORE-005 archived Observed-tenure witness | Agent 2 / integration | witness and app consumer implemented; bounded re-review passes, exact-head CI pending | bounded one-witness historical admission is available internally; native exposure and detached automatic repair remain blocked |
| Prepared overlay fence and source custody (design 13.1) | Agent 1 | design revision 3, unreviewed | repair relies only on the existing `resolve_studio_handoff` and `save_studio_source_checked`; if `inventory_generation` lands, rotating it becomes mandatory over the full list in design 10.3 |
| Native registration, UI hooks, INTERFACES rows | Agent 4 | not started | commands stay unregistered and nothing is callable from the renderer |

U-1 through U-11 are all decided and carried. No U-question remains open.

At the reviewed head, Agent 1's runtime implementation is only partial and its I-4 and C-3 work has
not started. Agent 2's tenure design has since **passed** adversarial review, correcting the stale
statement carried in earlier revisions, but none of it is implemented. Neither blocks this design
checkpoint; both are real prerequisites before Agent 3 implementation integrates.

## Coordination summary

- **Agent 1**: this scope consumes `resolve_studio_handoff`, `save_studio_source_checked` and the
  shared preparation pool, introduces no competing writer or second pool, and asks only that the
  writer's capability-parameter shape survive so the recovery capability is a second parameter of
  the same kind. If `inventory_generation` lands, every repair write and possible-I/O path must
  rotate it.
- **Agent 2**: T1 to T5 in design 13.2, chiefly that the tenure accessor stays `Option<u64>` with
  `None` as a hold and that fault tenure and issuer tenure are never collapsed. Handed back: a
  replacement invalidates a retained overlay's Closing basis; the work stays retained and the manual
  path is Agent 2's.
- **Agent 4**: registration, the version-2 query and raised cap in INTERFACES, UI hooks, and the
  proposed `studio-repair` workflow job, per design 13.3. The five app enums this scope extends are
  listed under Proposed API seams so the merge preserves both contracts.

## Next actions

The design is accepted, so these are implementation actions.

1. **Create a separate branch or worktree first.** Do not implement on the shared documentation
   checkout: another session resets it, which has already discarded uncommitted work twice.
2. Agree the tenure seam (T1 to T3, in its accepted verification/authoring form) with Agent 2 and
   the custody points with Agent 1 in writing before the first line of code. Both are design-only
   today, so an implementation that assumes either is available will not compile or will fail
   closed.
3. Implement in the design's order: core C-1 to C-8 with N1 to N7, then the owner record and its
   transitions with N8 to N10, then the Studio transaction with N11 to N20, then Registry with N21,
   then W-1 and both report paths with N26 and N26b, then distribution with N23 to N25 and N27,
   then the runtime, control and native surface with N28 to N30, then the reserved-slot and overflow
   lifecycle with N31b, N31c and N36 to N46, then the nineteen mutants.
4. Treat **N17** as the gating acceptance case: it is the one that proves a fault is reachable, is
   otherwise permanently unexitable, and is actually healed without injecting state on the new
   owner.
5. Request **review preamble 3** for the bounded implementation when there is executed evidence.
   The design PASS does not carry over: implementation, mutation execution and CI evidence are a
   separate verdict, and Gate 5 stays closed either way.
