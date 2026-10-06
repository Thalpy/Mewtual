# Gate 4 Agent 1: C-3 runtime adoption (design, revision 3)

Status: **step 1 (section 3) accepted for implementation; steps 2 to 5 need the decisions in
section 11 reviewed before each is built.** It extends
`GATE4-AGENT-1-DESIGN.md` section 9.2 (the cursor and I-4) and 5.5 (the overlay job). The storage
half of C-3 is implemented and reviewed; this is the runtime half, ledger row G4-A1-C3.

Revision 1 was a scratch draft. Its design review (2026-10-06, Opus, static, against `c9566b82`)
returned **not ready**: two blockers, five highs, seven mediums, eight lows. Every finding is
answered below; the table in section 9 maps each one to the section that answers it.

## 1. What exists, and the two facts that shape everything

- `EpochInventoryJob` (`store/epoch_recovery/inventory.rs`): a cursor plus a per-attempt restart
  budget (`MAX_INVENTORY_RESTARTS = 3`), `step_epoch_inventory_job(job, steps, Some((clock, ms)))`,
  a parked body that is validated detached (`ParkedEpochRecord::validate`, pure) and installed with
  `install_validated_job_record`, and `finish_epoch_inventory_job -> Complete | Restarted | Unstable`.
- **One inventory mints one budget.** `studio_storage_budget` requires a fresh inventory and
  rotates `studio_generation`, so a stale inventory cannot mint twice. One budget can serve
  several writes inside one visit (`enter_studio_budget_scope`), but not across visits.
- **A step processes one record per call**, with a one-entry minimum, and only Registry and Studio
  records are cacheable. Recovery, OwnerReceipts, Intents and DraftArchive are validated fresh on
  every scan, and `validation_fits` returns false for every fresh validation until measurement
  13.7 calibrates it. So under the current classifier **every uncached record parks**.

The second fact is what made revision 1's throughput premise false (review H2). It is why receive
converts last, and only behind evidence (section 7).

## 2. Two kinds of owner, two job placements

### 2.1 Overlay owners embed their own job (review B2)

Flow H's H1 (capture) and H5 (commit), and later Flow S's runtime S1b/S3, hold an
`OverlayOwnership`: per-actor admission plus one permit from the shared four-slot preparation
pool. Design 5.5 already places the commit cursor inside the overlay job, and 9.2 consequence 2
says the parked body "carries the job's original ownership ... no second overlay pool".

So an overlay owner's `EpochInventoryJob` lives **inside its own stage**, and its detached
validation borrows the stage's ownership: the ownership moves into the validation worker with the
parked body and comes back with the result, exactly as `HandoffPrepare` moves it today. No second
permit is ever taken. Revision 1 took a fresh permit for every purpose, which wedges the pool when
four H5 jobs hold all four permits and each parks (review B2).

Cancellation of that waiter abandons the overlay job (the worker keeps the ownership until it
ends, as I-2 already requires for `OverlayPlan`).

### 2.2 Non-overlay owners share one purpose-agnostic, turn-based job (review B1)

Replay's manual move, catch-up's persist and maintenance sites, and eventually receive, share
**one** job per actor in a new `InventoryRuntime` inside `StudioReceiver`.

- **No owner owns the job.** Whichever owner's turn needs a budget steps the shared job inside its
  own turn; the owner whose turn **completes** it mints the budget and uses it in that same visit.
- **No queue.** Revision 1's FIFO head could be an owner that only runs when `replay_ready()`
  holds, so a page arriving behind it made catch-up wait on the head while the head waited on
  catch-up (review B1). Without a head there is nothing to wait on.
- **Fairness is the existing turn allocation**: `gossip_runs`, `background_step`'s order and the
  `replay_ready()` gating. RT-002 is preserved by construction, because inventory work only ever
  runs inside a turn its owner was already given.

An inventory is a vault snapshot, not an owner's private view, so handing a completed one to
whichever owner completes it is correct. Every owner re-derives its own preconditions in the
completing visit (section 4).

## 3. Storage prerequisites (land first, each reviewed)

**S-1. A limits profile on the job (review H1).** Today the receive limits (1024 entries, 64
records, `STUDIO_RECEIVE_READ_BYTES`, the cold limit) are set on the cursor's fields by
`scan_studio_receive_inventory` and nowhere else, and `begin_epoch_inventory_job` plus every
restart path re-begin at the defaults. Converting receive, catch-up or replay to the job API would
silently widen them from 64 records to 65,536 and drop the cold limit. So:

- `EpochInventoryProfile { coverage, entry_limit, record_limit, byte_limit, cold_byte_limit }`,
  with `full()` and `receive()` constructors; `scan_studio_receive_inventory` becomes
  `receive()` applied to the existing scanner.
- `begin_epoch_inventory_job(profile)`; the job stores the profile and **every** restart re-begins
  with it.
- Test plus mutation: a receive-profile job still refuses at record 65 after a restart.
- Step 1 adds the mechanism only. **Each owner's profile, and the matching `THREAT-MODEL.md`
  correction for review L7, are made in the step that converts that owner**, not here (revision 2
  re-review). Automatic handoff uses the full limits today while the threat model says automatic
  inventory uses the local ones; that discrepancy is recorded, not yet changed.

**S-2. Stamp the generation at finish, check it at mint (review L8).** `studio_storage_budget`
does not check `inventory_generation`. Within one visit that is harmless; across a finish/mint split
it is not. The issued inventory records the generation it was finished under, and the mint
refuses a mismatch. The stamp is taken in the cursor's own finish (`finish_with`), so synchronous
scans carry it too. No legitimate path finishes and mints across an intervening five-family write
(receive's `save_server` goes through `write_server_record`, which does not rotate the generation);
a test that deliberately mints after a write now sees a different refusal (revision 2 re-review).

**S-3. A visit-level deadline helper (review H2, M2).** `drive_epoch_inventory_job(job, deadline)`
takes an **absolute** deadline that the receiver samples once per visit, then loops
`step_epoch_inventory_job` with the remaining time, stopping at `Parked`, at `Restarted`, at
traversal end, or when the remaining time is zero. **It never calls a step with zero remaining**,
because a step has a one-entry minimum and would otherwise overrun by a whole entry every visit.
One deadline covers the whole visit, shared by every owner that runs in it, so per-owner slices
cannot add up (`background_step` runs the H1 probe and then replay or catch-up in one visit).

**S-4. A pinned directory-iteration test (review L6).** The cursor keeps `read_dir` open across
visits while `servers/{id}.bin`, `.net` and `.cache` are written in the same directory. Pin what
the scan must do when a sibling file is created, replaced or removed between steps: a five-family
write rotates the generation; a non-family file must not fault, and it **counts toward the entry
limit, as every visited entry does, but never toward records or authenticated bytes**.

## 4. Lifecycle of the shared job (non-overlay owners)

```
enum InventoryState {
    Idle,
    Stepping { job, profile },                    // steps only inside an owner's turn
    Parked { job, profile, body },                // a body taken, awaiting detach this visit
    Validating { job, profile, token },           // body detached with its permit
    Backoff { until_ms, delay_ms },               // after Unstable
}
```

- **Permit before any step that can park (review H4).** A budgeted step is taken only after a
  permit is reserved from the shared pool; if the step did not park, the permit is released at the
  end of the visit. If the pool is full the job does not step at all, so a parked body is never
  resident without a permit, and its bytes are always inside 13.4's accounted sum.
- **`Parked` is distinct from `Validating` (review M4).** The body is detached by `detach()` in the
  same visit, placed after Prepare/PrepareRegistry and before the catch-up `in_flight` check and
  the overlay and handoff jobs. A visit that returns an error or is cancelled before `detach`
  leaves `Parked`, which `pause()` and lock release (below).
- **Validating cannot be stepped (review M7).** The type has no job to step until the result is
  installed, so a write landing while the worker runs restarts nothing, and a stale result cannot
  meet a new cursor.
- **Tokens (review H3).** Each detach carries a fresh `u64`; results are routed by token, never by
  target. New variants:
  `StudioBackgroundJob::InventoryValidate(body, permit, token)`,
  `StudioBackgroundResult::InventoryValidated(token, Result<ValidatedEpochRecord, AppError>)` and
  `StudioBackgroundResult::InventoryCancelled(token)`. The existing default arm,
  `Cancelled { preparation: None }`, clears catch-up's `pass`, `registry_pass` and `in_flight`;
  without its own variant a cancelled validation waiter would destroy an unrelated catch-up pass.
  On cancellation the job is dropped **without** charging a restart, and the worker keeps its
  permit until it ends, because the permit moved into the blocking closure.
- **Identity before any step (review M6, L3).** The numeric server and the registry mount are
  checked before every step, not only at install (a step checks only the generation). A mismatch
  drops the job without charging a restart.
- **Faults (review M7).** A fault poisons the cursor, so any error drops the job. A validation
  error that arrives in `complete()` is stored and surfaced once on the next owner turn that would
  have stepped, then the job is dropped.
- **Pause and lock (review H4, M6).** `pause()`, `clear_previews()` and the UI-lock reset release a
  job that is not `Validating` (dropping its plaintext and directory handle); a `Validating` job is
  marked abandoned and its result is dropped on arrival.
- **Backoff and wake (review M3, L1).** `Unstable` enters `Backoff` (30 s doubling to 300 s, reset
  on a completed inventory). `studio_pending` gains: true while `Parked` or `Stepping` with a
  permit, false while `Validating` (the result wakes the actor), and an owner term that only the
  inventory backoff is blocking is masked so the receiver does not spin once a second for up to
  300 s. `wake_in_at` merges the backoff deadline. There is no per-target hold for a store-wide
  condition; revision 1 named an "existing `InventoryUnstable` hold" that does not exist, and a
  per-target hold would repeat the penalty mistake recorded in `handoff.rs`.
- **Own writes (review M5).** The actor counts the five-family writes it performs itself. A restart
  caused only by them is not charged, so a user editing does not drive background owners into
  backoff. Writes by other actors sharing the store are charged.

## 5. The owners, site by site (review H5)

`pending` means the owner returns "not yet this visit" with its own work retained; `explicit`
means a native request that must answer now.

| site | kind | profile | on not-ready | priority / gate | notes |
|---|---|---|---|---|---|
| `receiver/replay.rs` manual evidence move | background | receive | keep `pass.manual`; retry next replay turn | replay turn, `replay_ready()` | **first to convert**; single site, already paced |
| replay's Ready path through `studio_transaction_with_publication` | background | receive | as above | replay turn | today reaches the synchronous scan in `studio.rs`; needs a seam that injects a budget into the ordinary Apply path (review H5) |
| `receiver/handoff.rs` H5 commit | overlay | full (L7 decision) | stay `Ready` with the embedded job | `replay_ready()` | embedded job, ownership lent (2.1) |
| `receiver/handoff.rs` H1 capture | overlay | full (L7) | new pre-capture stage holding target, basis, ownership and the job (review M1) | `replay_ready()` | basis and tenure re-derived in the completing visit |
| `receiver/catchup.rs` page persist (`catchup.rs:987` region) | background | receive | keep the PageReady pass | ungated today | |
| catch-up begin-pass (`:1108` region) | background | receive | **do not advance `selection`** until the budget is minted (review B1's second route) | | |
| catch-up authoritative serve (`:670` region) | background | receive | **put `self.service` back** | serving priority | otherwise the reserved interest is dropped every visit |
| `receiver/discovery.rs` (two sites), `registry.rs`, `registry_runtime.rs` (two) | background | receive | retain their attempt | as today | |
| `receiver/rotation.rs` | background | full | retain | as today | |
| `receiver/repair.rs` (seven sites) | **mixed**: `repair_fault` and `repair_registry_fault` are reached from explicit control requests | per site | explicit sites stay synchronous (6) | | **coordinate with Agent 3**; their writers |
| `studio/receiver.rs` receive | background | receive | packet retained (pre-drain) | `gossip_runs` | **last**, behind section 7 |
| `studio/control.rs` (five sites) | explicit | full | synchronous (6) | | archive finish depends on one visit |
| `studio.rs` ordinary Save / Create / Apply | explicit | full | synchronous (6) | | depends on one visit |
| `creative_references.rs::creative_pinned_cids` | reference scan | reference | n/a | lifecycle custody | out of scope: no budget |

`save_server`'s snapshot write (receive, catch-up's `budget()`) moves to the completing visit only,
so a not-ready visit does not rewrite the server record each time (review L4).

## 6. Explicit requests stay synchronous, as a recorded scope change

Ordinary Save, Create and Apply, and the five control actions, keep a synchronous scan through the
same job API with `budget: None` (never parks), each on a **fresh** job, never the runtime's shared
job (stepping a cursor with a parked record faults; review M5).

This is a scope change to G4-A1-C3 and to design section 15, and it is recorded as one rather
than assumed:

- 9.2's no-fallback rule governs overlay commit attempts; these are not overlay commits.
- Ordinary Save genuinely depends on one visit (reference refresh, transient pre-hold, PIX
  promotion, then the scan), and so does control archive finish (inspection freshness).
- Its maximum continuous custody is to be **measured and accepted explicitly** (17.1), not implied.
- It must be **unreachable from background owners**: replay's Apply path gets the budget seam in
  section 5, and background repair and rotation use the shared job.
- It is never used for Flow S S1b/S3 budget acquisition, which 9.2 forbids; that question must be
  closed before native Save registers.

If the review rules this out, the smallest alternative is the existing "Studio storage busy; retry
the same request" contract (native `studio.rs`) for the pre-write phase of the fully re-derivable
actions (acknowledge, restore pointer, dispose, release); archive finish and ordinary Save would
then need scheduled delivery.

## 7. Receive converts last, behind evidence (review H2)

Native paces receiver visits at one per second and the threat model says that cannot be bypassed;
one pass consumes at most one packet; and every uncached record parks. So a converted receive costs
at least one second per uncached record per packet, and several active servers sharing a store
restart each other's jobs into `Unstable` and backoff. Partial 13.7 data shows small-record
validation in microseconds, so parking those records buys nothing.

Receive converts only after **one** of:

- a reviewed conservative classifier from the partial 13.7 data, so small records of the uncached
  families validate inline; or
- batched validation: all bodies parked in one visit go to one detached job.

And after measuring visits per inventory and the restart rate with a second actor writing. If
receive stays synchronous, limitation L6 is stated to cover it.

## 8. Order of work

1. S-1 to S-4 (storage), one review.
2. Replay's manual move plus the Apply-path budget seam.
3. Handoff H5, then H1, with the embedded job and lent ownership.
4. Catch-up: persist and maintenance sites first, serve sites last; repair sites with Agent 3.
5. Receive, behind section 7.

Each step: focused tests and mutations, full suites, an adversarial review. The I-4 writer audit is
**re-run at activation** of step 2: until now every production scan was synchronous, so an
under-rotating writer was harmless; a parked cursor is what makes it unsafe.

## 9. Review findings, answered

| finding | answer |
|---|---|
| B1 queue head and `replay_ready()` circular wait; selection advanced before budget | 2.2 (no queue, turn-based); 5 (begin-pass keeps `selection`) |
| B2 second permit deadlocks the pool | 2.1 (embedded job, lent ownership) |
| H1 job API drops the receive limits | 3, S-1 |
| H2 throughput premise false | 1, 3 S-3, 7 |
| H3 no cancellation variant | 4 (tokens, `InventoryCancelled`) |
| H4 permit after body; pause strands it | 4 (permit before step; pause and lock release) |
| H5 owner inventory incomplete | 5 |
| M1 H1 has no stage surviving not-ready | 5 (pre-capture stage) |
| M2 one deadline per visit | 3 S-3 |
| M3 pending, wake, backoff | 4 |
| M4 detach ordering, Parked vs Validating | 4 |
| M5 explicit requests drain the shared job | 4 (own writes uncharged); 6 (fresh job) |
| M6 lock leaves plaintext resident | 4 |
| M7 faults | 4 |
| L1 nonexistent hold, per-target penalty | 4 |
| L2 queue bound | moot (no queue) |
| L3 cursor does not recheck mount on step | 4 (identity before any step) |
| L4 `save_server` placement | 5 |
| L5 one budget serves several writes in a visit | 1 (stated) |
| L6 directory iteration across visits | 3 S-4 |
| L7 handoff limits vs threat model | 3 S-1, 5 |
| L8 mint does not check `inventory_generation` | 3 S-2 |

## 10. Tests the plan adds

Deadlock regressions (the B1 sequence; four actors at H5 on a four-permit pool, no second permit);
pool full means no step and no resident body; a cancelled waiter keeps the permit until the worker
ends; cancelling the validation waiter leaves catch-up's `pass`, `registry_pass` and `in_flight`
intact; pause and lock release a non-detached job and its plaintext; `pending` false while
`Validating`, true when a result arrives, and Backoff reports its wake deadline; the receive profile
survives a restart (with a mutation); the visit deadline is deterministic under `SteppingClock` and
visits per inventory are pinned with uncached families present; another actor's write and an
explicit Save mid-job (packet retained, nothing pauses); token mismatch, mount change and a
validation error each lead to no pause and a fresh job; replay's Apply path does no synchronous
scan on a background turn; the serve reservation is restored on not-ready; and a mutation for each
new guard (token check, cancelled arm, permit-before-step, profile on restart).

## 11. Revision 2 re-review (2026-10-06): decisions for steps 2 to 5

The re-review (Opus, static, against `bea7e701`) closed B1, H1, H3 (shared job), H4, M2, M4, M6,
L2 to L5 and L8, left B2, H2, H5, M1, M3, M5, M7, L1, L6 and L7 partial, and found four new highs.
**Step 1 is ready**, with the corrections already applied to section 3. The decisions below
answer the rest; each is reviewed again with the step that builds it.

**One profile for the shared job (N-H1).** The shared job is `receive()` for every owner on it,
which is what rotation and catch-up use today, so nothing on it widens. Replay's ordinary Apply
path runs at full limits today (`studio.rs`) and is **not** moved onto the shared job: step 2
converts only the manual evidence move. The Apply path keeps its synchronous full-limit scan, and
its reference scan (`creative_pinned_cids()` when references are unknown, review N-M5), as a
**stated limitation**: a background turn can still run one unbounded synchronous scan through
replay's Apply until a later step gives it a budget seam that defers the Apply when references
are unknown.

**Overlay jobs must not starve serving (N-H2).** A visit in which an overlay job only stepped its
inventory does not return early; it falls through to replay and catch-up. Overlay stepping yields
to `handoff_priority()` exactly as H3 does, and takes at most half of the visit deadline.

**Overlay jobs must not take the pool's last permit (N-H3).** Overlay admission refuses when only
one permit is free, so non-overlay inventory always has a permit to step with. On that refusal the
actor drops a retained Registry provider, as `prepare_for` does.

**Catch-up is gated like receive (N-H4).** Step 4 does not ship until section 7's evidence exists
(a calibrated classifier or batched validation, plus measured visits per inventory and restart
rate), because while receive is synchronous its writes restart a catch-up job that only steps on
roughly one visit in four. Replay starving behind synchronous receive at step 2 is **accepted and
stated**: the manual move is not time-critical.

**Own writes, by token identity and bounded (N-M1).** A counter cannot prove a restart was caused
only by this actor. Instead the actor snapshots the store's inventory-generation token at the end
of each of its visits; at the next visit's start, pointer-equality with the current token proves
no foreign write landed in between, and only then is a restart uncharged. Uncharged restarts are
capped per job (three), so the actor's own steady writes cannot churn detached validations forever.
This needs a read-only token accessor on the store.

**The lent-ownership handoff stage (N-M2, H3, M7, L1).** A new handoff stage holds no ownership
while the validation runs and behaves like `Detached`: not runnable, no wake, exempt from
`release_if_stalled`. Its result is a handoff-routed
`InventoryValidated(token, Result<(validated, ownership)>)`; cancellation reuses
`HandoffCompletion::Cancelled(token)`. A validation error abandons the job and holds the target,
never pauses the receiver, and the worker releases the ownership itself on that error (RT-001).
`handoff_check_authority` then works unchanged: it abandons the job, and the returned ownership is
dropped as not this actor's. Instability in the embedded job uses the runtime backoff, never the
per-target hold (N-L4).

**A reachable H1 pre-capture stage (N-M3).** `handoff_probe` returns as soon as the runtime is
busy, so the stage needs its own `can_capture(now)` arm in `background_step`, and `runnable`,
`wake_in` and `release_if_stalled` must cover it. It records the tenure and MLS epoch it was opened
under, so `handoff_check_authority` can abandon it.

**Explicit repair sites, and split helpers (N-M4).** Section 6 adds `repair_fault` and
`repair_registry_fault`. `inventory_budget` and `budget()` serve both explicit and background
callers, so they are split into a synchronous explicit helper and the shared-job path before any
of their callers converts; the repair sites stay Agent 3's to coordinate.

**Pending terms named (M3).** "Stepping with a permit" cannot hold between visits, because the
permit is released at the end of a visit that did not park. So `studio_pending` adds exactly two
terms: a `Parked` body awaiting detach, and a job in `Backoff` whose deadline has passed. The owner
terms masked during backoff are only the inventory-blocked ones (an owner whose next step is
"wait for the budget"), never page-service interests.

**Smaller corrections.** `Parked` carries its permit, and `Validating` has no job field that could
be stepped (N-L1, M7). An owner checks its own preconditions before stepping, in the same turn
that may complete and mint (N-L5). A `Stepping` job that no owner has stepped for 32 visits is
dropped, closing its directory handle (N-L6).

**Order, as the re-review left it.** Step 1 now. Steps 2 and 3 are independent and each shippable
alone: step 3 needs N-H2, N-M2 and N-M3 built; step 2 needs N-M1, the profile decision and the
stated Apply-path limitation. The I-4 writer audit runs before whichever of them ships first,
because that is the first production code that parks a cursor. Step 4 waits for N-H4's evidence,
N-H3's admission rule, N-M4's helper split and Agent 3. Step 5 is unchanged.
