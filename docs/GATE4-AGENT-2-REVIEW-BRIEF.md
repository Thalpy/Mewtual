# Agent 2, Gate 4: adversarial review brief

Written for someone who is going to attack this work. It states what was built, what is actually
proven, what is not, and the three open issues. Everything here is checkable; where a claim rests on
something I did not execute, it says so.

- **Branch:** `gate4-agent1-runtime`, pushed through **`414fda06`**
- **Scope:** overlay lifecycle, provisional local work, repeated owner tenure
- **Design of record:** `docs/GATE4-AGENT-2-DESIGN.md`. **Ledger:** `docs/GATE4-AGENT-2-STATUS.md`
- **Gate status: P5 is FALSE and `studio_overlay_save` is unregistered.** Nothing below changes that

---

## 1. What this feature actually does

A user has a local draft that survived their document closing. It is theirs, it is not published,
and nobody else can see it. This scope gives them four things they can do with it, and makes each one
honest about what it costs:

| Verb | Command | What it must never do |
|---|---|---|
| **Look** | `studio_overlay_read`, `studio_overlay_lifecycle` | Turn reading into authority, or write anything |
| **Keep** | `studio_overlay_archive`, `_archive_read`, `_export` | Claim work is preserved when it is not |
| **Carry forward** | `studio_overlay_copy_preview`, `_copy_apply` | Imply a copy preserved the branch |
| **End** | `studio_overlay_dispose`, `_archive_release` | Destroy evidence the user did not confirm |

The load-bearing rule across all of it is **evidence before removal**: a disposal that says "preserve"
may only proceed if a durable archive of that exact branch already exists, and the only thing in the
system that destroys an archive is a separately confirmed release.

---

## 2. What is built

All nine native commands the design specifies, plus the machinery under them:

- **The draft archive family** - payload codec, record writer with its own cap and accounting, typed
  reader, reference collector, and the release path (the only thing that destroys one).
- **The disposal transaction**, D1-D6, in one accounted atomic replacement of the intent record.
- **The branch-generation namespace** - `branch_id = H(basis, generation)`, two-stage
  `classify_request` -> `admit_new_branch`. **Built but not wired: see issue (4).**
- **Repeated-owner tenure** - `OwnerTenure::joined`, leaf identity, `ObservedOwnerTenure`, the
  `catcoms-mls` M-1 receive rule, the v1 `Imported` migration, and the app seam
  (`StudioOwnerTenure`, `observed_owner_tenure`, `require_observed_owner_tenure`).
- **A mutation harness** (`.github/scripts/check-studio-overlay-lifecycle-mutations.py`) and its CI job.

---

## 3. What is actually proven

Executed, not asserted:

| Suite | Result | Where |
|---|---|---|
| `catcoms-sync --lib owner_tenure` | 11 passed | worktree at `a0803080` |
| `catcoms-replication --lib studio::` | 136 passed | worktree at `a0803080` |
| `catcoms-app --lib studio` | 317 passed, 6 ignored | worktree at `a0803080` |
| desktop `--lib studio::` | 45 passed | independent audit at `288bb30c` |
| mutation harness | 9 detected, 9 restored runs passing | under CI's `-D warnings` |

Independent audits confirmed, by reading and attacking the code:

- **P5 holds**, stated precisely: **no registered command path accepts new annotated work into a
  Closing overlay.** Do not broaden that to "no command writes anything touching a Closing document" -
  disposal deliberately replaces an intent record, and the archive family writes its own records.
- **The detached copy worker carries no live authority** - data, choice, scope, instance token and a
  permit, no group, device, key or store.
- **Destructive requests require an explicit typed confirmation** that cannot be cloned or reused
  across transactions, and the mutation separately checks the named evidence. **This is not the same
  as "unforgeable"**: the parser matches a public literal, so it is not a capability and it does not
  itself encode which branch or archive the user was shown. The evidence binding does that.

**The M-1 unreachability claim has been WITHDRAWN** - see 4b. The check is present and pre-merge; the
argument for not testing it was wrong.

**Run tests in a detached worktree, not the main tree.** Other agents' in-flight work currently
references `catcoms_rt::REQUEST_TIMEOUT` and `catcoms_sync::BlobPageOutcome`, which do not exist yet,
so `cargo test` cannot link anywhere in the workspace:

```
git worktree add --detach M:/review-wt 414fda06
cd M:/review-wt
CARGO_TARGET_DIR=M:/review-target RUST_MIN_STACK=33554432 cargo test -p catcoms-app --lib studio
```

---

## 4. Where I got things wrong

Read this part first if you want to know how much to trust the rest.

- **Nine vacuous tests, all mine**, found by mutation. Every one had the same shape: **the input never
  reached the guard the test named.** A cross-document copy pointed at an *empty* foreign projection
  died at "missing recovery title" rather than the scope check - and with that check deleted, the plan
  came back `Ready`, copying a foreign group's content into this document.
- **Several of them guarded fixes I had made in response to earlier reviews.** The `Imported` preserve
  regression is the clearest: mutating its flag to `false` left all eleven sync tests green, so the
  fix that stops an unverifiable tenure laundering itself into an observed one had been unprotected
  since the day it landed. **A fix and its proof are separate pieces of work, and I kept shipping the
  first while assuming the second.**
- **I told Agent 1 a false fact** - that no app call site consults the tenure accessors. Nine do. I
  grepped for the wrong symbol and reported the result as though I had checked the right one.
- **I broke the shared branch's CI** with a one-line rustfmt violation, which blocked every branch on
  that base. Format runs before Clippy and tests, so the root suite had not executed at all. I had run
  `cargo fmt` on one crate all session and never `--all`.
- **My mutation harness was not running under CI's flags.** It inherited `RUSTFLAGS` instead of setting
  it, so CI compiled mutants with `-D warnings` and I did not. That is why I reported "nine detected"
  in good faith while the job was red.

The last two were found by Agent 3, not by me.

---

## 4b. What the external review found, and what I did with each

An external adversarial review of `414fda06` returned CHANGES REQUIRED. Its dispositions:

| Its finding | My response |
|---|---|
| **High: preserving disposal does not establish archive durability before removal** | **I disagreed, and I was wrong. Now issue 0 below.** My "shared directory barrier" argument fails twice: `sync_directory` is `Ok(())` on `not(unix)`, so on Windows there is no barrier at all; and "if the fsync fails, neither is durable" is not a property of `fsync` - a failed flush means *not guaranteed*, not *nothing persisted*, which this family's own tests already reflect by treating a post-rename sync failure as committed. The closure is withdrawn |
| **My rollover trace was wrong** | **Accepted.** The retained manifest blocks the short sequence. Corrected in section 6 and in the status doc; my replication test already used the right one |
| **D4's regression needs an intervening disposal** | **Accepted.** Folded into issue 1 below |
| **M-1's unreachability waiver omits inline proposal lists** | **Accepted and withdrawn.** Every clause of my argument described *our builder*; a commit carries a list of `ProposalOrRef` and an inline proposal needs no stored entry, so a hostile existing member can send Remove(A)+Add(A) in one commit. "Our builder cannot produce it" was never evidence a peer cannot submit it. The check is present and pre-merge, so no known bypass; the waiver was the defect |
| "Unforgeable" overstates the confirmations | **Accepted**, corrected below |
| "Only the copy issue guards a durable write" | **Accepted**, corrected below: D4 and D1 guard destructive durable transitions too |
| The harness does not check isolation | Already recorded; its claim is corrected in the script |

**New positive evidence it supplied:** CI run `36723616483` passes both `lifecycle` and `overlay`
jobs on the merge checkout - 106 app overlay, 47 replication, 11 sync, and 9 mutations detected with
9 restored regressions passing, under `-D warnings`.

## 5. The open issues

### Issue 0 - preserving-disposal crash ordering: ORDERING CLOSED, PLATFORM BARRIER OPEN

**Update, after a second re-review.** It separated two obligations I had run together: a real
directory barrier, and the *order* in which it runs. Fixing `sync_directory` on Windows alone would
not have proved the ordering, because the first barrier covering the archive would still have been
the replacement's own - after the branch-removing rename.

**The ordering half is now fixed and anchored.** After D4 matches the archive and before anything is
removed, disposal hands the on-disk archive back to the archive writer, which takes its exact-retry
branch and performs a guarded, accounted, sync-only repair - file contents, then parent directory -
changing no bytes. One definition of "durably established", and it is the writer's. If it cannot
complete, disposal refuses with nothing removed.

Two tests, both observed through the transaction's own hooks rather than inferred from the result
(the result is identical either way, which is why a result-only test could never catch this):

- the archive's sync event strictly precedes the intent write;
- injecting failure **at the sync boundary itself** - not an after-write hook, which describes a
  durable-but-unaccounted record - refuses, and every persisted record is byte-identical afterwards.

**Mutation evidence:** skipping the barrier call entirely makes both tests fail at their own
assertions with the other ten disposal tests green - and, more tellingly, the disposal then
**succeeds** and records `Preserved` without the archive ever having been made durable. That is the
defect the review described, demonstrated. Swallowing the barrier's error is caught too, but by a
second defence: the failed sync has already closed both budgets, so the removal write is refused for
reconciliation. Only the test's assertion on the refusal *message* tells the two apart, which is now
the tenth harness entry.

**What remains open:** on `not(unix)`, `sync_directory` is still `Ok(())`, so the repair establishes
the archive's file contents but not its directory entry. That primitive is shared by every record
family and the decision is above this scope.

*Original entry, kept for the record:*

This is the one issue here that is **not** merely an evidence gap. The others are guards that work
and are untested; this is a guarantee this scope claims and does not have.

D4 authenticates and decodes the archive; it syncs nothing. I argued that was fine because the archive
and intent records share a `servers/` directory and the replacement's `atomic_write` ends in
`sync_directory` on that parent. **Two things break that:**

1. **`sync_directory` is `Ok(())` on `not(unix)`** (`store.rs`). On Windows there is no parent barrier
   at all, so there is no shared barrier to lean on. `fs::rename` does not supply one either - the
   pinned toolchain's `MoveFileExW` does not request write-through.
2. **"If the fsync fails, neither is durable" is false.** A failed flush means completion is not
   guaranteed, not that nothing reached stable storage. This family's own tests already treat a
   post-rename sync failure as **committed, not rolled back**.

What actually holds is narrower: **on Unix, a successfully completed replacement makes both namespace
changes durable.** Interrupted executions, and every execution where the barrier is a no-op, are not
covered.

*Why I have not fixed it:* `sync_directory` is shared by every record family, so the decision is above
this scope - implement a real Windows barrier, or refuse a preserving disposal before removal on a
platform that cannot provide one, or narrow the product's stated guarantee. Calling the existing no-op
helper again changes nothing.

*Attack it by:* injecting failure at the `atomic_write_with_hook_and_sync` boundary, between rename
and parent sync. Note that `WriteHooks::fail_after_write` is **not** that boundary - it runs after the
physical write completes and its own docs describe a durable-but-unaccounted record.

The `debug_assert` at the D4 site is kept for the one thing it proves - the two families are
co-located - and the comment now says explicitly that it proves nothing about durability.

### The three evidence gaps

**All three are gaps in evidence, not known-broken behaviour.** The code is currently correct in each
case. What is missing is anything that would tell you if it stopped being.

Each was found by deleting the guard and watching the whole suite stay green.

### Issue 1 - D4's metadata triple (`crates/catcoms-app/src/store/epoch_intents/disposal.rs:236-238`)

Before a preserving disposal destroys a branch, D4 compares the archive's `content`, `branch` and
`generation` against the live branch. **Delete all three and the 105-test overlay suite stays green**,
because `matches_branch`'s full-envelope comparison refuses first.

**This is the one I would fix first, because the masking guard does not cover the same ground.** The
code's own comment says why:

> the generation compare is the one that refuses an archive of a *previous generation* whose entries
> and content happen to be identical, which `content` alone cannot catch because `branch_hash` does
> not cover the generation

So there is a sequence where **only** the generation compare stands between the user and destroying a
branch whose "evidence" is for different work. `matches_branch` cannot see it. That case has no test.

**The obvious sequence does not work, and a review corrected mine.** "Archive N, dispose N, admit N+1
with identical entries" is blocked: the retained manifest remembers N's ids and `validate` refuses the
overlap. The manifest has to be replaced first:

```
G1 contains X;  archive A1 from G1;  dispose G1 with Preserve  (A1 survives)
G2 contains disjoint Y;              dispose G2 with Discard   (replaces G1's manifest, A1 survives)
G3 contains X again, same basis, same envelopes/order/timestamps
attempt Preserve against A1
```

The timestamps must genuinely match, or `matches_branch` masks the generation check again. Assert
before disposing:

```
A1.matches_branch(G3) == true
A1.content          == G3.branch_content
A1.generation       != G3.generation
A1.branch           != G3.branch_id
```

then require the refusal, with G3 and A1 both intact afterwards.

*Mutation discipline:* do not demand that deleting `branch` alone and `generation` alone each fail
uniquely - `branch_id` is `H(basis, generation)`, so they are mutually redundant by construction. Test
the semantic invariant: an archive from an earlier generation must not authorise this generation's
disposal, while full-envelope matching still succeeds.

### Issue 2 - `probe_copy_object` (`crates/catcoms-app/src/studio/copy.rs:143-165`)

An Index `Object` copy must name a Flipnote that actually exists. **Force it to `Ok(true)` and all
five desktop copy tests stay green** - nothing else checks it, and the app crate has no store-backed
copy fixture that could reach it.

Without it, a copy publishes an Index entry naming an object that is not there. (An earlier version of
this brief called it "the only one of the three that guards a durable write". That was wrong: D4 and
D1 both guard a destructive durable transition. What is distinctive here is that the bad outcome is a
*new durable record naming something absent*, rather than a removal.) It was added in response to a review finding and is modelled
directly on recovery's equivalent guard - which is why I believe it is correct, and also exactly what
every one of the nine vacuous tests looked like.

*Attack it by:* building an Index destination whose object was cleaned up, and checking the preview
downgrades to `MissingTarget` and the apply refuses.

### Issue 3 - D1 membership (`crates/catcoms-app/src/store/epoch_intents/disposal.rs:72-77`)

The check that whoever disposes is a current member of the group. **Delete it and the suite stays
green**, because the "stranger" in its test is also not the branch's author, so the authorship check
refuses first.

The case that separates them is real: **the branch's own author is removed from the group, then
disposes.** Authorship passes - they genuinely are the author - and only membership refuses.

*Attack it by:* removing the author from the group and disposing. This needs an MLS removal in the
fixture, which is the most expensive of the three and the least likely to be wrong today.

---

## 6. One more thing, and it is not an evidence gap

**The branch-generation namespace is built and unwired.** `classify_request`, `admit_new_branch` and
`new_admitted` have **zero non-test callers** anywhere in `catcoms-app`. The Save seam carries `basis`
and never a `branch`, so the two-stage classification is never consulted by production code.

**The short trace this brief originally gave was wrong**, and a review caught it. "Dispose G1, admit
G2, deliver a delayed G1 request" does **not** resurrect the work: the retained G1 manifest still
remembers G1's ids and `validate` refuses any state where an id is both in the live branch and in the
retained manifest. The reachable sequence needs that manifest replaced, since `dispose` overwrites the
previous one and does not advance the basis floor:

1. Accept G1 containing X; dispose G1.
2. Accept G2 containing **disjoint** Y on the still-eligible basis.
3. Dispose G2, replacing G1's manifest.
4. Deliver a delayed G1 request naming an operation from X.

The id is now not pending, not in the retained manifest, and not a completed or exact retry, while the
basis still matches - and a Save seam carrying only `basis` has nothing left to distinguish it from
new work. That is the case 6.6 says must return `Stale`.

It is unreachable today for exactly one reason: `studio_overlay_save` is unregistered. **That makes it
a P1 and P5 blocker rather than a live defect**, and it is the honest reason P1 cannot be called
reviewed - "lossless across restart and refusal" cannot be claimed for a lifecycle whose rollover
defence has no production caller. Wiring it is Flow S's job.

---

## 7. What I would most like attacked

In order:

1. **The claim in section 3 that the invariants hold.** Especially "no composition of commands writes
   into a Closing branch" - copy's apply does publish an ordinary Save, into the *destination*.
2. **Issue 1's sequence.** If a previous-generation archive with identical entries is *not* actually
   constructible, that changes its severity a lot.
3. **Whether `branch_content` is now right.** It used to hash the whole intent ledger, so an unrelated
   intent changed a branch nobody had touched, and a preserving disposal refused while holding an
   archive of exactly that branch. It now hashes the branch alone. Two hashes now exist with different
   scopes; check the other one (`Prepared.branch`) is still correct to be document-wide.
4. **The harness's honesty.** It runs each mutation with `--exact`, so siblings never execute and the
   script does **not** check isolation. That rests entirely on hand-runs.
5. **Anything in `GATE4-AGENT-2-STATUS.md` that the tests do not support.** That document has already
   carried two false claims that reviews caught.
