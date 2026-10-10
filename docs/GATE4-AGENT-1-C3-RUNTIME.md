# Gate 4 Agent 1: C-3 runtime adoption (design, revision 3)

Status: **step 1 (section 3) accepted for implementation; steps 2 to 5 need the decisions in
section 11 reviewed before each is built. Section 14 proposes the classifier from measurement 13.7;
it is at revision 2 after its design review and re-review, A and B are built, and step 3 stays
gated (14.5). Section 15 proposes a route to step 3; its design review found it not yet ready,
and 15.7 lists what step 3 needs, starting with design 9.1. 9.1 is built and the commit phase is
measured, so 15.8 is revision 2 of that route. Its design review (15.9) found no blocker and one
high: H5's cost grows with the source. 15.10 has since measured that: the commit alone exceeds
the visit for a maximal Flipnote with a full branch. 15.9's order of work replaces 15.8's, and
15.10 sizes its first item.** It extends
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
  **Superseded 2026-10-08:** the classifier is calibrated (section 14.2), so small records of those
  four families validate inline under a deadline. Registry and Studio still always detach when
  cold.

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

**Updated 2026-10-08.** "Every uncached record parks" is no longer true. The first condition above
now holds (section 14.2): small records of the uncached families validate inline. Cold Registry
and Studio records still park. The measurements in the last paragraph are still outstanding, so
receive still does not convert.

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
| M-3 read-only and duplicate paths rotate (completed-handoff page serve, duplicate page ingest, Registry receive sync, maintenance flush), so a polling peer restarts jobs | design 9.2's "not on reads" **corrected**; liveness is no longer at stake for replay (item 2). The completed-handoff page serve, which a polling peer drives on every request, is **fixed**: `sync_intent_unless_durable` skips a flush this mount already made of a file nothing has written since (a `RepeatSyncMemo` of path, length and token; exact under I-4, and a skipped flush does no I/O to rotate for). Its own review found one HIGH, that each note's own rotation emptied the memo so two targets served in turn still rotated every time; **fixed** by carrying entries across the memo's own flushes, which change no file's contents or names, with a failed flush never remembered. Bounded at 64 entries; a peer cycling through more completed targets than that is back to flushing on every serve (residual). Regressions: the completed-handoff serve, two files in turn, a failed sync, and the bound; four mutations killed. The duplicate page ingest (which a peer drives by pushing pages), Registry receive sync and maintenance flush sites remain a **follow-up**, needed before catch-up converts (step 4) |
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

## 13. Step 3 is gated on 13.7, like steps 4 and 5 (decision, 2026-10-07)

Section 11 called steps 2 and 3 "independent and each shippable alone". Step 2's review (HIGH-1)
showed why that is not true of step 3 as things stand. Until 13.7 calibrates `validation_fits`,
every uncached record parks, so a job needs one owner turn per Recovery, OwnerReceipts, Intents
or DraftArchive record, and any five-family write between two turns restarts it and throws that
progress away. Ordinary receive writes a Studio source on most packets. So in any active channel
a job may never finish.

Replay's manual move survives this by falling back to the synchronous scan it always ran (section
12, item 2). **H5 cannot**: it is an overlay commit, and design 9.2 forbids a fallback to an
unbounded single-visit scan there. Built now, step 3 would leave an automatic handoff held
indefinitely in any busy channel, where today it completes in one (expensive) visit. Limitation
L6 accepts "a commit is held and retried under sustained writes"; it did not price "sustained"
as "ordinary gossip".

So step 3 waits, with steps 4 and 5, for section 7's evidence: a reviewed conservative classifier
from 13.7 (so small records of the uncached families validate inline and a job finishes in one or
a few turns), or batched validation. **Tightened by 14.5 (2026-10-08):** "one or a few turns" is
not enough for H5. Under ordinary gossip it must finish in one visit with no park, and the
classifier alone does not get it there. The existing 13.7 data already narrows the classifier:

- **Recovery** validation is byte-linear (about 1.4 to 2.6 us per KiB accounting, 13 to 15 with
  references), so a small-record byte threshold is safe for it. **Qualified by 14.1:** the
  accounting figures are from opaque projections, and structured shapes are unmeasured.
- **Studio** validation is driven by structure, not bytes (a 130 KB record of 128 frames costs
  about 240 ms; a 3.7 MB title-only record about 24 ms), so no byte threshold is safe and Studio
  keeps detaching unless the validation cache already holds the record.
- **Registry** sits between (about 2 ms at 322 KB, 17 ms at 3.9 MB).
- **OwnerReceipts and Intents** are measured only at trivial sizes, and **DraftArchive** not at
  all. Those three, at their accepted ceilings, are the next measurements, and they decide whether
  the classifier makes the uncached families cheap enough for step 3 to finish in practice.
  **Measured 2026-10-08; section 14 is what follows from them.**

Flow R (G4-A1-R) follows step 3, because its commit needs the same embedded job; the sequencing
note in the status ledger already forbids building it on the unbounded path and splitting it
again.

## 14. The conservative classifier (proposal, revision 2, 2026-10-08)

Revision 1's design review (Opus, static, against `a62a7f80` plus this worktree's uncommitted
changes) found no blocker, three highs, three mediums and six lows. All are accepted; 14.7 maps each
to its answer. The most important is HIGH-1: **A and B do not unblock step 3**, and revision 1 was
wrong to propose that they did. Step 3 stays gated, and 14.5 says what would unblock it.

The measurements are in the status ledger under "The uncached families at their ceilings
(2026-10-08)". Part A is the classifier. Part B stops a detached result from being wasted when its
job restarts. Part C is an open question about the cache.

**Parts A and B are built (2026-10-08), in the store only.** The status ledger entry "C-3
classifier and refused-result memo, built" records the tests and mutations. Part B's runtime half,
where step 2's uncharged refresh calls the store entry point before discarding a result, lands
with step 2. Part C and step 3 are not built.

### 14.1 What the measurements allow

- **Accounting mode, four families can be bounded, each with a stated limit.**
  - **Recovery** costs about 2.3 us per KiB to 4 MiB, and 2.7 in the earlier contended profile.
    **That is on opaque projections only** (`stage_sized`: filler projection; empty tombstones,
    elements, conflicts and applied operations). The accounting decode does work per item: it
    decodes each item, `id()` re-encodes and hashes every snapshot, `slots.encode()` re-encodes
    them, the `completed_eviction` check recomputes ids, and `footprint` re-encodes the staged
    snapshot. A structured record is plausibly several times denser per byte, and **structured
    shapes are unmeasured** (review HIGH-3).
  - **Intents** costs at most 12.9 us per KiB at the upper median. Its densest measured shape is a
    Closing branch at the operation ceiling. The single worst sample is 14.7 us per KiB, a 64-op
    branch in run 4. The seed is copied as opaque bytes, so a larger seed only adds bytes.
  - **OwnerReceipts** is not a byte rate. The two journals measured are version 1, which verify
    nothing, which is why they fell below resolution. **Version-2 journals verify Ed25519
    signatures during decode** (`validate_v2` and `validate_receipt` call `verify_signature_only`),
    and so do fault-record pairs. Their cost is a fixed part plus one verification per signature,
    and nothing measured says what it is (review M-1).
  - **DraftArchive** accounting does no work that grows with size, by construction
    (`validate_record_body` calls `storage_record` and never decodes the payload).
- **Studio and Registry cannot be bounded by bytes in either mode.** A 130 KB Studio record of 128
  frames costs about 240 ms; a 3.7 MB title-only one costs about 24 ms.
- **Reference mode cannot be bounded by bytes for Intents.** `base_blob_cids` rebuilds the seed's
  graph: a one-operation branch with a trivial seed already costs 175 us per KiB, and a dense seed
  would cost what the same graph costs as a Studio record. Recovery's reference collection is
  measured only to 142 KiB.
- **No production path reaches the classifier in reference mode.** The one production reference
  scan, `creative_pinned_cids`, steps with no deadline (`EpochStorageScan::step`), so it never
  classifies. Every C-3 job, shared or embedded, is a budget inventory, which is accounting mode.

### 14.2 Part A: the rule

```rust
const SAFETY: u64 = 4;         // margin for a slower machine and run-to-run variance
const INLINE_CAP_MS: u64 = 25; // no inline validation is admitted against more than this
const FIXED_US: u64 = 100;     // per-record constant; every small record measured is under 16 us

/// (rate in us per KiB, envelope in physical bytes), accounting mode only.
fn calibration(family: EpochRecordKind) -> Option<(u64, u64)> {
    match family {
        // Measured on opaque projections only (14.1). The envelope is held where FIXED_US
        // dominates until structured shapes are measured, so a structured record several times
        // denser per byte still sits far inside the cap.
        Recovery => Some((3, 64 * 1024)),
        Intents => Some((16, 4_197_020)),
        // At most one version-2 signature fits in 747 bytes, and one verification (about 50 us)
        // fits inside FIXED_US. The small envelope carries this bound, not the rate.
        OwnerReceipts => Some((16, 747)),
        // The one envelope beyond what was measured (6 334 584 bytes). Accounting never decodes
        // the payload, so there is no size-dependent work to extrapolate.
        DraftArchive => Some((0, MAX_DRAFT_ARCHIVE_SEALED_BYTES as u64)),
        Registry | Studio => None, // driven by structure
    }
}

fn validation_fits(family, size, references, remaining_ms) -> bool {
    if references { return false; }                   // 14.1: no byte bound, no production caller
    let Some((rate, envelope)) = calibration(family) else { return false };
    if size > envelope { return false; }              // the envelope, see each family above
    let predicted_us = FIXED_US.saturating_add(rate.saturating_mul(size.div_ceil(1024)));
    // `remaining_ms` is floored to whole milliseconds, so up to 1 ms of it may already be gone.
    let budget_ms = remaining_ms.saturating_sub(1).min(INLINE_CAP_MS);
    predicted_us.saturating_mul(SAFETY) <= budget_ms.saturating_mul(1_000)
}
```

The largest record admitted, at a full budget (26 ms or more remaining):

| family | rate used | worst measured | largest admitted |
|---|---|---|---|
| Recovery | 3 us/KiB | 2.7 us/KiB, opaque projections only | 64 KiB (the envelope binds) |
| Intents | 16 us/KiB | 14.7 us/KiB, worst single sample | 384 KiB |
| OwnerReceipts | fixed part plus at most one signature | below resolution, version 1 only | 747 bytes (the envelope binds) |
| DraftArchive | 0 | below resolution to the payload ceiling | the family's sealed cap, by construction |
| Registry, Studio | - | structure-driven | never |
| any family, reference mode | - | - | never |

**What the bound means.** On the calibration host, in a release build, for a shape no denser than
the densest measured, an admitted validation is predicted to take at most a quarter of the budget,
so at most 6.25 ms. The factor of 4 is the margin for a slower machine and for variance: the three
quiet runs agreed within 10%, and contended runs earlier moved by up to 86%. On a machine `k` times
slower the hold is about `k x 6.25 ms`: it reaches the 25 ms cap at `k = 4`, and is about 63 ms at
`k = 10`. **For Recovery and OwnerReceipts the envelope carries the bound, not the rate.** At
64 KiB, even ten times the opaque Recovery rate is under 2 ms. For scale, reading and
authenticating a 4 MiB record already takes 8 ms inline whatever the classifier says, and a step's
one-entry minimum already lets one record's read overrun the slice.

**Debug builds are not covered.** They run validation roughly an order of magnitude slower. That
affects development and test builds only; production does not depend on it. The classifier's
decisions are deterministic **under a deterministic clock**. A test driven by `SystemClock` can
inline or park depending on real timing, so no such test may assert which happened.

**The rule tightens as the slice runs down.** With 2 ms left the budget is 1 ms, so only predictions
of 250 us or less inline: Recovery to about 50 KiB, Intents to about 9 KiB. **With 1 ms or less
left, nothing inlines.**

### 14.3 Part B: a detached result whose job restarted still warms the cache

A detached Studio or Registry validation is thrown away when its job restarts, on two paths:

- **In the store.** When the result comes back after a write has moved `inventory_generation`,
  `install_validated` returns `Invalidated` and `install_validated_job_record` restarts the job.
- **In the step-2 runtime, before the store sees it** (review HIGH-2b). An overtaken `Installing`
  job is refreshed through `restart_epoch_inventory_job_uncharged`, and its result is "discarded
  with that cursor" (`budget_turn`, `ccd00dbc` `receiver/inventory.rs`). Receive's writes are this
  actor's own, so for the first `OWN_RESTARTS` restarts of every job this is the path taken. That
  is the gossip case.

Either way, the restarted cursor reaches the same record cold and parks it again. Under writes
that arrive every turn, that can repeat indefinitely.

**Proposal, in three pieces.**

1. **Evict on mismatch** (review HIGH-2a). In `step_inner`, on any authenticated read of a
   Registry or Studio record, if the cache holds an entry for the same key with a different size
   or digest, remove it. That includes reference scans, which never consult the cache but whose
   `install_body` put replaces by key anyway. The cache keeps one version per record, and the file
   no longer has those bytes, so the entry can never hit again. The invariant this rests on, to be
   written beside the cache: **every put describes bytes already on disk.** All three warm sites
   keep it, and breaking it would only lose a warm entry, never return a wrong one. Without this, a stale entry left by a writer that does not warm (the core
   Studio writer, or a warm that `cache_studio_source_footprint` or `remember_installed_registry`
   declined) makes every later refused result look "not vacant", and nothing is ever warmed.
2. **One store entry point for a refused result.** A store method (name to be settled) takes the
   job and the validated result. It checks the result against the job's current cursor: same scan
   identity, same mount as the cursor **and as the store's current `registry_mount()`** (a cursor
   from another `ServerStore` must not warm this one; review LOW), and the record the cursor is
   awaiting. Then it warms under piece 3. `install_validated`'s two `Invalidated` exits call the
   same code. **Step 2's uncharged refresh calls it before discarding an `Installing` result**, and
   so will step 3's embedded job when it exists. The warm and `install_body`'s existing put go
   through **one memoize helper** that takes the validated body, so the value type is decided in
   one place (re-review MEDIUM-2: see 14.5's memo).

   **As built:** the helper takes the accounting record rather than the whole body. That is
   because `install_body` has already moved the body's CIDs out by the time it memoizes. Its
   comment says the signature must grow to take the Intents facts before the memo admits Intents.
   A test pins that it admits only Registry and Studio today. Step 2's merge is gated on its
   uncharged restart taking the pending result itself, so that no discard path can skip the memo
   (implementation review M-1).
3. **Put only if vacant.** After piece 1, "vacant" means nothing has been put for this record since
   the scan read it. If a write path put a newer version in between, that entry stays. If a writer
   that does not warm rewrote the record, the warmed entry is a harmless miss, and piece 1 evicts it
   on the next read.

- **Why this is sound.** A cache entry is keyed by family and filename hash, and `get` also
  requires the physical size and the blake3 digest of the authenticated plaintext to match the
  bytes a later scan actually read. `validate_record_body` is a pure function of those bytes and
  that size. So a result validated before a write is still the right answer for the bytes it was
  computed from. If the write changed this record, the digest no longer matches and the entry is
  simply a miss.
- **What it relaxes, to be recorded in design 9.2 and the threat model when built** (review LOW).
  Consequence 2 of 9.2 rechecks a returned validation's generation before it is consumed. A
  refused result is still never installed into an inventory. Its pure accounting record is
  memoized, which is a use of a result whose generation check failed.
- **What it buys.** A cold record that is not rewritten, and stays in the cache, is validated once
  rather than on every pass. This is not "at most once" in general: eviction (part C) and
  rewrites both bring the cost back.
- **What it does not cover.** Each of these costs one revalidation:
  - a result dropped by token, for a job released while its body was out;
  - the step-2 runtime's other discards of an `Installing` result: `lifecycle`'s idle drop, which
    has only `&ServerStore`; `release()`, which has no store; and `abandon_job()` (re-review
    LOW-1).

  A warm into a full cache also evicts its least recently used entry, which may have been useful.

### 14.4 Part C, an open question: the cache is 64 records and LRU

`RecordCache` holds 64 entries and evicts the least recently used (`MAX_RECORDS`). A full scan
reads every record in directory order. So in a vault with **more than 64** Studio and Registry
records, LRU evicts each entry before the next scan reaches it again, and **every one of them
misses on every scan**. Then each one parks, which costs a turn, and any write between turns can
restart the job. Parts A and B cannot help there. The inventory's own record bound is 65 536
(`MAX_ACCOUNTED_RECORDS`), so this is not a corner case.

**Scope** (review M-3). The `receive()` profile caps a job at 64 records and 256 KiB of uncached
bytes, so the shared job refuses before it can thrash. Part C matters only for full-profile jobs:
H5, H1 and rotation.

Two options:

1. **A larger, indexed cache.** An entry is about 170 bytes, so 4 096 entries is about 0.7 MiB per
   mount. The current `VecDeque` does a linear `position` and `retain` on every call, which would
   not scale to that size, so this means a keyed map with an access order. Vaults beyond the new
   capacity still thrash.
2. **Scan-resistant admission at the current size.** A put from a scan never evicts; only write
   paths do. A full scan then keeps its first 64 hits instead of none, but a large vault still
   parks everything past 64. As worded, it would also pin entries for deleted or rewritten records
   forever, so it needs part B's eviction on mismatch as well.

**Recommendation: option 1**, with part B's eviction on mismatch, sized to the full-profile vaults
step 3 is meant to support, and with limitation L5 amended. **It is necessary but not sufficient.**
A hit still reads, authenticates and hashes the whole record, so the total bytes traversed still
bound what one visit can finish (14.5). It is decided with step 3, since nothing before step 3
needs it.

### 14.5 What changes, what does not, and what this unblocks

**What A and B do.** They remove the parks of small uncached records, and they stop a detached
Studio or Registry validation from being lost when its job restarts. That shortens the step-2
shared job and the first pass of any job. **They help each route to step 3 named below**, which is
why they are worth building now. The memo does not replace A: first passes and changed records,
H5's own Intents record among them, still need the classifier.

**Step 3 stays gated** (review HIGH-1; revision 1 said otherwise). Design 9.2 says a commit
completes only when no `inventory_generation` rotation occurs for the duration of one scan. Under
ordinary gossip the receive path writes between nearly every pair of turns, so H5 must complete
**within one visit, with no park, inside the overlay's share of that visit**. That share is half
the deadline (N-H2), which is 125 ms; revision 1 used the 250 ms shared slice. A and B leave these
in the way:

- **an uncached record above its limit.** That means Recovery above 64 KiB, Intents above 384 KiB
  or OwnerReceipts above 747 bytes. These families have no cache (L5), so they park on every pass;
- **H5's own target Intents record**, whenever its Closing seed is above roughly 300 KiB. The
  classifier sees only the size, so it charges the opaque seed bytes at the per-entry rate;
- **any record reached when little of the slice is left;**
- **cold Studio and Registry records beyond the cache** (part C);
- **traversal itself.** Reading and authenticating costs about 2 us per KiB. Past roughly 60 MiB
  of five-family bytes on the calibration host, even a fully warm vault cannot finish in 125 ms.

So before step 3 is built it needs all three of:

1. **A route to completion in one visit with no park**, for the vaults it is meant to support.
   The candidate is a digest-keyed memo covering every accounting family, not only Registry and
   Studio, sized per part C option 1. That is an amendment to L5. After one complete validation,
   only records that changed would then cost a validation. **Its value must carry Intents' facts**
   (re-review MEDIUM-2). Today a hit returns `ValidatedRecordBody::accounting_only`, with
   `intent: None`, and `EpochIntentBudget::from_inventory` builds its Unconfirmed tally from
   `intent_facts()`. A memo on today's value type would drop live Unconfirmed branches from that
   tally, and the budget would be too generous. The alternative is some other progress mechanism
   that survives restarts.
2. **Measured full-profile traversal times for realistic vaults**, against the 125 ms share.
3. **A deterministic H5 test with a five-family write between every pair of turns**, showing that
   the handoff completes.

Until then H5 keeps today's behaviour: it completes in one visit, at the cost of an unbounded scan
in that visit.

**L6 must name each condition above, and the backoff arithmetic.** Each job warms about three
records before `Unstable`, then backs off for 30 s, doubling to 300 s. So a vault with about 20
cold Studio records could wait tens of minutes before a handoff could complete.

**Steps 4 and 5 are unchanged.** They still need section 7's measurements, including the restart
rate with a second actor writing.

### 14.6 Tests and mutations for A and B

**Existing tests whose subject is the detached stage** (review M-2). With A, any test that plants a
small Recovery, Intents, OwnerReceipts or DraftArchive record under a deadline and expects it to
park would see it validate inline instead. Seven are known so far:

- the five in `epoch_recovery/inventory.rs` that plant small Recovery records under a 250 ms
  deadline at a frozen clock:
  - `a_budgeted_cursor_parks_each_record_and_completes_through_the_detached_stage`;
  - `a_budgeted_scan_produces_the_same_inventory_as_an_unbudgeted_one`;
  - the two rail tests "after a record has been parked and installed";
  - `a_detached_validation_is_refused_by_the_wrong_scan_record_or_generation`;
- `driving_a_job_respects_the_visit_deadline_and_stops_at_a_park`;
- `c3_multi_family_scan_parks_records_from_several_families` (`performance.rs`, not ignored).

The opt-in profile harness needs the same, because it times `validate()` on every parked record.
**The list is complete** (re-review LOW-4).
`driving_a_job_treats_its_deadline_as_absolute` plants only non-family files.
`driving_a_job_loops_several_one_record_steps_in_one_visit` relies on Studio cache hits. Every
other job test passes no deadline. Running the suite and switching whatever fails would not be
enough anyway: it misses a test that keeps passing after it stops exercising the detached path, as
the inventory-equality test would. So **each switched test also asserts that something parked**.

Revision 1 proposed forcing the park with a deadline equal to the current time. That works for the
step API, but `drive_epoch_inventory_job` never begins a step with no time left, so drive tests
would not step at all.

Instead, a `#[cfg(test)]` switch makes `validation_fits` return false. Its name is still to be
settled. It lives **on `ServerStore`** and `step_inner` reads it. Revision 2 put it on the cursor,
but a cursor-level switch is lost whenever a job restarts, because `restart_job`,
`finish_epoch_inventory_job` and the uncharged refresh all open fresh cursors (re-review
MEDIUM-1). `studio_rotation_interruption` is the precedent for a test field on the store.
Unbudgeted scans never classify, so they are unaffected.

The tests and harness above use the switch, which also keeps them independent of the calibration
constants. That is a recorded change to what they test: "a record the classifier detaches parks",
with the classifier's own tests saying what it detaches.

Two neighbours need no switch:

- `a_supplied_deadline_stops_traversal_even_with_no_validation_to_classify` has nothing to classify.
- `a_budgeted_reference_scan_collects_the_same_cids_as_an_unbudgeted_one` is a reference scan,
  which never inlines.

`ccd00dbc`'s runtime tests park cold Studio sources, which detach by rule, so they need no switch
either.

**New tests:**

- **The rule as a table:**
  - each family in each mode, at 0 bytes, at the envelope, and one byte past it;
  - `remaining` at 0, at 1, exactly at the threshold, and one below it;
  - the 25 ms cap binding when more time remains;
  - `u64::MAX` size, with no overflow.
- **A real cursor at a frozen clock** under a 250 ms deadline: a small Recovery record is
  installed inline, nothing of it parks, a cold Studio record parks, and the inventory equals the
  unbudgeted one. This is the test that pins inlining.
- **A clock that advances on every read:** one small record inlines early in the slice, and a
  later one parks once the remaining time falls below its threshold.
- **The switch survives a restart:** a switched job that restarts still parks.
- **Part B:**
  - a Studio result refused as `Invalidated` warms the cache, and the restarted job reuses it
    (`reused_records` rises, nothing parks);
  - a stale entry for an older version is evicted on the read, so the refused result for the new
    version does warm;
  - a refused result does not displace an entry a write path put since the read;
  - a result from another scan, another mount, another store or another record warms nothing;
  - a record rewritten after its validation is a miss;
  - when step 2 lands, an own-write storm in which `reused_records` rises after an uncharged
    refresh.

**Mutations, each run to its failing test and restored:**

- `SAFETY` set to 1;
- the cap removed;
- the envelope check removed;
- Studio admitted;
- `references` ignored;
- the 1 ms floor removed;
- eviction on mismatch removed;
- the vacancy check removed;
- the store-mount check removed;
- the warm on a fault exit.

### 14.7 Revision 1's design review, answered

| finding | answer |
|---|---|
| HIGH-1, A and B do not meet step 3's gate; the slice is 125 ms | 14.5: step 3 stays gated, on three named conditions |
| HIGH-2a, a stale entry defeats the vacancy rule | 14.3 piece 1, eviction on mismatch |
| HIGH-2b, step 2's uncharged refresh drops the result first | 14.3 piece 2, one store entry point called before any discard |
| HIGH-3, Recovery's rate is from opaque projections | 14.1 and 14.2: envelope held at 64 KiB until structured shapes are measured |
| M-1, version-2 owner journals verify signatures | 14.1 and 14.2: rationale corrected; the 747-byte envelope admits at most one |
| M-2, two more affected tests; an expired deadline cannot drive | 14.6: the `#[cfg(test)]` switch, and the run-and-confirm procedure |
| M-3, part C's scope and options | 14.4 |
| LOW, `remaining_ms` is floored | 14.2: one millisecond subtracted |
| LOW, part B relaxes 9.2 consequence 2 | 14.3, recorded in 9.2 and the threat model when built |
| LOW, no check against the store's own mount | 14.3 piece 2 |
| LOW, the DraftArchive envelope and constant | 14.2 |
| LOW, determinism holds only under a deterministic clock | 14.2 |
| LOW, the status ledger said "three families" | fixed: four, Recovery included |

**Revision 2's re-review** (same reviewer, static) found no blocker or high, and said A and B are
ready to build once its two mediums are answered.

| finding | answer |
|---|---|
| MEDIUM-1, a cursor-level test switch is lost on restart | 14.6: the switch is on `ServerStore` |
| MEDIUM-2, an all-family memo must carry Intents' facts | 14.3 piece 2's memoize helper; 14.5's memo |
| LOW-1, other discard paths for an `Installing` result | 14.3, "what it does not cover" |
| LOW-2, section 13's Recovery bullet | qualified |
| LOW-3, "every route needs both" overclaimed | 14.5 reworded |
| LOW-4, the affected-test list is complete | 14.6: the list, plus a "something parked" assertion per switched test |

**Follow-up measurements this leaves.** Each one would let an envelope widen:

- **Structured Recovery shapes:**
  - an empty projection with the most applied operations, tombstones and elements;
  - the most one-byte-value conflicts;
  - a staged snapshot with a `completed_eviction`.
- **A version-2 owner journal and a fault record**, at the 27 KiB cap.

**Residual risks the review listed.** `SAFETY = 4` absorbs contention, shape uncertainty and slower
hardware at once, so re-derive it per component once those measurements exist. Debug builds can
hold an inline validation for about 63 ms.

## 15. A route to step 3 (revision 2 in 15.8, reviewed in 15.9: no blocker, one high)

**Nothing here is built.** Section 14.5 said what step 3 needs. This section proposes how to get
it, says what no proposal of this kind can get, and lists the measurements that decide whether
it is enough. **Its design review (15.7) found no blocker, but two highs mean it does not yet
support the decision it asks for.** Read 15.7 before the rest.

### 15.1 The target, restated

Design 9.2 says a commit completes only if no `inventory_generation` rotation lands during its
scan. Under ordinary gossip the receive path writes between nearly every pair of turns. So H5
must complete its full-profile scan **within one visit, with no park, inside the overlay's
125 ms share** (N-H2). `studio_storage_budget` then demands that the inventory's three
generation tokens all match the store's, so nothing short of a complete scan in that visit can
mint the budget.

In one visit, each record costs three things:
1. **Reading and authenticating it.** That is about 2 us per KiB on the calibration host, plus a
   per-file cost that has not been measured.
2. **Validating it.** A cache hit costs nothing. An inline validation is bounded by section 14.2.
   Anything else parks, which fails the visit.
3. **Installing it,** which is below resolution at the sizes measured.

### 15.2 Proposal M1: memoize every accounting family

Extend the validation memo from Registry and Studio to all six families, in accounting mode
only. A record validated once is then a hit on every later scan until its bytes change, whatever
its family or size. That removes the parks section 14.5 lists for large Recovery, Intents and
OwnerReceipts records, including H5's own target Intents record, after their first validation.

- **The value grows to carry Intents' facts:** `(StorageRecord, Option<EpochIntentInventoryFacts>)`.
  A hit restores them. Today a hit returns `intent: None`, which is safe only because Intents is
  never cached, and `memoize`'s comment already says this signature must grow first (re-review
  MEDIUM-2). `EpochIntentBudget::from_inventory` builds its Unconfirmed tally from those facts, so
  a test must pin that a hit carries them.
- **Why it is sound for every family.** The argument is part B's. An entry is bound to the
  filename hash, the physical size and the blake3 digest of the authenticated plaintext, and
  `validate_record_body` is a pure function of those bytes. That includes OwnerReceipts, whose
  version-2 decode verifies Ed25519 signatures over keys carried in the bytes themselves: that is
  deterministic over the bytes, so its result can be memoized like any other. Reference scans
  still never consult the memo.
- **What changes in `step_inner`.** `cacheable` loses its family restriction, and `memoize` loses
  its family gate. The cold-byte rail then counts only misses for every family, as it already does
  for Registry and Studio. That is the rail's stated meaning: work for fresh validation.
- **This amends L5**, which says an `inventory_cache` extension is not proposed.

### 15.3 Proposal M2: an indexed memo sized to the inventory's own bound

Section 14.4's analysis applies with more force once every family is memoized. Under LRU, a full
scan of more records than the memo holds misses on every record.

- **Capacity: `MAX_ACCOUNTED_RECORDS` (65 536).** A full scan then never thrashes. The memory
  bound is about one more inventory's worth: an entry is about 170 bytes plus index overhead,
  while a full-profile `EpochStorageInventory` already holds an entry per record during every
  scan.
- **Pruned to the vault on every completed full-coverage scan.** Such a scan has seen every key,
  so it drops entries for files no longer present. In steady state the memo then tracks the vault
  rather than its history.
- **Indexed:** a keyed map plus an access order (for example `HashMap` and a `BTreeMap` of ticks),
  so get, put and eviction are logarithmic. The current linear `VecDeque` would make a
  65 536-entry scan quadratic. No new dependency.
- **Eviction on mismatch and put-if-vacant are unchanged** (part B).

### 15.4 What no scan-based route removes: traversal

Every scan still reads and authenticates every record; a memo saves validation, never the read.
On the calibration host the read alone is about 2 us per KiB, so **about 60 MiB of five-family
bytes uses the whole 125 ms share** before any per-file cost. Beyond that, H5 cannot complete a
visit under sustained gossip with any scan-based inventory, memo or not.

The routes past that bound are not proposed:

- **Skipping authentication for files whose size and timestamp look unchanged.** This weakens a
  security check.
- **A write-through inventory**, where each five-family writer updates a held inventory under
  custody. This changes I-4's safety argument, from "every writer rotates" to "every writer
  updates correctly", and under-update would be the unsafe direction.

So with M1 and M2, **L6 becomes a stated vault-size bound.** For vaults whose traversal fits the
share, a handoff completes under gossip once its records are warm. For larger vaults, a handoff
is held until a quiet gap, which is L6's existing "held and retried under sustained writes",
priced. Today the same handoff completes in one expensive visit, so building step 3 trades that
visit's custody hold for this bound. That trade is the decision this proposal asks for.

### 15.5 Measurements and tests this needs before step 3 is built

- **Per-file traversal cost.** A vault of many small records, on NTFS, release build; the
  bytes-only figure exists.
- **Full-profile scan time with a warm memo**, for synthetic vaults of about 100, 1 000 and
  4 000 records of mixed families. This says where the bound of 15.4 actually sits.
- **The H5 write-every-turn test** (14.5, condition 3). With a warm vault and a receive-path
  write between every pair of turns, the handoff completes. Above the traversal bound it stays
  held, and is never committed from a partial scan.
- **For M1:**
  - an Intents hit restores the facts, and the Unconfirmed tally equals a cold scan's;
  - an OwnerReceipts hit equals a fresh validation;
  - mutations: drop the facts from the value; re-admit the family gate.
- **For M2:**
  - a full scan of more records than the old 64 reuses every warm entry;
  - pruning drops a deleted record's entry;
  - the bound is held at capacity;
  - a mutation that skips pruning is caught.

### 15.6 Questions for the review

1. Is memoizing every accounting family sound? In particular: OwnerReceipts, given decode-time
   signature verification; Intents' facts; and Recovery, given its per-snapshot work.
2. Is capacity at `MAX_ACCOUNTED_RECORDS` with pruning the right bound, or should it be smaller,
   with the traversal bound of 15.4 stated at that smaller size?
3. Is a vault-size bound on handoff completion under gossip acceptable as L6, or does it rule step
   3 out as designed? If the latter, which alternative should be designed instead?
4. Is anything in 15.4 wrong? Is there a sound route past the traversal bound that this misses?

### 15.7 Design review of revision 1 (2026-10-08, Opus, static): no blocker; two highs

**M1 is sound for all six families.** Every validation input is the family, the plaintext, and
the scope inside it, and server and document are decoded from that scope on every read. Size is
in the key. No validator reads the clock, membership, MLS state or the registry. OwnerReceipts
verifies signatures over keys carried in its own bytes. Consumers read only fields a hit already
restores, plus the Intents facts, which M1 adds. **M2's capacity is right.**

**What the review found:**

- **HIGH-1: the record written between visits stays cold.**
  - Receive writes after it scans. Its result reaches the memo only through
    `retain_received_studio_source`, which does nothing while another document holds the source
    slot. `cache_studio_source_footprint` also refuses sources over 8 MiB, and no Recovery,
    OwnerReceipts, Intents or DraftArchive writer warms at all.
  - The failure: Flipnote A is open, and a peer draws on Flipnote B. Each receive turn writes a new
    B cold. H5 reads it, Studio never inlines, it parks, and the next write restarts the job,
    forever.
  - **Fix:** warm the memo from every writer's own result (`SourceVersion` already carries
    digest, bytes and record), with a per-writer test that the warmed record equals a fresh
    validation.
- **HIGH-2: H5's own commit work is outside the cost model, and it is structure-driven.**
  - `commit_studio_handoff_with_io` calls `checked_studio_source`, which runs a full
    `restore_unit`, in the 240 ms class for 128 frames. An Index target adds one more restore per
    PutObject (`check_index_object_sources`), then reference walks, encoding and three durable
    writes.
  - **Design 9.1, "no graph restore on the commit path" (`VerifiedPersistedSource`), is unbuilt.**
  - So 15.4's "trades that visit's custody hold" overclaims: the H5 visit stays unbounded until
    9.1 exists, and the scan's share is 125 ms *minus* the commit's cost.
- **MEDIUM-1: the gossip premise and the vault-size bound do not occur together today.** The
  receive profile pauses on any vault over 64 records, 8 MiB read or 256 KiB cold. So writes
  between nearly every turn happen only in vaults of 64 records or fewer, whose traversal is at
  most 8 MiB, about 16 ms. In larger vaults, rotations come from rarer writers and from
  sync-repairs, which leave the bytes, and so the memo, valid. **15.4's bound only becomes real
  once step 5 lifts the receive limits.** L6 should be stated per regime.
- **MEDIUM-2:** `evict_mismatch`'s Registry-and-Studio gate must widen with M1, or stale entries
  of the other families block warming again (part B, piece 1). `accounting_only` and
  `memoize_refused` must carry the facts. The test pinning "Registry and Studio only" changes
  deliberately, and that is a recorded contract change.
- **MEDIUM-3: the memo starts empty at every launch.** Background jobs warm about three records
  per `Unstable` cycle, then back off from 30 s, doubling to 300 s. **Missed route:** a restart
  that added a memo entry is progress, so it should not be charged against
  `MAX_INVENTORY_RESTARTS` or backoff. Measure time to first handoff from an empty memo.
- **MEDIUM-4: M1 changes what automatic receive admits.** Warm bytes stop counting against the
  256 KiB cold rail for every family, so whether receive pauses becomes history-dependent. That is
  the rail's intent, but it is user-visible: update HANDOVER's limitation and test both
  directions.
- **LOW:**
  - Memory is about 20 to 23 MiB at 65 536 entries, resident for the mount's life, not one
    inventory's worth.
  - Prune only after `finish_with` succeeds, and only within that scan's coverage.
  - The 60 MiB figure has no margin; with the classifier's factor of 4 it is about 15 MiB.
- **A route the doc missed, offered as a design question:** record touched paths inside the
  `EpochMutation` primitives, which already take the path. A cursor could then span visits and
  re-read only those paths at completion, plus `confirm_listing`. That removes the traversal bound
  without a write-through inventory. It carries an audit obligation of I-4's class, and still
  needs HIGH-1's writer warms.

**What step 3 needs, as this review leaves it.** In dependency order:

1. **Design 9.1, built:** H5 commits from a `VerifiedPersistedSource`, with no graph restore and
   no per-PutObject restore for an Index. Without it no inventory route bounds the H5 visit.
   **Done 2026-10-08.** H5 still does seed graph loads and `blob_cids` projections, which item 2
   measures.
2. **The commit phase measured on its own:** 128 frames, and an Index with many PutObjects. This
   sets what the scan has left of 125 ms.
3. **M1, with writer warms (HIGH-1) and MEDIUM-2's three gates.**
4. **M2, with pruning as LOW defines it.**
5. **Either the traversal measurements of 15.5, or the touched-path route designed and reviewed.**
6. **The H5 write-every-turn test, in HIGH-1's variants:** writes to a non-retained Studio
   document, to a source over 8 MiB, and to a Recovery record over 64 KiB.

**Revision 2 of this section waits on 1 and 2.** 9.1 is part of the accepted Flow H design and the
largest of these items, so it comes first.

### 15.8 Revision 2 (2026-10-09): 15.7 answered, with 1 and 2 done

Both inputs revision 2 waited on now exist:
- **9.1 is built** (`17dd54fc`).
- **The commit phase is measured on its own**, in release on a host shared with two other agents'
  builds (STATUS, "H5's repeated terms, priced"). H5's commit is:
  - about 25 ms for one operation;
  - 83 ms at 256 title operations;
  - 45 ms for an Index at its object cap;
  - **113 ms at 256 frames.**

So **the scan's share of the 125 ms visit depends on the branch:**
- about 100 ms for a short one;
- about 40 ms at a full title branch;
- **about 12 ms at a full frame branch.**

The repeated snapshot encodes are not where the commit's time goes (each is under the clock's
resolution). So F3 of the design 18.3 review records them as a deviation, not a fix. The repeated
`blob_cids` projections are: about 21 ms for H5's three at 256 frames. Computing it once recovers
about 14 ms, which raises the full-frame share to about 26 ms. That is item 0 of the order of work
below. The rest of the commit's growth is unattributed, and attributing it is item 0's second
half.

**Nothing below is built.** It answers 15.7 finding by finding, then orders the work.

#### HIGH-1: every byte-changing writer warms the memo it invalidated

A read-only static map of the 24 production `epoch_mutation_guard()` sites (2026-10-09, at
`0808f5a2`) found them complete: no family record is written or removed without the guard. Seven
of them replace a record's bytes, and every one of those has, in scope right after its write, the
plaintext it sealed, the physical size (`plain.len() + 40`) and the `StorageRecord` it charged. The
others flush unchanged bytes, which leaves a warm entry valid, or remove a record, which M2's
pruning handles.

The warm sits at those seven sites, after the write returns and the guard drops, not in their
callers. A caller-side warm is what HIGH-1 found missing: `retain_received_studio_source` does
nothing while another document holds the source slot, and `cache_studio_source_footprint` refuses
anything over 8 MiB. The memo entry is metadata, about 330 bytes whatever the source's size, so the
8 MiB bound belongs to the retained graph and not to the memo.

| family | writer (site at `0808f5a2`) | entry produced by |
|---|---|---|
| DraftArchive | `write_studio_draft_archive_with_io` (`epoch_draft_archive.rs:255`) | the validator |
| Intents | `write_prepared_intents` (`epoch_intents.rs:864`) | the validator |
| Intents | `retire_included_with_io` (`epoch_intents/retirement.rs:355`) | the validator |
| Recovery | `update_epoch_recovery_accounted_with_writer` (`epoch_recovery.rs:374`) | the validator |
| OwnerReceipts | `write_epoch_owner_state` (`epoch_owner.rs:642`) | the validator |
| Registry | `update_registry_with_io`, replace branch (`epoch_registry.rs:492`) | the writer's facts |
| Studio | `save_studio_source_checked`, replace branch (`epoch_studio.rs:704`) | the writer's facts |

**Two ways of producing the entry, chosen per family:**

- **The validator, for the four families whose accounting validation is cheap.** The writer calls
  `validate_record_body(family, plain, scope, server, document, size, false)` on the plaintext it
  just sealed. That keeps the memo's one invariant (every put is the true pure validation of the
  bytes its digest names) true by construction, and it gives Intents the facts M1 needs from the
  same function a scan uses. The cost moves into the writer:
  - Intents accounting tracks entries, about 11 ms at the largest ledger measured, microseconds
    at ordinary sizes;
  - Recovery, 9 ms at 4 MiB;
  - DraftArchive, nothing that grows with size;
  - OwnerReceipts, its signature checks over at most nine receipts.
- **The writer's own facts, for Registry and Studio**, where validation is a restore. Both
  already warm this way: `remember_installed_registry`, and `SourceVersion` through
  `cache_studio_source_footprint`. Each writer gets a test that its entry equals a fresh
  `validate_record_body` of the bytes on disk, which is the review's per-writer equality test.

The Studio warm covers every Studio writer at once, because they all end in
`save_studio_source_checked`:
- gossip ingest;
- catch-up pages;
- adoption;
- rotation;
- repair;
- local Save;
- `send_saved_studio_once`;
- H5.

So the HIGH-1 failure (Flipnote A open, a peer drawing on B, each receive turn writing B cold)
closes, for any source size.

**Order with respect to I-4.** The guard rotates `inventory_generation` before the write. The warm
comes after the write and puts an entry for bytes now on disk. A scan captured before the write is
still refused by the rotation. The restarted scan then hits the entry, which is the point.
- **A write that fails before its rename** warms nothing.
- **One that returns `CommittedButNotDurable`** has its bytes in place, so it may warm. That is
  harmless either way, since a hit needs those bytes' digest.

#### MEDIUM-2: the three gates widen together

1. `evict_mismatch` runs on every family's read, not only Registry and Studio's.
2. `ValidatedRecordBody::accounting_only` and `memoize_refused` carry
   `Option<EpochIntentInventoryFacts>`, and a hit restores them.
3. `a_budgeted_scan_memoizes_only_registry_and_studio_records` changes deliberately, to "every
   family is memoized in accounting mode, none in reference mode". That is a recorded contract
   change.

M1's tests are those of 15.5:
- an Intents hit restores the facts, and the Unconfirmed tally equals a cold scan's;
- an OwnerReceipts hit equals a fresh validation;
- mutations: drop the facts from the value; re-admit the family gate.

The oracle in every memo test is a scan run after emptying the memo (`fresh_inventory`, added for
the design 18.3 review's F5), because a plain accounting scan consults the same memo.

#### MEDIUM-1: L6, stated per regime

- **Today: automatic receive pauses on any vault over 64 records, 8 MiB read or 256 KiB cold.**
  Writes between nearly every turn therefore happen only in vaults of at most 64 records.
  - Their traversal is at most 8 MiB, about 16 ms at 2 us per KiB, before any per-file cost,
    which 15.5 still has to measure. That fits the share at a title branch (about 40 ms).
  - **It does not fit at a full frame branch** (about 12 ms). Even with `blob_cids` computed once
    (about 26 ms), it fits only in vaults of under about 3 MiB of five-family bytes, with the
    classifier's factor of 4 as margin (corrected by 15.9 from "about 5 MiB", which matched no
    convention). So until
    item 0's attribution finds more, a full frame branch in a vault near the receive limit is held
    under gossip until a quiet gap, as L6 says.
  - With M1 and the writer warms, every record but the one just written is a hit, and that one
    was warmed by its writer.
  - So step 3 completes under gossip in this regime **for branches whose commit leaves the share
    the vault's traversal needs**, which item 4's measurements turn into a table.
  - The current 64-entry LRU holds such a vault exactly, but with no headroom, which is one
    reason M2 still comes first.
- **Larger vaults, today.** Rotations come from rarer writers: local Save, sync repairs and
  maintenance. A scan may span several visits.
  - Without M2 the 64-entry LRU thrashes on more than 64 records, so every scan of such a vault
    parks each Studio and Registry record again, and a scan of a few hundred records takes as
    many detached validations.
  - With M2 the second scan is all hits, and completes in a few visits between rare writes.
- **After step 5 lifts the receive limits, the traversal bound becomes real.** At 2 us per KiB,
  with the classifier's factor of 4 as margin, the bound is about 5 MiB of five-family bytes per
  visit at a 40 ms share, and about 1.5 MiB at a full frame branch's 12 ms. Past it, a handoff
  under sustained gossip is held until a quiet gap. That is L6's existing "held and retried
  under sustained writes", now priced.
  - **The touched-path cursor 15.7 offered** is the route past the bound, because it re-reads
    only paths written since the scan began.
  - **It is deferred to step 5**, where the bound first binds. It needs its own design and an
    audit of I-4's class.

#### MEDIUM-3: a restart that made progress is not charged

An attempt overtaken by a write is charged against `MAX_INVENTORY_RESTARTS` and backoff **only if
it memoized nothing it found cold**. The job counts the entries its own validations added (inline
installs and refused results), and does not count writer warms.

**Why this is bounded:** every byte-changing writer now warms its own record, so the only cold
records a job can meet are those cold since launch. Each uncharged restart therefore warms at
least one of a finite set, at most `MAX_ACCOUNTED_RECORDS`.

What to measure: the time to a first handoff from an empty memo, on 100- and 1 000-record vaults.

#### MEDIUM-4: receive admission becomes history-dependent

With M1, warm bytes stop counting against the 256 KiB cold rail for every family, so whether a
receive pauses depends on what has been read before. That is the rail's stated meaning, work for
fresh validation, but it is user-visible.

What it needs:
- HANDOVER's limitation is rewritten to say so;
- a test pins both directions: a vault that pauses cold and proceeds warm, and a write that makes
  it cold again.

#### The lows

- **Memory** is about 330 bytes per entry and, with pruning, proportional to the vault's record
  count: about 21 MiB at 65 536 records, resident for the mount's life. A vault that size already
  holds an inventory entry per record during every scan.
- **Pruning** runs only after `finish_with` succeeds, and only within that scan's coverage.
- **The 60 MiB figure in 15.4** is superseded by the per-regime bound above. It had no margin, and
  it assumed the whole 125 ms rather than what the commit leaves.

#### Order of work for step 3

0. **Shrink H5's commit at frame-heavy branches.**
   - Compute the candidate's `blob_cids` once and pass it to the three places that project it.
     This is a pure function of the candidate H5 already holds, so it changes no boundary, and it
     recovers about 14 ms at 256 frames.
   - Then attribute the rest of the commit's growth with branch length. The profile prices
     encodes, projections and seed loads, and none of them is it.
1. **M1 with the writer warms and MEDIUM-2's three gates**, as one checkpoint with its own
   implementation review. Its tests are listed above, plus the per-writer equality tests.
2. **MEDIUM-3's progress rule**, with a test that a job warmed record by record across restarts
   is never charged, and one that an attempt warming nothing is.
3. **M2:** an indexed memo at `MAX_ACCOUNTED_RECORDS`, with pruning. Its tests are 15.5's.
4. **15.5's traversal measurements**, on NTFS in release:
   - the per-file cost;
   - warm full scans of about 100, 1 000 and 4 000 mixed records.

   They decide whether the regime boundaries above sit where this section says.
5. **The H5 write-every-turn test**, in HIGH-1's three variants: a non-retained Studio document,
   a source over 8 MiB, and a Recovery record over 64 KiB.
6. **Step 3 itself**, then Flow R.

#### Questions for the review

1. Is the validator-at-write form acceptable on the Save and receive paths? Its worst case is
   about 11 ms, for a maximal Intents ledger. The alternative is the writer-facts form for
   Intents, which costs nothing but carries an equality obligation like Studio's.
2. Is L6 stated per regime acceptable, with the touched-path cursor deferred to step 5? In
   particular, is "a full frame branch in a near-limit vault waits for a quiet gap under gossip"
   an acceptable limitation for step 3, or must item 0 bring the frame-branch commit under a
   stated figure first?
3. Is MEDIUM-3's rule, charged only when nothing cold was memoized, sound and bounded as argued?
4. Does any of the seven writer sites warm bytes that are not, or were not, on disk? Each warm
   follows its write's success, but `CommittedButNotDurable` is the edge.

### 15.9 Design review of revision 2 (2026-10-09, Opus, static): no blocker; one high

Reviewed against `fcda06cd` and the revised share at `751046a3`. **No path was found where a warm
weakens I-4:**
- the guard rotates the token before the write;
- the warm comes after it;
- `finish_with` checks the token and the listing whatever the memo holds.

**The four questions, answered:**
1. **Validator at write: yes, and better than the writer-facts form for Intents.** Intents'
   accounting record does not depend on content, so the facts form would save only the decode, and
   the decode is the validity check a hit would otherwise skip for good. The conditions are M4's
   contract below, and stating Recovery's real ceiling (L2).
2. **L6 per regime: not as stated** (HIGH-1, M1, M5).
3. **MEDIUM-3: sound in direction, not bounded as argued** (M2).
4. **No bytes that are not on disk, in production.** The one non-test `WriteHooks` returns its
   input unchanged. `CommittedButNotDurable` needs a decision (M4).

**HIGH-1: the share is not a bound, because H5's terms grow with the whole source, which no
measurement varies.**
- `blob_cids` walks the projection, the seed's CIDs and every signed operation, and
  `base_blob_cids` rebuilds the seed's graph.
- The profile's frame branches start from a source with **one** frame. Its 1, 4 and 7 ms
  projections at 32, 128 and 256 frames extrapolate to about 27 ms each at `FLIPNOTE_MAX_FRAMES`
  (999), **for a one-operation branch**. With three candidate projections and the base one, the
  commit then takes most of the visit.
- **The commit has no margin.** One sample read 248 ms against a 113 ms upper median, while the
  traversal gets the classifier's factor of 4.
- **The per-file cost is already measured but not subtracted:** 11 ms for a 64-record vault in
  the same run, about the whole full-frame share before any bytes.
- **The margins are applied inconsistently.** The regime-1 claim uses none. The post-step-5 bounds
  use 4. The 15.8 "under about 5 MiB" matches neither convention: about 12.7 MiB unmargined, about
  3.2 MiB with the margin. **Corrected here to about 3 MiB.**
- **The consequence:** "wait for a quiet gap" covers handoffs of *large* flipnotes, whatever the
  branch length, in exactly the channels where drawing keeps gossip going. Today such a handoff
  completes in one long visit, so as stated step 3 would be a user-visible regression for them.

**Mediums:**
- **M1, nothing divides the visit between scan and commit.**
  - **The problem:** the commit cannot be paused, and 15.8 names a share without saying how the
    runtime enforces it.
  - **The fix:** derive the scan's deadline from the share less a conservative prediction of the
    commit (from the branch's operations and kinds and the source's frames), times a margin. Or
    accept the overrun and price it in L6.
- **M2, MEDIUM-3's bound.**
  - **Order:** the rule was ordered before M2. With the 64-entry LRU, a vault of more than 64
    memoizable records meets evicted records cold on every pass, so every pass "memoized something
    cold" and is never charged.
  - **Weak premises:** the claim that every writer warms is false (M3) and fragile. Stale entries
    for deleted records also survive until a complete scan prunes them.
  - **Scale:** up to 65 536 uncharged restarts, while an H5 job in `Ready` holds a pool permit.
  - **The fix:** credit each key at most once per attempt, add a hard ceiling on uncharged
    restarts, and land the rule after M2.
- **M3, eight byte-replacing writers, not seven.** `update_epoch_recovery_with_writer`
  (`epoch_recovery.rs:485`) is reached from the public `update_epoch_recovery`, which has no
  production caller but backs the inventory tests' `stage` helper and the src-tauri tests. The
  24-site count is right: 8 writes, 14 syncs, 2 removes.
  - **The fix:** warm there too.
  - **Better:** make a forgotten warm impossible. `EpochMutation::write` returns a `#[must_use]`
    token that must be passed to the warm or skipped with a reason, as I-4 does with the guard.
- **M4, the warm's contract.**
  1. **It never fails the writer.** On a validator error after a durable write it skips and
     `debug_assert`s. At H5's barrier 1 a propagated error would strand a Prepared record.
  2. **It derives key, scope, server and document from the plaintext**, through one helper
     shared with `step_inner`, not from the writer's arguments.
  3. **It skips bytes a test hook substituted.**
  4. **The writer-facts warms (Registry, Studio) are cross-checked** against
     `validate_record_body` on every warm under `cfg(any(test, debug_assertions))`, not by one
     fixture per writer, since a hit is never contradicted by a fresh validation afterwards. And
     "Registry already warms this way" was inaccurate. `remember_installed_registry` re-reads the
     file. The new warm would be the first built from the live, just-mutated unit.
  5. **`CommittedButNotDurable` warms:** the bytes are visible. As structured, the `?` skips it,
     and the exact retry's sync branch never warms.
- **M5, regime 2's premise is wrong.** Every local edit writes the intent and then the source
  (`epoch_studio.rs:402-429`), whatever the receive pause says. So large vaults see writes at
  editing rate in active use, and the traversal bound binds today, not only after step 5. The H5
  test needs a local-edit writer variant.

**Lows:**
- **L1:** MEDIUM-2's test contract is reworded to "memoized in both modes, consulted only in
  accounting mode". `install_body` memoizes during reference scans, which 14.3 piece 1 relies on.
- **L2:** Recovery's ceiling is 18 MiB plus 2 088 bytes, unmeasured above 4 MiB: about 40 ms or
  more at write time, extrapolated. That is on the rotation, adoption and settlement paths, which
  are not gossip-rate. Acceptable, but stated.
- **L3:** the write-every-turn test needs a clock that advances with work, since a frozen
  `ManualClock` proves nothing about "one visit". It also needs more variants:
  - an Intents record over 384 KiB;
  - OwnerReceipts over 747 bytes;
  - a Registry write;
  - a local edit;
  - a cold memo after remount.
- **L4:** text M1 makes stale must change with it:
  - `THREAT-MODEL.md` around 303-309 and 869-874 (the memo would hold provider ids and timestamps);
  - design 9.2 consequence 2;
  - `cache.rs`'s module comment;
  - the doc comments on `memoize` and `memoize_overtaken_inventory_result`.
- **L5:** the test helper `validated_clone` sets `intent: None`, and would plant fact-less Intents
  entries.
- **L6:** H5's own intent writes would pay the write-time validation too, about 0.8 ms at a full
  Closing branch.
- **L7:** "a write that makes it cold again" cannot come from any production writer once all
  warm. Use a remount, which is also the real case.

**The order of work, as the review leaves it.** This replaces 15.8's list:
1. **Item 0, widened to every term that grows with the source**, with the source-axis
   measurement.
   - Measure: base sources of 256, 512 and 999 frames, crossed with one-operation and full
     branches, plus a long signed history.
   - Fix: compute every candidate projection once, plus `base_blob_cids` and the snapshot
     encodes.
   - State the share as a function of source and branch, with a margin on the commit.
2. **M2**, the indexed and pruned memo.
3. **M1 with writer warms at all eight writers**: the forced-warm token, and M4's contract.
4. **MEDIUM-3** with per-key credit and a ceiling.
5. **The rule that divides the visit** (M1 of this review).
6. **The traversal measurements** of 15.5.
7. **The H5 write-every-turn test**, with L3's clock and variants.
8. **The touched-path cursor decision.** If item 1 cannot leave a margined share for regime 1's
   traversal (about 11 ms of per-file cost plus 2 us per KiB), the cursor moves into step 3
   rather than step 5.
9. **Step 3**, then Flow R.

**Residual risks:**
- a writer-facts warm that diverges from validation, which M4's cross-check mitigates;
- pruning only after a complete full scan;
- the memo resident for the mount's life, UI lock included.

### 15.10 The source axis, measured (2026-10-09): HIGH-1 holds

15.9's first measurement: `profile_studio_overlay_handoff_source_axis_{256,512,998}`. Each times
H1 to H5 for a one-operation and a full title branch over a Flipnote whose base already holds that
many frames. Release build, on a host shared with other agents' builds. Upper medians of 3 trials,
in milliseconds. The one-frame rows are the stage profile's.

| base frames | branch ops | H5 commit | one `blob_cids` | one seed graph | H2 (detached) | H4 (detached) |
|---|---|---|---|---|---|---|
| 1 | 1 | 27 | < 1 | 1 | 2 | 0 |
| 256 | 1 | 42 | 1 | 3 | 28 | 6 |
| 512 | 1 | 56 | 3 | 6 | 57 | 12 |
| 998 | 1 | **86** | 6 | 13 | 119 | 26 |
| 1 | 256 | 82 | 1 | < 1 | 857 | 242 |
| 256 | 256 | 99 | 3 | 3 | 3 651 | 885 |
| 512 | 256 | 118 | 6 | 6 | 6 873 | 1 662 |
| 998 | 256 | **147** | 8 | 13 | 13 291 | 2 966 |

**What it settles:**
- **The commit grows with the source, about 0.06 ms per base frame, even at one operation.** The
  one-operation commit on a maximal Flipnote is 86 ms. With a full branch it is 147 ms, past
  the 125 ms visit before any scan. As 15.9 HIGH-1 predicted, the share is a function of source
  and branch, and at the corner it is negative.
- **The repeated source terms are more than half of the one-operation commit at 998 frames:**
  - four candidate `blob_cids`, about 24 ms (corrected from "three" by 15.12 L1: barrier 1, the
    Source write's hold, resolve's flush hold and resolve's check);
  - two seed graph loads through `base_blob_cids`, about 26 ms.
  Snapshot encodes stay at or below the clock's resolution.
- **H1 stays small:** 2 to 5 ms. So does H5's budget inventory: 1 to 3 ms here, for a small vault.

**Item 0, sized by these figures.** Two halves:
1. **Compute the reference check's sets once in H5** and reuse them in resolve: one candidate
   projection and one seed graph, about 19 ms back at 998 frames (15.11).
2. **Carry what the detached stages already computed,** and cut the writer's two holds: H2 holds
   the seed graph and H4 the candidate. That is up to about 31 ms more. Sets carried from a
   worker are worker-supplied, so this half needs its own binding and review, as 9.1's facts
   did.

With both, the projection estimate is about 36 ms at 998 frames and one operation. At 998 frames
with a full branch it is still about 95 ms, because the remaining growth with branch length (82
to 147 ms between the one-frame and 998-frame full-branch rows) is not yet attributed.

**What that leaves for 15.9 item 8 (the touched-path cursor).**
- **What regime 1 needs:** a near-limit vault's traversal costs about 27 ms unmargined (11 ms of
  per-file cost plus 16 ms for 8 MiB), and about 75 ms with the review's factor of 4 on the bytes.
- **What it gets:** a maximal Flipnote with a full branch would leave about 25 ms after item 0.
- **So no scan-based step 3 completes there under gossip.** Neither does a touched-path cursor,
  which saves traversal but not the commit.
- **What remains is a choice:**
  - accept, as a stated L6 regression, that such handoffs wait for a quiet gap (today they
    complete in one long visit);
  - or keep this corner on today's single long visit;
  - either way, first attribute and cut the branch-length growth.

**Recommendation: build item 0's first half, then re-measure, before deciding.** It is mechanical
and changes no boundary. The second half and the branch-length attribution come next. Item 8 is
decided on the post-item-0 table, not on extrapolation.

### 15.11 Item 0, first half: H5 computes its reference sets once (proposal, 2026-10-09)

**Where the projections are.** In the H5 path today, the candidate's `blob_cids` and the seed's
`base_blob_cids` (a graph load) run here:

| site | candidate `blob_cids` | seed `base_blob_cids` |
|---|---|---|
| barrier 1: `check_handoff_references(&prepared, &candidate, &state)` (`handoff.rs:414`) | 1 | 1 |
| the Source write: `save_studio_source_checked`'s `hold_creative(unit.blob_cids())` (`epoch_studio.rs:645`) | 1 | none |
| resolve, Complete arm: its flush-only save's hold (the same writer) | 1 | none |
| resolve, Complete arm: `check_handoff_references(metadata, &source.unit, &state)` (`handoff.rs:725`) | 1 | 1 |

That is four candidate projections and two seed graph loads. At 998 frames that comes to about
24 + 26 ms.

**The proposal.** Change only the two reference checks, which are pure.
1. **Split the check.** `check_handoff_references` becomes a pure computation,
   `handoff_references(metadata, source) -> (candidate_cids, base_cids)`, plus a check
   `check_handoff_references_with(&refs, state)`. The old entry point stays, as the two composed,
   for its non-H5 callers: restart resolution and repair.
2. **Barrier 1 computes `refs` once** from `prepared` and `candidate`, and checks with them.
3. **`VerifiedPersistedSource` carries `refs`.** H5 hands them to
   `verify_persisted_studio_source`, which stores them next to the unit and the landed snapshot it
   already binds. `into_checked` returns them, and resolve's Complete arm, on the `Some(proof)`
   path only, checks with them instead of recomputing. The `None` path, restart resolution,
   recomputes as now.
4. **The writer's two holds are unchanged.** They protect pixels before potentially durable I/O,
   and the shared writer's signature stays as it is. Cutting them is item 0's second half, with
   the carried H4 projection.

Saved at 998 frames: one candidate projection and one seed graph load, about 19 ms.

**Why carrying them is sound.**
- **The sets are pure functions of `candidate` and of `prepared`'s overlay base.** `prepared` is
  the overlay H5 itself installs as Prepared and that resolve reads back as `metadata`. Its base
  (seed) is fixed for the branch's life.
- **The proof binds the unit to the landed bytes** (9.1). The candidate is moved into the writer
  and returned unchanged in content:
  - `check_studio_handoff_write` takes `&mut` only for `snapshot()`, `evidence` and
    `preserves_vault_source`, which save or read the Automerge document but do not edit it;
  - the writer encodes the unit; it does not change it.

  So the candidate's references at barrier 1 are the persisted unit's references.
- **The proof is spendable only under the same inventory generation** (A3). So no five-family
  write lands between computing the sets and using them.
- **Resolve's `metadata` on the proof path is the Prepared record H5 wrote.** Barrier 1 checked
  `prepared` against H2's `state`, while resolve checks `metadata` against the state it reads
  then. The second check is therefore still against the current pending set: only the projection
  is reused, never the pending intents.

**Tests:**
- **A projection counter, as `test-counters` counts reconstructions:** a `cfg(feature)` counter in
  `StudioEpoch::blob_cids` and `StudioOverlay::base_blob_cids`. An H5 commit performs exactly
  three candidate projections (barrier 1 and the two holds) and one seed load; before, four and
  two.
- **The existing reference regressions,** unchanged:
  `studio_overlay_handoff_rechecks_source_after_prepared_before_candidate_write` and the base-blob
  release refusal.
- **A new negative:** a proof is constructed only by H5, so a `Some(proof)` resolve with sets that
  do not match the unit is unreachable. A unit test of `into_checked` pins that the sets travel
  with the unit they came from.

**Out of scope:** the writer's holds, carrying H2's and H4's projections (the second half), and the
branch-length growth.

**Open questions for the review:**
1. Is there any H5-to-resolve path on which the proof's unit can differ in content from barrier
   1's candidate?
2. Is reusing the seed set on resolve's proof path sound, given that resolve may run in a later
   visit after a restart? (On restart the proof does not exist, and the `None` path recomputes.)

### 15.12 Design review of 15.11 (2026-10-09, Opus, static): no blocker; one high

**The two questions, answered:**
1. **No.** The current code does not change the unit's content between barrier 1 and resolve, but
   that rests on convention: no `&mut` method on the path edits it, and the types do not enforce
   that.
2. **Yes.** The proof is minted and spent within one call and never persisted. So after a restart
   only the `None` path exists, and it recomputes. The base is fixed for the branch's life.

**HIGH-1: the reference check this change rewires has never been exercised.**
- No test reaches "handoff would release a base blob reference", and no CI mutation targets it.
  The two tests 15.11 cited do not touch it. Agent 1's N19 was never built.
- No honest flow can trip it: H2's `check_overlay_successor` forces the successor's seed to be the
  overlay's base, so the candidate always covers the base.
- So a carried set that was swapped or emptied, or a missing overlay turned into an empty set,
  would make the proof-path check vacuous, with every test still green.
- 15.11's proposed `into_checked` test is tautological: it proves only that what went in comes
  out.

**MEDIUM-1: 15.11's generation argument is false.** Both of H5's writes rotate the generation
after the sets are computed, and the proof captures it only at verification. What actually holds
the sets to resolve's `metadata` today:
- one exclusive borrow;
- barrier 2's metadata hash;
- the absence of production write hooks.

Nothing checks that the carried seed set belongs to the branch resolve reads. The fix:
- carry `prepared.overlay().basis()` with the sets, and require it on the proof path;
- `base_blob_cids` depends only on fields the basis fingerprint covers;
- keep the sets outside `VerifiedPersistedSource`'s proof claim.

**MEDIUM-2:** the `verified-restore` mutant replaces `into_checked`'s call. If that call's return
type changes, the mutant stops compiling, which is the `index-commit` failure again.

**Lows:**
- **L1:** 15.10 counted three candidate projections, not four. Corrected above.
- **L2:** `evidence` takes `&`. `snapshot()` and `preserves_vault_source` need `&mut` for a change
  lookup, not a save. The conclusion stands.
- **L3:** `test-counters` exists only in F1's held commits, not on the pushed line. The counter
  should be an app-side wrapper, or land with that feature.
- **L4:** the old entry point's callers also include H1's interrupted-Prepared resolution,
  adoption and rotation.
- **L5:** the seed set should be an `Option`, `Some` required on the proof path.
- **L6, an alternative:** have the shared writer return the set it computes for its hold. Resolve
  could then reuse it on both paths with nothing carried, at the cost of the writer's signature.

**Revised plan.**

**Step A**, independent of item 0, and closing a gap that predates it:
1. Extract the coverage rule as a pure function: is every base blob covered by the candidate or a
   still-pending intent?
2. Unit-test it three ways:
   - it refuses an uncovered base;
   - it accepts coverage that comes only from a pending intent;
   - it refuses once that intent is gone;
   - plus `None` handling.
3. Add a CI mutation on the subset test.

**Step B**, item 0's first half, built only after A:
- carry `(basis, candidate set, Some(seed set))`, and require the basis on the proof path;
- a `cfg(test)` recompute-and-compare on the proof path, kept out of any counter;
- a mutation that empties the carried seed set, which that cross-check detects;
- a hook-driven negative that swaps the intent record after the Source write;
- the `verified-restore` mutant updated in the same change, under `RUSTFLAGS='-D warnings'`;
- 15.11's text corrected per MEDIUM-1, L2 and L4.

**Not before B is re-reviewed:** L6's writer-return alternative, item 0's second half, and the
branch-length attribution.

### 15.13 Step A, built (2026-10-09)

**What changed.** The rule is `base_blobs_covered(base, candidate, pending)` in
`store/epoch_studio/handoff.rs`: every base blob must be a candidate blob or a pending intent's,
and `None` (no overlay) is covered. `check_handoff_references` computes the three sets as before
and calls it, so its two H5 callers and every other caller are unchanged in behaviour. The error
order is unchanged too: the candidate's set, then the intents', then the base's.

**Tests**, in the handoff `references` tests:
- `..._refuses_a_base_blob_nothing_retains`: an uncovered base blob, and the same case once the
  retaining intent is gone;
- `..._counts_a_pending_intent`: coverage from a pending intent alone, and from the candidate
  alone;
- `..._with_no_base_accepts`: `None`, and an empty base.

**CI, three handoff-harness entries,** each DETECTED at its named assertion and PASS restored under
`RUSTFLAGS='-D warnings'`:
- `base-coverage`: `all` turned into `any`;
- `base-coverage-pending`: the pending term removed;
- `base-coverage-none`: `is_none_or` turned into `is_some_and`.

**What it does not pin:** that `check_handoff_references` calls the rule at all. No honest flow
reaches the refusal, so removing the call fails no test; the wiring stays checked by inspection,
which is HIGH-1's point. Step B's `cfg(test)` recompute-and-compare is what will exercise the call
on the proof path.

### 15.14 Decision (2026-10-09): H5 stays one visit; step 3's target is relaxed for it

**Decided by the project owner.** H5's commit stays a single custody visit, as it is today. The
125 ms visit target no longer applies to it, and this is recorded as technical debt rather than
designed away.

**What is accepted.** The commit grows with the source and the branch (15.10). Both are capped:
the base by `FLIPNOTE_MAX_FRAMES` (999) and the branch by `MAX_STUDIO_OVERLAY_OPS` (256). At both
caps it measured 147 ms (upper median of 3, release, shared host). The largest single sample in
F3's runs was 248 ms. Below the caps it is shorter: 86 ms at 998 frames and one operation, and
under 60 ms at 512 frames. It happens once per handoff, when a closed document's local draft is
transferred.

**As an attack surface.** Another member can fill a shared Flipnote to the frame cap, but the
branch is the handing-off device's own operations, and a device hands off only documents it has
itself drafted on while they were Closing. So the most a peer can force is one commit over a
maximal source for each such draft: a pause of roughly 0.1 to 0.15 s per handoff, with no
amplification. No cap change is needed. Lowering `FLIPNOTE_MAX_FRAMES` would be a format change,
and saves only about 0.06 ms per frame.

**What changes in the plan:**
- **Deferred, not dropped:** step B (item 0's first half) and the rest of 15.12's route, including
  the touched-path cursor decision (15.9 item 8).
- **Kept:** step A (15.13). It closed a test gap that predates item 0 and stands on its own.
- **Next:** Flow R, building its commit (R3) on the same single-visit path as H5.
- **If the target is restored later,** both H5 and R3 need splitting, not only H5. The 15.10 table
  and 15.12's plan are where that work starts.

Also recorded in `THREAT-MODEL.md` (the pause a member can cause) and in `HANDOVER.md`'s known
limitations.

## 16. The remaining owners: the prerequisite first, then a decision (2026-10-10)

**Decided by the project owner.** What remains of C-3's adoption is catch-up (section 5's
persist, begin-pass and serve sites, discovery, Registry and rotation), Agent 3's repair sites,
receive, H1, and replay's Apply seam. None of them is converted until the prerequisite below is
built and section 7's measurement is taken. The numbers then decide, site by site, whether it is
converted or recorded as debt, as H5 was (15.14).

**Why the prerequisite comes first.** Section 7 gates catch-up and receive on two measurements:
visits per inventory, and the restart rate with another writer active. Without writer-side warms
the outcome is already known (15.7, HIGH-1). With document A open and a peer drawing on B, every
receive turn writes B cold. A converted job therefore parks on B each turn and is overtaken by the
next packet, so it never finishes. Measuring before the warms would only confirm that.

**The prerequisite is 15.9's items 2 to 4:** M2, then M1 with writer warms at all eight writers
(the forced-warm token and 15.9's M4 contract), then MEDIUM-3's progress rule with per-key credit
and a ceiling. All three also cut today's synchronous cost, since a warm record is never validated
under custody again, so they are worth building whatever the decision. Item 1 (H5's
source-growing terms) stays deferred with H5 (15.14).

### 16.1 M2, built (2026-10-10)

`RecordCache` (`inventory/cache.rs`) now has four properties:
- **Indexed:** two `BTreeMap`s, the entries by key and an access order by a never-reused tick, so
  get, put and eviction are logarithmic. `EpochRecordKind` is `Ord` but not `Hash`, so this needs
  no new derive or dependency.
- **Bounded at `MAX_ACCOUNTED_RECORDS`** (65 536), the most records any inventory can account, so
  a complete scan never thrashes it. It was a 64-entry LRU, which re-validated every record of a
  larger vault on every scan.
- **Pruned at every finish that issues an inventory.** `prune_inventory_cache` runs in the three
  such paths (`EpochStorageScan::finish`, `finish_epoch_inventory_job` and
  `finish_epoch_storage_scan`), and only after `finish_with` has issued the inventory. It drops
  entries of the scan's own covered families that the scan did not find (15.7, LOW). The
  reference-scan finish issues no inventory and does not prune; it could, soundly.
- **One coverage rule.** `EpochInventoryCoverage::covers(family)` is now asked by both the
  traversal (`storage_name`) and pruning, so a scan cannot prune a family it did not read. It is
  equivalent, case by case, to the four checks `storage_name` used to make inline.

`finish_epoch_storage_scan` now takes `&mut self`. Its only callers are tests; one closure in
`epoch_registry/tests.rs` changed from `&ServerStore` to `&mut ServerStore`.

**Tests, each guard broken on purpose:**

| test | guards |
|---|---|
| `inventory_cache_is_bounded_lru_and_requires_family_size_and_full_digest` (rewritten for the new bound, a recorded contract change from 64) | the bound; a hit counts as a use; the index and order agree. A hit that does not refresh its tick fails it |
| `pruning_drops_only_covered_entries_the_scan_did_not_find` | prune's two predicates, at unit level |
| `a_scan_of_more_records_than_the_old_lru_held_reuses_every_warm_entry` (72 Studio records) | with the bound put back to 64 it fails, revalidating all 72 on the second scan, `(72, 0)` against `(0, 72)` |
| `a_completed_scan_prunes_a_removed_records_entry_within_its_coverage` | pruning disabled: the removed record's entry survives. Coverage ignored: a Recovery-only scan prunes a Studio entry |

All 127 inventory tests pass. `THREAT-MODEL.md` now gives the memo's real bound and residency.
Memory is about 300 bytes an entry and about 20 MiB at the bound, resident for the mount's life.
The bound is hard, whatever puts an entry.

**Review of M2 (2026-10-10, Opus, static, at `310770d0`): no blocker or high.** It checked the
coverage refactor against all 30 (coverage, family) pairs. It confirmed that pruning runs only on
`finish_with` success, can never drop a record the scan found, and leaves every non-scan put
(each describes bytes on disk) alone. It also found nothing quadratic, and the memory figures
fair.

| finding | what | disposition |
|---|---|---|
| MEDIUM-1 | pruning was pinned on only one of the three finish paths: deleting it from the job or cursor path failed nothing | **fixed:** the prune test runs its whole sequence through each path (`Finish::{Scan, Job, Cursor}`); each deletion now fails it, naming the path |
| MEDIUM-2 | `ARCHITECTURE.md` and `INTERFACES.md` still said "64-entry LRU" | **fixed** |
| LOW-1 | the bound rested on `entries` and `order` staying in step, unchecked after `evict_mismatch`: one that removed from `entries` only would let the memo grow past the bound | **fixed:** every removal goes through one helper; `put` evicts in a loop until below the bound; consistency is `debug_assert`ed after every mutation; a test of eviction then refilling to the bound. That mutant now fails three tests |
| LOW-2 | "every finish path" was not literal: the reference-scan finish does not prune | **reworded** to "every inventory-issuing finish", in code and docs; pruning there would be sound, and is left out because that finish issues no inventory |
| LOW-3 | THREAT-MODEL said memory is bounded by the vault's record count | **reworded:** only the 65 536 bound is unconditional; entries for deleted records stay until a covering scan completes, which receive's scan never does on a vault over 64 records |
| LOW-4 | `get`'s size check, and coverage for families other than Studio, had no test | **fixed:** a size-mismatched `get` with the right digest must miss; a Registry entry must survive scans that do not read Registry and be pruned by one that does. Each mutant now fails |

All 128 inventory tests pass.

**Next: M1** (15.2), with 15.8's HIGH-1 writer warms at the eight writers 15.9's M3 counts, the
forced-warm token, M4's contract, MEDIUM-2's three gates and 15.9's L1, L4 and L5. Its residual
risk from this review: each writer warm must land after its write's rename, in the same custody
visit, or a scan completing between the two could prune the new record's warm entry.

### 16.2 M1's memo half, built (2026-10-10)

M1 has two halves.
- **The memo half, here:** entirely in `inventory.rs` and `inventory/cache.rs`. 15.7 judged
  memoizing every family sound on its own, without the warms.
- **The writer half:** the warms at the eight writers, the forced-warm token and 15.9's M4
  contract. Five of the eight writer files are Agent 2's or Agent 3's, so it waits on their
  go-ahead.

**What changed:**
- **The value carries the facts.** A memo entry is the accounting record plus, for an Intents
  record, its `EpochIntentInventoryFacts`; `get` returns both and `accounting_only` restores
  both. `put_validated` (and `put_if_vacant`) take the facts and **refuse an entry of the wrong
  shape**: an Intents entry without facts, or another family's with them. The refusal
  `debug_assert`s and returns false. A fact-less Intents hit would undercount the Unconfirmed
  tally and mint too generous a budget. `put` keeps its signature for the facts-less writer
  paths (Registry and Studio, Agent 3's new `warm_registry_repair_inventory` included), so no
  other agent's file changed.
- **MEDIUM-2's three gates widened together:** the scan's `cacheable` (every family, accounting
  mode only), `memoize`'s family gate (removed) and `evict_mismatch` on every family's read.
- **L5:** the test helper `validated_clone` copies the facts.
- **L4:** stale text corrected in THREAT-MODEL (twice), ARCHITECTURE, INTERFACES,
  BACKEND-IMPLEMENTATION, design 9.2 consequence 2, `cache.rs`, and the docs of `memoize` and
  `memoize_overtaken_inventory_result`.
- **MEDIUM-4:** HANDOVER and THREAT-MODEL state that receive admission is history-dependent for
  every family.

**Tests, each guard broken on purpose:**

| test | guards; the mutant that fails it |
|---|---|
| `c3_every_family_is_memoized_and_a_hit_restores_a_fresh_validation` (`performance.rs`; the five-family vault, and one with a Closing overlay branch whose facts carry provenance) | a second scan reuses every record, and its inventory (identity, accounting, Intents facts) equals a cold scan's; a reference scan reuses nothing. The lookup gate narrowed back to Registry and Studio fails it ("a record was validated again"); a hit that drops the facts trips the cache's shape refusal first, at the re-memoize; the old family gate fails it |
| `a_budgeted_scan_memoizes_every_family` (renamed from `a_budgeted_scan_memoizes_only_registry_and_studio_records`, a recorded contract change) | a budgeted scan memoizes its Recovery, DraftArchive and Studio records, and a second reuses all three. The old family gate fails it |
| `an_intents_entry_without_its_facts_is_refused`, `facts_on_another_familys_entry_are_refused` (`cache.rs`; `should_panic` under `debug_assertions`, a returned `false` otherwise) | the shape refusal; removing it fails both |
| `a_stale_recovery_entry_is_evicted_so_a_refused_result_still_warms` | eviction on read for a family other than Registry and Studio; the old eviction gate fails it |
| `c3_receive_admission_depends_on_what_is_warm_and_a_remount_resets_it` (three 100 KiB Recovery records) | MEDIUM-4's both directions: refused cold at the rail, admitted on the next attempt (the refused scan warmed what it validated), refused again after a remount. Before M1 the second step fails |

**Two existing tests needed a cold scan restored**, a recorded precondition change rather than a
loosening. `a_budgeted_scan_produces_the_same_inventory_as_an_unbudgeted_one` and
`the_aggregate_byte_rail_still_refuses_after_a_record_has_been_parked_and_installed` each
require every record cold, and an earlier scan in the same test now warms Recovery records too.
Each empties the memo first, so each runs exactly the scan it was written for. A third,
`studio_inventory_cache_never_skips_reference_enumeration`, asserted that a warm scan reused
exactly one record, its Studio wrapper, when that was the only memoizable one. It now asserts that
the warm scan reuses every record, the Studio wrapper among them (a recorded contract change); its
reference-scan half, which reuses nothing, is unchanged.

**Residual, recorded in THREAT-MODEL:** the memo keeps an Unconfirmed branch's provider device
id, observed MLS epoch and observation time resident for the mount's life, UI lock included.
The same facts are in that record on this device's disk; nothing leaves the device.
