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

An independent audit separately confirmed the hard invariants by reading and attacking the code: P5
holds (no registered command and no composition of commands writes into a Closing branch); both
confirmation tokens are unforgeable and non-transferable; the detached copy worker carries no live
authority; and the `catcoms-mls` M-1 unreachability claim survives attack.

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

## 5. The three open issues

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

So there is a specific reachable sequence - archive generation N, dispose it, admit generation N+1
with identical entries, then preserve-dispose against the stale archive - where **only** the
generation compare stands between the user and destroying a branch whose "evidence" is for different
work. `matches_branch` cannot see it. That case has no test at all.

*Attack it by:* constructing that sequence and checking the refusal actually happens, and for the
right reason.

### Issue 2 - `probe_copy_object` (`crates/catcoms-app/src/studio/copy.rs:143-165`)

An Index `Object` copy must name a Flipnote that actually exists. **Force it to `Ok(true)` and all
five desktop copy tests stay green** - nothing else checks it, and the app crate has no store-backed
copy fixture that could reach it.

**It is the only one of the three that guards a durable write.** Without it, a copy publishes an Index
entry naming an object that is not there. It was added in response to a review finding and is modelled
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

Trace design 6.6's own worked example against the code: dispose G1 on basis B, admit G2 on B, deliver
a delayed G1 request. It is not a completed retry, not an exact retry, no longer pending, and the
basis still matches - so it is treated as new authoring and appended onto **G2**. Work resurrected
into a branch the user never put it in. That is precisely the case 6.6 says must return `Stale`.

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
