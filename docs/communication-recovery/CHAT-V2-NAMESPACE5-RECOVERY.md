# Namespace5 recovery and compatibility

This increment covers recovery and compatibility for namespace5, the member-outbox layout:

- real JOIN lifecycle coverage without governance4;
- an interrupted ordinary JOIN stage at open;
- a real predecessor-binary gate in CI;
- recovery from a torn FIRST declaration seed.

**Status:** implemented on `chat-v2-member-outbox-production`. A read-only adversarial design review found no blocker. Its one HIGH, five MEDIUMs and ten LOWs are folded in below. The focused CI lane runs the tests. Nothing here activates Chat V2 or grants any member, save or publication authority.

Background:

- [CHAT-V2-MEMBER-OUTBOX.md](CHAT-V2-MEMBER-OUTBOX.md), "Declared namespace5";
- the declaration writer, `write_member_outbox_layout_declaration` in `crates/catcoms-app/src/store/persistence/member_outbox_layout.rs`;
- the cold inspector, `inspect_and_repair_member_outbox_declaration` in `crates/catcoms-app/src/store/critical_namespaces/member_outbox_layout/declaration.rs`.

## 1. JOIN lifecycle on ID5 without governance4

The governance wrapper already ran a real JOIN lifecycle with both 4 and 5. Its Current3 equivalent (`actual_join_lifecycle_on_current3_preserves_five_exactly_without_four`) runs a Current3 opener with the member handler and checkpoint-JOIN execution:

1. It declares 3 and then 5.
2. It runs save, pause, resume, activation (which migrates the JOIN payload) and the first arm.

After every step:

- the ID5 descriptor is byte-identical;
- the entry set and order are unchanged;
- governance4 is absent from both the catalog and the Store;
- the namespace5 mount still validates.

The same JOIN-executing opener without the member handler refuses, and a cold remount (no RNG) writes nothing.

## 2. Interrupted ordinary JOIN stage at open

`an_interrupted_join_stage_on_a_five_vault_settles_cold_or_stays_preserved` uses the JOIN writer's own cuts on a settled ID5 vault (Current3, no 4):

| Fault | What it leaves | Cold open with the member handler |
| --- | --- | --- |
| 1 | A complete synced stage, before the rename | Selects that exact stage. ID5 stays byte-identical and the mount validates. |
| 2 | The rename done, but its directory barrier failed: no stage, candidate already selected | Settles. |
| 4 | A 17-byte prefix | Every opener refuses and the tree is preserved. |
| 5 | A zero-length stage | Every opener refuses and the tree is preserved. |

In every case, openers without the handler, and the emulated predecessor, refuse before any cleaner and change nothing.

`an_authenticated_stage_that_drops_or_changes_five_is_preserved_and_refused` covers the negative fixture: a stage sealed with the vault key that drops ID5, or changes its descriptor, is preserved and refused.

## 3. Real predecessor-binary gate (CI)

The emulation flag inside the current binary is supplemented by the actual code at `ce402364`, the last commit before namespace5.

That commit is not on `main`. It is reachable through PR #86's branch, and through the head ref GitHub keeps for the pull request, as long as that history is not rewritten. If it ever becomes unreachable, the fetch fails loudly rather than skipping the gate. Pushing a tag for it would remove this dependency; that is left to the primary.

The script fetches the commit explicitly and asserts the full SHA.

The focused lane (`.github/scripts/check-chat-v2-member-save.py`) runs three steps after its cohorts:

1. **Fixtures (head).** `predecessor_fixtures::generate_namespace5_predecessor_fixtures` is an ignored case outside every cohort filter. It writes 23 vaults and a manifest of their names.
2. **Predecessor.**
   - The script runs `git worktree add` at the pinned commit.
   - It copies `.github/predecessor/namespace5_gate.rs` in as `crates/catcoms-app/tests/namespace5_gate.rs`.
   - It proves with `git status --porcelain` that this file is the only change.
   - It runs the test with the shared target directory.
   - The harness uses only `ce402364`'s public API: all ten openers. These are the default, catalog-read-pool, pending-JOIN and JOIN-execution `ServerStore` openers, plus the archive, Current3 and governance wrappers, each wrapper with and without JOIN execution.
3. **Round trip (head).** `verify_namespace5_predecessor_round_trip` reopens what the predecessor wrote.

The harness owns every fixture's expected verdicts. The head's manifest must list exactly the harness's names, so a head change cannot relabel a fenced state as openable. The classes:

- **Control.** `c3` (settled 3) and `c34` (3 plus 4) have no namespace5. Each opener's verdict is pinned, and the matching wrapper must fully open the vault (open, retry, ready store). A copy, path, passphrase or format problem therefore fails here first, before any refusal can pass vacuously.
- **Refuse.** Every opener must refuse, and the whole tree must be unchanged afterwards (every name and byte). The refuse fixtures are:
  - settled 5, 4 then 5, and 5 then 4;
  - a used namespace (its parent holds a context);
  - every warm declaration cut except the claim-only one;
  - JOIN armed, an interrupted complete JOIN stage, and a JOIN prefix.
- **May-open.** `cut241` (zero-byte claim) and `d227` (abandoned torn seed, no fence). Every opener's verdict must equal its verdict on `c3`, and the member objects must survive. The full predecessor Current3 and governance runs (open, retry, ready store) are kept under `after/`. The governance run declares 4 over the claim, which is exactly the downgrade interleaving.
- **Unfenced.** `cut243-unfenced` breaks the gate on purpose: cut 243 with its fence removed must behave like `c3`. That proves cut 243 is refused because of the fence.

The round trip reopens all four `after/` trees with the member handler:

1. Open, with no RNG, must abandon the claim or torn seed.
2. The warm retry must declare 5, appended after exactly the entries the predecessor left (3, or 3 then 4). The predecessor's ID4 entry must be byte-identical afterwards, and both mounted layouts must validate.
3. Both markers and the parent must end up present, with no seed or stage left.

Counts are pinned in the head tests, the harness and the script: 23 fixtures, 10 openers, 4 round trips, and exactly one passing test per step. The job timeout is 90 minutes, and the predecessor build step has 40 of them.

## 4. First-torn declaration seed recovery

### The problem

The previous forward order was:

1. empty legacy stage fence;
2. seed;
3. marker;
4. parent directory;
5. candidate stage;
6. rename onto current (commit);
7. seed removal.

A crash before the first seed was complete left three cold states that every opener refused forever:

- the fence alone (cut 240);
- the fence plus an empty seed (241);
- the fence plus a seed prefix (242).

The seed frame is `nonce || ciphertext` with no length header. A truncated seed and a corrupt complete one are therefore indistinguishable: both simply fail to authenticate.

The fence alone could not be attributed to namespace5. It is byte-identical to two other states:

- governance4's own first-torn fence, which is governance's unrecovered gap and outside this increment;
- a zero-length JOIN stage, whose reviewed cold policy is preserve-and-refuse.

### Change 1: claim the seed name before the fence

The new forward order starts with a claim:

1. **Claim.** `create_new` the seed with zero bytes, hold the handle, sync it, then a directory barrier.
2. **Fence.** The empty stage, synced, then a directory barrier.
3. **Seed content.** Written into the SAME held claim file, then synced, then a directory barrier.

Everything after that is unchanged.

Why this works:

- **Namespace5 never creates a fence without the claim.** The head's warm writer refuses to start over any existing stage. Each head writer refuses while the other's seed exists.
- **A zero-byte claim carries nothing and depends on no catalog.** A predecessor ignores the unknown name and may open the vault while only the claim exists, which is harmless.
- **The fence precedes every content-bearing member output:** seed bytes, marker, parent and candidate.
- **The predecessor is fenced from those outputs until commit, or until abandonment starts.** While its disposal runs, an abandoned unauthenticated seed with no fence is not fenced, and it carries nothing either.
- **New-binary openers without the handler still refuse the claim.** Presence of any kind refuses.

Fault cuts, in write order:

| Cut | State after the cut |
| --- | --- |
| 241 | claim only |
| 240 | claim plus fence |
| 242 | torn seed content |
| 243 onwards | unchanged |

### Change 2: the cold first-torn role

`codec::classify_seed` answers one question: did the seed authenticate? It returns `Unauthenticated` only in two cases:

- the frame is shorter than one nonce plus tag (40 bytes);
- AEAD opening under the member seed key fails with `KeystoreError::Decrypt`.

Any other error refuses. Bytes that open are `Authenticated`, and an authenticated seed whose bounds or grammar are invalid is preserved and refused (it may be a future version's). A 40-byte frame is opened, not assumed torn.

The inspector abandons the declaration only when ALL of the following hold, all checked from the same paid inventory before the first unlink:

1. The opener carries the member handler, its capacity checks pass, and the declaration fee is reserved.
2. No archive (ID2) or Current3 (ID3) declaration seed exists in any form. A governance4 seed is allowed only in the sub-role below.
3. The current catalog authenticates and parses under the configured grammar. It names the archive and Current3, and does NOT name ID5.
4. The marker and the parent directory are absent. They are re-proven absent at the unlink edge.
5. The seed is a real file of at most `MAX_SEED_FRAME` bytes, held, and `Unauthenticated`.
6. The settled Current3 root and marker are proven read-only (`check_current_marker_before_old_gates`). So an unsettled Current3 layout is refused before anything is touched.

   This is not a proof that every later gate will accept the vault. The disposal does not run those gates' other checks (the owner-operations intent, the archive/JOIN gates). One known case mutates and is then refused:
   1. A claim sits beside a predecessor JOIN stage, opened through the governance+member wrapper while 4 is undeclared.
   2. The member inspector disposes the claim.
   3. The governance inspector then refuses the stage, as it already refuses any stage while 4 is undeclared.

   That loses only the zero-byte claim, and the Current3+member opener then settles the stage. Only unauthenticated bytes or a zero-byte stage can ever be lost this way.
7. The stage has a classified role:

| Stage found | Role |
| --- | --- |
| absent | nothing to remove |
| a real zero-byte file | the fence: unlinked first |
| non-empty, and an authenticated ordinary successor of current that does not name ID5 | `Untouched`. Namespace5's own stage bytes follow a complete seed and always name ID5, so this can only be a predecessor's JOIN write in the claim window. It is left to the Current3/JOIN settlement. |
| anything else non-empty | refuse |

**Disposal** (`dispose_first_torn_member_outbox_seed`) uses no RNG and no reseal:

1. If the stage is the fence, re-prove its held identity and zero length, then unlink it. Drop every handle to it, run a directory barrier, then require the name to be NotFound.
2. Re-prove that the held seed still has exactly the classified bytes, then unlink it. Drop the handle, run a barrier, then require NotFound.
3. Report namespace5 as undeclared. The warm retry declares again with fresh randomness.

The NotFound proof matters on Windows: a name another process holds open can stay delete-pending under legacy delete semantics, and a power loss can then resurrect it. Any result other than NotFound refuses before the next object is touched.

The fence goes first because every intermediate is then first-torn again: an unauthenticated seed, with or without a fence. Removing the seed first would leave a fence with no claim, which is ambiguous.

The cold cuts are 226 (after the fence unlink), 227 (after its barrier) and 228 (after the seed unlink). They use a one-shot thread-local seam, because the Store and its fault atomic are created inside the open that runs them. Tests assert that the seam fired.

**Governance sub-role.** It covers a governance4 seed beside the member seed:

- **Cause.** Only a predecessor can produce this state. Its governance opener does not know the member seed name, so in the claim window it can begin a governance declaration and crash inside it.
- **The deadlock it avoids.** Without this role, the head's member side refuses the governance seed and its governance side refuses the member seed.
- **When it applies.** The opener must carry the governance handler, and the governance seed must authenticate and parse as a COMPLETE governance seed. The member seed is then disposed and the stage is never touched (it can only be governance's), and governance finishes cold.
- **A torn governance first seed** is governance's own unrecovered state. Both seeds are then preserved and refused, rather than one being disposed in front of the other's refusal.

**Why this cannot turn corruption into an assumed empty namespace.** "Undeclared" rests on three facts:

- the authenticated current catalog lacks ID5;
- the marker is absent;
- the parent directory is absent.

It never rests on the seed, which can confer nothing unless it authenticates. Only an unauthenticated seed and a zero-byte stage are ever discarded, and neither can be selected, adopted or replayed. Every opener takes the exclusive vault-session lock (`acquire_vault_session`) before any catalog work. The cold state is therefore crash debris, never a live writer's work.

### Preserved and refused

- an authenticated seed whose content is invalid;
- an unauthenticated seed beside a marker, a parent, a partial or foreign stage, an ID5 catalog entry (for example, a used namespace that lost its marker and parent), another namespace's seed, or a torn governance seed;
- a seed that is not a real file;
- a current catalog that is missing, unauthenticated or unparseable;
- a fence with no claim: another writer's empty stage, or fence-first debris from a namespace5 build that predates the claim. Such builds never shipped, so that debris needs no recovery.

### Residuals

- **Bit rot.** Bit rot in a COMPLETE seed before the marker exists (cut 243) is also unauthenticated, so it is disposed. Nothing downstream existed.
- **Future formats.** A future writer of `chat-critical-namespaces.member-outbox.seed` must seal under the v1 seed key domain, so this version preserves its seed as authenticated-unsupported. Otherwise it must use another name: under a different domain, this version would dispose it on a downgrade.
- **Zero-byte stages from other writers.** In the claim window, a predecessor's own zero-byte governance fence (its cut 229), or a zero-length JOIN stage (its fault 5), looks like the claim's fence and is unlinked. It holds no bytes, and the current catalog is unchanged. Both cases are regression-pinned.
- **Predecessor-only recoveries.** A predecessor's torn governance FIRST seed in the claim window stays refused, because it is governance's own gap. A predecessor's partial JOIN stage there stays refused, per the JOIN policy. The predecessor itself, which ignores the claim, can settle its own complete work.
- **Out of scope, as elsewhere:** torn media despite a successful sync, external rollback, and arbitrary corruption. Governance4's own first-torn recovery is not part of this increment.

### Tests

Current3 (`current_layout/tests/member_outbox.rs`):

- **Matrix.** Cuts 240, 241 and 242 are abandoned at a cold open with no RNG, then declared by the retry. Every later cut finishes with no RNG. The emulated predecessor opens at 241 and refuses every other cut.
- **Disposal cuts.** Reopening at 226, 227 or 228 completes. At 226 and 227 the emulated predecessor opens the unfenced intermediate without touching the seed.
- **H1 regression.** With the seam reporting the name still present, the open refuses with the seed untouched, and a reopen completes.
- **Refusal classes.** Each is checked byte for byte (recursive tree).
- **M2.** A predecessor JOIN complete stage in the claim window settles, and only the claim is disposed. A partial stage refuses. A zero-length one is unlinked with the claim (the residual above).
- **Positive control.** The hand-built envelope in the authenticated-stage refusal test must reproduce the real catalog byte for byte, so that refusal cannot be vacuous.

Governance+member (`governance_layout/tests/member_outbox.rs`):

- The same cut matrix through the governance wrapper.
- The disposal cuts through the wrapper.
- The M1 interleavings: governance cuts 229 and 232 to 238 recover, and the torn governance seeds at 230 and 231 refuse with everything preserved.

The focused counts are now 16 (current mount) and 7 (governance mount).
