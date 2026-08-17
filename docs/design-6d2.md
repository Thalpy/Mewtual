# Mewtual 6d-2; Design (concurrent-commit fork resolution + proposal/commit split)

Output of a design + adversarial-review `Workflow` (3 capability investigations
verified against the **openmls 0.8.1 source**, 3 independent designs, adversarial
critique, synthesis). This is the implementation contract for 6d-2. The exhaustive
version is in the session task output; this is the load-bearing summary.

## The decision

Lift the 6d-1 single-committer limit safely. All three naive designs shared one
**structural bug**: their fork-resolution routed the winner's commit through the
existing apply-gate `if designated_committer() != record.committer_device { reject }`
— but in a real fork the winner is *not* the loser's designated committer, so the
winner is rejected and the loser never converges. The fix:

- **Gate on "authorized committer with a valid per-commit signature", not leaf-index
  equality.** `authorize_committer` does roster-lookup-then-verify (find the current
  member whose `signature_key` content-addresses `committer_device`, verify the sig
  against that raw key) + a rank bound (`leaf_index ≤ designated_index +
  max_committer_rank`).
- **`base_authenticator` distinguishes fork from lag.** Same `commit_epoch` + same
  `base_authenticator` ⇒ genuine same-base fork → tie-break by lowest `commit_id`.
  Same epoch number + *different* `base_authenticator` ⇒ deep divergence → refuse to
  tie-break, route to catch-up / raise `ForkTooDeep` (remediation deferred to 6d-3).
- **Make forks rare and shallow, then resolve with `clear_pending_commit`.** Do not
  try to heal deep partition divergence in 6d-2.

## Data model (deltas vs current code)

`CommitRecord` gains two fields (hard wire cutover; bump control label to
`catcoms/control/v2`, add a 1-byte tag: `CTRL_COMMIT=0`, `CTRL_PROPOSAL=1`,
`CTRL_REVOKE=2`):

```
CommitRecord { group_id, commit_epoch, committer_device, mls_commit,
               base_authenticator: [u8;32],   // epoch_authenticator() BEFORE the commit
               committer_sig: [u8;64] }        // leaf-key sig over the auth transcript
```

- `commit_id = BLAKE3("catcoms/commit-id/v1" ‖ group_id ‖ commit_epoch ‖ base_authenticator ‖ committer_device ‖ mls_commit)`; derived, never stored; the tie-break key (lowest wins; content-addressed, no clock/order input).
- `commit_auth_transcript = "catcoms/commit-auth/v1" ‖ group_id ‖ commit_epoch ‖ base_authenticator ‖ committer_device ‖ BLAKE3(mls_commit)`; signed by the committer's MLS leaf key. **openmls still independently authenticates the inner commit** via `process_incoming`; `committer_sig` is *authorization*, not state authentication.

`SyncConfig` adds: `max_committer_rank` (default 1), `stage_decision_window_ms`
(250), `max_pending_proposals` (256), `max_revoked` (4096).

## openmls 0.8.1 call map (all confirmed present)

| Step | call | status |
|---|---|---|
| stage without advancing epoch | `commit_builder().add_proposals(owned).load_psks(storage).build(rand,crypto,signer,f).stage_commit(provider)` → `PendingCommit(Member)` | ✅ |
| merge own staged commit | `merge_pending_commit` | ✅ |
| roll back loser (Member-only, no epoch change, preserves keys) | `clear_pending_commit(storage)` | ✅ |
| apply remote winner; inspect adds pre-merge | `process_message`→`StagedCommitMessage`; `StagedCommit::add_proposals().key_package()`; `merge_staged_commit` | ✅ |
| fork-vs-lag base binding | `epoch_authenticator()` | ✅ |
| author proposal (6d-2b) | `propose_add_member`/`propose_remove_member`; ingest via `process_message`→`store_pending_proposal` (two-step, NOT auto-queued) | ✅ |
| **external-commit self-heal** | `export_group_info(crypto,signer,true)` *does* emit `external_pub` on demand (no `config.rs` change needed) | ⛔ **deferred to 6d-3**; `leftmost_free_index` reindexes leaves, destabilizing committer rank mid-recovery; only needed for the deep-partition case we instead refuse |

Proposals are committed **by value** (`CommitBuilder.add_proposals(owned)`), not
by-ref (`commit_to_pending_proposals`), so a node that missed the proposal gossip
still has the KeyPackage/Remove inline in the commit → no `MissingProposal`.

## Fork resolution + loser recovery

Production becomes **stage → broadcast → bounded-wait → merge/abort**:
- `serve_join` stages (does not merge), broadcasts the signed `CommitRecord`, returns
  a `JoinPending` ack, and sends the signed Welcome **only after merge** (the
  **provisional-Welcome fix**: a losing committer must not strand a joiner on a dead
  commit). The Welcome stays inviter-signed, so the inviter remains the admitter in
  6d-2a/b (committer-decoupled admission deferred to 6d-3).
- `on_control` at `commit_epoch == current`: `authorize_committer` → if we have a
  same-base staged commit, tie-break by `commit_id` (loser `abort_staged` →
  `clear_pending_commit`, re-issue at the new epoch; winner `merge_staged_self`);
  different `base_authenticator` ⇒ refuse (`ForkTooDeep`). A node with no staged
  commit buffers same-epoch commits for one `stage_decision_window_ms` and applies
  the lowest `commit_id` seen, so even a non-participant records the deterministic
  winner (order/clock independent).
- The loser rolls **forward** E→E+1 (never skips), still holding valid keys
  (`clear_pending_commit` preserves epoch secrets). No ExternalInit on the loser path.

The `is_operational()` freeze (openmls errors on `propose_*`/`add_members` while a
commit is staged) is bounded by `stage_decision_window_ms`; inbound proposals are
buffered (plain writes, no openmls call) and packed into the next epoch.

## Single-use across members + joiner-nonce binding

- **Consumption is derived from committed history**, not a side-channel vote:
  refactor `process_incoming` to surface the `StagedCommit` before merge; for each
  Add proposal, `validate_membership_binding(kp)` (factored from the current
  `add_member_via_invite` checks: `group_id` matches, `invite_nonce` live, leaf key
  content-addresses `device_id`) runs on **every** member; on success
  `consumed.insert(nonce)`. Same commits ⇒ same consumed-set on every node, forge-proof,
  never evicted.
- **Revocation rides a `CTRL_REVOKE` op** (signed by a current member), fail-safe and
  bounded (`max_revoked`); evicting a revoke only re-permits a nonce whose Add is still
  gated by the consumed-set.
- Double-claim handling: same-base fork → one winner (single consumption); sequential
  → second Add rejected at apply on every node (nonce already consumed); deep
  partition → `ForkTooDeep` (detected, not remediated; honest residual).
- Joiner nonce reuses `MembershipCredential{device_id, group_id, invite_nonce}`
  (already in the MLS leaf, MLS-authenticated); the 6d-2 addition is all-members
  apply-time re-validation.

## Staging (strict dependency order)

- **6d-2a**; stage/merge/abort split + signed `CommitRecord` + `authorize_committer`
  gate + `commit_id`/`base_authenticator` tie-break + loser rollback/re-issue +
  provisional-Welcome + contest window. (Implemented in two commits: **(1) foundations**
 ; primitives + signed records + gate, single-committer synchronous join preserved;
  **(2) fork resolution**; staging + tie-break + provisional Welcome.)
- **6d-2b**; proposal/commit split (`ProposalRecord` on control topic, by-value
  packing, deterministic order); non-committers drive removes.
- **6d-2c**; history-derived consumed-set + all-members apply-time binding +
  replicated revoke.
- **6d-2d**; joiner-nonce binding hardening + `ForkTooDeep` surfacing + PCS on remove.
- **6d-3 (deferred):** external-commit self-heal, committer-decoupled Welcome, topic
  rotation on removal, deep-partition remediation (`fork_resolution::reboot`).

## Invariants (test every sub-block against these)

At most one commit merges per epoch on every node; deterministic convergence
(identical final `(epoch, roster, epoch_authenticator)` independent of receive
order/clock); no epoch skip on rollback; forward secrecy (a loser's aborted commit is
never merged/served); no stranded joiner (Welcome only for a merged commit); authority
(only a signed committer of rank ≤ `max_committer_rank` advances an epoch); fork-vs-lag
(differing `base_authenticator` never tie-broken); every new buffer bounded.

## Adversarial review of the 6d-2a implementation (must-read)

A post-implementation `Workflow` review (42 agents, 26 confirmed findings) returned
**default path `max_committer_rank=0`: safe-as-is** (the shipping single-committer
path is untouched), but **opt-in path `max_committer_rank>=1`: must-fix before
relying on it.** The flag therefore stays **OFF by default** until the items below
land. What was found and what was done:

- **I1; CRITICAL (open): the clock-window contest can converge two honest nodes to
  different winners.** Each node times its contest window from its *own* clock when
  it sees its first candidate; a lower-`commit_id` candidate that arrives after a
  node already resolved is dropped (`Ordering::Less`). So under real async timing two
  nodes can finalize different winners at the same epoch; permanent divergence, and
  MLS can't un-merge. **A wall-clock window is only a best-effort barrier; true
  convergence needs the single-serializer proposal/commit model (6d-2b)**, where the
  designated committer is the sole committer in steady state and forks only occur on
  committer *failover*. Until 6d-2b (and likely a published decision-record / barrier
  for the failover race), the flag must not be enabled in any real deployment. This
  is the gate on turning on concurrent committers.
- **I2; HIGH (fixed):** the loser abort/apply path swallowed errors and discarded
  the contest, wedging the node on a storage failure. Now logs the real error and, if
  resolution doesn't advance the epoch, enqueues commit-catch-up to self-heal.
- **I3; HIGH (fixed):** an epoch advance via `drain_pending_commits` (catch-up) left
  a stale `PendingResolve`, silently overwriting our staged tracking. Added
  `discard_stale_contest` (aborts our staged commit + clears `pending`), called in
  `drain_pending_commits` and `contest_commit`.
- **I4; MEDIUM (fixed):** `run_once` returned after catch-up before checking the
  contest window, stretching it. `resolve_pending_if_expired` now runs *before* the
  catch-up early-return.
- **I6; INFO (fixed):** `commit_id` ties now break on the full `mls_commit` bytes, so
  even an (astronomically unlikely) BLAKE3 collision stays deterministic.
- **I7; doc (this section):** 6d-2a fork resolution covers **Removes and staged
  Adds** (the two-phase Welcome-push join is implemented); both are gated by
  `max_committer_rank>=1` and share the I1 limitation. Joins on the default config
  remain synchronous single-committer.

Confirmed-correct (re-verified, no action): the authorize-by-signature gate, the
`base_authenticator` fork-vs-lag refusal, `commit_id` determinism, the rank bound,
catch-up membership auth, and forward secrecy of aborted commits (never recorded /
served).

## Honest residuals (deferred)

Deep-partition single-use double-claim is **detected, not remediated** (`ForkTooDeep`;
needs 6d-3 reboot). Committer-decoupled admission deferred (Welcome is inviter-signed).
External-commit self-heal deferred (API present, but leaf reindex destabilizes ranking).
Topic rotation on removal deferred. v2 is a hard wire cutover (pre-release; coordinated
upgrade). The `is_operational` freeze is bounded (250ms) but real; a latency floor on
back-to-back membership changes.
