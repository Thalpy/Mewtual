# Design: only merged commits enter the commit log

Branch `mls-commit-log`, stacked on PR #57 (`crypto-bigint-upgrade`, openmls 0.9.2).

## 1. The bug

A `CommitRecord` is `(group_id, commit_epoch, committer_device, mls_commit, base_authenticator,
committer_sig)`. `authorize_committer` checks that the committer is a current member within
`max_committer_rank` and that its signature covers the record. The signature covers whatever bytes
the committer put in `mls_commit`. Nothing checks that those bytes are a commit.

`apply_commit_in_order` (and both fork-resolution apply branches) call
`ServerGroup::process_incoming` and treat any `Ok` as an applied commit: they run
`snapshot_epoch_keys`/`evict_past_keys`, count `commits_applied`, and `record_commit` the record into
the log that `KIND_COMMIT_CATCHUP` serves. `process_incoming` also returns `Ok` for:

| Bytes in `mls_commit` | Today |
|---|---|
| an application message | decrypted, `Ok(Application)`, logged; openmls consumes that sender's ratchet generation here |
| a proposal | `Ok(Other)`, logged |
| our own pending commit, or a message claiming our leaf (0.9) | refused by #57 |
| an external commit | refused (`NewMemberCommit`) |
| a commit for another epoch | refused by openmls, but the record label is never compared with the MLS epoch |

Consequences: an authorized committer (or, at rank >= 1, any committer in the window) can fill the
256-record log with non-commits, pushing real commits out and stranding lagging members. A wrapped
application message is decrypted on the commit path, which consumes its ratchet generation; no
production feature sends MLS application messages today (content travels as exporter-keyed CRDT
ops), so that has no product effect now, but the commit path should not decrypt it. A lagging member
counts the non-commit as `Verified { applied }` progress. A non-commit buffered at a future epoch
occupies that slot (`or_insert`) until an apply attempt frees it.

PR #57 did not change `catcoms-sync`. It made `process_incoming` refuse the two own-leaf variants and
name every content variant; this change builds on that.

## 2. Rule

A record enters the log only if processing it produced a **merged staged commit** that moved the
group from exactly `record.commit_epoch` to `record.commit_epoch + 1`. Anything else is refused with
a typed reason and never recorded. There is no other "proper path" to send it to: the control topic
carries only commit records, proposals have no handler (6d-2 is deferred), and application data
travels on the channel topics.

## 3. Change

### catcoms-mls

`ServerGroup::apply_commit(device, bytes, expected_epoch) -> Result<AppliedCommit, MlsError>`, the
only entry point the commit paths use:

1. **Header checks, before any decryption.** RFC 9420 `PrivateMessage` keeps `group_id`, `epoch` and
   `content_type` in the clear, and openmls exposes them on `ProtocolMessage` (0.8.1 and 0.9.2).
   openmls binds `content_type` into the AEAD's additional data and uses it to pick the ratchet and
   the body decoder, so a header that lies does not decrypt. Refuse unless: the bytes are exactly
   one protocol message (`tls_deserialize_exact`: openmls's own parse ignores trailing bytes, which
   would let `commit || junk` into the log and make the fork tie-break free to grind); the wire
   format is `PrivateMessage`; the content type is `Commit`; the group id is ours; the header
   epoch equals `expected_epoch` and the group's current epoch. So a wrapped application message or
   proposal is never decrypted, and its ratchet generation is untouched.
2. Process through `process_incoming`, so the existing commit checks run unchanged: own-leaf
   refusal (#57), external-sender refusal, credential binding of every Add, M-1. Require
   `CommitApplied`; anything else is refused (unreachable with a `Commit` header).
3. openmls then always advances by exactly one (`diff.rs`), so that is a `debug_assert`, not an
   error: refusing after the merge would leave the group advanced while the caller skipped its log,
   rotation and eviction. No test can break it.

Refusals are `MlsError::NotACommit(CommitRefusal)`, where `CommitRefusal` names the reason
(`NotProtocolMessage`, `NotCiphertext`, `NotCommitContent`, `WrongGroup`, `WrongEpoch {..}`,
`NotStagedCommit`). openmls failures stay `Protocol`.

`catcoms_mls::commit_header(bytes)` parses the header alone (no group state);
`check_commit_header(bytes, group_id, expected_epoch, current_epoch)` is step 1 in full, and both
`apply_commit` and the sync layer's pre-check call it, so the two cannot drift apart.

`process_incoming` keeps its contract for other callers and tests. Its doc says commit-log callers
must use `apply_commit`.

### catcoms-sync

Two predicates, kept apart:

- *apply-time* (`commit_record_refusal`): the header matches the record's label and group **and**
  the label is the current epoch;
- *label-only* (`CommitRecord::header_matches_label`): the header matches the record's own label
  and group, whatever the current epoch.

Where they run:

- **Apply** (`apply_commit_in_order`, the fork-contest loser and pure-applier branches): call
  `apply_commit(.., record.commit_epoch)`. Only `Ok` records, counts, rotates and evicts. In
  `apply_commit_in_order`, the apply-time predicate runs after authorization and before the
  past-key snapshot, so a refused record leaves the epoch, log, key window and buffer as they
  were.
- **Fork contest** (`contest_commit`, only at `max_committer_rank >= 1`): the label-only predicate
  after authorization. A wrapped application message or proposal could otherwise win the
  tie-break, every node would abort its staged commit for it, and nobody would advance.
- **Restore:** the label-only predicate drops header-mismatched entries from a persisted
  `commit_log` (older builds could have logged one). Older builds logged commit-type content only
  after openmls merged it at epoch == label, so this cannot drop a real commit.
- **Not at intake.** `on_control` and catch-up bundles buffer future-labelled records exactly as
  before. The stranded-member detection (`Stranded`, `MembershipChainGap`) reads its evidence from
  those records, and its tests use opaque stub records; refusing them before buffering would
  change what `Stranded` means. A buffered non-commit is refused when its slot comes up.
- **Outcome:** `SyncStats::commit_records_refused` (new) counts authenticated records refused as
  not this epoch's commit, at apply or at the contest, with a warn log naming the reason. It does
  not count openmls refusals of commit-shaped records (own-leaf, M-1, credential binding,
  undecryptable); those keep their existing log line. An older peer re-serving the same record
  counts again each round. `CommitCatchupOutcome::Verified { applied }` now counts only real
  commits.
- Own commits (`commit_remove_now`, the admission paths, a winning merge) are unchanged: those
  records come from this node's own `stage_*`/`add_member` plus merge, so they are commits by
  construction.
- Serving is unchanged: with apply, contest and restore guarded, the log holds only commits this
  node merged or built.

## 4. Tests

catcoms-mls (`group/commit_gate_tests.rs`), each asserting the exact `CommitRefusal` variant (a
coarser assertion would still pass with the header check removed, because openmls refuses most of
these later for other reasons) and an unchanged epoch: application message (and the **same** node
decrypts it afterwards, so its ratchet generation was not used); proposal; external commit;
commit for another epoch, either way round; replay of an already-merged commit, under its own label
and relabelled; our own pending commit (refused by #57's own-leaf rule, stays pending); a Welcome,
garbage bytes, another group's commit, and a real commit with bytes appended; the happy path with
and without a removal; and the frozen openmls 0.8.1 fixtures (#57): a 0.8.1 member snapshot
refuses the 0.8.1 application message on the commit path (and can still read it) and applies the
0.8.1 commit through `apply_commit`.

catcoms-sync (`commit_log_tests.rs`), through the real `on_control`, catch-up, restore and
fork-contest code, with records signed by an authorized committer:

- an application message, **another member's** proposal (Bob's own would already be refused by
  #57's own-leaf rule) and an external commit are refused, not logged, not counted, the past-key
  window is untouched, and the real commit for that epoch then applies;
- the buffered-drain path: an application message from the next epoch, labelled with it, is
  buffered, then refused when the drain reaches it (before this change it was logged there);
- a commit relabelled one epoch ahead is buffered, then refused when its slot comes up; a replay
  relabelled to the current epoch is refused; neither is logged and the chain carries on;
- catch-up from a peer whose log holds a non-commit ahead of the real commit: the requester refuses
  it, applies both real commits, reports `Verified { applied: 2 }`, and its own log stays clean;
- restore drops an application message, garbage bytes and a mislabelled real commit, and keeps
  the real commit;
- fork contest: a wrapped application message ground below the real candidate's `commit_id` cannot
  win; an unappliable but header-valid winner (section 6) is not recorded and queues catch-up.

Break-it checks, run: with the apply guard reverted to "any `Ok` is applied", the live,
relabel and catch-up tests fail; without the restore filter, the restore test fails; without the
contest check, the contest test fails on the epoch (the junk wins and the real removal never
merges). The unappliable-winner test pins residual behaviour and passes either way. openmls
already refused the relabelled and replayed cases (as a `Protocol` error), so those tests pin the
typed refusal and its counter.

The proposal and external-commit fixtures need raw openmls calls. catcoms-sync has no openmls
dependency, so catcoms-mls gains a `test-support` feature, enabled only by catcoms-sync's
dev-dependency. This is the same pattern as catcoms-replication's `test-counters`: with resolver 2,
a dev-dependency's feature never reaches a non-test build.

## 5. Compatibility

- **Wire:** unchanged. `CommitRecord`, the control envelope and `KIND_COMMIT_CATCHUP` are as before,
  and every commit an honest build produces passes the new checks.
- **Mixed versions:** older builds still log non-commits and serve them. This build refuses them
  when it applies, so they cannot reach its log or move its epoch. But they still take room in an
  older peer's 256-record log and in the prefix it serves, and real commits already pushed out of
  an older peer's log stay unrecoverable from that peer even after it upgrades. At
  `max_committer_rank >= 1` only, a fork contest that mixes builds can split: this build refuses
  a non-commit candidate that an older build accepts as the winner.
- **Persistence:** format unchanged. Load filters header-mismatched records. A record that passes
  the header check but would fail to apply cannot be detected without processing it; if one
  survives from alpha data it is served, and requesters on this build refuse it.
- **openmls:** uses only `ProtocolMessage::{wire_format, group_id, epoch, content_type}` and
  `ProcessedMessageContent`, which are the same in 0.8.1 and 0.9.2.

## 6. Out of scope, and residuals

- **Buffer squatting (pre-existing).** `buffer_future_commit` runs before authorization, from
  `on_control` and from catch-up bundles. Anyone who can publish on the control topic can forge a
  record for a future epoch (the group id travels in clear in `CommitRecord`). It takes the slot
  (`or_insert`) and sets the gap. If labelled past the group's real head, no apply ever runs, and
  the commit catch-up task may keep re-queueing and marking honest sources failed. Follow-up: only
  an authorized record should set the gap or displace an unauthenticated squatter. Not changed
  here: it is about the buffer, not the log, and it moves the stranded-detection evidence.
- **Unappliable contest winners (rank >= 1 only).** An in-window committer can still win a fork
  contest with a header-valid commit that does not apply (undecryptable, or failing M-1 or the
  credential binding), grinding `commit_id` over the bytes. Nothing is recorded, but every node
  aborts its staged commit and nobody advances. `max_committer_rank` is 0 by default and
  THREAT-MODEL R3 already says not to raise it.
- **Committer not bound to the MLS sender.** `committer_device` is not compared with the commit's
  MLS sender leaf, so an authorized committer can relay another member's commit. That is a
  real, merged commit, so it is in the log by this design's rule. Binding the two is a follow-up.
  At rank >= 1 it has a sharper edge (pre-existing, from #57's own-leaf rule): an in-window
  committer re-wraps the real candidate's exact bytes under its own device, `commit_id` differs,
  and it wins about half the time. The real committer then aborts its staged commit and refuses
  the winner, which is its own commit, so it is stranded while everyone else advances. Smallest
  fix: in the loser branch, merge the staged commit when the winner's bytes are our own.
- **Restoring an older snapshot (pre-existing).** A committer that restores a snapshot taken before
  its own last commit has that commit refused when catch-up serves it back, as under 0.8.
- The authorized committer can still withhold commits or flood the control topic. That is the
  existing trust placed in the single committer, not a log-integrity question.
- In the contest branches, `snapshot_epoch_keys` still runs before `apply_commit`. With the contest
  check, a non-commit cannot reach them, and the snapshot is idempotent.
- Bug A (expired KeyPackage leaves) is a separate PR: `mls-leaf-lifetime`. Its review found that a
  refused Add at the current epoch is misreported by commit catch-up as `Stranded` or as
  `Verified { applied: 0 }`. This change makes refusals typed at the MLS layer but does not add
  a catch-up outcome for them; that is a follow-up for both.

## 7. Design review (AGENTS.md skeleton, read-only)

No blocker, no high. Findings and what was done:

| Severity | Finding | Outcome |
|---|---|---|
| MEDIUM | Intake refusal would break stranded detection and its tests | Intake dropped; guard at apply, contest and restore (section 3) |
| MEDIUM | Section 6 misstated buffer squatting (unauthenticated, can stall recovery) | Section 6 corrected; follow-up |
| MEDIUM | Fork contest poisonable without a header check; no contest test | Contest check and two contest tests added; the unappliable-winner residual is documented |
| MEDIUM | Tests must pin the exact refusal and the same node's ratchet; the post-merge check is untestable | Tests assert exact variants and the same node; post-merge check is a `debug_assert` |
| LOW | One predicate would refuse future commits at intake | Two named predicates |
| LOW | `tls_deserialize` ignores trailing bytes | `tls_deserialize_exact`, with a test |
| LOW | Intake decodes unauthenticated bodies | No intake; apply and contest run after authorization |
| LOW | `Internal` after a merge strands bookkeeping | `debug_assert` |
| LOW | Counter semantics; `NotStagedCommit` doc | Documented (section 3); doc fixed |
| LOW | Mixed-version cost understated | Section 5 corrected |

## 8. Review of the implemented diff (AGENTS.md skeleton, read-only)

No blocker, no high. Confirmed: the header cannot lie (AEAD-bound), honest commits from every
build pass (PURE_CIPHERTEXT from the start, records labelled with the pre-commit epoch), the
restore filter cannot drop a real commit, the `debug_assert` holds (self-removal also advances),
`test-support` cannot reach a production build, nothing matches exhaustively on `MlsError` or
builds `SyncStats` literally.

| Severity | Finding | Outcome |
|---|---|---|
| MEDIUM | catcoms-app rank-1 tests, sync integration tests and `cargo deny` not run; HANDOVER claimed checks | Run: catcoms-sync in full (925 unit plus integration), catcoms-app filtered to `succession`, `repair`, `commit`, `join` and `fork` (155 passed, 0 failed; the rest of catcoms-app is left to CI), `cargo deny check` ok. HANDOVER claims nothing beyond the PR |
| LOW | ARCHITECTURE §4b/§4c still said `process_incoming` and understated forged future records | Updated |
| LOW | THREAT-MODEL overstated ("everything" goes through `apply_commit`; key window) | Reworded: received records; key window on the in-order path |
| LOW | HANDOVER said the leaf-lifetime fix was done | Reworded: in review |
| LOW | Mixed-version contest split at rank >= 1 | Section 5 |
| LOW | Re-wrapped commit can strand the real committer at rank >= 1 (pre-existing) | Section 6, follow-up with the smallest fix |
| LOW | Sync proposal case used Bob's own proposal | Now Carol's, wrapped by Alice |
| LOW | Drain path not pinned for a record only the new guard refuses | New drain test |
| LOW | Pre-check-before-snapshot not tested | Past-key window asserted unchanged, and shown to grow on a real commit |
| LOW | No older-build fixture | 0.8.1 fixture test through `apply_commit` |
| LOW | Pure-applier contest with junk; unauthorized records not counted | Not added: the contest check is one line on a path the contest test already drives; unauthorized records are rejected before the counter by design (`authorize_committer` logs them) |
| LOW | Header checks duplicated in two crates | One `check_commit_header` used by both |
| LOW | `catcomsctl` stats line lacks the counter | Added |
