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

## 12. Step 1 as built, and the plan for step 2

**Step 1 is implemented** (`2df3564f`): S-1 to S-4 as in section 3, with one correction its
review forced. The first cut of `drive_epoch_inventory_job` passed its absolute deadline to the
cursor's relative step, which adds it to the current time, so each step's own expiry was disabled;
the cursor now has `step_until`, which takes an absolute deadline as given, and a regression pins
the exact entry count. An issued inventory starts with a token matching nothing, so only the
finish stamp makes it mintable.

**Step 2, the replay manual move: staged, not yet shipped.** The site is the end of
`StudioReceiver::replay_step` (`receiver/replay.rs`), reached when a pass's order is empty and
`pass.manual` still holds evidence to move into recovery. Until step 2 it scanned synchronously
and minted. The staged build (private worktree; its design review, I-4 writer audit and
adversarial review are recorded below; it waits only on Agent 2's go-ahead for the two shared
files) is:

1. **`receiver/inventory.rs` (new), `InventoryRuntime`**, a field of `StudioReceiver`: the
   section 4 state machine over one boxed `EpochInventoryJob` with
   `EpochInventoryProfile::receive()`, in states `Idle | Stepping | Parked | Validating |
   Installing | Backoff`. `Installing` holds a result that arrived outside custody until the next
   owner turn installs it, which is also where a validation error surfaces (M7). The entry is
   `budget_turn(store, id, group, clock, deadline_ms, pool) -> InventoryTurn` with
   `InventoryTurn = Ready(Box<EpochStudioBudget>) | NotYet | Unstable`; `Ready` is minted in the
   call that finished the job.
2. **The replay site** keeps `pass` (its `manual` set and empty `order`) on `NotYet` and returns
   `Ok(None)`; every replay turn re-derives the evidence and re-screens `pass.manual` before
   reaching the site, which is N-L5's rule. It does **not** depend on the job finishing. On
   `Unstable` (including any turn inside the runtime's backoff), or once the pass has waited
   `MANUAL_MOVE_PATIENCE_MS` (60 s) since it first asked, it mints its budget the way it did before
   step 2, from one synchronous `receive()`-profile scan in that visit, and moves the evidence. On
   the patience path it first releases whatever the job holds; on `Unstable` the runtime keeps its
   backoff for its other owners. This is the answer to the review's HIGH-1, below. It is outside
   9.2's no-fallback rule, which governs overlay commits (section 6), and it adds no custody class:
   it is the scan this site always ran, under the limits every receive packet already pays.
3. **The visit deadline** is `background_step`'s one clock sample plus `INVENTORY_SLICE_MS`
   (250 ms, an experiment value until 13.7, like H3's slice), passed down to replay (S-3), so
   time the H-stages spent comes out of replay's share.
4. **Detached validation**: `StudioBackgroundJob::InventoryValidate(InventoryDetach { body,
   permit, token })`, `StudioBackgroundResult::InventoryValidated(token, Box<Result>)` and
   `InventoryCancelled(token)` in `receiver/catchup.rs`, placed in `detach()` after
   Prepare/PrepareRegistry and before the `in_flight` check. The permit is reserved from the
   shared preparation pool before any step; a full pool means no step. Item 4's earlier wording
   also had the inventory refuse the last free permit; that contradicted N-H3, which puts the
   last-permit rule on **overlay admission** so that non-overlay inventory always has one. N-H3 is
   step 3's to build; step 2 takes any free permit and holds nothing while it waits.
5. **Own writes (N-M1)** by token identity, not counting. `ServerStore` gains a crate-private,
   opaque `EpochInventoryQuiet` mark (comparable, never installable) and
   `restart_epoch_inventory_job_uncharged`, which gives an overtaken job a fresh cursor without
   charging `MAX_INVENTORY_RESTARTS`. `StudioReceiver::run` now wraps the visit: `begin_visit`
   compares the mark before anything in the visit can write and, if the token moved since the last
   visit ended, taints the current cursor as overtaken by someone else until a fresh cursor
   replaces it; `end_visit` marks the token on every exit, early returns included. Only an
   untainted job is refreshed uncharged, at most three times per job, and a cursor that never
   stepped (its turn found the pool full) is refreshed free without counting. A restart found on
   resuming keeps stepping in the same turn, since nothing can overtake the fresh cursor inside an
   exclusive visit. Control requests that do not run through `run` look foreign and are charged:
   the safe direction (`Apply` and `ApplyOverlayCopy` controls call `run` after their own
   preparation writes, which is the same direction). N-M1 softens own-write restarts; it does not
   make the job immune to writes between turns, which is why item 2 does not depend on the job.
6. **Lifecycle**: `pause()` and `clear_previews()` (which the UI-lock reset calls) release the
   job; a mount or numeric-server change drops it. A `Validating` job's late result carries a
   token nothing waits on and is dropped on arrival.
7. **Stated limitation**: replay's ordinary Apply path still scans synchronously (section 11).

Three places where the build departs from sections 4 and 11, each forced by a defect the plan had:

- **`studio_pending` gains one term, not two: a `Parked` body awaiting detach.** An expired
  `Backoff` (and an `Installing` result) is consumed only by an owner's turn, and owners are paced
  by their own schedules. Reporting either as driver work would hold the driver at its active
  cadence until the owner's next turn, and for an expired backoff no owner wants any more, forever.
  An expired backoff is instead cleared by the next visit's lifecycle. **There is no wake term.**
  The price, which the review asked to be written down: in a quiet actor a result waits for the
  native five-second idle wake plus replay's alternation, so the job advances about one uncached
  record per six seconds.
- **The idle drop is 30 s since an owner last asked, not 32 visits** (N-L6). Every inbound packet
  is a visit, so under sustained gossip 32 visits can pass between two of replay's paced turns,
  and dropping a job its owner still wants restarts it every time: a livelock for the owner.
  `Installing` is covered by the same drop.
- **Pending was planned as "true when a result arrives"** (section 10). That is the spin above;
  the regression now pins the opposite.

**What step 2 actually buys, stated honestly.** Until 13.7 calibrates the classifier every
uncached record parks, so the turn-based path finishes in bounded custody per visit only for a
vault with few Recovery, OwnerReceipts, Intents and DraftArchive records and no write between its
turns. Otherwise the move ends in the synchronous fallback, which costs what it always did. Step 2
is therefore the runtime, its lifecycle and its tests, ready for the classifier, rather than a
custody improvement for busy vaults today.

### Review of the staged step 2 (2026-10-06, Opus, static, worktree at `2df3564f`)

No blocker. Dispositions:

| finding | disposition |
|---|---|
| **HIGH-1** the move can stall indefinitely under ordinary writes between turns (uncached families are re-validated on every fresh cursor), and while it waits its pass holds replay for every target; the N-M1 doc claim was false | **fixed**: item 2's synchronous fallback on `Unstable` and after 60 s of patience, so the pass's hold on replay is bounded; the module doc corrected. Regressions: a write between every visit still completes; a backoff turn scans synchronously with nothing detached and keeps the backoff; a job that can never step falls back no earlier and no later than its patience |
| MEDIUM-1 the H3/H4/M6 wiring untested at the receiver | **fixed**: one receiver-level test, against the state the production `run` leaves: a parked body is pending; it detaches with catch-up's `in_flight` set; a pre-cancelled validation routes to the runtime (`idle`, permit back); `pause` and `clear_previews` release a parked body and its permit. Each of the five mutations named was executed and killed |
| MEDIUM-2 replay tests on the global four-slot pool | **fixed**: `watch()` injects a private pool. A full-suite failure of `registry_runtime::studio_held_registry_page_is_discarded_after_fault_or_checkpoint_replacement` (passes alone) is consistent with this contention |
| LOW-1 quiet check after the early return | **fixed**: `begin_visit` at the top of `run`; regression with an early-return visit |
| LOW-2 validation errors pause the receiver; the documents disagreed | **decided and stated**: a validation error, or a worker `JoinError`, surfaces as the error the synchronous scan it replaces would have raised, and so pauses the receiver as before. Section 10's "no pause" wording is superseded. The `JoinError` mapping has no test (a panicking validator cannot be injected without a new seam); recorded gap |
| LOW-3 a resume-time restart wasted the turn; a never-stepped cursor spent the own-restart cap | **fixed**: the drive loop keeps stepping after such a restart; a cursor with no progress is refreshed free and uncounted; regression for the second |
| LOW-4 comment and document drift; HANDOVER and THREAT-MODEL; no diagnostic for a waiting move | comments and this section **fixed**; HANDOVER and THREAT-MODEL are updated in the commit that ships step 2; a diagnostic for a waiting or backing-off move is a **follow-up** |
| LOW-5 the uncharged-restart waiver was `pub` | **fixed**: `pub(crate)`, with the mark and its comparison |

**Re-review of the fixes (2026-10-06, the same reviewer, static):** no blocker or high; HIGH-1,
MEDIUM-1 and MEDIUM-2 closed; `confirm_listing` judged correct and fail-closed (same classifier as
the traversal, exact-key and count checks, entry limit consistent, reference path checked before
protection is installed). Its findings:

| finding | disposition |
|---|---|
| MEDIUM (design question) with 60 s of patience, a vault holding more than about ten uncached records cannot finish the turn-based path in a quiet actor, so the common case pays the delay and the detached validations and then runs the old scan; ship before 13.7 or not | **decided: ship.** Nothing unsafe follows, maximum custody is unchanged, the move is not time-critical, and the runtime, its lifecycle and its tests are what steps 3 to 5 and the classifier need. Stated in "What step 2 actually buys" above and in HANDOVER when it lands. Revisit the patience figure when 13.7 lands |
| LOW-1 the patience path's `release()` lifted a store-wide backoff | **fixed**: `abandon_job()` drops the job and keeps a backoff |
| LOW-2 the patience clock lives on the pass, so a rebuilt pass restarts it | **accepted**: the writes that rebuild a pass also restart the job, so `Unstable` still bounds the wait |
| LOW-3 a stale module-doc sentence (`Unstable` carrying a time); the `end_visit` doc's control-request claim | **fixed** |
| LOW-4 the cancellation test asserted only the inventory side | **fixed**: catch-up's `in_flight` is held through the cancelled completion and asserted still set. A waiter cancelled while its blocking closure already runs is still untested |
| LOW-5 holes in the raw-fs gate: a `#[cfg(test)]` on a non-item could swallow production lines; `File::options()` and explicit syncs unmatched; AGENTS.md does not list the gate | **fixed** in the script: only items are skipped, an unclosed skip fails the gate, `};` closes, and the pattern covers `File::options` and `sync_all`/`sync_data`. The hardened scanner immediately found a site the first version had swallowed through a `#[cfg(test)]` struct-field initializer: `remove_server`'s unlink of the non-family `.bin`, `.net` and `.cache` files, benign and already audited, now allow-listed with its reason. The scanner relies on rustfmt layout, which CI enforces. AGENTS.md is a local, git-ignored file, so the line adding the gate to its handoff checks is proposed to the user rather than made |

The re-review also asks THREAT-MODEL to record a behaviour change: on a filesystem whose directory
streams are unstable (SMB, FUSE), synchronous receive and reference scans now refuse, and so
pause, instead of issuing an undercounted inventory. That goes in with the commit that ships step
2, beside the bounded member-driven delay of a manual move.

Residual risks the review listed, accepted for step 2: a `Stepping` job holds a `ReadDir` on
`servers/` for up to 30 s between visits; overlay and handoff work can exhaust the pool and starve
the job until N-H3 (step 3), which now only delays the move until its patience runs out; a parked
body survives a visit whose lease is cancelled or that errors without pausing, until the lock
reset or the next good visit; each waiting turn re-derives the replay evidence; the mount-change
drop is pinned only by the store's own mount check.

### I-4 writer audit for step 2 (2026-10-06, Opus, static, at `2df3564f`)

Required by section 8 before the first production code that parks a cursor. No blocker or high:
no writer was found that mutates a five-family file without rotating `inventory_generation` first,
and every cross-visit consumer rechecks the token. The audited writer list is recorded in the
status ledger. Dispositions:

| finding | disposition |
|---|---|
| M-1 raw `std::fs` mutation compiles in every store module; only the guarded primitives are type-level | **fixed**: `scripts/check-store-raw-fs.sh`, in CI beside the ambient gate, refuses raw mutation in non-test store code outside `mod persistence`, the store's directory creation at open, and the three per-family sync helpers that take `&EpochMutation` (each allowed exactly once, and a stale allowance fails). Verified by planting a raw `fs::write`. The overclaiming comments in `store.rs` and design 9.2 corrected |
| M-2 a `ReadDir` held across visits beside non-family churn (`save_server` on most packets) can skip an entry on a filesystem without stable directory streams; the inventory would undercount | **fixed**: every finish, budget and reference scan, confirms the traversal against one fresh names-only listing; a mismatch is an invalidation (the job restarts), refused at the public finish. Regressions for a missed record, an extra record, the job's restart and the reference scan; the check's removal was executed and killed |
| M-3 read-only and duplicate paths rotate (completed-handoff page serve, duplicate page ingest, Registry receive sync, maintenance flush), so a polling peer restarts jobs | design 9.2's "not on reads" **corrected**; liveness is no longer at stake for replay (item 2). Memoising already-durable sync-repairs per mount is a **follow-up**, needed before catch-up converts (step 4) |
| L-1 the token was captured after `read_dir`, which on Windows reads the first entry | **fixed**: all three tokens are cloned before the directory is touched |
| L-2 `EpochIntentBudget::from_inventory`, `records_for_server` and `EpochStorageBudget::from_inventory` skip the S-2 token check | **follow-up**: no production minting path uses them today; narrow or check them before one does |
| L-3 no per-site rotation tests for the Registry family, the Studio unchanged and discovery syncs, the handoff syncs, retirement and `flush_checked_epoch_intents` | **follow-up**: structurally guaranteed by the guard parameter, and M-1's gate now covers the bypass that would make the gap matter |

Tests (focused, all passing): thirteen runtime unit tests in `receiver/inventory/tests.rs` against
a real store, real records, a manual clock and a private pool (park and detach with the permit,
full pool, token routing and abandoned results, cancellation, a validation error surfaced once,
the charged restart storm and its backoff, own writes uncharged up to the cap, a foreign write in
an earlier gap still charged, an early-return visit not laundering one, a never-stepped cursor
refreshed free, a packet burst not dropping a wanted job, numeric-server change); in
`studio_exchange/tests/replay.rs` the existing restart test now drives the actor's
detach/complete loop (a recorded contract change: the manual move no longer completes in one
`run`) and asserts the budget came through a detached validation, that a result's arrival does
not make the receiver pending, and that every visit leaves its mark, plus the four HIGH-1 and
MEDIUM-1 regressions above; and the M-2 regressions in the store. Mutations executed and killed:
no permit, any-token install, no backoff, error not surfaced, no idle drop, server change ignored,
`Installing` never installed, `Installing` reported pending, no `Installing` idle drop, a
synchronous scan at the replay site, no end-of-visit mark, taint never set, no own-restart cap,
taint not cleared by a restart, no fallback on `Unstable`, no patience, no cancellation arm, no
release in `pause`, no release in `clear_previews`, detach behind `in_flight`, no pending term, no
listing check.

Files touched outside Agent 1's own: `studio/receiver.rs` (the field, `run` wrapping `run_visit`
for the mark, the lifecycle call, the deadline in `background_step`, the pending term, the pause
and clear hooks) and `studio/receiver/catchup.rs` (the variants, their `run`/`detach`/`complete`
arms and the test label; `preparation_pool` made `pub(super)`). Both are areas where Agent 2 works,
so the edits are kept to those lines and are ported only once Agent 2 confirms neither file is
mid-edit.
