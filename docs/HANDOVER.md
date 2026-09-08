# Mewtual; Handover

Authoritative current-state document. Read this first, then
[`INTERFACES.md`](INTERFACES.md) (the API/seam schema) and
[`ARCHITECTURE.md`](ARCHITECTURE.md) (decisions + the adversarial-review fixes).
[`THREAT-MODEL.md`](THREAT-MODEL.md) tracks what a modified ("hacked") client can/can't do;
the protocol- vs honest-client-enforced boundary and the hardening backlog.

## Status (as of 2026-08-22)

- **Flipnote gate 1, epoch-zero art/frame causal validator (2026-09-08; verified).**
  `studio::validate_frame_change` checks insert/remove/replace frame and title/fps mutations.
  Exact record bytes bind the full actor, canonical domain envelope and independently derived
  left/right origins; timestamps remain bounded author assertions. Origins use the sender's
  complete dependency-frontier projection and the first direct child, including hidden/deleted
  insertion nodes. A receiver-only smaller collision winner or newly inserted sibling cannot
  change a valid old placement. Existing observed IDs cannot be reused; remove/replace and an
  explicit predecessor require nondeleted causal targets. Immutable headers/evidence, fresh
  markers and all-and-only same-property mutable predecessors follow the Index contract.

  One private historical path reuses the bounded frame materializer with keys/all values/winner
  read at the same heads. The public current reader retains its zero-operation/zero-change
  pristine check; private empty heads mean empty causal past, and unknown/nonempty erased heads
  reject. There is no randomized/replayed fork, but Automerge still repeats historical clock
  work. Bounds are not latency qualification; accepted-size profiling remains necessary before
  production scheduling. This slice adds no performance/finality promise.

  All 25 frame tests pass (11 new causal tests), including hidden-child descendants,
  old/proper-subset frontiers, collision origins, metadata/record/identity tampering, predecessor
  omissions/duplicates/cross-key hiding, signed ordering/concurrent deletion, restart and full
  doc/log/gate rollback on rejection. An independent-root retry hits Automerge's duplicate actor
  sequence rejection; a distinct fresh causal retry proves the marker conflict path. Neither is
  confused with the accepted exact sealed-envelope retry. Actual-diff adversarial review found
  no blocker/high/medium and one Low regression gap: a losing birth already known to the author
  was not separately tested as the first direct child. The added test pins both delivery orders,
  rejects omitted/descendant origins, and closes the Low on re-review. No findings remain.

  Verification passed after that test was added: `cargo test --all --all-features` (including
  all 141 replication unit tests; existing ignored probes unchanged),
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192),
  `npm.cmd --prefix apps/desktop test` (1144), `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `bash scripts/check-no-ambient.sh` via Git Bash, and `git diff --check`.
  Focused callback tests and `cargo check -p catcoms-replication` also pass.

  This remains a pure epoch-zero semantic callback with a TEST-ONLY writer/reader-preflight
  harness. P1 must authenticate the signed actor/member/server/physical scope; prior history
  must already be trusted. Aggregate edit/cap policy, exact typed checkpoint/recovery preflight,
  durable intent/blob/reference plumbing and production Studio commands are still required.
  Over-cap frames remain semantic targets so trimming is possible, not so production editing
  can ignore the cap policy. Sound/score/export operations and checkpoint epochs refuse.

  Next is typed checkpoint/recovery representation and exact aggregate preflight for the Index
  and art path, including extending the validators against real verified seeds. Gate 1 and all
  seven product gates remain open; no production Save/Load, UI or game/avatar work is included.
  User release workflow/releasing-document edits remain excluded. Local commit only; remote
  push still awaits destination approval.

- **Flipnote gate 1, epoch-zero Index causal validator (2026-09-08; verified).**
  `studio::validate_index_change` now checks the exact root mutation for an Index domain op:
  full change-actor record binding, fresh marker, immutable headers/insertion/deletion evidence,
  and all-and-only same-property predecessors for a mutable register. It checks targets at the
  author's dependency frontier, including overflow and deletion evidence. Receiver-only targets
  and predecessors cannot authorize an edit; a genuinely concurrent deletion does not reject it.
  Same-id concurrent creations remain legal; observed-id reuse and resurrection are refused.

  Ten focused tests pass, including two admitted signed authors in both delivery orders,
  empty/partial/duplicate/foreign-property predecessors, header/marker/record tampering, causal
  unknown targets, overflow mutation, exact sealed retry, restart and rollback of the entire
  document/log/gate on semantic or preflight failure. The retry regression deliberately forges a
  fresh causal change: identical Automerge puts alone create no change and return the original
  delta, which transport dedup is meant to accept. Actual-diff review found no blocker/high/medium
  and one Low test gap: the duplicate-predecessor case also omitted a required predecessor. It
  now includes the complete expected set plus a duplicate, asserts serialization retains it, and
  independently proves duplicate rejection. The focused all-features rerun passes and re-review
  has no remaining findings.

  Verification passed:

  - `cargo test -p catcoms-replication studio::index::change::` (10 passed)
  - `cargo test -p catcoms-replication --all-features studio::index::change::`
    (10 passed after the review regression was strengthened)
  - `cargo test --all --all-features` (app 380 passed / 8 existing ignored; replication 130;
    sync 214; all workspace integration/doc suites passed; existing ignored tests unchanged)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1144 passed)
  - `cargo check -p catcoms-replication`
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check` and staged diff checks

  Production code was unchanged while the full suites ran; the review strengthened a test only,
  verified by the focused rerun above. No frontend/native source changed, so frontend static/build,
  native `cargo check` and screenshots were not required. This slice is locally committed only;
  destination approval for pushing remains outstanding. The user's release changes are excluded.

  This is a pure semantic callback, not production admission. Tests use a TEST-ONLY writer and
  reader-only preflight to isolate the boundary; no checkpoint-size guarantee follows. P1 must
  independently authenticate the signed actor/member/server/physical scope and run exact typed
  checkpoint/recovery preflight. `before` must already be accepted authenticated history, not a
  peer snapshot. Epochs above zero refuse pending the real seed representation. Exact local retry
  handling must resolve the retained signed envelope, not treat marker-only NoChange as equality.

  Gate 1 remains open: frame causal validation and typed checkpoint/recovery encoding/preflight
  are next. All seven product gates remain open; the tested helper count is not a percentage.
  No UI, game/avatar, native command, live Studio write or P1 finality guarantee changed.

- **Flipnote gate 1, read-only art/frame projection (2026-09-08; verified).**
  `studio::FlipnoteFrameProjection` adds deterministic frame order, Automerge pixel/title/fps
  register winners with all live alternatives, provenance-bearing deletions and explicit
  999-frame / cumulative 8 MiB cap flags. All insertion records remain ordering anchors, even
  when their frame is deleted or another insertion of the same id wins. The full frame map keeps
  hidden/over-cap CID evidence for later recovery/reference consumers; this is not retention
  integration. Root object/channel/epoch/dimensions are checked against every concurrent value.
  Server provenance remains caller-supplied until the signed admission boundary is implemented.

  Design review refined the sequence contract in creative section 2.9 and P1 section 4, without
  changing P1 finality: recorded left/right origins preserve immediate sequential placement;
  op ids break ties in the same gap, not arbitrary concurrent pairs that observed different gaps.
  A right-origin forest avoids the confirmed generic-Kahn ordering defect (X,Y plus later C
  before X must give C,X,Y, not Y,C,X). The walk is iterative and rejects missing/wrong-parent/
  cyclic origins. `after:null` means prepend, matching the current fixture's code despite its
  stale append comment; missing-predecessor Restore chooses the current last live frame before
  preparing a new intent. No UI code changed. Origin metadata still needs causal-delta validation.

  Fourteen new frame tests pass via focused runs: real 1000-frame and exact-byte boundaries,
  concurrent same/different gaps in both delivery orders, collision/deletion anchors, replacement
  conflicts, erased root history, strict unsupported-state rejection, metadata framing/scope,
  valid-in-isolation anchor equivocation, deep 5000-node chains, restart and redacted diagnostics.
  Actual-diff review's two Low findings are fixed: server-scope wording and a stronger anchor
  equivocation regression. The optional same-parent/different-right-origin test was added too.
  Read-only re-review reports no remaining findings. Final verification passed:

  - Focused `studio::frames::` tests and the subsequent framing/concurrency/equivocation regressions
  - `cargo test --all --all-features` (app 380 passed / 8 existing ignored; replication 120;
    sync 214; all workspace integration/doc suites passed; other existing ignored tests unchanged)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1144 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check` and staged diff checks

  No runtime or test bytes changed after that verification. No frontend/native source changed,
  so frontend static/build, native `cargo check` and visual screenshots were not required.
  This slice is locally committed only; destination approval for pushing remains outstanding.
  The user's release workflow/documentation changes are preserved and excluded.

  This is an art-only reader: unsupported sound/score/export records reject, including score:null.
  It supplies no signed delta validator, exact checkpoint/recovery encoding, actor/native command,
  production Save/Load or playback/export capability. Timestamps are author assertions, never
  freshness/ordering authority. Gate 1 remains open; next is signed causal admission and exact
  checkpoint/recovery preflight for the art path. Other operation families remain unavailable
  until their stateful support lands. Games/avatar work stays paused; UI stays user-owned.

- **Flipnote gate 1, read-only StudioIndex projection (2026-09-08; verified).**
  `studio::StudioIndexProjection` reads the channel's object list from actual Automerge state.
  It keeps immutable insertion candidates by derived operation id, chooses the smallest for
  a same-object collision, uses Automerge's actual mutable title/expiry winner, and retains all
  live alternatives with full asserted author/nonce provenance. Deletions retain provenance too
  and win over every insertion of their id. The first 64 live object ids are visible; overflow
  and deleted objects remain explicit evidence rather than disappearing at the display cap.
  Every concurrent header/record is checked, including hidden values. Reader limits bound
  primitives and visible key/value bytes, separately from future signed-log/seed preflight.

  Twenty focused Studio tests pass (11 new index tests). Actual-diff adversarial review found
  one Medium: deleting all root keys made historical epoch-zero state look pristine. The reader
  now also requires zero primitive operations and zero changes for the empty-root exception;
  committed delete-all regressions cover headers alone and content plus markers. Re-review has
  no remaining findings. The earlier boolean-tombstone design was changed before completion to
  retain operation-id/full-author evidence needed by later recovery encoding.

  Capacity tests use real CRDTs and a private lower-limit seam for exact/one-below byte and
  primitive accounting, with the production 6 MiB inclusive/overflow arithmetic pinned
  separately. The initial multi-MiB fixture run was stopped because construction was slow;
  public reader limits are unchanged. These tests do not claim a maximal Studio checkpoint or
  gated edit has been accepted.

  Final verification passed:

  - `cargo test -p catcoms-replication studio::` (20 passed)
  - `cargo test --all --all-features` (app 380 passed / 8 existing ignored; replication 106;
    sync 214; all workspace integration/doc suites passed; other existing ignored tests unchanged)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1144 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check` and staged diff checks

  No runtime or test bytes changed after that verification. No frontend/native source changed,
  so frontend static/build, native `cargo check` and visual screenshots were not required.
  This slice is locally committed only; destination approval for pushing remains outstanding.

  This remains a read-only core slice. No authenticated Studio delta validator, Flipnote frame
  projection, checkpoint/recovery encoder, actor command, native binding or UI is added.
  Assertions in index records are not authenticated by materialization; future signed-delta
  validation must verify authorship, causal target existence, immutable writes and predecessors.
  Gate 1 and all seven delivery gates remain open. Next is the Flipnote frame projection, then
  causal admission and exact checkpoint/recovery preflight; production Save/Load is still gate 2.
  Games/avatar and UI implementation remain untouched. The release workflow/documentation
  changes in the worktree belong to the user and are excluded from this slice.

- **Flipnote gate 1, operation-schema substep (2026-09-08; verified).**
  `catcoms_replication::studio` adds closed IndexOp/FlipnoteOp codecs, complete DomainOp size
  and target checks, full verified-creator binding, safe integer/identifier/header bounds,
  three-state expiry and immutable validated jam patch recipes. Twenty-four shared byte vectors
  exercise the actual TypeScript canonical serializer and jam hash alongside Rust roundtrips.
  The design review caught the fixture's numeric expiry mismatch; the Rust contract preserves
  absent/null/timestamp, with no sentinel-zero reinterpretation. That fixture/view adapter is
  explicit gate-2 work, not an implied UI change in this slice.

  Nine focused Rust tests and four frontend compatibility tests pass. Actual-diff adversarial
  review and re-review have no remaining findings: its two Low coverage gaps were fixed with
  syntactically valid deep JSON and 18 shared patch range vectors exercised by both languages.
  No Studio materializer, causal change validator, exact checkpoint preflight, persistence
  adapter or production write path is added;
  gate 1 remains open. Next is that stateful gate-1 materializer/admission work, not another P1
  platform refactor. Games/avatar and UI implementation remain untouched.

  Final verification passed (including the added review regressions):

  - `cargo test -p catcoms-replication studio::` (9 passed)
  - `node --experimental-strip-types --test apps/desktop/src/studio-wire.test.ts` (4 passed)
  - `cargo test --all --all-features` (app 380 passed / 8 existing ignored; replication 95;
    sync 214; workspace integration and doc suites passed)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1144 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `npm.cmd --prefix apps/desktop run check` (0 errors/warnings)
  - `npm.cmd --prefix apps/desktop run build` (passed; large-bundle advisory remains)
  - `git diff --check` and staged diff checks

  No implementation/test bytes changed after that final verification. No native or UI source
  changed, so separate native `cargo check` and visual screenshots were not required. This slice
  is locally committed only; destination approval for pushing remains outstanding. The user's
  release workflow and release documentation edits were preserved and excluded from the commit.

- **Flipnote scope reset (2026-09-08).** The user paused games and asked to focus on Flipnote.
  Game-only avatar consent/profile changes are paused; the wider Creative Suite backlog is not
  the current completion target. `BACKEND-IMPLEMENTATION.md` now defines seven delivery gates:
  typed documents, durable one-device Save/Load, sharing/joining, rotation/recovery, advisory
  claims, sound/export and production acceptance/UI contract handoff. The older 25%/65%
  estimates in historical entries below are retired, not reaffirmed current estimates.
  Next is the missing Rust StudioIndex/Flipnote domain operations and preflight, leading to
  a real create/save/restart/reopen test; required P1 storage/lifecycle work is included in that
  path rather than bypassed. Automatic shared-runtime work must still solve the measured dense
  registry request cost and snapshot/lifecycle fences. No new P1 guarantees or unrelated
  document consumers are added. Canonical UI remains user-owned.

  This update changes planning documentation only. Read-only actual-diff adversarial review
  found no findings; `git diff --check` passed. Runtime suites were not rerun; the previous
  code slice's verification is recorded below. `db979dd` is locally committed, not pushed;
  destination approval remains outstanding. Unrelated release edits remain untouched.

- **P1 indexed registry restoration (2026-09-08).** Restore uses Automerge's applied graph
  metadata for Boolean dependency/duplicate checks instead of rebuilding raw predecessor changes.
  A change whose dependencies exactly equal all current heads can use indexed current-view
  property reads; only that path skips the semantic validator's already-proven dependency check.
  The restore loop derives the fact locally after authentication and dependency admission.
  Concurrent/older/proper-subset views remain historical; live edit/ingest authorization and
  semantics are unchanged. No
  signatures, typed semantics, predecessor/marker checks, seed/gate verification or projection
  preflight are removed, and no format, limit, request deadline or UI changes.

  Five focused regressions pass: differential seeded/unrotated branch delivery, current-head
  proper subsets, marker-only edits, cross-property/seed-slot attacks, queued versus applied
  lookup equivalence, and missing/re-enveloped duplicate changes rejecting before semantics.
  Read-only actual-diff design/re-review found no remaining findings; its initial Low missing
  proper-subset regression was added. All four release probes pass on the final production code.

  The performance result is deliberately limited: byte-heavy pages measured 33–37 ms versus
  42–48 ms at baseline, while 8,002 small operations still take about **11 seconds per page**
  versus about 13 at baseline. The dense request-deadline problem remains unresolved; neither
  the feature nor automatic scheduling is ready. `P1-PERFORMANCE.md` retains both tables and
  the intermediate run's variability. Next is bounded off-executor reconstruction/source reuse
  with exact version/authority checks, then sole vault ownership and whole-server snapshot
  ordering. Overall estimates remain about 25% backend / 65% P1, each ±10 percentage points.

  Required verification passed on the final code and regression source:

  - `cargo test --all --all-features` (app 380 passed / 8 ignored: 4 existing + 4 opt-in probes;
    replication 86; sync 214; every workspace unit, integration and doc suite passed)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1140 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check`

  The final documentation review's Low ambiguous "unchanged edit/ingest" wording is corrected
  to unchanged authorization/semantics. No runtime or test code changed after verification;
  only this result record followed. Frontend static/build/visual checks and separate native
  `cargo check` were not required because neither frontend nor bridge source changed.
  User release edits and canonical UI remain untouched. This is not backend completion.

- **P1 saved-source performance evidence (2026-09-08).** Added an opt-in release harness using
  actual signed/typed admission, accounted vault encoding, restore, provider and
  `Server::serve_registry_page` paths. Two always-run smoke tests fully drain unrotated and real
  receipted successor histories; four explicit ignored profiling cases exercise near-4-MiB
  byte-heavy and small-op logs, 65 current-member roots, and a seeded full tail. Exact hashes,
  dependencies, page caps, real capacity rejection and unchanged vault bytes are asserted. No
  production behavior, UI, wire format or limit changes. Setup's batch save is test-only.

  `P1-PERFORMANCE.md` records the commands, phase boundaries, raw results and honest memory/cache
  limitations. The important result is 8,002 small operations / 4,194,072 signed bytes:
  **12.8 seconds to restore and about 13 seconds per full saved-source page**, versus 29 ms for
  a page from an already restored source. The actual valid source exceeds provider/client request
  deadlines. Byte-heavy and seeded full tails took 42–48 ms per page; 65 roots took 16 ms. The
  immediate next step is investigate redundant causal-history work in restoration, then bounded
  off-executor work/source reuse with exact version/authority checks. Do not enable automatic
  service or claim a safe runtime work budget from the existing byte/rate caps. Shared vault
  ownership and whole-server snapshot ordering remain required afterward.

  All four release profiling cases passed. Read-only actual-worktree review found no
  blocker/high/medium or harness defect; its Low stale publication-state sentence in the threat
  model is corrected. Required verification passed on the final test source:

  - `cargo test --all --all-features` (app 380 passed / 8 ignored: 4 existing + 4 opt-in probes;
    replication 81, sync 214; all workspace unit, integration and doc suites passed)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1140 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check`

  No code changed after these checks; only results/status documentation followed. Frontend
  static/build/visual checks and separate native `cargo check` were not needed because neither
  source changed. User release edits and canonical UI remain untouched. Overall estimates stay
  about 25% backend / 65% P1, each ±10 percentage points; no integration milestone is closed here.

- **P1 checked receipt publication completion (2026-09-08).**
  Kind-21 owner-proof serving now connects an accepted local reply-channel handoff to the exact
  accounted owner-journal completion, under the same synchronous Server/store/mount/server gate.
  `Responder::try_respond` is additive; success is channel acceptance, not driver admission or
  peer delivery. Hints, dropped receivers, expired requests and stale owner evidence cannot
  mint completion authority. Sync's private non-Clone handoff rechecks runtime, MLS/full owner,
  observed tenure, logical bucket, watch generation and request expiry before use. Existing
  fire-and-forget reply callers retain their behavior; the legacy sync wrapper drops the token.

  A completion write can fail after a peer already received the proof. The owner reports error,
  invalidates uncertain accounting, and re-hands off exactly the same receipt after restart/rescan.
  No response is retracted and no delivery guarantee is inferred. The actual joined fixture now
  generates, installs, serves/completes, edits the seeded successor and rotates a second time with
  unchanged tenure inheritance. Recovery-first retirement remains a separate mandatory barrier.

  Focused checks pass: checked responder regression, twelve sync receipt-head tests, five app/store
  head tests including before-write/after-rename completion failure and restart, and the two-rotation
  joined fixture. Read-only actual-diff adversarial review found no remaining actionable findings.
  Required verification passed:

  - `cargo test --all --all-features` (app 378 passed / 4 existing ignored; replication 81;
    sync 214; runtime 20; all workspace unit, integration and doc suites passed)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1140 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings` (after boxing the receipt inside
    the private handoff to keep its enum small)
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check`

  No runtime code changed after these checks. Frontend static/build/visual checks and separate
  native `cargo check` were not needed: neither frontend nor native bridge source changed.
  Completion currently requires serving an eligible kind-21 query; quiet/solo owner progress
  still needs orchestration. Automatic scheduling, durable repair, all managed document types
  and UI integration remain open; no overall
  completion percentage increase is claimed. The previous owner-rotation slice is committed/pushed
  as `fd2943f`; unrelated release files and the canonical UI remain untouched.

  Next lifecycle prerequisite: native `persist_captured` captures an actor snapshot before its
  later store write. Automatic P1 owner preparation must share that numeric-server persistence
  ordering (or an equivalent exact-incarnation write fence), so an older captured snapshot cannot
  overwrite the newer durable MLS/tenure state after a receipt is issued. Also measure the maximal
  registry source rebuild before assigning an aggregate work budget; current fixed memory/rate
  caps do not establish actor latency. Neither prerequisite is enabled or solved by this slice.

- **P1 explicit owner registry rotation (2026-09-08).**
  `Server::rotate_registry_owner_step` requires the current mount/server-bound durable owner
  snapshot permit. Under exclusive sync/store borrows it flushes the checked source, verifies
  both inventories, derives a new eligible close/seed/receipt or resumes the exact saved choice,
  journals close and receipt together, seals, and runs the existing recovery-first adjacent
  installer. The private core builder rejects wrong owners, Fault/adoption, Closing new issuance,
  terminal epochs, malformed public receipt fields and over-64-head sources without truncation.
  Inheritance comes from the installed opening at succession and repeats the journal baseline.

  The owner vault wrapper gains an explicit optional v2 extension containing the selected receipt
  hash and one bounded close; no-extension records and inner journal v1 are unchanged. The 8,488-
  byte physical cap stays accounted as protocol/reserve space; old readers reject new extensions.
  Pending receipts without a close hold as `DecisionNeedsClose`, never regenerate against newer
  heads. Re-saving/completing a matching receipt retains its close. A crash after journal save but
  before sealing resumes identical bytes and sends later Open edits into recovery. Installed retry
  preserves subsequent edits. Publication is explicitly pending: the current query path does not
  mark completion, and a different decision waits for a real publication-completion driver.

  Focused tests pass: five core owner-decision regressions (real 65-head cap and A-to-B-to-A
  ownership included); three store rotation tests (post-barrier crashes, uncertain combined write,
  legacy hold); one actual joined owner-generation/head-proof/seed-fetch/newcomer-install test;
  sixteen owner store/inventory tests including the new extension codec; snapshot-permit callback
  revocation coverage. Read-only actual-diff review found no blocker/high/medium or production
  defect. Its Low coverage finding is fixed: an actual accounted prepare/completion/reload test
  pins preservation of a newer pending close on old completion and removal of stale close
  provenance on a different generic prepare. Static re-review has no remaining findings.
  Required verification passed after that regression:

  - `cargo test --all --all-features` (app 377 passed / 4 existing ignored; replication 81;
    sync 212; all workspace unit, integration and doc suites passed)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1140 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check`

  No runtime code changed after these checks. Frontend static/build/visual checks and separate
  native `cargo check` were not needed: neither frontend nor native bridge source changed.
  Canonical UI, native bridge, and unrelated release files are untouched. No overall completion
  increase is claimed for this explicit integration step; automatic scheduling, publication
  completion, durable repair and all-family Studio acceptance remain open.

- **P1 scoped recovery-first registry installation (2026-09-08).**
  `Server::install_registry_seed_step` connects the private kind-21/22 selection to accounted
  vault adoption. Runtime, MLS, full local member, current owner/tenure, superseding discovery,
  receiver-clock expiry and physical mount/server are checked before synchronous persistence
  under exclusive sync/store borrows. The optional-seed callback lets the full source and receipt
  cross their own save barrier before seed availability or recovery parsing; saved Fault cannot
  be suppressed by a withholding seed provider. Invalid inventory still fails closed.

  Whole-source typed recovery is durable before atomic successor replacement. Pending eviction
  warnings preserve their ids/deadlines across retarget and restart; acknowledgment or seven-day
  advancement is explicit, and an expired fetch handle requires fresh discovery. Exact installed
  retries flush the actual successor without reseeding newer edits or rewriting recovery. Adoption
  never retires author intents. Uncertain writes require inventory reconciliation before retry.
  Old watches and actual queued old-epoch pages cannot save into the successor; a new watch/pass
  catches up the installed epoch through the normal kind-20 route.

  Focused verification: six store tests (including seven write/flush failure points, post-rename
  restart plus a new edit, warning retarget/restart/ack/timeout, invalid inventory, missing seed
  and corrupt recovery); four app seed tests (including actual joined discovery/fetch/install/
  catch-up); thirteen sync seed tests. Read-only actual-diff review found no blocker/high or
  production defect. Its Low finding was a vacuous queued-page test; the fixture now retains a
  real PageReady epoch-zero response and checks rejection, unchanged successor/recovery and no
  saved-page advancement after installation. Static re-review is clear with no remaining findings.
  Required verification passed:

  - `cargo test --all --all-features` (app 371 passed / 4 existing ignored; replication 76;
    sync 212; all workspace unit, integration and doc suites passed)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1140 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings` (after removing a test-only
    unnecessary clone of a Copy value)
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check`

  No runtime code changed after these checks. Frontend static/build/visual checks and separate
  native `cargo check` were not needed: neither frontend nor native bridge source changed.

  **Remaining:** automatic actor/native lifecycle ownership and aggregate scheduling, all-family
  Studio materializers/discovery/events, durable repair and complete full-quota settlement handling.
  This is a cooperative backend transaction, not a lease or a complete product. Overall backend
  estimate is about 25% versus about 65% for P1 (each ±10 percentage points), UI excluded. The
  acceptance checklist remains open; canonical UI and unrelated release files are untouched.

- **P1 distant-checkpoint adoption core (2026-09-08).** Registry epochs can now freeze a whole
  source against a freshly selected distant checkpoint, including a new-owner rewind. No accepted
  source operation is dropped. Explicit outer restart v2 embeds adoption-only receipt-book v3;
  ordinary books and epochs keep their existing formats and strict adjacent settlement rules.
  Restart binds the original seed, log and gate metadata, independently checks same-tenure
  non-regression against the actual opening, and retains opening/prior-target equivocation below
  the high-water. A successful typed Fault outcome must be saved by the future store adapter
  before reporting installation failure, even if seed/recovery work fails.

  `RegistryAdoptionPlan` verifies the exact raw seed and builds bounded Rewound recovery for
  the whole prior version, including seed-only pointers and terminal epoch 4096. Its identity
  excludes destination receipts, quarantine and quota-owner changes, so retargets reuse the same
  staged warning/deadline. The stricter source fingerprint still invalidates stale plans.
  `adopted_successor` constructs one separate seed-backed epoch, preserves receipt/repair state,
  and retires no intents. Exact opening retries preserve newer edits. Ordinary settlement cannot
  consume adoption state even when the selected closed epoch equals the source epoch.

  Nine focused regressions pass. Read-only design/diff review identified and resolved one Medium
  restart splice: an R0/R1 book must not select below a source opened by R10 in the same tenure.
  The regression failed before the fix and passes afterward. Both Low findings are fixed:
  post-seal quarantine/retarget/restart preserves recovery-warning identity, and stale interface
  wording is corrected. Static re-review has no remaining code findings or blocker/high.
  Required verification passed:

  - `cargo test --all --all-features` (app 363 passed / 4 existing ignored; replication 76;
    all workspace unit, integration and doc suites passed)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1140 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check`

  No code changed after these suite runs. Frontend check/build/visual checks and separate native
  `cargo check` were not needed: neither frontend nor native bridge source changed.

  **Next:** connect the scoped kind-21/22 pass to the accounted recovery-first vault transaction,
  then normal open-epoch catch-up. This core is not a durable installation or network permit.
  The installer must recheck mount/server/current authority/high-water/inventory, persist Fault
  independently of recovery failure, and retain Closing across failed writes or warnings. A
  seven-day warning can outlive the 60-second fetch handle; resumption requires fresh discovery.
  No actor/native scheduling, UI, intent retirement on seed matches, or final UI guide is added.
  The backend acceptance checklist remains open; no completion-percentage increase is claimed
  for this core prerequisite. Unrelated release files remain untouched.

- **P1 expected-hash registry seed fetch (2026-09-08).** Additive kind 22 now fetches the exact
  owner-selected Automerge seed from any independently proven current-member endpoint. The
  cooperative vault provider serves only the installed opening seed, not the latest receipt's
  potentially unavailable successor. Closing can still serve its installed opening; Fault
  refuses. Reads verify the complete registry inventory, including absence, and never create,
  rewrite, prune or publish a source. The actual joined-Server test covers unavailable-before-
  installation, then fetching the durably installed checkpoint through both network routes.

  Private discovery provenance is minted only inside the fresh kind-21 response/proof check.
  Runtime, MLS, full requester/owner, tenure and bucket supersession accompany the receipt.
  Public mutable answer fields cannot mint a fetch pass. New authenticated owner discovery for
  a bucket (even the same hash), any MLS transition or runtime replacement revokes older passes.
  App passes also capture mount/server, rechecked by `registry_seed_ready`; discovery still holds
  a shared store borrow across await, while seed fetch itself borrows no vault. There are no
  actor/native entry points or automatic lifecycle workers for this path yet.

  Four non-Clone retained passes are charged before discovery and each holds at most one 2-MiB
  verified seed, three one-second-paced attempts and a fixed 60-second lifetime. Revocation or
  expiry does not refund retained memory until Drop. Four independent outbound permits stay
  charged until driver termination after cancellation. Kind 22 binds full identities, current
  MLS, actual transports and complete query/response; its ten-second deadline is checked even
  after a ready response or expensive validation. Raw seeds use the existing 512-byte to 1-MiB
  padding ladder inside group AEAD. Above 1 MiB, encoded size remains visible. The 2,097,312-byte
  response cap includes all framing and is enforced after transport buffering, before copies
  and decryption. Exact hash/checksum, raw change shape, then typed registry validation must pass.

  Eight five-second metadata-only provider requests, one per full identity, use independent
  preauth/requester/source rails. Provider responder handoff is not delivery, and the client
  permits do not account for provider-driver response buffers. A fetched seed is not installed
  state, a lease, receipt advancement or permission to discard provisional edits.

  Read-only design review identified the opening-vs-latest distinction, discovery provenance,
  complete framing overhead and existing 512-byte padding floor; implementation preserves them.
  Actual-diff review found no blocker/high or production defect. Both low findings are fixed:
  the matching-receipt-hash/invalid-registry-schema regression and old threat-model wording.
  Static re-review has no remaining findings. Focused coverage passes: 12 sync, one replication
  lifecycle test, and two app tests (real joined vault/network path plus restart/lost-source).

  Required verification passed; the final root run includes the added review regression:

  - `cargo test --all --all-features` (app 363 passed / 4 existing ignored; replication 67;
    workspace unit, integration and doc suites passed)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1140 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check`

  No runtime code changed after the native/frontend suite runs. No frontend static/build/visual
  checks or separate native `cargo check` were needed for this core/sync/app-only slice.
  UI/native source and unrelated release work are untouched.

  **Next:** recovery-first newcomer installation from the scoped fetched seed, preserving
  provisional epoch-zero edits. `with_registry_seed` is only a trusted-local synchronous borrow:
  copied receipt/checkpoint values must not become an unchecked deferred install permit. The
  installer still needs exact mount, local high-water/fault/inventory checks and recovery-before-
  replacement. Automatic actor/lock scheduling, maximum-source latency, provider response-buffer
  acceptance, durable signed repairs and the wider Creative backend checklist remain incomplete.
  No final UI guide or 100% claim is issued for this transport prerequisite.

- **P1 keyed registry receipt-head discovery (2026-09-08).** Kind 21 now answers by logical
  bucket key, without requiring the requester's knowledge of the provider's concrete epoch.
  The cooperative Server adapter returns provisional hints or a nonce-bound current-owner
  selection proof; it neither installs a seed nor claims that seed is available/verified.
  Full requester/provider identities, actual transport, current MLS and the complete query are
  bound. Request nonces are minted internally; a non-owner relay or replayed inner proof cannot
  become fresh owner authority. Replies remain raw records, not durable admission permits.

  Explicit LOCAL preparation saves the whole-server MLS/tenure snapshot and mints an opaque
  runtime/MLS/owner/mount/server-bound permit. Remote queries never initiate that uncapped legacy
  serialization. Serving checks source and owner-journal inventories, rejects faults/corruption/
  lost indexed files, and prefers pending owner decisions over published ones. Fresh proofs
  require exact source-head/journal equality plus a current snapshot permit. Disagreement or
  stale preparation produces hints, not an older fallback proof. Source/parent flush and journal
  re-save precede signing. No source edit, intent retirement, pruning or mark-published occurs.

  At most eight five-second requests are queued, one per full identity, with separate preauth,
  requester and source service rails. Four outbound permits remain charged until actual transport
  termination after cancellation. Synchronous source work rechecks the deadline before replying;
  the client also rejects a ready response processed at/after its ten-second deadline. Logical
  watches survive rotation but are revoked on exact watch/runtime/mount replacement.

  Design review caught the remote-triggered whole-server snapshot cost and the need for exact
  source/journal/inventory agreement; both are addressed. Actual-diff review found no production
  defect. Its low-priority rejection-coverage gaps are fixed, and re-review is clear. Ten sync
  tests cover canonical bounds, independent outer bindings, fresh nonces/inner replay, non-owner
  relays, actual member removal while queued, rates, deadlines and driver-owned cancellation.
  Four app tests cover joined-member network discovery, restart, journal lag/faults, a lost indexed
  source and uncertain flush/reconciliation. The existing owner-journal crash tests cover its
  reused durable rewrite primitive. UI/native source and unrelated release work remain untouched.

  Required verification passed (the final workspace run includes the review's added regressions):

  - `cargo test -p catcoms-sync receipt_head_ --lib` (10 passed)
  - `cargo test -p catcoms-app registry_head_ --lib` (4 passed)
  - `cargo test --all --all-features` (app 361 passed / 4 existing ignored, replication 66,
    sync 199; remaining workspace unit, integration and doc suites passed)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1140 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check`

  No frontend static/build/visual checks or separate native `cargo check` were needed for this
  Rust core/app/sync-only slice. No runtime code changed after the native/frontend suite runs.

  **Next:** expected-hash seed fetch and recovery-first newcomer installation. Capture/recheck
  runtime, MLS and discovery authority there: public `ReceiptHeadAnswer.proof: Some` is not an
  installation permit. Automatic actor/vault lifecycle scheduling, maximum-source latency
  measurement and durable signed repair retention/serving remain incomplete. Unknown owner
  tenure remains the previous slice's honest availability limitation. The final UI guide is
  still deferred until the full backend acceptance checklist is actually complete.

- **P1 independently observed owner tenure (2026-09-08).** Sync now records the start of the
  current owner's tenure from actual applied MLS transitions and saves that observation in the
  same authenticated snapshot as the group. This distinguishes A-to-B-to-A ownership without
  trusting a restored receipt to establish its own currency. The founding group starts known
  at zero. Welcome joins and old snapshots start Unknown; same-owner commits preserve Unknown.
  An Add into a recycled lowest leaf can change the owner, and is observed just like a Remove.

  All production merge paths use one synchronous observation seam. The adversarial diff review
  identified a post-merge helper-error edge: MLS can advance before later serialization fails.
  Observation now follows the actual group before propagating either result. The regression
  preserves the original error while proving the matching group/tenure can still be saved.
  Re-review found no remaining implementation findings. A missed hook or panic fails closed:
  the getter reports Unknown and a mismatched snapshot is refused.

  Eight new regressions cover founding/joining, strict tail framing and partial-tail rejection,
  legacy upgrade/restart, real staged winner/loser/applier and ordered commits, A-to-B-to-A,
  synchronous Remove, invite/companion Add, unknown newly joined owners, and post-merge errors.
  Two existing registry fixtures now perform their setup Add through the same observation seam;
  their original assertions are unchanged. Required verification passed on the final code:

  - `cargo test -p catcoms-sync --lib` (189 passed)
  - `cargo test --all --all-features` (app 357 passed / 4 existing ignored, replication 66,
    sync 189; remaining workspace unit, integration and doc suites passed)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1140 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `git diff --check`

  **Limits/next:** this getter is independent local evidence, not a publication permit. The
  receipt-head publisher still needs to flush the matching MLS snapshot and irrevocable owner
  decision and recheck current membership, fault and tenure. Legacy or newly joined owners can
  remain Unknown indefinitely; assigning the current epoch or copying a receipt is not a safe
  availability fallback. Old snapshots load as Unknown; older binaries reject the new tail,
  so this is backward-read support, not downgrade compatibility. Keyed receipt-head/expected-seed
  discovery and runtime ownership remain next. No UI/native source changed; no static/build/
  visual frontend check or separate native `cargo check` was needed for this sync-only slice.

- **P1 durable registry receiver continuation (2026-09-08).** The cooperative receiver now
  derives heads/seed from checked durable state and retains at most one page in each of four
  watch-bound passes. Fetching never borrows the vault; persistence validates the entire page
  through the existing typed gate in a detached epoch, then advances only after one accounted
  atomic write. Bad middle operations/dependencies save none of the page; duplicate and empty
  terminal pages still verify inventory/current Open scope and flush held bytes. An uncertain
  rename pauses at the same page/cursor until reconciliation and explicit retry. There is no
  durable provider cursor or receiver-authored intent, network ack, finality or currency claim.

  Passes pin full requester/provider identities, exact watch/runtime and physical mount. They
  keep a fixed ten-minute receiver-clock lifetime, one-second request/persist pacing, bounded
  attempts and aggregate input, including duplicates. Cancellation is retryable without refunds;
  revoked handles retain their four-slot capacity until dropped. The adversarial review found
  an honest divergent-head restart loop and a futile stale-MLS persistence retry. Both are fixed:
  one initial empty-head fallback retains seed/provider/charged limits, and MLS advancement
  discards unusable pending ciphertext into RestartRequired. Re-review found no remaining
  implementation findings. The real membership-change fixture now explicitly subscribes to
  control traffic before expecting the third member's commit; no membership check was weakened.

  Focused verification: nine receiver tests (including actual joined-member divergent branches,
  duplicate pages, cancellation, watch/mount replacement, source sealing and MLS advancement)
  plus three atomic batch-save/crash/restart tests passed. Existing core page tests additionally
  pin wide-frontier fallback and verified checkpoint-seed extraction without snapshot mutation.
  Required verification passed on the combined worktree (including the separately committed
  file-reliability slice, which this change does not modify):

  - `cargo test -p catcoms-app registry_receiver_ --lib` (9 passed)
  - `cargo test -p catcoms-app registry_page_batch --lib` (3 passed)
  - `cargo test --all --all-features` (app 357 passed / 4 existing ignored, replication 66,
    sync 181; remaining workspace unit, integration and doc suites passed)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (192 passed)
  - `npm.cmd --prefix apps/desktop test` (1140 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`
  - `git diff --check`

  **Next:** keyed receipt-head/expected-seed discovery and runtime/coordinator ownership. The
  adapters are intentionally cooperative, not an automatic actor/native worker. Maximum-epoch
  source rebuild latency still needs measurement before automatic scheduling. UI is untouched;
  [BACKEND-IMPLEMENTATION.md](BACKEND-IMPLEMENTATION.md) remains the completion checklist.
  The next design check identified a prerequisite: sync currently tracks the current owner but
  not the observed start of that tenure. Fresh owner head proofs must wait for durable observed
  tenure tracking; a restored receipt's own tenure field is not independent evidence (A-B-A).

- **Autonomous backend completion / P1 authenticated registry pages (2026-09-07).** The owner
  requested continued backend implementation, periodic verified pushes, adversarial reviews,
  and final Markdown UI integration guidelines. UI remains owner-owned; the canonical mockups
  and unrelated release changes are untouched. [BACKEND-IMPLEMENTATION.md](BACKEND-IMPLEMENTATION.md)
  is the acceptance checklist. P1 remains roughly **65% (+/-10 points)**: this closes another
  network adapter, not the remaining orchestration/discovery/Studio integration.

  Kind 20 now authenticates registry page requests over the actual requester transport identity.
  `run_once` retains at most eight watched requests, one per full requester, under independent
  preauth, requester and source-read rates. `Server::serve_registry_request_step` binds the
  provider/watch to the same runtime, physical vault, numeric server and bucket before draining
  the checked saved source. Client `request_registry_page` requires a pre-existing current
  bound-member endpoint proof, caps signed replies before copying, verifies a query-bound
  domain-separated signature, and returns unadmitted typed pages without legacy fallback.
  Four outbound permits remain charged through actual transport termination after cancellation.
  Neither a response handoff nor cursor completion means delivery, finality or currency.

  Seven sync regressions cover canonical framing, request/response binding, key/epoch changes,
  expiry/watch replacement, all resource rails, and delayed-driver cancellation accounting.
  App coverage includes a genuinely joined second member fetching 32+1 saved operations, explicit
  durable admission and vault reopen, plus provider/watch/server/mount rejection before I/O.
  The initial mount test correctly hit the vault's single-writer lock; it now closes the old
  mount before reopening, preserving that security invariant. Design review's retained-request
  capacity and endpoint metadata concerns are fixed. Actual-diff review and re-review have no
  remaining findings; the low-priority alternate-current-signer and stale-proof test gaps are fixed.
  Maximum-epoch source-read timing is an explicit incomplete follow-up before automatic scheduling,
  recorded in the acceptance checklist. Verification passed:

  - `cargo test -p catcoms-sync registry_page_ --lib` (7 focused tests, including review regressions)
  - `cargo test -p catcoms-app registry_page_network --lib` and the corrected adapter regression;
    all five app page tests also passed in the full suite
  - `cargo test --all --all-features` (app 334 passed / 4 existing ignored, sync 176, replication 66;
    workspace integration/doc tests passed, existing ignored harness/probe tests unchanged)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190 passed)
  - `npm.cmd --prefix apps/desktop test` (1135 passed)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh`
  - `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`
  - `git diff --check`

  **Next:** a bounded durable receiver continuation driver, then keyed receipt-head/seed discovery
  and runtime/coordinator ownership. The network test drives receive admission explicitly; no
  automatic actor/native page driver, Studio materializer or new UI command is claimed here.

- **P1 bounded registry page serving (2026-09-07).** Rough P1 backend estimate: **65%, with
  about +/-10 percentage points uncertainty**. This is an engineering estimate, not a count of
  commits and not Creative Suite/UI readiness. The protocol core, checked persistence, registry
  settlement/recovery and cooperative gossip paths exist. Automatic runtime ownership, managed
  catch-up/discovery, remaining recovery/repair/succession integration and Studio consumers are
  substantial unfinished work.

  `Server::{begin_registry_page_provider, serve_registry_page}` now read accepted registry
  history from the checked vault and reseal bounded pages under current MLS. An 81-byte HMAC
  cursor binds exact provider/requester/scope, initial heads/seed, a fixed accepted-log prefix,
  position and ten-minute monotonic lifetime. New appends do not reset or extend the prefix;
  changed prefixes restart, byte-identical reloads can continue. Up to 32 operations and 512 KiB
  of framed ciphertext fit per page. More than 64 independent heads can still complete from
  an empty initial frontier without repeatedly receiving the same prefix.

  Missing removed-author history reports `HistoricalAuthorizationRequired`; it is not silently
  skipped or accepted under the provider's identity. Already-delivered cursor history does not
  block later current-author descendants when an author is removed between pages. A rotated
  epoch requires the requester's claimed verified seed. Claims are not possession proofs, and
  prefix completion is not currency/finality. Runtime/mount, membership, caps, MAC and expiry
  check before source I/O; concrete-id matching requires the captured bucket's checked load.
  No source writes, intent retirement, subscriptions or delivery acknowledgements occur.

  **Next:** authenticated request routing with aggregate source-read limits, receiver paging and
  durable admission, then receipt-head/seed discovery and automatic coordinator/native lifecycle
  ownership. This is a callable page-serving backend, not automatic reconnection or a new wire
  request/response format. UI and unrelated release changes remain untouched.
  Twelve focused regressions cover append/reload progress, current-MLS resealing, 65 independent
  heads, exact padded byte limits, seed requirements, removed-author dependencies, cursor scope/
  tampering/expiry, provider/mount replacement and rejection before corrupt-source reads. The
  HMAC framing golden vector was independently reproduced with .NET HMACSHA256. Read-only
  design/diff review and re-review have no remaining findings; the review's already-delivered
  removed-author case and pre-I/O wording clarification are fixed. Verification passed:

  - `cargo test -p catcoms-replication --lib registry_page -- --nocapture` (9 tests)
  - `cargo test -p catcoms-app --lib registry_page -- --nocapture` (3 tests)
  - `cargo test --all --all-features` (existing ignored harness/probe tests unchanged)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190 tests)
  - `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`
  - `npm.cmd --prefix apps/desktop test` (1,135 tests)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh` (Git Bash), worktree/staged `git diff --check`

  The final backend/native runs include the review fix. Frontend static/build/visual checks were
  not run for this backend-only slice. HMAC/zeroize are now direct replication dependencies at
  already-locked versions; both workspace lockfiles record the edges, with no package upgrades.
  No UI or native bridge source changed; concurrent release work is outside this review.

- **P1 opt-in registry gossip receiver (2026-09-07).** Actual network ticks now route watched
  registry traffic into a bounded authenticated inbox, separate from generic Automerge documents.
  The Server drain persists one packet through the existing typed gate/accounting/barrier. Tests
  exchange an edit between two genuinely joined members using the prior replay sender and current
  `sync_once`, then verify duplicate handling and durable state after a vault reopen. This is a
  callable backend gossip path, not actor-owned or automatic desktop synchronization.

  Watches bind sync instance, full group, physical mount, captured local server, bucket, concrete
  epoch and fresh watch generation. Registering is synchronous; explicit flush or the next tick
  reconciles subscriptions. Cancelled/failed subscribes and unsubscribes retain one uncertain topic
  for cleanup and keep the retry flag armed. Rewatch/unwatch drops old queued work without resetting
  rate debt.
  Full current receiver/author, current MLS, canonical bucket/domain and exact watched topic are
  checked before queueing and authority is rechecked before store I/O. The inbox has 16 compact
  packets; pre-auth is globally 50/s burst 200, per full-author/document 10/s burst 50, with 4096
  bounded limiter rows. The global rail can be exhausted by one sender; it does not promise fairness.
  Queueing earns no accepted-op counter, delivery ack or finality. Failed/stale/over-cap input is
  dropped without an ack; sender retry or future catch-up must recover it. Past/future MLS frames
  have no managed recovery path yet. New persisted epochs require new watches.

  **Next:** actor/native store and budget ownership, bounded replay/drain scheduling and lifecycle
  cancellation; managed catch-up, registry/receipt-head/seed discovery; settlement-wide capacity
  handling and Studio consumers. UI remains user-owned; release-workflow changes are excluded.
  No wire or persistence format changed. The review found a cancelled-unsubscribe/rewatch race
  (reproduced by a failing test, then fixed) and old-mount revocation being over-gated (fixed while
  retaining exact generation checks). Read-only design/diff review and final re-review have no
  remaining findings. Verification passed on the corrected code:

  - `cargo test -p catcoms-app --lib registry_receive -- --nocapture` (3 tests)
  - `cargo test -p catcoms-sync --lib registry_inbox -- --nocapture` (6 tests)
  - `cargo test --all --all-features` (existing ignored harness/probe tests unchanged)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190 tests)
  - `npm.cmd --prefix apps/desktop test` (1,135 tests)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh` (Git Bash) and `git diff --check`

  Frontend static/build/visual checks were not run for this backend-only slice. No UI or native
  bridge source changed. Concurrent release work is not certified by this review.

- **P1 cooperative registry sender (2026-09-07).** The Server adapter now connects saved-intent
  replay to driver-acknowledged one-shot publication. A cursor binds the exact sync instance at
  begin, rejecting same-device/group replacement; exclusive Server/store borrows span checked
  preparation through dispatch. The actual local full identity, MLS scope and current blinded
  registry topic are checked before sending. No legacy document-map entry or retry outbox is used.
  Only Submitted advances. Duplicate, refusal, cancellation and unwind preserve the saved id and
  per-pass deadline for fresh resealing; every outcome keeps the durable intent. Cancellation
  after driver admission remains ambiguous, not rollback or delivery proof.

  **Next:** bounded live coordinator ownership/wakeups and native lifecycle cancellation, managed
  registry receive/catch-up, receipt-head/seed discovery, settlement-wide capacity handling and
  Studio/actor consumers. This is a cooperative backend API, not an autonomous worker or a new
  frontend feature. No driver deadline, aggregate pass cap or gossip-size change is claimed.
  UI remains user-owned; concurrent upload/release source and documentation are excluded.

  Nine focused regressions cover exact submitted bytes/typed registry receive, transport refusals,
  Duplicate, cancellation, lost acknowledgement, unwind, fresh resealing without reauthoring,
  replaced sync/mount, failed storage and Closing gates, conservative holds, malformed/foreign/
  oversized packets, current MLS/routing and redacted diagnostics. The typed receive test is not
  live managed-ingress/newcomer coverage. Read-only design/diff review and re-review have no
  remaining findings. The review's low test gap is fixed: rejected packets have a subscribed
  observer and explicit error checks, proving NoPeers cannot masquerade as authorization refusal.
  Verification passed:

  - `cargo test --all --all-features` (existing ignored harness/probe tests unchanged)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190 tests)
  - `npm.cmd --prefix apps/desktop test` (1,135 tests)
  - `cargo test --all --all-features --lib registry_send -- --nocapture` (final 6 tests)
  - `cargo test --all --all-features --lib registry_publication -- --nocapture` (final 3 tests)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh` (Git Bash) and worktree/staged `git diff --check`

  Initial crate-focused checks also passed; an intermediate app rerun hit stale compiled sync
  metadata. Crate-scoped generated-cache cleanup restored compilation; the final full suite and
  focused reruns include all final source changes. Frontend static/build/visual checks were not
  run for this backend-only slice. Concurrent upload/release work is not certified by this review.

- **Detached file fetches, first-index convergence and local kept copies (2026-09-07).**
  File chunk/range network waits now run in bounded tasks, with driver-level connected-only admission,
  cancellation and transport-retirement accounting. The actor validates exact current file/MLS
  authority again before storing a response. Independent first `ROOT/FILES` list conflicts share one
  deterministic bounded read/mutation projection. The desktop separates cached bytes from remote
  availability and offers explicit per-file Keep / Check and repair / Release with a separate sealed
  1 GiB / 32-file local store, durable commit, restart verification state and exclusive directory lease.
  See [the implementation and remaining limits](design-file-reliability.md). Remote confirmations,
  automatic replication, holder-aware eviction, unlisted standalone export and whole-variant Keep
  retry after a corrupt presence-ranked candidate remain follow-ups. Queued commands are interleaved;
  legacy actor operations and all-local-copy background-sync delay are not claimed solved.

  Verification on the integrated tree (2026-09-08): `cargo test --all --all-features` passed
  1,179 tests (six existing ignored harness/probe tests); the separate Tauri suite passed 192;
  `npm --prefix apps/desktop test` passed 1,140. Root and Tauri formatting checks, root Clippy
  with `-D warnings`, the ambient-dependency gate, Tauri `cargo check`, frontend `check`/`build`
  and `git diff --check` passed. Build retains the existing bundle-size advisory. Read-only
  adversarial design/diff review and integration re-review have no blocker/high findings;
  the limits above remain explicit follow-ups. Visual fixture inspection could not run because
  this session exposed no browser surface; no screenshot or visual validation is claimed.

- **File re-upload and encrypted manifest variants (2026-09-07).** Upload dedup now verifies a
  complete local copy before reusing metadata or discarding staged bytes. Missing/unreadable copies
  receive a fresh attested repair; repeat repairs reuse only the current device's verified exact
  listing. Downloads and previews resolve up to four compatible randomized encryptions, trying all
  local variants before network waits. Whole-file and per-chunk integrity remain enforced; media
  caches bind the entire current variant set. Storage selects an exact-verified compatible local
  variant instead of hiding every duplicate encryption. Regression coverage includes both upload
  paths, remote fallback, forged ownership, bounded verification, and repaired downloads/previews
  after a complete vault close/reopen with abandoned-upload staging cleanup.
  Publication's complete local verification can still occupy the actor for a whole file. Differing
  MIME or legacy chunk layouts remain explicit conflicts. The later reliability slice above resolves
  concurrent first-list visibility and schedules chunk/range network waits outside the actor.
  Malicious incompatible/over-four variant claims can still deny
  resolution. Older clients keep their prior conflicting-manifest behavior until upgraded; the
  wire and persistence encodings are unchanged. These are not claims of universal image/transfer
  reliability.

- **P1 one-shot transport prerequisite (2026-09-07).** `MeshTransport::publish_once` now waits
  for one actual driver attempt instead of treating command enqueueing as publication. Production
  uses a separate command that cannot enter the legacy `pending_publish` ciphertext retry queue.
  Unsupported transports fail closed; the deterministic memory broker implements bounded immediate
  fan-out. Existing chat publication, UI and wire/persistence formats are unchanged.

  There are at most 16 queued/being-attempted compact payloads per mesh service (512 KiB each,
  64-byte topics). Both semaphore and shared command-queue saturation return Busy without waiting.
  Cancelled commands retain capacity until drained. The driver suppresses a dropped future only
  when cancellation is observed before its final admission check; later races/ack loss cannot
  retract an attempt. Normal libp2p cache/handler effects remain possible even after NoPeers or
  QueuesFull. Submitted and Duplicate are distinct, and neither proves delivery or retires intents.

  **Next:** use this seam in the cooperative replay sender with actual session/server, membership,
  MLS epoch and Open-gate checks; then live coordinator ownership/aggregate scheduling. Registry
  receive/catch-up, receipt-head/seed discovery, settlement-wide capacity handling and Studio/actor
  integration still remain. The seam's 512-KiB bound is not a guarantee the current gossip size
  configuration accepts every P1 operation; that limit remains unchanged and reports TooLarge.

  Eleven focused regressions cover driver acknowledgement/classification, exact input bounds,
  compact slice ownership, semaphore/shared-queue saturation, cancellation before and after
  admission, shutdown, no implicit retry, unsupported fallback, memory fan-out and real libp2p
  memory-swarm submission/reception. Read-only design and actual-diff adversarial reviews found
  no blocker/high/medium. The low coverage suggestion is fixed: capacity is asserted inside an
  attempt and an injected unwind releases it, reports Closed to the waiter and queues no retry.
  The final eight net tests pass, including this test-only addition after the full suite started.
  Verification passed:

  - `cargo test -p catcoms-net publish_once -- --nocapture` (initial 7 tests), then
    `cargo test -p catcoms-net --lib publish_once -- --nocapture` (final 8 tests)
  - `cargo test -p catcoms-rt publish_once -- --nocapture` (3 tests)
  - `cargo test --all --all-features` (existing ignored harness/probe tests unchanged)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190 tests)
  - `npm.cmd --prefix apps/desktop test` (1,135 tests)
  - `cargo clippy --all-targets --all-features -- -D warnings` (including final rerun)
  - `cargo fmt -p catcoms-net -p catcoms-rt -- --check`
  - `bash scripts/check-no-ambient.sh` (Git Bash) and `git diff --check`

  `cargo fmt --all -- --check` was run but reports only concurrent, unrelated upload-code
  formatting differences. Those app/native upload and release-workflow/documentation changes
  are excluded from this commit/review; some continued after full-suite compilation, so these
  results do not certify the evolving unrelated work. Frontend static/build/visual checks were
  not run for this transport-only slice; it changes no UI or native bridge source.

- **P1 cooperative registry replay pass (2026-09-07).** `begin_registry_replay` snapshots only
  the actual member's saved intent ids from a ledger checked against both inventories.
  `step_registry_replay` attempts at most one existing checked replay, paced at 100 ms per pass
  by the injected monotonic clock. The pass keeps at most 10,000 boxed ids (320,000 payload bytes),
  no operation bodies or ciphertext queue. Begin captures an explicit concrete epoch id; it is
  not a check that the source is current/Open. Every actual attempt still performs those checks.

  Prepared work waits for an opaque, exact-attempt submission ticket. Duplicate/stale/cross-pass
  acknowledgements cannot skip an edit. A failed send retries the same saved id with a fresh
  reseal; errors/unwinds pause before cursor advancement and retain the charged deadline. Held
  ids are visited once without removing their intents. Missing/retired ids pause, not silently
  succeed. Completion counts snapshot traversal, including holds, never delivery/finality or
  the current ledger being empty. Later additions/rotation require a fresh pass.

  Stable physical-mount binding rejects reuse after vault reopen, independently of rotating
  intent-budget freshness. It is NOT UI-lock or server-incarnation authorization: the store may
  stay mounted while the UI locks. Lost tickets/abandoned prepared results recover by dropping
  and restarting the pass; no timeout advances it. No wire or persistence format changed.

  **Next:** live coordinator ownership, bounded aggregate scheduling and send-time lifecycle/gate
  checks, actual network publication, settlement-wide capacity handling and receipt-head/seed
  discovery, then Studio/actor consumers. This is a cooperative backend driver, not an autonomous
  worker or an end-to-end feature. UI remains user-owned; concurrent release changes are excluded.

  Nine focused pass regressions pass: own-id selection/new additions, exact/stale/cross-pass
  acknowledgements, held-once traversal, clock boundaries/overflow, wrong mount/group/device/epoch,
  member removal, failed post-rename writes/unwinds, lost prepared results across reopen, real
  receipt retirement/rotation and the maximal 10,000-intent ledger. Design, actual-diff and final
  documentation adversarial reviews have no remaining findings. The review's comment correction
  distinguishes an abandoned Prepared result from a Held result, which advances traversal only.
  Verification passed:

  - `cargo check -p catcoms-app`
  - `cargo test -p catcoms-app registry_pass -- --nocapture` (9 tests); the submission/ticket test
    was rerun after adding an explicit wall-clock-jump assertion.
  - `cargo test --all --all-features` (existing ignored harness/probe tests unchanged)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190 tests)
  - `npm.cmd --prefix apps/desktop test` (1,135 tests)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `bash scripts/check-no-ambient.sh` (Git Bash) and worktree/staged `git diff --check`

  Frontend static/build/visual checks were not run for this backend-only slice; no UI or bridge
  source changed here. Concurrent release-workflow/documentation changes are not part of its review.

- **P1 author-owned registry replay step (2026-09-07).** `replay_registry_intent` now takes a
  saved intent id and captured concrete epoch id, never a replacement body/nonce. It requires
  the actual current member to be the original author, checks both budgets and the saved Open
  epoch, and validates/account-checks every retained/staged typed recovery slot. It reuses the
  existing intent-then-epoch durability barriers before returning prepared ciphertext.

  New authoring is held on current/recovered pointer deletion or a higher current admitted/overflow
  hint, preserving the saved intent. Stable registry keys lack intent-origin epochs, so this is
  deliberately conservative; two-snapshot eviction also makes absent deletion evidence best-effort,
  not permanent protection. Exact authenticated current-log matches instead reseal the original
  change without changing newer state, including a Tombstone saved before a failed flush. A marker
  or same id with different body never earns that exception. No new wire/persistence format.

  **Next:** bounded replay scheduling/publication, settlement-wide capacity handling, receipt-head/
  seed discovery and live actor/Studio integration. This is one bounded store step, not automatic
  live replay. Held edits need later explicit recovery UX; they are not deleted or marked final.
  UI remains user-owned. Concurrent frontend/release/networking changes are excluded from this slice.

  Focused verification: one core and eight store replay tests pass, including real rotation,
  staged/retained tombstones, marker spoofing, foreign author, stale epoch, current-log retries,
  superseded admitted/overflow hints, malformed recovery, all-family accounting and failed
  writes/flushes/unwinds. Read-only adversarial review found no blocker/high/medium; its flush-test
  gap and overbroad heading are fixed. Verification passed:

  - `cargo check -p catcoms-app`
  - `cargo test -p catcoms-replication registry_replay -- --nocapture` and
    `cargo test -p catcoms-app registry_replay -- --nocapture`; the final flush matrix also passed
    its focused run and the full root suite.
  - `cargo test --all --all-features` (existing ignored harness/probe tests unchanged)
  - `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190 tests)
  - `npm.cmd --prefix apps/desktop test` (1,135 tests)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings` (final rerun passed after an
    unrelated concurrent sync lint issue was corrected; this slice did not edit that code)
  - `cargo clippy -p catcoms-app -p catcoms-replication --all-targets --all-features --no-deps -- -D warnings`
  - `bash scripts/check-no-ambient.sh` and `git diff --check`

  These runs include the working tree's concurrent changes, not an adversarial review of them.
  Frontend static/build/visual checks were not run for this backend-only slice; no UI or bridge
  source changed here.

- **P1 registry checkpoint installation (2026-09-07).** The store can now select a checked
  successor under one exclusive borrow: flush the full Closing source, durably save typed recovery,
  hold any pending eviction warning, durably retire only full-envelope-matching receipt-covered
  intents, then atomically replace the source with its verified seed and preserved receipt book.
  Excluded/unaccepted intents remain pending. Exact installed retries only flush the actual
  successor, preserving newer edits, seals and intents. Local edits now require the captured
  concrete document id; retrying an old Save cannot reauthor it after its markers are retired.

  A conflicting current-owner opening receipt faults both Open and Closing successors without
  losing their accepted content or newer high-water receipt. Fault evidence must anchor to the
  exact opening receipt. The bounded local ReceiptBook codec uses v2 only for a delayed fault
  below a newer high-water; ordinary v1 remains unchanged, and older readers fail closed on v2.
  Repair-sequence state survives construction of the successor.

  **Next:** automatic author-owned replay, settlement-wide capacity reservation, receipt-head/seed
  discovery and live coordinator/actor/Studio consumers. The registry store transaction is tested,
  not live-wired. Restore/Copy/Export and repair/rewind remain unwired. Conservative first/second
  recovery reservations and the physical 64-MiB intent replacement cap can still hold Closing at
  full quota; no early deletion credit or universal full-quota progress is claimed. UI remains
  user-owned; concurrent UI/release changes are excluded from this slice.

  Ten new core/store tests cover seed-only edit dependencies, repair-state preservation, delayed
  opening equivocation in Open/Closing, v1/v2 canonicality, included/excluded/unaccepted intents,
  stale Save and installed retries (including a newer seal), empty/corrupt recovery, exact eviction
  acknowledgement, physical intent headroom and nine before-write/after-rename/panic cases.
  Adversarial design, actual-diff and final documentation reviews have no remaining findings.
  The review's version canonicality and retry close-signature checks are fixed with regressions.
  Final checks passed: `cargo check -p catcoms-app`,
  `cargo test -p catcoms-replication registry_checkpoint -- --nocapture` (2), focused store tests,
  `cargo test --all --all-features`,
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190),
  `npm.cmd --prefix apps/desktop test` (1135), `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `bash scripts/check-no-ambient.sh` (Git Bash), and worktree/staged `git diff --check`.
  Frontend static/build/visual checks were not run: this slice changes no frontend/native source.

- **P1 typed registry recovery staging (2026-09-07).** `stage_registry_recovery` now recomputes
  the exact receipt-bound plan from checked saved Closing state, validates source accounting and
  every existing typed registry recovery slot before a nonempty save, and persists through the accounted
  adapter. It returns saved slots/warnings, not installation authority. Source history and intents
  remain unchanged. Empty evidence consumes no slot; third-slot retries retain the warning deadline.

  `RegistryRecovery` defines a canonical, bounded local payload for full source pointers, overflow,
  pointer-key tombstones and excluded author-bound domain operations. It validates the generic
  wrapper, full scope, ordering/disjointness, derived ids and the exact aggregate 6-MiB cap. It does
  not invent Studio-style random ids/authors for registry keys. The source opening close is carried
  in recovery's base field. Snapshot identity excludes quarantine and quota-owner bookkeeping, so
  late packets cannot manufacture extra recovery versions. Generic `RecoverySnapshot` Debug now
  redacts content too. The generic wire/persistence envelope is unchanged; registry payload v1 is
  specified in design-epoch-close section 10. Unknown/opaque old registry payloads fail closed.

  **Next:** crash-safe successor installation, settlement-wide reservation and intent retirement;
  then discovery and live coordinator/actor/Studio integration. Repair/rewind-specific typed records
  and Restore/Copy/Export actions remain unwired. At the content ceiling, first/second recovery
  snapshots can still refuse rather than crediting an unperformed source deletion. UI stays with
  the user; no frontend/native source was changed by this slice.

  Review/test targets: `registry/recovery.rs`, `registry_epoch/settlement.rs`,
  `store/epoch_registry/recovery.rs` and their adjacent tests. Seven new core tests and five store
  tests cover golden encoding, wrapper/scope/count/key/author rejection, payload size bounds,
  valid overflow, malformed excluded operations, tombstone-only evidence, empty plans,
  quarantine-stable ids, real 2-MiB source histories,
  restart, failed saves before/after rename, stale inventory, storage refusal, invalid old slots,
  and the bounded third-slot warning. Rotated-source coverage is also expanded. Focused tests
  pass. Adversarial design, actual-diff and documentation reviews have no remaining actionable
  findings; the review's overflow/domain-decoder coverage and empty-path wording findings are fixed.
  Final verification passes: `cargo test -p catcoms-replication --lib registry_recovery` (7),
  `cargo test -p catcoms-app registry_recovery_stage` (5), `cargo check -p catcoms-app`,
  `cargo test --all --all-features`,
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190),
  `npm.cmd --prefix apps/desktop test` (1135), `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `bash scripts/check-no-ambient.sh` (Git Bash on Windows), and `git diff --check`.
  Frontend static/build and visual checks were not run for this backend-only slice. Concurrent
  UI/release changes are excluded from this slice and its commit.

- **P1 receipt-bound registry settlement preparation (2026-09-07).** Rough implementation
  estimate: **55%, with about +/-10 percentage points uncertainty**, for P1 backend work, not
  the whole Creative Suite or end-to-end readiness. Core validation, journals, storage admission
  and durable registry edits are implemented; settlement/recovery, discovery and live integration
  remain substantial. UI remains user-owned and untouched.

  `RegistryEpoch::prepare_settlement` and the read-only
  `ServerStore::plan_registry_settlement` reload/check Closing state and its exact held current-owner
  receipt, rebuild the selected dependency closure, verify the deterministic checkpoint, and
  partition accepted operation ids into included/excluded recovery inputs. The full source
  projection preserves overflow/tombstones. Excluded peer-authored operations retain attribution
  but grant no replay authority. A whole-source fingerprint prevents conflating two peers with the
  same receipt and different excluded edits. Source history, gate and durable bytes are unchanged.
  No wire or persistence format changed; no UI, actor or transport was wired.

  **Next:** typed registry recovery persistence plus crash-safe successor installation and intent
  retirement. A plan is not an installation permit or a guarantee that recovery fits its cap:
  that transaction must revalidate source/authority under the gate and persist recovery first.
  Then receipt-head/seed/held-history discovery, the complete-budget coordinator, and actor/Studio
  consumers. No source pruning or end-to-end settlement is claimed by this preparation slice.

  Human/adversarial review target: `registry_epoch/settlement.rs` and its tests;
  `store/epoch_registry.rs::plan_registry_settlement` and `tests/settlement.rs`. Seven core and one
  store regression pass: real 2-MiB close threshold, exact successor dependencies, equal receipts
  with differing excluded logs, malformed/missing closure, wrong seed, Fault/stale owner, removed
  close author covered by a current receipt, peer tombstone attribution, quarantined content,
  restart stability and corrupt vault rejection. Adversarial design, actual-diff and documentation
  reviews report no remaining findings. The review's rotated-source coverage gap is fixed: an
  epoch-1 plan preserves a seed-only inherited pointer, excludes late edits, survives restart and
  produces an isolated epoch-2 seed. Final verification passes:
  `cargo test -p catcoms-replication --lib registry_settlement` (7),
  `cargo test -p catcoms-app registry_store_settlement_plan` (1),
  `cargo test --all --all-features`,
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190),
  `npm.cmd --prefix apps/desktop test` (1132), `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `bash scripts/check-no-ambient.sh` (Git Bash on Windows), and `git diff --check`.
  No frontend/native source was changed by this slice; frontend static/build checks and visual
  validation were not run. Concurrent unrelated UI/release/documentation changes are excluded
  from this slice and its commit.

- **P1 durable local registry publication preparation (2026-09-07).**
  `ServerStore::edit_registry_epoch` connects the existing intent and registry adapters under one
  exclusive store borrow: canonical/type/scope/current-author validation, intent save/flush,
  then checked edit and epoch save/flush. No ciphertext returns before both barriers. A failure
  after the first retains the intent for retry/recovery. Markers never retire intents.
  The caller supplies one stable nonce/envelope per logical operation; retries reseal its exact
  retained signed change, even after newer heads or a restart. They never reauthor another delta.
  A retained id with different body rejects before journaling, including when the log arrived
  through inbound ingest with no local ledger. Closing/Fault refuses new local edits and retries.

  `RegistryEpoch::{validate_local_edit,edit_or_reseal}` provide those bounded typed/retry checks.
  Snapshot comparison now uses the normalized restored state before/after mutation: refreshing
  the current quota owner alone does not force an ordinary replacement copy at the content cap.
  Actual saved bytes remain authenticated/accounted and are flushed, not silently rewritten.
  There is no wire/persistence format change and no UI or network integration in this slice.

  **Next:** recovery-first successor installation/settlement and intent retirement; receipt-head,
  seed and held-history serving/discovery; the complete-budget coordinator; then actor/Studio
  integration. Local publication is now prepared durably, but the future sender must recheck
  session/server incarnation, membership, MLS epoch and Open immediately before sending. This
  is not an automatic replay loop, an outbox or end-to-end P1 readiness.

  Human/adversarial review target: `store/epoch_registry.rs`,
  `store/epoch_registry/tests/local.rs`, and `registry_epoch.rs` local retry helpers. Try every
  intent/epoch write and flush boundary, a restart with newer heads, a conflicting nonce without
  a ledger, a stale intent budget, a closed epoch, removal of the author, and owner succession
  at the content cap. Nine new regressions (seven store tests with failure matrices, two core
  tests) pass. `cargo test -p catcoms-app registry_` (21 tests) and
  `cargo test -p catcoms-replication registry_local` (2 tests) pass, as do the full
  `cargo test --all --all-features`,
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190 tests), and
  `npm.cmd --prefix apps/desktop test` (1109 tests) suites. `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `bash scripts/check-no-ambient.sh` (Git Bash on Windows), and diff checks pass.
  Adversarial design review identified retained-body conflict and owner-refresh cap cases;
  both are implemented and regression-tested. Actual-diff and documentation reviews report no
  remaining actionable findings. No UI/native source was changed, so visual validation and
  frontend static/build checks were not run. Concurrent unrelated changes are excluded from this
  slice and its commit.

- **P1 durable registry ingress and sealing (2026-09-06).** `ServerStore` now has
  `load_registry_epoch`, `ingest_registry_epoch` and `seal_registry_epoch`, returning only
  detached read-only `EpochRegistryState`. They reload the checked raw seed/signed log/gate/book
  and vault-save before returning admission or a seal. There is no arbitrary-save API. Seals
  retain all source content; late operations persist only bounded quarantine hashes, not accepted
  edits. Retry after uncertain rename authenticates and flushes identical bytes without a copy.
  Failed writes, flushes or writer panics grant no success and require accounting reconciliation.

  `scan_epoch_storage_with_registry` / `cleanup_epoch_storage_staging_with_registry` explicitly
  cover recovery, owner journals, intents and registry records without widening older APIs.
  Historical inventory shares the full restart validator but returns metadata only, so it needs
  no current owner and cannot grant editing authority. Peer-writable registry history and seed
  bytes charge ordinary content; only exact receipt growth charges protocol. This preserves room
  to seal and journal the owner decision when content is full. Unpublished registry attempts
  conservatively charge content and may require explicit cleanup before a fresh budget fits.

  **Remaining at this stage:** local publication preparation/resealing is now added above;
  held-op serving and successor
  installation and recovery-first multi-record settlement; fault/tenure orchestration;
  receipt-head/seed discovery; the sole complete-budget coordinator and live ingress work/rate
  limits; actor/Studio consumers. Each saved mutation currently rebuilds a bounded graph, so this
  adapter is deliberately not automatically invoked by transport. This advances persistence,
  not end-to-end P1 readiness. UI files remain user-owned and untouched.

  Human/adversarial review target: `store/epoch_registry.rs` and its adjacent tests, the four-family
  branches in `epoch_recovery/{inventory,cleanup}.rs`, and `registry_epoch.rs` historical inspection.
  Try a failed write before/after rename, a failed duplicate flush, a valid receipt for missing
  history, an indexed file disappearing, scope/inner-snapshot substitution, owner removal, a
  content-full owner seal, and crash-orphan cleanup. Assert no accepted-op acknowledgement before
  durability, no source retirement, no mutable authority from inventory, and exact physical pools.
  Focused tests: 11 store regressions plus one core inspection/accounting regression pass.
  `cargo test -p catcoms-app registry_store` and `cargo test -p catcoms-replication registry_`
  pass. Final `cargo test --all --all-features`,
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190 tests), and
  `npm.cmd --prefix apps/desktop test` (1037 tests) pass. `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `bash scripts/check-no-ambient.sh` (Git Bash on Windows), and diff checks pass.
  Adversarial design review corrected receipt headroom accounting; implementation review fixed
  needless budget invalidation on expected missing history and added inner-snapshot corruption
  coverage. Re-review reports no remaining actionable blocker/high/medium finding. No frontend
  or native source was changed; visual validation and frontend build/static checks were not run.

- **P1 registry restart coordinator (2026-09-05).** `catcoms_replication::registry_epoch::RegistryEpoch`
  privately owns the typed registry document, gate and receipt book. Its versioned, bounded
  restart unit carries the raw receipt-bound seed and signed log, not an independent Automerge
  save. Restore checks scope, signatures, causal dependencies, typed changes, full gate accounting
  and the Open/Closing/Fault receipt matrix before returning an editable object. Historical local
  admission survives owner/member changes; fresh receipts still require current-owner/tenure
  verification and future share exemptions refresh from the verified group.
  Receipt admission retains the entire source log. Duplicate post-seal arrivals now reuse their
  quarantine slot; previously they could produce a gate snapshot that its decoder rejected.
  At this stage it was an **in-memory coordinator and restart codec**; the 2026-09-06 slice above
  adds its inbound/sealing vault adapter, not live settlement. Intent-before-edit and
  snapshot-before-publish ordering remain the caller's responsibility. It cannot prune, finish
  settlement, acknowledge recovery, or select a successor in place of its source. Storage
  inventory/accounting for this record is now implemented above; receipt/seed discovery,
  recovery-first multi-record transactions, repair/adoption orchestration and Studio consumers remain.
  A different new-tenure receipt during Closing fails closed without replacing the saved seal;
  applying that rewind needs the deferred recovery worker. A checked held-op resealing/serving
  accessor is also needed for crash-after-persist/before-publish retries before live integration.

  Rough P1 implementation estimate requested by the user: **45%, with about ±10 percentage
  points uncertainty**. This is an engineering estimate, not a usability/readiness percentage.
  Core protocol and storage primitives are furthest along; production settlement, network joining
  and end-to-end recovery remain substantial. UI remains owner-owned and unchanged by this slice.

  Review focus for a second human/adversarial pass: `registry_epoch.rs` constructors/seal/restore,
  `doc.rs::restore_domain_log`, and `epoch.rs::verify_restart`. Try splicing individually valid
  saved parts, substituting a seed, omitting dependencies, replaying a post-seal operation, and
  restoring after owner removal. The invariant is exact signed-log/gate agreement with no
  independently trusted materialized document, and no source retirement on receipt admission.

  Verification: 13 new regressions (12 `registry_epoch` tests and the receipt restart-matrix
  test), plus all 25 `epoch_close` integration tests, pass. Final `cargo test --all --all-features`,
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190 tests), and
  `npm.cmd --prefix apps/desktop test` (1037 tests) pass. `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`, `scripts/check-no-ambient.sh`
  (Git Bash on Windows) and diff checks pass. Adversarial review
  found and fixed fabricated fault evidence and hidden same-tenure inheritance conflicts on
  restore; re-review reports no remaining blocker/high/medium finding. The listed integration
  limitations remain explicit follow-ups, not completed guarantees.
  One root-suite run timed out in the existing real-TCP
  `join_over_a_relay_then_upgrade_to_direct_via_dcutr_over_real_tcp` test after transport dial
  failures; `cargo test -p catcoms-sync --all-features --test tcp_dcutr_e2e` passed unchanged in
  isolation (0.16 s), followed by a passing complete root rerun. No timeout, assertion or
  networking code was changed by this slice.

- **Creative frontend dependency audit and C0c blob seam (2026-09-05).** UI remains owner-owned.
  `publish_pix` and `request_blob_bounded` now reach Server/actor/Tauri via `creative.rs` and
  `creative_blobs.rs`; see the exact IPC contract in INTERFACES section 5. The Rust PIX1 validator
  matches the frontend format; storage checks include bounded sealed reads and dedup/promotion.
  Focused coverage includes malformed/canonical pixels, exact/over limits, corrupt cached files,
  reopen, authenticated network refusal before storage, cancellation and stale bridge completion.
  PIX publication fails closed on the memory fallback after a failed disk-store attachment.
  Review fixes also persist Unix blob-directory ancestry and cover post-promotion flush/retry,
  invalid signatures, wrong CIDs, wrong request nonces and outsider responses. Low follow-up:
  ancestor directory syncing currently requires read access to otherwise execute-only parents;
  a future trusted vault-root durability boundary could narrow that requirement.
  This saves an immutable blob, **not a flipnote/frame list**; the frontend still uses its local
  placeholder map until the owner connects it. C0c also still needs the reference enumerator,
  profile result and consented-avatar persistence work. No UI files were changed by this slice.

  | Frontend dependency | Integrated now | Remaining |
  |---|---|---|
  | 1. PIX publication / bounded fetch | Storage -> sync -> Server -> actor -> native commands | Frontend invocation, exact record-length/format checks at consumption |
  | 2. Creative pins / retention | Existing wiki pins and generic retention only | `creative_pinned_cids`, concurrent-reference enumeration and retention wiring; expiry GC is not live today |
  | 3. Studio materializers | Tags 15/16 and fail-closed generic P1 paths | Rust StudioIndex/StudioObject domain validation/projection |
  | 4. Studio preflight | Generic projection preflight and registry implementation | 999-frame/8 MiB/4096-sfx/64-patch Studio rules |
  | 5. Studio commands | Actor infrastructure only | list/read/apply plus index variants through native bridge |
  | 6. Studio/settlement events | Core settlement models only | AppEvent and bridge forwarding, no real UI updates yet |
  | 7. Newcomer discovery | Registry/checkpoint/receipt-head primitives | Keyed network requests and catch-up coordinator |
  | 8. Draw claims | Existing jam transport only | Shared admission, full-identity signal bridge, draw channel and claims |
  | 9. Recovery actions | Vault stage/ack/advance APIs | Settlement wiring, typed Restore/copy/export and bridge commands |
  | 10. `.pixa` and chat doodles | Existing fileshare machinery | Export codec/validator, attachment schema and publication/reference integration |

  Roadmap: unlock independent blob I/O now, then finish the P1 coordinator/transaction/discovery
  and typed Studio integration. P1 reuses replication and vault storage; its new job is safe,
  bounded history retirement, not image-byte editing. Tested primitives are not completed features.

  Verification: 15 new regression tests; focused storage `bounded_`, sync `bounded_blob`, app
  `pix_` and native `creative_bridge` checks passed. Final `cargo test --all --all-features`,
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (190 tests), and
  `npm --prefix apps/desktop test` (1037 tests, invoked with npm.cmd on Windows) passed.
  Root/native formatting, root clippy with `-D warnings`, native cargo check, the ambient-dependency
  gate and diff checks passed. One intermediate native run failed the existing real-socket
  `the_listen_port_prefers_the_seed_derived_home_port` test; isolated and full reruns passed without
  changing it. Final adversarial re-review found no blocker/high/medium issue; the low portability
  follow-up above remains. No frontend runtime changes or visual changes were made.

- **P1 epoch-close backend work continues (2026-09-05).** Revision 5 of
  [`design-epoch-close.md`](design-epoch-close.md) is accepted. The first replication-core slice
  adds the four new stable document tags, backward-compatible v2 signed domain-operation
  envelopes, exact close validation, owner receipt/journal and peer fault state, a persisted
  server/document-bound `Open -> Closing -> Settled/Fault` gate, a vault-serializable bounded
  intent ledger, and the
  two-retained-plus-one-staged recovery transition. P1 types fail closed on the legacy edit/ingest
  path and require a type-specific inbound change validator. This is **not end-to-end P1 yet**.
  The checkpoint/registry slice now adds deterministic raw checkpoint seeds, receipt/hash/schema verification,
  isolated checkpoint installation, seed-origin vault restore, per-edit/inbound projection
  preflight, and the typed 256-bucket registry with stable 2048-pointer admission and tombstone
  reclamation. Tests exercise real encrypted registry edits after checkpoint/restart, closed-head
  projection excluding later work, malformed seeds/deltas, maximal bucket capacity and retirement.
  Adversarial fixes add bounded raw-column parsing, inner-author roster checks, causal predecessor
  binding, and idempotent edits after concurrent bucket creation. Legacy v1 change framing stays
  compatible; P1 v2 rejects compressed changes before parsing.
  The recovery-persistence slice adds scoped `ServerStore` load/stage/acknowledge/advance APIs:
  one atomically replaced vault-sealed record retains the two versions, staged version and original
  warning deadline. Exact completed-eviction metadata makes post-rename flush failures retryable.
  Recovery encoding checks its aggregate 6 MiB cap before allocation. This API is not wired to
  settlement or incoming traffic: shared storage admission, crash-orphan cleanup wiring, and server
  removal retention remain prerequisites. The three-slot limit is logical; replacement temporarily
  duplicates ciphertext. Recovery records currently remain after leaving, like held blobs.
  Storage admission now has a server-bound `EpochStorageBudget` plus an accounted recovery-save
  adapter: 1984 MiB content + 16 MiB protocol + 48 MiB settlement inside 2 GiB, full old/new/temp
  peak accounting, document-pinned reserve, and fail-closed reservations after uncertain writes.
  Tests cover forgotten guards, exact physical/pool reconciliation, restart and cap refusals before
  disk writes. Recovery inventory discovery now scans canonical vault files without a registry,
  verifies scope and physical pools, and accounts temporary siblings without reading/promoting them.
  It holds exclusive store access across bounded steps; failures or parser panics poison the scan.
  Unknown orphan ownership blocks per-server composition, not just the affected filename.
  Explicit recovery-staging cleanup is now implemented: an exclusive bounded pass removes only
  canonical unpublished siblings, preserves all logical recovery slots and legacy files, syncs
  successful batches, then hands off to a fresh inventory without releasing the store borrow.
  Errors/panics can leave partial removals but yield no completion or accounting refund; an empty
  retry still runs the directory-sync step. Tests cover failure/restart/reconciliation/retry,
  no-destination first-write orphans and hardlinks. It is not automatically run on user vaults.
  Owner receipt decisions now have an accounted vault-sealed prepare/mark-published adapter.
  Every mutation reloads under exclusive store access and saves before returning, including exact
  retries after post-rename flush failures. A returning owner can replace an unfinished older-tenure
  decision only with a strictly newer verified tenure; same-tenure choices remain irrevocable.
  Tests cover real MLS A-to-B-to-A tenure changes, restart, failed writes, stale completion,
  malformed scopes and shared recovery/receipt reserve ownership. This is not a live publisher:
  the coordinator still must validate the close/seed before signing and recheck authority at send.
  A combined `scan_epoch_storage` / `cleanup_epoch_storage_staging` now covers recovery files and
  `.owner-receipts` journals under one exclusive store borrow. Coverage is explicit and survives
  cleanup-to-scan handoff. Matching orphan destinations requires both namespace and digest; owner
  bodies retain their own small cap and protocol-pool accounting. Mixed-family tests cover failed
  writes, restart, cleanup, reconciliation and exact publication retry without touching saved choices.
  Existing recovery-only APIs keep their scope and old type names as aliases. Neither path is
  automatically invoked on user vaults; other managed types, server-removal lifecycle and the sole
  production budget/coordinator still need integration.
  Local intent preparation now has a vault-backed, append-only adapter: the local MLS device
  supplies the author, current membership and full scope are checked, and a new intent is sealed
  before the call succeeds. There is no intent-retirement API yet. Exact retries flush the
  authenticated unchanged final file and parent without another copy, including after a
  post-rename failure at the cap. Other writes still pay ordinary-content replacement peak.
  Explicit `*_with_intents` inventory/cleanup covers all three implemented families without
  changing either older API's coverage. A vault-wide `EpochIntentBudget` counts final ciphertext,
  framing, unpublished siblings and replacement peak inside 64 MiB; this is conservatively
  stricter than payload-only accounting. A private mount/generation token invalidates stale or
  duplicate budgets before any intent write/sync/cleanup attempt. Intent temporary bytes charge
  ordinary content, not the settlement reserve. Tests cover restart, both caps, cross-vault/group
  rejection, uncertain writes, stale inventory, mixed-family accounting and both document limits.
  Preparing an envelope is not type-specific validation, a live edit or replay; the coordinator
  still must validate domain semantics, serialize all record families and retire intents only
  with the checkpoint/recovery transaction. No new path runs automatically on user vaults.
  Follow-up coverage: native Windows reparse/junction refusal and forced enumeration-order
  fixtures (current tests vary creation order; attribution itself occurs only after EOF).
  Intent review follow-ups: an already-accepted author removed through actual MLS membership,
  and a caught panic specifically during retry-sync (outsider rejection, writer panic and sync
  error are covered). Final adversarial review found no blocker/high/medium defect in this slice.
  Budgets still require every managed record type; multi-record settlement,
  complete managed-type cleanup and sole-writer wiring remain deferred. No guessed future deletion grants headroom, and
  the low-level unaccounted save is not a production admission path.
  Studio-specific materializers, settlement/storage transactions, keyed catch-up and receipt-head
  discovery, application events and Studio integration remain. Catch-up integration must distinguish
  new submissions from already-admitted history by removed authors; future Studio epoch-0 logical
  keys must bind the server, as registry keys already do, since the signed operation envelope does
  not separately bind a group id. The user owns UI implementation;
  the canonical HTML/PNG references are recorded in `design-creative-suite.md`.
  That document's section 7 now tracks backend milestones and the local/fixture UI work that can
  proceed independently. P1 is in progress; there is no end-to-end shared Studio yet.

- **Phases 0 → 10 COMPLETE. The live work is the desktop client's real-time layer;
  group voice (phases 1–3 shipped; see [§ Voice](#voice-group-calls)).**
  The protocol stack (0–7) is done, and every networking + NAT-traversal path is proven
  end-to-end over **real TCP sockets**; direct / rendezvous-discovered / relayed /
  DCUtR-upgraded; plus the consolidated security suite. **Phase 8** built the UI-facing
  **`catcoms-app`** product model + async **event-stream actor** and the **Tauri 2 +
  Svelte desktop app** (`apps/desktop`): a multi-server rail, name-addressed channels
  with symmetric catch-up, a live roster with presence, profiles/avatars, a **fileshare
  browser** over **content-addressed blob fetch** (chunked, resumable-by-chunk), a status
  feed, a wiki, DMs + friends, and the whole chat product layer (search, edit/delete,
  reactions, replies, @mentions, pins, cross-server inbox). **Phase 9** added **disk
  persistence + encryption-at-rest** (passphrase vault → sealed per-server snapshots:
  close the app, reopen, enter the passphrase, everything is back). **Phase 10** was the
  UI/product overhaul + roles & permissions. Since then: **group voice**, and two
  networking QoL slices; best-effort **UPnP/PCP/NAT-PMP plus PCPv6 firewall pinholes** (an auto-reachable founder: no relay, no
  port-forward, `bf28db9`), **persistable relay/rendezvous identities** (a stable peer id,
  so invites embedding an infra multiaddr survive a restart, `f317c5c`), and **AutoNAT v2**
  scoped dial-back diagnostics (client on members, experimental server opt-in only on explicitly
  configured relay/rendezvous infrastructure). The current reachability slice also ships guarded
  AutoNAT serving, 60-second two-way reply codes, and default-off per-server standing member
  switchboards with direct-first joiner consent. Mewtual still operates no public fallback server;
  a mutually unreachable first pair needs a direct/public route, configured relay, or third party.
  **2026-08:** the **desktop UI overhaul** (11u–11w): tokens-first "operator terminal"
  redesign with full user customisation, **server livery + shared icons**
  (`DocType::Livery`), the **verify-identity** surface, **channel topics**, and the
  delivery-states design (D1–D3 in progress).
  **11x: wiki history + edit review + nested pages.** The wiki doc gained three more
  NUL-reserved keys beside `\u{0}meta` (older peers merge them blind): `\u{0}hist`
  (per-page revisions: author fp, ts, full-body snapshot, kind edit/approve/auto/reject/
  rollback/delete/rename), `\u{0}pending` (member edits awaiting review) and `\u{0}cfg`
  (`review_days`, 0=off, ≤30, owner/admin-set). With review on, a member's save queues;
  owner/admin approve/decline; **auto-acceptance at the deadline is READ-TIME** (effective
  `wiki_map()` overlays expired pendings; nobody races to apply in the CRDT) and is folded
  into stored history by the next direct write under a **deterministic rev id (= pending
  id)** so concurrent folds converge. Member delete/rename are gated while review is on;
  pending bodies count toward `wiki_pinned_cids`. Sidebar: `/`-separated names render as a
  collapsible tree (`wikitree.ts`); history browser + review queue diff via `linediff.ts`
  (Myers). New `Server` methods: `wiki_history`, `wiki_pending_edits`, `wiki_review_days`,
  `set_wiki_review_days`, `approve_wiki_edit`, `reject_wiki_edit`, `restore_wiki_page`;
  `write_wiki_page` now returns `queued: bool`, mirrored through actor + bridge
  (`get_wiki_history`/`get_wiki_pending`/`get_wiki_review_days`/`set_wiki_review_days`/
  `approve_wiki_edit`/`reject_wiki_edit`/`restore_wiki_page`).
  Also **wiki infoboxes** (`infobox.ts`, pure + unit-tested): a `{{Infobox …}}` block is lifted
  by `renderWiki` **before** either converter runs and rendered with that page's own inline
  renderer, so one syntax serves both markdown and wikitext pages; it emits a `<table
  class="wiki-infobox">` (only `colspan` was added to the sanitizer allow-list). An image field
  accepts **only** the `![alt](cid:HEX)` marker: a bare content address would be invisible to
  the never-decay scan and could expire while the card still referenced it.
  **493 tests passing** as of 2026-08-19 (was 221 in 2026-07; the GUI WebView
  is the one manually-verified surface; both halves compile). See Known limitations.
- **Corrections (2026-08-19).** A reachability field failure ("timed out connecting to the
  server" when a remote user redeems an invite) triggered a design pass plus four adversarial
  reviews, and they found several claims in *this document* to be false. Recorded here because
  a status doc that overstates is worse than no status doc. See
  [`design-zeroconf-reachability.md`](design-zeroconf-reachability.md) for the full defect list.
  - **Live member presence never worked.** It is listed as shipped in the phase-8 row below.
    `connected_member_fingerprints` reads `peer_records`, and member PEX was never wired into
    the product (`publish_self_record` / `request_pex` had no callers outside `catcoms-sync`'s
    own tests), so the map was permanently empty: roster online dots always dark, "N online"
    always zero. Same root cause killed the 9g cross-session peer re-dial (it re-dialled
    nothing) and made the eclipse advisory fire **unconditionally for every group of 4+**.
    Fixed in `32dab2a`. The lesson worth keeping: crate-level tests all passed while three
    shipped features were dead, because nothing tested the product path above `catcoms-sync`.
  - **"`connection_limits` on every swarm" was not true**: `RelayBehaviour`, the one
    internet-exposed swarm, had none. Being fixed alongside the relay/rendezvous hardening.
  - **The desktop node regenerated its network identity on every launch** and bound a random
    port, so every invite issued before a restart was addressed to a machine that no longer
    existed, and after a restart invites carried a loopback address only. Fixed in `0af1583`.
- Both CRITICALs the 6e-3d design pass found are **closed and adversarially reviewed**:
  **A1** (the pre-existing bug where the gossip topics hashed the plaintext-invite
  `group_id`, so any invite-holder could read all topics) and **Sybil-C1** (the
  catch-up source-trust hole). See [`design-6e-rendezvous.md`](design-6e-rendezvous.md).
- The four security-critical closing slices (6–9) each passed an **adversarial-review
  workflow before commit**; one **blocking** finding was fixed in each of 3d-7 (a
  member-on-member PEX CPU-DoS; receive-side bundle uncapped) and 3d-8 (a variable-time
  cache integrity-tag compare; timing oracle); 3d-6 and 3d-9 were SOUND with cheap
  hardening folded in. The whole block ends with a **memory end-to-end test**: a joiner
  discovers the inviter at a zero-knowledge rendezvous and joins with **no hard-coded
  server address**.
- Toolchain pinned **Rust 1.89.0** (`rust-toolchain.toml`; automerge 0.10 needs it).
- 11 library crates + 1 binary (+ `apps/desktop`, its own workspace). The protocol
  layers are tested deterministically with
  N in-process nodes over an in-memory transport; the mesh is *additionally* tested
  over **real libp2p**; the memory transport (real swarms/Noise/req-resp), TCP
  loopback (real sockets, multi-process `serve`/`join`), a circuit relay, a DCUtR
  hole-punch upgrade, and a rendezvous register→discover.
- Local-only repo (`git init`'d, no remote). Commits are linear on `main`,
  one per phase/block.

## What Mewtual is

A peer-to-peer, serverless, end-to-end-encrypted, invite-only group comms system;
"Signal + federation". Each "server/connection" is its own MLS (RFC 9420) group;
per-device identity; encrypted CRDT documents (channels/wiki/status/calendar)
replicated over a libp2p mesh with zero-knowledge relays. Targets Linux, Windows,
Android via a Rust core + (eventually) Tauri 2.

The four locked decisions and the pre-implementation adversarial-review fixes are
in [`ARCHITECTURE.md`](ARCHITECTURE.md) §1–§2; **read them; they constrain everything.**

## Crate map

| Crate | Responsibility |
|-------|----------------|
| `catcoms-wire` | Canonical, injective, length-prefixed codec; domain-separated key-derivation contexts (`DocType`, `exporter_context`). |
| `catcoms-rt` | The **seams**: `Clock`, RNG (`OsCryptoRng`/`CryptoRngCore`), and `MeshTransport` (pub/sub + request/response) with an in-memory `MemNetwork`/`Hub` for tests. |
| `catcoms-crypto` | Content-addressed `DeviceId`; Ed25519 device keys; multi-device pairing primitives (`PairingRequest`/SAS/`DeviceCertificate` group-bound/`DeviceRevocation`/`MasterHandoff`; v2 origin-rooted, depth-1; the v1 account-key chain module was deleted); the unified key hierarchy (`Dek`→HKDF subkeys), XChaCha20 `seal`/`unseal`, tiered `SecureKeyStore`. |
| `catcoms-mls` | MLS group core (openmls 0.8): `MlsDevice`, `ServerGroup` (create/add/remove/process/epoch/`channel_secret`), single-use device-bound `InviteToken` (now `INVITE_DOMAIN` **v2**, carrying a signature-bound `rendezvous: Vec<String>`; `mint_invite_with_rendezvous`) + `InviteLedger`, `AddOutcome`, `designated_committer`; **`media_secret(call_id)`** (MLS exporter under `MEDIA_EXPORTER_LABEL` → the per-call voice key). |
| `catcoms-replication` | Encrypted CRDT docs (automerge 0.10): inner-signed `SignedOp`, `SealedOp` (per-epoch channel-key sealing), `EncryptedDoc` (edit/ingest/catch-up). |
| `catcoms-storage` | Content-addressed `Cid` blob stores (mem + fs); per-file encryption (`FileRef`, per-file wrap nonce); `RetentionIndex` (3-scope expiry, GC with decorrelated eviction + `HolderOracle` probe). |
| `catcoms-net` | libp2p `MeshService` realizing `MeshTransport` (gossipsub + request/response over Noise+yamux). NAT traversal: relay-client + **circuit-relay-v2** + **DCUtR** hole-punch (`next_direct_upgrade()`), router mapping via libp2p **UPnP IGD**, actor-owned IPv4 **PCP/NAT-PMP**, and a narrow internal **PCPv6 firewall-pinhole** client for stable TCP and UDP/QUIC ports, plus an **AutoNAT v2 client** whose `AutoNatResult` is scoped to one address/server/test. PCPv6 binds an exact GUA to the default gateway and native index from the OS IPv6 route table, uses request-bound 96-bit nonces, requests five-minute leases, honors assigned lifetimes up to 24 hours, and has independent family/interface ownership and stale-worker generations. Lease timing is monotonic; response options are accepted within RFC bounds. Bounded/coalesced `PortMappingSnapshot`, `AutoNatSnapshot`, and `RelayAddressSnapshot` streams retain authoritative current state for late consumers without unbounded diagnostic queues; public mapping/manual owners are reference-counted, withdrawn routes prune evidence, and failed probe/MAP attempts retry. Relay/rendezvous swarms serve v2 dial-backs only after explicit experimental `--enable-autonat`; a first-declared pre-socket guard requires one direct public target at the request connection's exact source IP and charges peer/source-prefix/node/concurrency caps. Same-NAT port probing and metadata remain bounded residuals, so serving stays off by default. Ordinary members never serve anonymous AutoNAT. Standalone zero-knowledge infra: `build_relay_swarm`/`run_relay` and `build_rendezvous_swarm`/`run_rendezvous` (`RelayBehaviour`/`RendezvousBehaviour`). **Rendezvous client** in `MeshBehaviour`: `rendezvous_register`/`rendezvous_discover`; discovered records surface via `next_discovered()` (per-response capped), registration grants/TTLs via `next_registered()`, and neither stream auto-dials. Repair adds fail-closed connected-only request/notify verbs and an actor-revalidated direct-only peer-bound batch of at most two routes. `add_external_address()` (register without a relay), and the trust-split rendezvous-address validators `validate_operator_rendezvous_addrs()` / `validate_invite_rendezvous_addrs()` (both reject circuit / require one `/p2p/` / distinct PeerIds; the invite one additionally requires a globally routable IP literal, allowing loopback only when the whole set is loopback, while the operator one still permits a DNS name because rung 4 needs one). Address classification for the whole workspace lives in `addr` (P13). Router mapping is opt-in at the library builder and explicitly enabled by the desktop; loopback-only services never touch the gateway. `connection_limits` on every swarm. Tracing-instrumented. |
| `catcoms-discovery` | **Pure** eclipse-resistance layer (no I/O, no ambient time/RNG). `DiscoveryPolicy` ranks discovered candidates into a bounded, Clock-paced/RNG-jittered **dial plan** (the only thing that decides what to dial): member-tag-verified → multi-rendezvous-corroborated → cache → junk-last, ≤1 root/rendezvous, roster-clamped, seq-freshness. Advisory `EclipseDetector` (D/R/S + hysteresis; never gates). Cross-session `AddressCache` (proven members, RNG-jittered eviction, BLAKE3 keyed integrity tag → tamper-detected on load; SQLCipher backing deferred). |
| `catcoms-sync` | `ChannelSync`: replication + membership over the transport. Blinded **member-only gossip topics keyed under `ns_secret_L`** that rotate on member removal (routing label `L`), grandfathered re-subscription window. The **join handshake** transfers the **routing state** (sealed, signature-bound). Membership **commit propagation**; **missed-commit recovery** with **signed catch-up responses** (nonce+epoch anti-replay) + a **two-pool peer model**; bounded zeroized **past-epoch key window**. Discovery surface: rotation-aware/TTL-renewed rendezvous, **member PEX**, sealed address cache, endpoint-scheduled redial, pairwise path evidence, derived HyParView-like active/passive views, local CYCLON age/source diversity, two-helper SWIM-style observations, queued exact-descriptor reciprocal repair and manual bounded redial. A newer signed omission removes previous live/sealed routes; old IPs are not retained as automatic backups. Ordinary members forward only small authenticated repair controls over existing proven paths, never application traffic or anonymous probes. Voice: **`KIND_CALL_SIGNAL`** (authenticated members-only push of an *opaque* SDP/ICE payload; signed, freshness-bound, `from` = the verified signer; **not** deduped, FIFO-bounded) + `media_key`. `SyncStats`. |
| `catcoms-app` | **Product model**; the UI-facing facade over the stack (so a GUI never touches MLS/automerge). `Server<T,R>` (found/join/open_channel/send_message/messages/members/invite), the canonical chat-message schema (`append_message`/`read_messages`), and the async **event-stream actor** (`spawn` → `ServerActor` commands + `AppEvent` stream: `ChannelUpdated{channel,change}`/`MembersChanged`/`Closed`; voice adds `MediaKey`/`SendCallSignal` commands + a `CallSignal` event). `ChannelUpdated` carries a typed `ChannelChange` (appended / re-rendered / topic / jukebox) and `channel_heads()` serves the unread projection: see [INTERFACES § 10](INTERFACES.md#10-channel-deltas--unread-state-catcoms-app--tauri-bridge--desktop). |
| `catcoms-log` | `tracing` subscriber init plus the canonical bounded diagnostics hub/ring. Safe admission destructively minimizes runtime text/addresses/targets/names; library-actor trace tokens normalize at the RingLayer and rejoin native operation/event stages. Native event envelopes carry a non-persisted session/trace proof so the webview can return an already-normalized trace without either trusting arbitrary UI hex or computing `H(H(raw))`. Public issue reports use a separate native allowlist renderer with per-row capture mode/epoch. `init_debug(debug, dir)` writes a separately bounded **raw** `debug_log_<ts>.txt`; that file bypasses Safe minimization and may contain arbitrary tracing/console/error prose, so it must be reviewed before sharing. |
| `apps/desktop` | **Tauri 2 + Svelte 5 desktop app** (its own cargo workspace, excluded from the root). A thin `#[tauri::command]` bridge (`src-tauri`) over the `catcoms-app` actor + a Svelte frontend; the whole product surface (rail/channels/files/status/wiki/DMs/profiles) plus the **WebRTC voice mesh** (all media-plane code is frontend). The WebView is the one manually-verified surface; `npm install && npm run tauri dev`. |
| `bins/catcomsctl` | Dev CLI. `demo` runs the whole stack end-to-end (in-process); `serve`/`join` run it across **real OS processes over TCP** (optionally `serve --relay`); `relay` and `rendezvous` run the zero-knowledge infra nodes (`--identity <file>` persists the keypair for a **stable peer id across restarts**, so invites embedding the address keep working); `recover` drives the 6d-1b miss-and-heal path; `--debug`/`--stats`. |

## Build / verify ritual (run before every commit)

```sh
cargo build --all
cargo clippy --all-targets --all-features -- -D warnings   # must be clean
cargo fmt --all -- --check                                  # must be clean
cargo test --all                                            # all green
bash scripts/check-no-ambient.sh                            # ambient-dependency gate
```

For a change that touches the desktop app (its own cargo workspace; the root
`--all` does **not** cover it), also run, in `apps/desktop`:

```sh
npm run check          # svelte-check: must be 0 errors / 0 warnings
npm run build          # vite build
cargo check --manifest-path src-tauri/Cargo.toml   # the bridge half
```

PowerShell helper to sum test results (Windows dev box):
```pwsh
$out = cargo test --all 2>&1 | Out-String
([regex]::Matches($out,"(\d+) passed")|%{[int]$_.Groups[1].Value}|measure -sum).Sum
```
(`-match "FAILED"` is a false positive; it case-insensitively matches "0 failed".)

## Dev loop

```sh
cargo run -p catcomsctl -- demo                 # in-process: found -> invite -> join -> E2E chat -> converge
cargo run -p catcomsctl -- recover --stats      # 6d-1b: a member misses a commit and self-heals
cargo run -p catcomsctl -- --stats demo         # print per-node SyncStats counters
RUST_LOG=catcoms_sync=trace cargo run -p catcomsctl -- demo
# Real multi-process over libp2p (terminals 1+2; add --host <ip> to cross machines):
cargo run -p catcomsctl -- serve --port 9000 --invite-file invite.txt
cargo run -p catcomsctl -- join  --invite-file invite.txt
# NAT traversal + discovery infra (each runs until Ctrl-C):
cargo run -p catcomsctl -- relay --port 4000        # zero-knowledge circuit relay
cargo run -p catcomsctl -- rendezvous --port 5000   # zero-knowledge rendezvous
# ...for a DEPLOYED node, persist the identity so its peer id (and every invite that
# embeds its multiaddr) survives a restart:
cargo run -p catcomsctl -- relay --port 4000 --identity relay.key
# Discovery: the server registers at a rendezvous under the invite's join_ns; the
# joiner discovers it there and joins with NO hard-coded server address:
cargo run -p catcomsctl -- serve --port 9000 --rendezvous /ip4/<rz-ip>/tcp/5000/p2p/<rz-id>
cargo run -p catcomsctl -- join                     # discover -> dial (via DiscoveryPolicy) -> join
```
`demo` runs both members in one process over the in-memory transport; `serve`/`join`
run the *same* join + catch-up path across **separate OS processes over real libp2p
TCP** (verified, incl. through a relay).

## Working conventions (important; keep doing these)

- **Block by block, test-gated.** Build a coherent block, make all tests pass + the
  full ritual clean, commit, then continue. Efficacy over speed.
- **No ambient time/RNG.** All time flows through `Clock`, all randomness through an
  injected `CryptoRngCore`. The only sanctioned OS sources are
  `catcoms-rt/src/{clock,rng}.rs`; `scripts/check-no-ambient.sh` enforces it (extend
  its allowlist deliberately).
- **Adversarial-review workflows for security-critical protocol code.** This caught
  a genuine **HIGH** in the network join (group-substitution) and corrected the
  membership-linearization design against the openmls source. Pattern: design/build,
  then run a `Workflow` of hostile reviewers (crypto / DoS / guarantee-preservation /
  distributed-systems), fold findings in, then commit. Don't skip it for membership /
  admission / key-handling changes.
- **Commit messages** end with a `Co-Authored-By: Rosemary` line. Use
  `git commit -F <file>` for messages containing `==`/quotes (PowerShell here-strings
  mangle them).
- **Memory**: `~/.claude/projects/.../memory/` holds durable facts; this repo's
  `docs/` holds the detailed handover. Keep both current.

## Roadmap & status

| Phase | Block | State |
|------:|-------|-------|
| 0 | workspace, `Clock`/`Transport` seams, wire format, CI | ✅ `016e3a0` |
| 1 | device identity + unified key hierarchy | ✅ `e5a292e` |
| 2 | MLS group core (local) | ✅ `196bf14` |
| 3 | single-use device-bound invites | ✅ `30bd856` |
| 4 | encrypted CRDT replication | ✅ `7db8efd` |
| 5 | storage & retention | ✅ `b797ffc` |
| 6a | libp2p `MeshService` over the seam | ✅ `0949151` |
| 6b | channel sync over the mesh + tracing | ✅ `c947021` |
|; | `catcomsctl` CLI + debug-file logging | ✅ `91c53d3` |
| 6c | network join handshake (inviter-authenticated) | ✅ `61d990c` |
| 6d-1a | membership commit propagation (single committer) | ✅ `89b5492` |
| 6d-1b | commit-catch-up recovery + ordered replay + past-epoch key window | ✅ `16a6427` |
| 6d-2a (1/2) | signed commit records + authorize-by-signature gate | ✅ `09d4cc5` |
| 6d-2a (2a) | MLS staged-commit primitives (stage/merge/abort) | ✅ `e577a11` |
| 6d-2a (2b) | sync-layer fork resolution (commit_id tie-break + contest window) | ✅ `939eb41` |
| 6d-2a (2c) | two-phase staged-Add join (provisional Welcome push) + review fixes I2–I6 | ✅ `42f9b7f` |
| 6d-2b (1) | **single-serializer remove** (members *request*; the designated committer alone commits); the convergence-safe model, on by default | ✅ `63ac788` |
| 6d-2b (2) | **all-members apply-time Add-binding validation** (every member rejects an Add not bound to this group / its own leaf key) | ✅ `0a1a276` |
| 6d-2b (3…) | by-value proposal batching · history-derived single-use · committer-decoupled admission | planned |
| 6e (1) | **full stack over real libp2p**; join handshake + encrypted catch-up over `MeshService` (Noise + request/response) | ✅ `f1d2713` |
| 6e (2) | **multi-process `catcomsctl serve`/`join` over TCP**; two OS processes, real sockets, verified | ✅ `73904f1` |
| 6e (3a) | **relay infrastructure**; relay-capable swarm (relay-client + DCUtR + identify) + relay server; a client reserves a circuit slot | ✅ `84827b1` |
| 6e (3b) | **end-to-end through a relay**; `catcomsctl relay`; `serve --relay` reserves + advertises the circuit address; `join` dials it. Verified across 3 real processes | ✅ `2f196b1` |
| 6e (3c) | **DCUtR hole-punch**; a relayed link auto-upgrades to a direct one; the upgrade is surfaced via `MeshService::next_direct_upgrade()`. TCP-loopback test asserts the upgrade event path | ✅ `7173e5e` |
| 6e-3d | **rendezvous discovery + eclipse-resistance**; 9 slices; design contract in [`design-6e-rendezvous.md`](design-6e-rendezvous.md) (`bd5e0d1`) | in progress (1–5/9) |
| 6e-3d-1 | per-removal routing secret `ns_secret_L` + `rendezvous_namespaces()` (rotate on removal; removed-member exclusion) | ✅ `eb6a952` |
| 6e-3d-2a | routing-state **transfer on join** (sealed, epoch-keyed) so joiners converge on topics/namespaces | ✅ `cb5c168` (prep `837bd4f`) |
| 6e-3d-2b | **re-key gossip topics from `ns_secret_L`** (member-only, rotate on removal); **closes A1**; adversarially reviewed | ✅ `1d9e3f2` |
| 6e-3d-3 | zero-knowledge **rendezvous server** + `catcomsctl rendezvous` | ✅ `924df09` |
| 6e-3d-4 | **rendezvous client** in `MeshBehaviour` (register/discover, surfaced, **no auto-dial**) + `connection_limits` | ✅ `fe1ed2a` |
| 6e-3d-5 | **signed catch-up responses + two-pool peer model**; **Sybil-C1** source-trust; adversarially reviewed | ✅ `726691b` |
| 6e-3d-6 | **`catcoms-discovery` `DiscoveryPolicy`** (pure: rank/clamp/dial-budget) + catch-up **nonce/epoch anti-replay** + pre-dial **membership tag**; reviewed SOUND | ✅ `2fedcd0` |
| 6e-3d-7 | **member PEX** (`KIND_PEX`, self-signed `PeerDescriptor`, responder-signed, capped/rate-limited); reviewed (blocking receive-cap DoS fixed) | ✅ `762ef63` |
| 6e-3d-8 | **advisory eclipse detector** (D/R/S, hysteresis, never gates) + **cross-session address cache** (tamper-detected); reviewed (blocking timing-oracle fixed) | ✅ `ca8493f` |
| 6e-3d-9 | **invite rewiring** (`rendezvous` vector, `INVITE_DOMAIN` v2) + pre-join **`join_ns`** + `serve --rendezvous`/`join` **discover→dial→join** end-to-end; reviewed SOUND | ✅ `f31a0c7` |
| 7a | **full-stack end-to-end over real TCP sockets**; founder binds an ephemeral loopback port; a fresh device dials it over real OS sockets, runs the MLS join, and converges | ✅ `798b50f` |
| 7b | **consolidated security suite**; threat-model → where-proven map + cross-layer scenarios (`an_eclipse_caution_never_gates_a_removal`, `a_removed_member_is_excluded_from_the_rotated_namespace`) | ✅ `ec5638e` |
| 7c | **rendezvous discovery bootstrap over real TCP**; joiner discovers the inviter under `join_ns` and joins with no hard-coded address, over OS sockets | ✅ `a168c1d` |
| 7d | **relayed full-stack join over real TCP**; server reachable only via a circuit relay; join + catch-up over the relayed connection (NAT traversal) | ✅ `0c2a6d8` |
| 7e | **DCUtR-upgraded full-stack path over real TCP**; a relayed join that hole-punches to a direct link (`next_direct_upgrade`), driven through a complete join + converge | ✅ `ff4c63f` |
| 8a | **`catcoms-app` product model**; UI-facing `Server` facade + canonical chat-message schema (the typed boundary the GUI is built against) | ✅ `1332051` |
| 8b-1 | **async event-stream actor**; `spawn(server)` → commands in / events out (ChannelUpdated, MembersChanged); the substrate the Tauri bridge drives | ✅ `c73929c` |
| 8b-2 | **Tauri 2 + Svelte desktop app** (`apps/desktop`); found/open/send/read over the actor bridge; both halves compile (WebView manually verified) | ✅ `7c5f72e` |
| 8c | **invite + join in the desktop UI**; found mints a single-use invite (loopback bootstrap); a second instance pastes it, dials, joins, and converges (two instances can talk over real TCP) | ✅ `61f2ec3` |
| 8d | **multi-channel**; name-addressed channels (`catcoms-app::channel_id`); channel-list sidebar + "join #channel" + per-channel view/unread in the UI | ✅ `d2ec4d3` |
| 8e | **member roster + chat polish**; live Members panel (device-id fingerprints + "you"), own-message bubbles | ✅ `e77a33d` |
| 8f | **member profiles (backend)**; `DocType::Profile` (tag 9) + a shared per-server profile doc `{name,color,font,effect}` keyed by device fingerprint; messages now authored by fingerprint (name/style resolved from the author's profile at render time); actor seeds/serves/converges profiles | ✅ `bcf61db` |
| 8g | **profile editor + rich rendering**; "Your profile" editor (name, color, font, animated effect); roster + message authors resolve fingerprint → profile (rainbow colour-wave / wave / pulse); own-message keys on local fingerprint | ✅ `612965f` |
| 8h | **member avatars**; `Profile.avatar` (inline bytes in the profile doc, `MAX_AVATAR_BYTES` = 64 KiB; base64 across IPC); UI canvas-downscales to a 128px JPEG; circular avatars in roster + messages with an initials fallback | ✅ `9e9b878` |
| 8i | **per-channel history catch-up**; the bridge remembers the join peer (`catchup_peer`); opening any channel catches it up from that peer (joiner side), so ad-hoc channels show backlog, not just live. Asymmetry: a founder opening a joiner-created channel still relies on live gossip (superseded by 8j) | ✅ `ec3fc90` |
| 8j | **symmetric (any-peer) catch-up**; `ChannelSync::request_catchup_best` (+ `now_ms`) catches up from the best known peer (proven member, else any known peer); `Server::request_channel_catchup_any` + actor `CatchUpAny`; bridge `open_channel` uses it (dropped `catchup_peer`). Either side gets the backlog of a channel the other created | ✅ `479da21` |
| 8k | **chat UX polish**; messages carry a clock-stamped `ts` (canonical schema + `ChatMessage.ts`, stamped via `ChannelSync::now_ms`); UI shows HH:MM + auto-scrolls to newest | ✅ `f202530` |
| 8l | **content-addressed blob fetch over the mesh**; `ChannelSync` holds a `BlobStore`; `KIND_BLOB_FETCH` request/response (members-only, responder-signed/bound, 16 MiB cap, **per-requester rate limit**; folded from adversarial review since blob is the strongest amplifier); `put/get/has_blob`, `request_blob(_best)`. Re-hashes served bytes vs the requested CID before storing (no cache-poisoning). Foundation for large avatars + fileshare | ✅ `e0c3c8e` |
| 8m | **avatars over the blob layer**; the profile doc stores the avatar's `avatar_cid` (not inline bytes); `set_profile` puts the blob, `profiles()` resolves the CID against the local store, the actor proactively `fetch_missing_avatars` (always-try, since the holder-peer is often only known after the profile arrives) and re-emits. Public `Profile.avatar` (bytes) unchanged, so bridge/UI untouched | ✅ `5bc31f4` |
| 8n | **fileshare browser**; per-server file index (`DocType::FileIndex`): `add_file`/`files`/`download_file`/`open_files`/`request_files_catchup` + `FileEntry`; actor `AddFile`/`Files`/`DownloadFile`/`CatchUpFiles` + `FilesUpdated`; bridge base64↔CID-hex; UI "Files" panel (upload/list/download). Blobs plaintext-at-rest, members-only; `seal_file` encryption-at-rest + chunked transfer deferred | ✅ `66b06ce` |
| 8o | **cross-network founding/joining**; bind `0.0.0.0`; founder advertises a reachable address (LAN/public IP, `host:port`, or relay-circuit multiaddr) in the invite; joining dials **all** bootstrap addresses. Same-machine/LAN/port-forwarded internet all work. Pure `tcp_port`/`build_advertised` helpers unit-tested | ✅ `2ba19d3` |
| 8p | **multi-server**; bridge `AppState` is a `HashMap<u64, ServerEntry>` (each its own `Server`/actor); every command takes a `server` id, every event is tagged with it (`actor_of` clones the actor out so the registry lock is never held across an await); `found`/`join` return `{server, channel}` + register, `leave_server` shuts down. UI: a Discord-style server rail with per-server `ServerState` (channels/active/unread/invite/dot), active-server data loaded on switch + tagged events | ✅ `37fc6e7` |
| 8q | **relay-circuit founding**; `found_server` gains an optional `relay` multiaddr; dials it, reserves a circuit (`listen_on(relay/p2p-circuit)`), and puts the relayed address first in the invite (mirrors `catcomsctl serve --relay`). Joiner unchanged (8o dial-all handles a relayed bootstrap). Zero-config NAT traversal with a relay node, no port-forward | ✅ `5bf3970` |
| 8r/8s | **security-review hardening**; adversarial review of 8m–8q (no blocking findings); `fetch_missing_avatars` per-pass bounded (8r), `MemoryBlobStore` size-bounded (8s); + a desktop [User Guide](USER_GUIDE.md) | ✅ `0d0f3cf`/`173d168` |
| 8t | **status feed**; per-server post stream on `DocType::Status` (reuses the message schema): `open_status`/`post_status`/`statuses`/`request_status_catchup`; actor `PostStatus`/`Statuses`/`CatchUpStatus` + `StatusUpdated`; bridge + a "Status" UI panel | ✅ `64d3812` |
| 8u | **wiki**; one per-server `DocType::Wiki` doc (map page→body): `open_wiki`/`wiki_pages`/`read_wiki_page`/`write_wiki_page`/`request_wiki_catchup`; actor `WikiPages`/`ReadWikiPage`/`WriteWikiPage`/`CatchUpWiki` + `WikiUpdated` (full-map change tracking); bridge + a Chat/Wiki main-pane toggle (page list + editor, dirty-flag preserves in-progress edits) | ✅ `e6091aa` |
| 8v | **char-level wiki merge**; page bodies are automerge `Text` (`update_text` diff-splice); concurrent same-page edits merge char-by-char. + reworked a flaky file-convergence test to deterministic request/response catch-up (MemNetwork emits no `PeerConnected`, so gossip/peer-discovery timing was racy; the blob fetch itself is tested at the sync layer) | ✅ `1f61599` |
| 9 | **disk persistence + encryption-at-rest**; designed in [`design-persistence.md`](design-persistence.md). **9a** key vault (passphrase-sealed `Dek`→`KeyHierarchy`) ✅ `2bbae5d`-prev, **9b** sealing blob store (encrypted at rest, plaintext-CID-addressed) ✅ `2bbae5d`, **9c** snapshottable MLS state (`snapshot_server`/`restore_server`; the pivotal slice; adversarially reviewed) ✅, **9d** doc persistence (`EncryptedDoc::snapshot`/`restore`) ✅, **9e** sync-state assembly (`ChannelSync::snapshot`/`restore`; MLS+docs+routing+ledger+commit_log+peer_records into one `Zeroizing` blob, restored onto a fresh transport; adversarially reviewed) ✅, **9f** vault-sealed `ServerStore` (`servers/<id>.bin`+`registry.bin`, atomic, wrong-passphrase-safe) + `Server::snapshot`/`restore` + actor `Snapshot` command + the desktop **passphrase gate** (`unlock` reloads each server onto a fresh transport) and **save-on-mutation**; close/reopen the app, enter the passphrase, servers + history are back (read offline) ✅, **9g** re-dial persisted peers on reload (`peer_addrs_from_snapshot` → fresh mesh bootstrap; a reloaded joiner reconnects to stable-address peers) ✅, **9h** per-file encryption: **9h-a** wired `SealingBlobStore`/`FsBlobStore` per server (files+avatars persist + sealed at rest under `blob_key`) ✅, **9h-b** stable per-group file-wrap key minted at founding + bundled into the join transfer; `seal_file`/`open_file` so files are e2e ciphertext keyed by ciphertext CID (adversarially reviewed) ✅. **Phase 9 complete.** | **✅ done (9a–9h)** |
| 10 | **desktop UI / product overhaul** ([plan](../../.claude/plans/moonlit-puzzling-karp.md)). **10a** tabbed nav (Chat·Files·Status·Wiki·Profile) + Settings overlay + invite placement, **10b** rich-text renderer (`render.ts`: `marked` + DOMPurify; `[[links]]`/`:emoji:`/`![cid embeds]` tokens; sanitizer allows no media/raw-HTML; placeholders only), **10c** fileshare folders (`FileEntry.path`, traversal-safe) + chat drag-drop media embeds + media resolver (builds `<img/video/audio>` in code from CID-verified blobs) **+10e** status media; *adversarially reviewed, 0 security findings*, **10d** wiki overhaul (markdown render + Read/Edit + `[[links]]` nav + backlinks via `get_wiki_map` + media + in-app help), **10f** custom emoji via the `emoji/` fileshare folder (picker + `:code:` render + Settings manage), **10g** notification sounds (Web Audio chime, Settings toggle), **10h** roles/permissions (`MemberRoles` doc tag 6; owner = MLS designated committer; admin grants + role-gated `mint_invite`; Settings→Server role manager); *adversarially reviewed; enforcement is honestly documented as **policy-layer/advisory** (admin grants forgeable by a modified client; owner-signed grants + committer-side join re-check are the named follow-up)*. All 16 user UI requests delivered. | **✅ done (10a–10h)** |
| 8… | **✅ rendezvous auto-discovery in the UI**; found registers at a zero-knowledge rendezvous; a joiner pasting that invite is discovered there and joins with **no hard-coded address** ([`design-rendezvous-ui.md`](design-rendezvous-ui.md), reviewed). **✅ chunked large-file transfer**; a file splits into chunks (each its own content-addressed blob) described by a `FileManifest`; the per-blob 16 MiB cap now bounds only a chunk (whole-file cap 256 MiB), the blob rate limit became a per-requester **bytes-budget**, and download reassembles + verifies the whole-file plaintext cid ([`design-chunked-transfer.md`](design-chunked-transfer.md), reviewed). **✅ post-join steady-state discovery**; after joining, a member periodically re-registers/discovers at the rendezvous under its rotation-aware namespaces and dials other members (re-finds the group after a restart, no fresh invite); `MeshTransport` extended (libp2p-free, default-inert verbs), a per-server bridge timer drives `AppCommand::DriveDiscovery` (real-time off the deterministic-time seam), persisted rz config ([`design-postjoin-discovery.md`](design-postjoin-discovery.md), reviewed). **✅ dedup-safe blob GC** (delete now reclaims a deleted file's orphaned chunk blobs, keeping any chunk another file references), **✅ download progress** (per-chunk `DownloadProgress` events → a UI progress bar; the whole-buffer-IPC/non-blocking-actor refactor stays deferred) **+ a Downloads tab** (per-server, newest-first list of queued/downloading/done/failed transfers + a "clear finished" action; shows the **live provider**; the signed responder that actually served each chunk, surfaced authenticated via `request_blob_best_provider` (the responder signs the request-bound, content-verified blob response, so the fingerprint is unspoofable), falling back to the uploader as the source), **+ file-browser availability** (each file is colour-coded by local availability; `●` on this device / `◐` partial _h/t_ / `○` downloadable / `○` no peers online; via a new `files_view` that counts held chunks per file + a cheap reachable-peer flag, zero network cost; refreshed on tab-open / files-updated / post-download), **+ channel viewer is now chat-only** (the channel list hides outside Chat; the roster stays), **+ live member presence** (`ChannelSync` now keeps an accurate `connected_peers` set; `PeerDisconnected` was previously dropped; surfaced as roster online dots + an "N online" count via `connected_member_fingerprints`, which matches each member by **its own** signed `peer_id` so a forged record can't steal another's presence; the availability hint's `has_peers` now uses this live set, fixing the staleness), **+ per-member presence detail** (the frontend tracks observed connect/disconnect transitions to show "Online · 5m" / "Last seen 5m ago" in the roster tooltip, member menu, and an inline last-seen; durations only for transitions actually witnessed this session, refreshed by a 60s tick), **+ DMs + friends (phase 1)**; a DM is a 2-person server flagged `is_dm` (a backward-compatible registry trailing block; the signed invite + network path are unchanged, so per-server unlinkability is preserved); a DMs circle on the rail opens a DM-home (friends/DM list + the conversation reusing the chat view), **New DM** founds a DM + surfaces its invite as a friend code, **Add friend** redeems a pasted code ([`design-dms-friends.md`](design-dms-friends.md), reviewed; no protocol/security change). **✅ phase 2: friends-list sortings**; a DMs-only `message_stats`/`dm_stats` (count + timestamps + distinct active days, no message text) drives sorting the friends list by **recent** / **most active** (msgs ÷ active days) / **reconnect** (volume × silence) / **A–Z**, with a per-DM last-message hint (reviewed). **✅ phase 3: in-band "Add friend"**; a roster action on an *online* member founds a DM and delivers its invite over the shared server via a new authenticated `KIND_DM_INVITE` request (membership+signature+freshness, like PEX; `from` = the verified signer, unforgeable; payload opaque/inert, validated only on accept; queue bounded+deduped+transient). The recipient sees a pending friend request (DMs-circle badge + a list) and accepts with one click; offline targets fall back to the friend code (reviewed: auth/no-spoof + inert-payload + DoS-bound all hold). **DMs + friends complete.** **✅ non-blocking download**; a large download no longer freezes the server actor: the bridge fetches the file **one chunk per actor command** (`file_download_plan` + `fetch_file_chunk`), so the actor returns to its loop between chunks and interleaves messages/sync; it reassembles, emits per-chunk progress, and verifies the whole-file content address bridge-side (reviewed; equivalent + integrity undiminished). **✅ eclipse `D` accuracy**; `observe_eclipse`'s reachable-devices now uses the live `connected_member_fingerprints` instead of the monotonic `member_peers`, so it stops under-warning after a node loses its peers. **✅ in-channel message search**; Ctrl+F (or the 🔍 header button) opens a search bar over the active conversation's messages; matches are highlighted, Enter/Shift+Enter (or ↑/↓) step through them scrolling each into view, with an _n / m_ counter; closes on Esc or a channel/server switch (frontend-only). **✅ advanced search**; the same bar gained a filter panel (Ctrl+Shift+F, or the "Filters (n)" toggle): **from** a member · **after**/**before** a local-day date · **has** image/video/audio/file/link (embeds classified by the fileshare index's MIME via `safeMime`, so a non-media or not-yet-indexed cid reads as a plain attachment) · **is** reply/has-replies/pinned/edited/mentions-me/from-me · **reactions** any/mine/a specific emoji; all AND-combined and usable with an *empty* query, plus a **sort** (oldest/newest/author A–Z/most reactions) that orders both the ↑/↓ stepping and a new click-to-jump result list (first 50 rendered, count disclosed). The match cursor is *clamped* rather than reset so a filter edit or an incoming message can't strand the highlight; the author/emoji pickers are lazy deriveds over the loaded messages, and the media regex only runs when a has-filter is active (frontend-only; still scoped to the loaded backlog). **✅ advanced search, round 2**; search became **server-wide**: an **In** scope (this channel / all channels / a specific one) builds a *corpus* of `{channel, index, message}` hits, fetching each non-open channel once via the existing `get_messages` into a snapshot dropped when the search closes (the open channel always reads the live `messages`, so it can't go stale). A hit in another channel is reached by clicking it or stepping onto it: `switchTo` gained a `keepSearch` flag (and is now `async`, awaiting `refresh`) so the jump lands by **message id** in the freshly-loaded channel, with the outgoing channel snapshotted first so its hits don't blink out mid-switch; *refining* a query never jumps channels, only ↑/↓ and clicks do. **From** and a new **Mentions** filter are member **typeaheads** (roster ∪ corpus authors, ↑/↓/Enter/click, emptying the box drops the filter); mentions match the `@[Name]` marker via the shared `mentionName` normalizer, so a since-renamed member matches under the name they were mentioned by. Also: **Today/7d/30d** date shortcuts, **case-sensitive** + **whole-word** match modifiers (the latter bounds on non-word-or-edge, since `\b` misbehaves on a punctuation-edged query), a **most replies** sort, and reply counts computed over the *corpus* (not just the open channel) so "has replies" and that sort stay right server-wide. Result rows carry the channel; the sorts key on timestamp rather than corpus position, since a multi-channel corpus is grouped by channel. **✅ edit + delete your own messages**; messages now carry a stable random `id` (list indices are unstable under CRDT merges); a member can edit (inline, with an "(edited)" tag) or delete its own messages via `Server::edit_message`/`delete_message` (a soft own-author gate; honest-client-only, the documented R6 residual since message content isn't authenticated). The change-detector switched from message-count to a content signature so an edit (count unchanged) refreshes everyone (reviewed: CRDT ops merge-safe, no empty/stale op). **✅ message moderation**; owner/admin can delete *any* member's message (not just their own; edit stays own-only), honest-client gated like file deletion (R6); offered in servers, not DMs. **✅ jump-to-unread**; a per-`server:channel` read mark (localStorage) renders a "New messages" divider + an "↑ New" jump button; the mark advances to the latest once seen. **✅ emoji reactions**; toggle a reaction on any message (quick-picker + right-click "React…"); chips show counts and highlight your own. Stored as flat scalar keys `"<emoji>\x1f<fp>"=true` written **directly on the message map** (no sub-object), so concurrent reactors write distinct keys that all survive a merge; no concurrent-create loss for *any* message, legacy included (5-lens adversarial review → this superseded an earlier pre-created-container design that still lost reactions on old-client-authored messages; a two-replica fork/merge convergence test pins the invariant; emoji validated at the trust boundary). The content signature folds reactions so a peer's reaction refreshes everyone. **✅ reply / threading**; reply to any message (right-click → "Reply" or the composer banner); messages carry an immutable `reply_to` parent-id (written only when it's a reply, so plain messages stay key-clean; no concurrency hazard, it's set once at creation), rendered as a clickable parent-quote that jumps to + flashes the original (degrades to "original message" if the parent isn't loaded). `Server::send_reply` threads it; `send_message` stays a 2-arg delegate (no test churn). Reviewed: sound, backward-compatible, all dangling/lifecycle paths degrade gracefully. **✅ @mentions + reply notifications**; type `@` for a member autocomplete that inserts an `@[Name]` marker (frontend-only; mentions ride in message text, no CRDT change), rendered as a highlighted chip via a new `marked` tokenizer (DOMPurify-sanitized) with a stronger self-highlight; a sidebar `@` badge marks any active-server channel with an unseen message that mentions you or replies to one of yours (scoped to the active server, where your per-server identity is known; cleared on read). Insertion + detection share a `mentionName` normalizer so odd names round-trip. Reviewed: no XSS, the mid-fetch server-switch race guarded, name-based matching is best-effort by design. **✅ custom-emoji reactions**; the reaction picker also offers the server's custom `:name:` emoji (the `emoji/` fileshare folder), and reaction chips render a custom emoji as its image (graceful `:name:` text fallback where the emoji file isn't held); backend unchanged (it already accepts any emoji string). **✅ cross-server inbox**; a dedicated rail icon (📥) opens its own screen listing every message that @-mentions you or replies to one of yours, across **all** servers/DMs, newest first, each showing who/where/when + a one-click jump (with unseen highlighting + a rail badge). Backend-driven: `Server::inbox` scans each server's channels in-process and resolves author names (per-server identity); the bridge `get_inbox` aggregates under a lock-free actor snapshot; a 1.5s-debounced reload keeps it live. The backend reuses the UI's exact `@[Name]` normalization (`normalize_mention_name`) so detection matches insertion. Reviewed: no blocking, the marker-normalization divergence + jump-to-unlisted-channel + timer-leak all fixed. **✅ reply-count thread affordance** (a "💬 N replies" chip under any message that has replies, jumping to the first) **+ distinct mention chime** (a brighter rising triad when a message mentions/replies to you, vs the two-note chime for ordinary messages; wired into both the open channel and the per-channel scan). **✅ message pinning**; owner/admin can pin/unpin any message (honest-client gated, R6); a 📌 marks pinned messages inline and a header "📌 N" opens a panel listing them with jump-to/unpin. Stored as a `pinned` flag **directly on the message map** (merge-safe like the reactions design; concurrent pins of different messages can't conflict, a pin/unpin race is clean LWW); the change-detector folds it so a peer's pin refreshes everyone; an idempotent guard avoids a redundant op. Reviewed: ship, no blocking. **✅ rich composer**; `||spoiler||` tags (a new `marked` tokenizer rendering a blacked-out span revealed on click, DOMPurify-allowlisted), a composer **formatting toolbar** (bold/italic/strike/code/spoiler that wrap the selection, + Ctrl+B/Ctrl+I), and **per-channel drafts** (in-memory: switching channels/servers preserves what you'd typed, cleared on send). **✅ message-action UX fix**; edit/picker were rendering on every legacy (empty-id) message because `editingId/reactionPickerFor === ""` matched `m.id === ""`; now gated on a truthy id, plus a Discord-style hover toolbar (react/reply/⋯-more) on each message. **✅ bug fixes (user-reported):** profile name/styling reverted on reload because `spawn` *unconditionally* re-seeded the profile from the founding display name; now seeds only when absent, so a restored profile survives (regression-tested); `saveProfile` also keeps the rail label in sync when it was still tracking your name. Inline media (status/chat embeds + custom emoji) vanished after a tab switch because the resolution `$effect` didn't track `view` (tab switch destroys+recreates the DOM with fresh, unresolved placeholders); now re-resolves on `view` change (cheap; the embed cache holds the decrypted bytes). The file-info preview no longer hangs on "Loading preview…" forever; a failed fetch now surfaces "preview unavailable". Composer: emoji button moved right, the inline formatting toolbar replaced by a Settings → Message-formatting help section (Ctrl+B/I kept). **✅ emoji/sticker size**; custom emoji can be created at a chosen size (Emoji/Medium/Large/Sticker, capped 160px), encoded as a `~<px>` suffix in the emoji's filename so it's shared with everyone (no backend change); inline `:code:` renders at that size, reactions/pickers stay small. **✅ profile cards + customisation**; clicking a member's avatar/name opens a profile card (avatar, styled name, role, a self-set **description/bio**, an Add-friend button for online members); the Profile gained `description` + `bubble` fields (CRDT, additive/backward-compatible). The **message bubble** is now customisable per member (color/gradient presets), applied to that author's messages; the value is sanitized (colors/gradients only, no CSS injection) and the description renders as escaped text. **✅ discovery record-seq surfacing** (real anti-replay freshness) **+ advisory `EclipseDetector` surfacing** (isolation banner; never gates). Remaining: AddressCache persistence · true streaming download · TTL-aware re-registration | rendezvous + chunking + post-join discovery + final polish **done**; rest planned |
| 10+ | roles hardening: **owner-only member removal PROTOCOL-enforced** (`request_remove` rejects a non-owner; the committer ignores any inbound remove request not from the owner; THREAT-MODEL R1 closed) + **owner/admin file deletion** (`delete_file` role-gated; reviewed); DONE. **✅ Functional admin invites (Option C, owner-serialized)**; an admin broadcasts a signed `CTRL_ADD_REQUEST`; the **owner alone** runs the MLS Add (single committer → no fork) + relays a re-signed Welcome (joiner verification unchanged); offline-queued until the owner is online ([`design-admin-invites.md`](design-admin-invites.md), reviewed, no blocking findings). **✅ Replay-proof grant revocation (THREAT-MODEL item 3)**; authoritative admin set is **owner-local** (`ChannelSync::admin_roster`, persisted); the admission gate reads it (a malicious member can't write it), the CRDT `roster` is owner-signed display-only ([`design-grant-revocation.md`](design-grant-revocation.md), reviewed). UI now lets **admins mint invites**. Remaining: file-delete protocol gate (low stakes) · sticky/transferable ownership · blob GC after delete. Do **not** enable `max_committer_rank ≥ 1`. See [`THREAT-MODEL.md`](THREAT-MODEL.md). | **✅ done** |
| 10++ | **embed-persistence fix** + **file info pane** + **feedback button**: inline image/emoji embeds vanished after a restart/HMR; the resolve `$effect` ran before `{@html}` committed its placeholders and never re-ran; fixed with `tick()` (+ a dev-HMR `unlock` guard against duplicate actors). Clicking a file opens an **info pane** (preview · local-availability via `file_available`/`has_blob` · uploader/size/type/folder/cid · Download · owner/admin Delete). A 💬 rail button composes a bug/feature report to the clipboard (serverless, so copy-and-share). | ✅ `dd44446`, `8d9e371` |
| 10+++ | **chat layout polish**; chat is edge-to-edge (no bordered box / distinct background, trimmed channel padding; bubbles float on the app background) and the bubble presets were re-picked dark enough for white text (+ a text-shadow on custom bubbles). Frontend-only | ✅ `43457ff` |
| **11** | **GROUP VOICE**; E2E real-time audio, design in [`design-voice.md`](design-voice.md). See [§ Voice](#voice-group-calls) | ✅ 11a–11e (phases 1–3 of the design; phase 4 planned) |
| 11a | **voice phase 1; crypto + signalling core.** `MEDIA_EXPORTER_LABEL` + `ServerGroup::media_secret(call_id)` derive a 32-byte per-call key from the MLS exporter at the current epoch (every member derives it **locally**, never on the wire; distinct calls → distinct keys), surfaced `ChannelSync::media_key` → `Server::media_key` → actor `MediaKey` → bridge `call_media_key`. New authenticated push `KIND_CALL_SIGNAL` (= 8) mirroring `KIND_DM_INVITE`; members-only, Ed25519-signed, freshness-bound, `from` = the verified signer; payload **opaque** to the core, **not** deduped (every ICE candidate must arrive), FIFO-bounded (`MAX_PENDING_CALL_SIGNALS`). Actor drains per loop → `CallSignal` event → bridge `call-signal` (base64) | ✅ `bd483b5` |
| 11b | **voice phase 2; WebRTC mesh + call UI.** Full mesh (`RTCPeerConnection` per pair, no server in the media path → DTLS-SRTP is end-to-end); SDP/ICE ride the 11a authenticated push, so the DTLS fingerprints **can't be MITM'd**. Protocol: start → "ring" online members; accept → "hello" → existing participants "offer"; "answer"/"ice" per edge; "bye" tears one down (a newcomer auto-meshes with everyone). UI: header 📞 Call, a floating call bar (participant avatars, mute, leave) surviving channel/tab switches, an incoming prompt | ✅ `52a64e2` |
| 11c | **voice NAT traversal**; configurable ICE servers: STUN on by default (hole-punch across most home NATs), optional TURN (relays still-SRTP-encrypted audio when no direct path exists; TURN can't decrypt). User-editable in Settings → Calls, persisted locally (blank STUN = LAN-only). Call bar shows live status ("connecting…" / "N connected" / "check NAT/TURN"). Note: **signalling still rides the mesh**, so the members must already be mesh-connected | ✅ `1e2a698` |
| 11d | **channel-scoped voice rooms + presence + notifications**; a room is per **channel** (the channel id doubles as call id **and** media-key id); participants heartbeat ("voice-ping"), everyone tracks `{server:channel → {fp: lastSeen}}` with a staleness timeout + cleanup tick, so each channel shows a live "🔊 N in voice" pill and the header reads "Join voice (N)". A room you're *not* in going active raises a banner + chime, gated by a per-server "notify me of voice calls" toggle. Frontend-only | ✅ `b93164e` |
| 11e | **server-provided TURN**; the operator sets one TURN endpoint (Server settings) that rides the invite as a `.turn.<b64json>` suffix, stripped by the joiner before the bare hex reaches `join_server` and stored per-server in localStorage; `iceServers()` merges it with the user's personal STUN/TURN. Frontend-only, **no protocol change**: TURN is a non-secret hint (media is E2E DTLS-SRTP, so a hostile TURN relays only ciphertext or the call falls back), so it needs no signing and doesn't touch invite crypto | ✅ `7492f92` |
| 11n | **networking QoL (historical first slice)**; best-effort **UPnP IGD** (`upnp` feature + `MeshBehaviour.upnp`) plus stable relay/rendezvous identities. The original labels incorrectly said NAT-PMP was included; it was not until 11n++. | ✅ `bf28db9`, `f317c5c` |
| 11n+ | **AutoNAT v2 + honest diagnostics.** `MeshBehaviour` requests nonce-verified callbacks; relay/rendezvous swarms contain an experimental opt-in server, with 64 pending callbacks plus exact-source direct-target and peer/prefix/node pre-dial policy. Explicit advertised public direct candidates are tested. The desktop retains/ranks bounded per-address/server evidence for founders, successful joiners and reloads, prunes withdrawn routes, excludes relay circuits from direct status, reports failed joins accurately, and never widens one address/server/moment into universal reachability. No public infrastructure is bundled or defaulted, and pairwise/recurring state plus automatic relay escalation remain open. | ✅ updated 2026-08-21 |
| 11n++ | **PCP/NAT-PMP + PCPv6 + live Connectivity assistant.** Pinned `portmapper` 0.18 clients probe/map stable IPv4 TCP and UDP/QUIC ports without duplicating libp2p's UPnP leases. A separate narrow RFC 6887 MAP client requests five-minute IPv6 firewall pinholes on the exact listener/interface gateway, honors assigned lifetimes up to 24 hours, and uses monotonic renewal/expiry. Bounded authoritative snapshots key family/interface ownership separately, reference-count duplicate/manual owners, reject non-public results with their reason, retry failed probes/MAP requests, remove expired mapping-derived and relay routes from the swarm/live bootstrap/peer record, prune stale AutoNAT evidence, refresh the next displayed invite, and update onboarding + Settings live. An identical configured GUA remains an unverified candidate after its pinhole expires. The shared UI includes typed one-time-help and standing-switchboard states. Already copied signed invites are immutable; no mesh STUN path exists. A granted pinhole remains unverified until an address-scoped AutoNAT callback. | ✅ updated 2026-08-21 |
| 11n+++ | **Two-way replies + member switchboards.** A failed direct join can emit a MAC-bound, four-candidate, 60-second reply; every callback proves possession before receiving the bearer invite/KeyPackage, replacement is explicit and actor capabilities are generation/replay bounded. Per-server standing hosting is default-off: current members publish two-minute signed offers, fresh `mewtual-invite-v3:` envelopes carry each complete helper-signed offer under the inviter endorsement (so the inviter cannot replace routes or extend consent), and onboarding requires consent before helper dials. A helper must have a live exact peer-record route to the named inviter, forwards only bounded admission frames, and applies the exact MLS Add before it becomes the joiner's first sync path. It does not admit or provide a general circuit relay. This helps established groups only; it cannot connect the first mutually unreachable pair without a direct/public/relay route. | ✅ implemented and adversarially reviewed 2026-08-21 |
| 11u | **desktop UI overhaul; tokens-first "operator terminal" redesign + user customisation.** A declared **token layer** replaced ~90 hardcoded hexes (the old CSS referenced `var(--accent, …)` etc. but *never declared them*; two palettes shipped at once); default preset **Nightshade** (purple-shifted slate) + `aurum`/`verdant`/`garnet`/`slate`, semantic colours have fixed jobs in every theme (green=presence, gold=mentions, red=danger). Reskin: flat **timestamp-gutter** message log (day dividers; grouping never crosses midnight), mono micro-labels, dedicated member column (online/offline groups, role abbrevs), global **status bar** (node/peers/vault/rendezvous/transfers/own-id), squircle rail with hand-drawn **SVG line-icons** (emoji chrome fully replaced; found+fixed `.call-start`'s green never applying under `button.ghost` specificity; the 📎 stays by request). Nav: **surfaces strip** atop the content column + **contextual sidebar** (chat=channels · wiki=pages · files=folders+actions · transfers=clear) killing the wiki double-sidebar; **Ctrl+K quick switcher**. Customisation: Settings → Appearance (preset, accent override, compact density, terminal-chrome scanlines, flatten-bubbles, flat-icons) persisted in `catcoms.appearance`; Discord-style **name styles** (gradient/neon effects, script/caps fonts, swatch picker; `fxClass` now **sanitizes peer-supplied effect ids** before they reach a class attribute); default unicode emoji under the server set in the picker. Frontend-only | ✅ `ba8c20a`, `4ff7b99` |
| 11v | **server livery + shared server icon** ([`design-livery.md`](design-livery.md)); owner/admin publishes a colour scheme members inherit; per-server user opt-out. `DocType::Livery = 10`, one CRDT doc per server mirroring the Profile path end to end (lazy open, doc sync + **snapshot catch-up**, generic persistence); writes owner/admin-gated at the same policy layer as roles; sizes capped (`MAX_LIVERY_*`); values stored opaquely and **validated client-side** (preset allow-list, `#rrggbb`, colour-token allow-list; recolor-only by construction, semantics untouchable, no URL-shaped values). Precedence: user per-server opt-out > livery > own appearance. **Server icon** rides the same doc (additive `icon` key, 64 KiB cap, own `set_server_icon` command; `set_livery` is a read-modify-write that preserves it); rail shows it live via `livery-changed`, viewers can prefer monograms ("flat server icons"). Round-trip + icon-survival tests | ✅ `9d128fb`, `1bea14a` |
| 11w | **verify dialog + channel topics.** Out-of-band **identity verification** surface (the eclipse banner's "verify a member out of band" finally has UI): both fingerprints in read-aloud 4-char groups, explicit wording, **local-only** verified marks (`catcoms.verified.<server>`, never gossiped, no crypto weight) → ✓ in roster/profile. **Channel topics**: there is *no backend channel registry* (id = BLAKE3(name), the list is frontend-local), so the topic is a ROOT **LWW scalar in the channel's own doc**; replicates/seals/catches-up like messages, 256-byte cap, **any member** may set (channels are open-create; a topic is content), rides `channel-updated`; header click-to-edit UI. Tests: topic round-trip/cap/multibyte, two-node convergence with a non-owner writer, sealed-store reload. Delivery-states design written ([`design-delivery-states.md`](design-delivery-states.md)): sync-derived (`their_heads` ⊇ op hash), **no read receipts by design**; D1–D3 in progress | ✅ `1757732`, `3c19d07` |
| 11x | **delivery states + channel topics + member badges.** *Delivery* ([`design-delivery-states.md`](design-delivery-states.md)): signed causal evidence remains the compatibility path, augmented in 2026-08 by an authenticated connected-only receipt for an exact bounded document/change target, so quiet recipients confirm without authoring a reply; evidence is positive within the current roster (counts can fall when membership changes) and **no read receipts exist by design**. Actor keeps a bounded id→hash map, emits a complete bounded `delivery-changed` snapshot (≤1/s/channel); UI replaces omitted/changed rows so stale holders cannot stand in for a new roster. Gutter ticks `✕ ◌ ~ ✓✓` (red only for "no peers reachable"; partial says only how many peers hold it; the double tick requires the whole roster) keep honest hover copy. *Topics*: no channel registry exists (id = BLAKE3(name)) so the topic is a ROOT LWW scalar in the channel's own doc (256 B, any member, `channel-updated`), click-to-edit in the header. *Badges*: `DocType::Badges = 11`, owner/admin-assigned `fp → {label, color}` chips (roster/profile/role-manager + inline editor); role names reserved backend-side, ignored client-side. Badges re-key to user ids under [`design-multi-device.md`](design-multi-device.md) M3 | ✅ `5b97524` `77f184d` `3c19d07` `1bc07ed` `790b5a2` |
| 11y | **safe livery customisation + events/news + event refs + unlock minigames.** *Customisation* ([`design-livery-customisation-safety.md`](design-livery-customisation-safety.md); raw HTML/CSS is RCE/overlay-phishing in a `csp: null` WebView; **rejected**, incl. for profiles): radius/font/pattern as **catalog ids** in the existing bounded tokens map (client validates per key) + **custom cursor** as inline re-encoded image bytes (own `set_server_cursor`; `set_livery`/icon/cursor mutually preserving, test-pinned; read-side deep validation incl. a minimum-opaque-area anti-griefing floor; `, auto` fallback always). *Events*: the reserved `DocType::Calendar = 4` finally lands; status-path mirror, any-member create, author/owner-admin delete, ⧗ surface (Ctrl+7) + sidebar next-5; **news feed** = inbox Mentions｜News toggle aggregating upcoming events + recent status posts across servers client-side (wiki joins once saves carry timestamps). *Event refs*: `[title](event:ID)`; the "+" picker's fourth kind, seam-tested against the renderer grammar. *Unlock minigames*: Passphrase (recommended) ｜ Spell (24 glyphs, indexed) ｜ Melody (pitch-class piano: on-screen, DAW home row, **Web MIDI**); every method encodes to a scheme-prefixed string into the **unchanged vault KDF**, with a live entropy meter (red <28 / gold <44 / green ≥44 bits). CSP hardening for the WebView remains a named follow-up | ✅ `f253930` `e10515c` `c9a4e66` `e16efc4` `405e896` `5e73e5e` |
| 11z | **multi-device M1+M2; pairing primitives + the grant ceremony** ([`design-multi-device.md`](design-multi-device.md) v2.2; **adversarially reviewed pre-commit**, BLOCKING findings fixed). Model per owner review: the **origin device is the identity root** (no account key; chain depth 1; master transferable-not-distributable via monotonic `MasterHandoff`), one device per single-use grant. M1 (`f6b7386`): `PairingRequest` / 6-digit **SAS** (domain-separated BLAKE3, bias < 2⁻⁴³) / `DeviceCertificate` + `DeviceRevocation` (carry-the-pubkey verification mirroring `InviteToken::verify_self`; names reject control/bidi/zero-width). M2 (`fe618d3`): the **offline-first paste ceremony**; begin → read (backend stores THE pending ceremony; **mint takes no blob**; TOCTOU closed, the human gate exists backend-side) → SAS-gated popup (pre-mint comparator = **device code**, SAS = post-delivery check; **scope disclosed**; decline **burns the nonce**) → passphrase-sealed all-server bundle (vault primitives verbatim + distinct HKDF label; ≥ 8-char transport passphrase; the sealed bundle is the only object ever linking per-server identities) → open (every cert verified FOR this device; certs **group-bound in the signed payload**). Per-server signing via a narrow `SignDeviceCert` actor command; keys never surface. Dead v1 account-key `cert.rs` (709 lines, zero users) deleted; `/v2` domains prevent cross-verify. M3 (admission via the owner-serialized queue) in progress | ✅ `e44bfe3` `cd8e300` `f6b7386` `fe618d3` |
| 11z-2 | **multi-device M3–M6; admission, attribution, revocation, carry channels** ([`design-multi-device.md`](design-multi-device.md); M3 **adversarially reviewed**, 3 BLOCKING findings fixed). **M3+M4** (`ba8a8d1`): a companion joins by presenting its group-bound `DeviceCertificate` through the owner-serialized add queue (`CTRL_DEVICE_ADD`, single committer → no fork); the owner-signed `Devices` doc (`DocType::Devices`) gives every member the companion→origin map for attribution; UI nests companions under their member with a mono device tag, owner device panel, "join granted servers" flow. Review fixes: the `Devices` doc entry now carries the **owner's signature** (a certificate proves an origin *wanted* a device, not that the group *admitted* it; an unsigned entry can't poison the depth-1 gate or spoof attribution); the relay path **authenticates before republishing** onto the control topic; a **per-origin device cap** bounds owner-executed Adds; asymmetric freshness; the invite self-gate treats an unreadable roster as "relay, let the owner decide". **M5** (`cca00bb`): `revoke_device` (origin-signed `DeviceRevocation`, owner-enforced MLS Remove, honoured only when the origin matches the companion's *registered* origin so A can't evict B's device) + `remove_member` **cascades** to a kicked member's companion leaves. **M6** (`bcd5e17`): QR + a hand-rolled acoustic FSK modem carry pairing blobs (and invites), both unit-tested. **Deliberate deferral:** `MasterHandoff`'s primitive is committed but inert; consuming it (per-group master state + monotonic seq) is future work; the common flows don't need it. | ✅ `ba8a8d1` `cca00bb` `bcd5e17` |
| 11z-3 | **melody lock, engraved; chords + note values + playback** (client-only; the vault KDF stays untouched). The melody minigame graduates from a note-name chip list to a real **grand staff** rendered from `melody.ts` (pure, unit-tested): diatonic step placement (C♯ shares C's line and carries an accidental), auto-ledger lines with middle C in the gap, second-interval head offsets, stem direction per staff, note-head shape/flag by duration, chord symbols over the bar, and a viewBox that grows to whatever register the tune reaches. Input becomes **held**, not triggered: keys sustain while down (on-screen, home row, and Web MIDI note-off is now handled), overlapping notes collapse into **one chord event**, and hold time quantises to eighth/quarter/half/whole. **`melody:v2` → `v3`**; `60+64+67.2-62.0` (chord tones joined with `+`, ascending and de-duplicated so fingering order cannot fork the secret; the `.N` duration class is omitted *entirely* when rhythm is off, so the two modes can never collide). v2 joins v1 in retirement: a vault sealed under either must be re-entered under a scheme this build can still produce. Rhythm is **opt-out** (persisted locally) because it is the one setting that can lock a correct player out of a vault with no recovery path; the entropy model stays deliberately pessimistic (+2 bits per extra chord tone, +1.5 for a duration class). Also: **1–7** jump register and z/x blip the new bottom C, ▶ playback of the *recorded* durations with the sounding event lit on the staff, and the piano keys are finally ivory and ebony. | ✅ |
| 11z-4 | **wiki overhaul; per-page md/wikitext, auto-contents, page tools.** The wiki grows Wikipedia's bones without losing the friendly path: each page declares a **render format**; `md` (default) or `wiki` (a MediaWiki-wikitext subset); toggled per page in the editor and **shared with every member** (stored in the wiki CRDT under a reserved NUL-prefixed root meta key `"\u{0}meta"`, a Map so it's *invisible to older readers* whose `read_wiki_map` only materializes `Text` values; name validation now rejects NUL-prefixed/>120-char pages). Backend: `wiki_meta`/`set_wiki_page_format`/`delete_wiki_page`/`rename_wiki_page` (rename = copy+delete; a concurrent edit on the old key loses, documented) + actor commands with error replies + 4 bridge commands; the wiki change-detector compares **bodies and formats** (a toggle is body-invariant); meta reads/deletes span **all conflicting meta maps** (`get_all`) since two members lazily creating the map concurrently would otherwise silently drop one side's formats; merge test pins it. Renderer: new pure `wikitext.ts` (no DOM; node-testable; `== headings ==`, `'''bold'''`/`''italic''`, `* / #` nested lists, `; :` definitions, `{\| \|}` tables + caption, leading-space pre, `<nowiki>`, http(s)-only external links, `{{templates}}` inert) emitting the **same placeholder vocabulary** as the markdown path (render.ts now imports its token builders; byte-identical by construction, same DOMPurify allow-list +`caption`/`dl`/`dt`/`dd`), fuzzed 60k hostile inputs; **piped `[[Page\|label]]` links everywhere** (chat/status/wiki); `#REDIRECT [[Target]]` + `__TOC__`/`__NOTOC__` helpers. UI (matched to the operator-terminal chat chrome): article view with title rule, **auto-Contents box** (3+ headings, hierarchical numbering, hide/show), hover-a-heading **section-edit jumps**, redirect-following with a "Redirected from" notice, backlinks as **What links here**; editor gains the md/wiki switch, a format-aware toolbar (B/I/H2/H3/link/lists/table, Ctrl+B/I/S), **live side-by-side preview**, and per-page in-memory **drafts** (following a link no longer discards unsaved edits); page header rename + two-step delete; sidebar `wt` badge. 124 frontend + 79 catcoms-app tests green | ✅ |
| 11z-5 | **sigil lock; multi-factor magic circle; `spell:v1:` RETIRED** (client-only; the vault KDF stays untouched). The spell minigame is **deleted**; `spell:v1:` can no longer be produced, so a vault sealed under it must be re-entered under a surviving scheme, the same retirement contract as melody v1/v2; `UnlockMethod` is now `"pass" \| "sigil" \| "melody"`. Its replacement is one screen, freely re-editable factors assembling into a single SVG magic circle: a **path over a fixed 19-node lattice** (centre + inner 6 + outer 12; **indices frozen forever**, geometry cosmetic; hard-snap hit-testing; disjoint catch discs *smaller than the node art* give dead-zone hysteresis, pinned by `min-spacing > 2·CATCH_R`; order + direction significant, multi-stroke via pointer-lift with `_` separators), optional **per-node colour marks** (4 variants cycling on tap, fully independent of the path; tap-vs-drag split by a pure `classifyGesture`; one node within `TAP_SLOP` = mark, a second node = stroke, a long one-node wander = nothing; each variant a distinct **shape** as well as hue for colour-blind users; keyboard `C` cycles), a **focus-emoji SET** (select/deselect toggle, ≤ 8; each element codepoint-encoded; lowercase hex `-`-joined, never catalog index; NFC, VS16 + skin tones ALWAYS stripped, ZWJ kept; the set is **canonically sorted + de-duplicated** so toggle order can't fork the secret; the `normalizeEvent` lesson; `+`-joined like chord tones), and a **magic word** (NFC + trim, case preserved, **length-prefixed** so a delimiter inside the word can't collide two secrets). Wire format **`sigil:v1:<path>:<colours-19>:<emoji-set>:<len>:<word>`**; colours a fixed-width 19-digit 0–3 field, all-zeros encoded literally; `""` when path/emoji/word missing (marks optional). ⚠ **v1 amended IN PLACE** (emoji field became a set, colour field inserted); legal only because sigil:v1 never sealed a committed vault; **any test vault sealed with the earlier working-tree build will not reopen.** The ring inscription leaks NOTHING: a **constant-count** rune band (42, full circumference; per-character runes leak length, tiled repeats leak it via the period) derived from `(session seed ⊕ fnv1a(word))`, reseeded per mount, so typing reshuffles it visibly but a photo recovers nothing; opt-in "show my word". Entropy stays pessimistic (2.5 bits/hop; Android-pattern bias; 6 first emoji + 3/extra cap 15; same popular head, correlated picks; word 4+2/char cap 20; marks 1.5/non-default node cap 12). The **cat summon (≤900 ms) runs concurrently with the KDF** and aborts on failure; the particle rAF loop stops on unlock/teardown/`visibilitychange`; `prefers-reduced-motion` skips both. `sigil.ts` pure + node-tested (42 tests) mirroring `melody.ts` | ✅ |
| 11z-6 | **fileshare pass; upload dedup · circulation expiry + wiki pin · Properties · toasts · wiki "+" picker** (follow-up to 11z-4 after owner feedback: "attaching doesn't seem to work", wanted toasts, the chat "+" in the wiki, no-reupload on same hash, wiki files exempt from decay, expiry + used-in + right-click Properties). *Attach fix + toasts* (`074a058`): the wiki attach pipeline was sound but **silent**; the marker appended off-screen at the textarea bottom with no confirmation; embeds now insert **at the caret** (edit-mode auto-switch, focus restored) and a **toast stack** (info/ok/err, in-place morphing) narrates every upload path (wiki/chat/status/files) + wiki save/rename/delete/format, failures loud with the real error. The composer's **"+" insert picker became a shared snippet** with an `insertTarget` routing to the chat or wiki caret; it opens from the wiki toolbar (drops down; composer's drops up) alongside a relocated 📎. *Upload dedup* (`5d2dc0e`): `add_file` computes the plaintext cid first; same name+normalized-folder → idempotent no-op returning the existing cid; different name/folder → one new index entry carrying the twin's encoded `FileManifest` **verbatim** (same ciphertext-cid chunks + wrapped keys → zero new storage; ciphertext identity can never dedup since seal_file is randomized per chunk). Backend-only `delete_file_at(cid, path)` unlists one listing (public `delete_file` still unlists all); dedup-safe GC composes (shared chunks survive until the last listing goes). Tests prove blob inventory unchanged AND manifests byte-identical. *Expiry + pin + usage* (`8461d8b`): `FileEntry.expires: FileExpiry{Unrecorded\|Never\|At(ms)}` (doc key `"exp"`: absent=legacy, automerge **explicit Null**=keep-forever, int=deadline; three states survive a merge), stamped `now+30d` on both add paths (dedup relistings stamp fresh); `set_file_expiry` per-listing (uploader/owner/admin R6 gate, proven-as-gate in tests); `wiki_pinned_cids()` **derived** from live wiki bodies (both `](cid:` and `](file:` grammars); un-pins when the page drops the embed; `file_usage(cid)` inbox-style scan → wiki pages + status/chat counts. **HONEST SCOPE: metadata + surfacing only; `RetentionIndex` is still unwired, nothing evicts yet**; `wiki_pinned_cids` carries a MUST-consult note for the future retention GC. UI: Properties gains **Circulates until** (pinned → forever → date `· in 30 days` → `not recorded`) + plain-language non-deletion note, **Used in** (clickable wiki pages, chat/status counts), **Keep forever** toggle, **right-click → Properties on any embed** (chat/status/wiki share the context path), Files-tab 📌 on pinned rows. Known tradeoffs doc-commented: dedup'd listing inherits first upload's mime; per-device index view; dedup of never-downloaded content lists chunks held elsewhere. 91 catcoms-app + 150 frontend tests | ✅ `074a058` `5d2dc0e` `8461d8b` + UI in tree |
| 11z-7 | **360 server space (orbit view); client-only.** A memory-palace overlay over the rail (mockup-approved on the shared design canvas): servers hang as billboards on a sphere around a rotation-only camera (yaw wrap + pitch clamp ±60°, `space.ts` pure + node-tested: project/unproject round-trip, lasso capture, group-offset carry, defensive store parse). **No WebGL**: the backdrop is a CSS 3D cube (4 SVG walls + floor/ceiling, tokens-only so presets/accent recolor the room; `den` ships the sleeping mascot on the windowsill) and icons are JS-projected onto a flat layer with the **same focal length** (`spaceF`, window-derived) so the layers never drift. Gestures: drag looks (grab semantics), **press-and-hold grows a lasso** from the cursor (one gesture for 1..N servers; capture → the constellation rides the aim as angular offsets → click drops; capture-phase click swallow keeps the drop from opening a server; pointer capture starts only when a drag/lasso commits so plain clicks reach the buttons), tap opens (`switchServer` folds the view), right-click → return-to-tray. **Tray** = hold `[T]` (keyup/blur-safe) or pinned via the hotkey chip: unplaced/new servers wait there; tap flies one to the reticle. **State reads**: unread/dot = breathing accent glow, mentions keep the gold rail badge, hover pulses in the server's **livery accent** (`--sp-a`, fetched per server on open) + name label. Backdrops: `den`/`ridge`/`void` presets + **custom equirect 2:1 image** (canvas-downscaled data URL; v1 shows equirect quarters flat on the cube walls, near-field distortion accepted). Persistence `catcoms.space` localStorage, **per-device by design** (like desktop icon positions); Settings · Appearance gains backdrop tiles + custom upload + forget-placements. Ctrl+O toggles; Escape chain releases carry → pinned tray → view; lock closes it. All motion behind `data-motion` + `prefers-reduced-motion`. 216 frontend tests | ✅ in tree |
| 11z-8 | **Discord-style settings takeover + profile banner + animated avatars + gradient creators** (mockup-approved on the "Mewtual Settings" design canvas). Both 560px overlay cards became **full-window takeovers** (`.stx`, z-40 under the titlebar; `.overlay` dialogs at z-50 still open above): sidebar of pages (mono category rules, label search filter, active = accent-dim + inset rule), one page at a time, ESC ring; Esc chain unchanged. **User pages**: My Profile (shared `profileEditor` snippet, also rendered by the Ctrl+5 surface so the two can't drift) · Devices · Vault & Lock (lock now) · Verification (own fingerprint + copy, verified-members list → Verify dialog) · Appearance (+ **chat text size** `appearance.scale` 70–140% via `--fs-msg` override on top of density, + **timestamp clock** `appearance.clock` auto/12/24 through `fmtTime`) · Server Space · Notifications · Voice & Calls · Chat & Media (formatting cheatsheet) · Keybinds (static list) · Network · Updates. **Server pages**: Overview / Livery / Members / Badges (editor extracted from the roster) / Devices / Invites (quick-invite button deep-links here via `openServerSettings(id, page)`) / Emoji / Calls & Relay (shared TURN moved out of Overview) / Leave. **Live preview rails** (`.stx-prev`) on Appearance + My Profile (theme-token minis, so they track every tweak for free) and Livery (draft-painted: aside overrides `--accent`/`--accent-hi` from `liveryDraft`). **Profile banner end-to-end**: backend `Profile.banner` by content address (`MAX_BANNER_BYTES` 256KiB, `"banner_cid"` sibling key, resolved in `profiles()`/`fetch_missing_avatars` under the same per-pass budget; `set_profile` command gained a REQUIRED base64 `banner` arg) + editor upload (640px-wide JPEG re-encode) + profile-card render. **Animated avatars**: uploads keep raw bytes for GIF/WebP under the 64KiB cap (banners 256KiB) instead of the JPEG freeze; render sites sniff mime from the base64 magic (`imgSrc`); `.avatar`/`.rail-img` gained `object-fit: cover` since originals may be non-square. **Name styles pass 2**: fonts rounded/gothic, static effects outline/retro/glitch (tokens-only shadows), and a **custom gradient creator**: two stops + angle packed into the opaque effect string `grad2-rrggbb-rrggbb-deg` (old builds see an unknown class → flat colour fallback; `.fx-grad2` clips, the image rides inline via `fxStyle`), same creator for **custom bubble gradients** (existing opaque bubble channel). NOT built from the mockup (deliberate): Security + Server Nodes pages (protocol work: need design docs + review first), notification granularity, vault re-key, keybind remapping, motion-off animated-avatar freeze. **Message-log header restyle** (owner feedback on the first cut): header rows put the AVATAR in the timestamp gutter (34px: the gutter was dead space beside the name line, so the picture grew for free) with name · linked-device tag · verified ✓ · admin badge chip · inline time (+ delivery tick) after; grouped rows keep the gutter time; avatar click opens the profile. The preview rails now render the REAL `.messages` markup via a `previewLog` snippet fed by the profile draft (header + grouped row, mention chip, bubble/flatten reflected), so previews cannot drift and update live with every knob. **Round 3** (owner feedback): the effect picker's two gradient tiles merged into ONE multi-stop creator (`grad2-` now packs **2..8 stops** + angle; the old "gradient" accent-mix effect left the picker but still renders on peers who wear it); animated effects (rainbow/wave/pulse) now freeze under `data-motion="off"` too (matching reduced-motion) and their tiles dim with a says-why title when motion is off, answering "some effects don't seem to do anything". **Livery ground tint**: two colour stops washed into Nightshade's grounds in JS (`hexMix`; floor/rail toward stop A at ~30%, panels/elev/border toward stop B) and written as plain `#rrggbb` into the EXISTING `--bg-0/--panel/--bg-elev/--border/--border-soft` allow-list entries, so every client's read-side sanitizer already accepts it: custom-tinted (pseudo-gradient across surface hierarchy) server themes with zero protocol change; the livery preview rail applies all draft colour tokens inline (8 of 16 token slots used worst-case). **Round 4**: ANIMATED name gradients: optional `-a<speed>[r]` suffix on the grad2 effect string (speed 1..10 → 12s..1.2s linear loop; `r` reverses; scroll follows the gradient's own angle): `fxStyle` repeats the first stop and scrolls one 200% tile period via `@keyframes fx-grad2-scroll`; frozen under `data-motion="off"` + reduced-motion with `!important` (the animation arrives inline). Creator gained a Scroll speed slider + direction flip; stop wells wrap (they overflowed the column at 8 stops). Ground tint SPLIT per owner ask: independent Background (floor+rail: `--bg-0`/`--border-soft`) and Sidebars (`--panel`/`--bg-elev`/`--border`) targets, each colour + 0-60% intensity slider (the mix ratio: the "opacity"), still plain-hex allow-list tokens so no protocol change. True background-image gradients/custom textures for livery surfaces REMAIN DEFERRED: one hex per token is the wire format every client sanitizes, and panels are opaque (a floor texture would barely show without a compositing rework), so that is a design-doc slice (widen livery with a validated image key like icon/cursor + translucent-surface pass), not a quick patch. **Round 5** (owner smoke test): the takeover's two zones had the app's surfaces INVERTED (nav on `--bg-0`, content on `--panel`); now nav = `--panel` and content = `--bg-0`, matching sidebar-over-floor, with preview cards on `--panel` and the mini chat/log on `--bg-0`. ESC ring floats absolutely in the zone corner instead of holding a 68px column, so preview cards run the rail's full width beneath it (rail 300px, top padding clears the ring). Livery preview now shows the WHOLE draft, not just colours: preset (the four palette rules gained a bare `[data-preset]` selector so a scoped element can wear a full palette: no value duplication), corners (`--r`/`--r-lg`), interface font (`--ui`, with `font-family` re-stated on `.stx-prev` so a scoped var reaches it), background pattern (scoped copies of the `:root`-only pattern rules, aimed at the preview's chat surfaces) and the server icon in the mini rail: all via `liveryDraftVars()` + two data-attributes. | ✅ in tree |
| 11z-12 | **Jam polish round 3** (owner feedback). LEGACY ONLY button removed (alpha: engine's `setLegacyOnly` kept, no UI). REMAP moved out of the drawer: a gear beside KEYS/PADS opens Settings · Voice & Calls, which gained an "Instrument keys" section (same slot-then-press capture; the keydown capture branch is now standalone and armed only from Settings). **Take lead-in trim**: recorder time runs from the REC press, so waiting before playing put seconds of dead air at the front of every take: the jukebox "played" silence long enough to read as broken, and sheets opened with rows of empty bars: kept takes now trim to a 300ms pickup (old `.jamtake` files keep their recorded lead). Take deck hardening: deck presses resume a suspended synth context (gesture-time), the AUDIO-BLOCKED chip now also covers the elementless take deck, and `JUKEBOX.TAKE.FETCH_FAILED/INVALID/STARTED` diag codes make the next silent failure explain itself. Sheets: stems flip down above the middle line, faint dashed beat guides inside bars make honest off-grid placement read as rhythm (verified by headless-Edge renders of synthetic runs; pitch placement checked exact: G#4 second line, C5 third space). `mediaKind` take-detection pinned by tests (extension AND exact mime, never sniffed from JSON). 884 frontend tests, svelte-check 0/0, build clean | ✅ in tree |
| 11z-11 | **Jam takes: sheet-music export, `.jamtake` share format, jukebox replay** (owner-feedback round 2). *Sheet export*: `jam-sheet.ts` (pure, 6 tests) quantizes a validated take to its own tempo's sixteenth grid and engraves standalone SVG: one labelled staff section per PLAYER (lanes grouped by src, so reconnects never split a player; clef by median pitch; ledger lines; durations round DOWN to plain values, no ties/dots by declared scope), drum lanes as single-line x-head staves with pad tags, player names escaped as untrusted text. Saved by new tauri command `save_jam_sheet` (registered + listed in `tauri-command-security.ts`): unlocked-session gate, name pinned to the exporter's own `mewtual-take-NN-date.svg` shape, body must be a bounded `<svg>…</svg>` document, then the existing `write_download` + reveal path, toast on the frontend. *`.jamtake`*: a take row's share button seals the take JSON into the encrypted share via the ordinary streamed upload (`addSharedFile`, mime `application/x-mewtual-jamtake`; guarded on activeServerId === callServer so it cannot silently seal into another server's share). *Jukebox replay*: `mediaKind` gained kind `take` (+ picker TAKES chip); a queued `.jamtake` passes the SAME availability/trust gates as media, is fetched whole via `download_file` (bounded by the format's 512KiB cap; the one-caller guard test in `inline-transfer.test.ts` deliberately widened to name exactly two readers), validated by `parseJamTakeJson`, cached per cid, and driven by the take scheduler from the room's transport: `jamStartTakePlayback` gained offset + deckCid deck mode, `jukePos` ages the offset on the wall clock for takes (no element clock; DJ included), pause/stale/track-change/DJ-gone all stop the deck, deafened listeners keep counting but dispatch nothing, and only the DJ advances when it runs out. Also from feedback: saved patches moved into a DOWNWARD `jam-custom` drawer behind one CUSTOM·n tile (the row stopped growing right), and three OSC-stack redesign sketches (A Tabs / B Layers / C Shapes, each with motivation + tradeoff) are on the canvas's Jam Layer page AWAITING the owner's pick. GOTCHA: the Edit tool JSON-decodes a `\u0000` escape typed into its parameters into a real NUL, so it can neither match nor write that escape: byte-level fixes go through PowerShell, and edits near that line must anchor away from it (this row itself shipped one such raw NUL before being repaired the same way). *Owner picked option B (LAYERS)*, built same pass: the osc stack renders as mixer-style layer rows (collapsed = wave glyph + one-line summary + level bar, so the BLEND reads at a glance; exactly one layer unfolds its wave buttons + st/ct/lvl knobs; per-layer x remove, dashed +layer add, min 1 max 3). NOTHING removed by the fold: envelope/filter/LFO/room-sends sections are untouched and every per-osc control lives in the open layer. **BUG FIXED en route**: the first editor's wave buttons indexed by INST_TILES order (TRI first) while the wire's `w` indexes PATCH_OSC_WAVES (sine first), so clicking TRI actually selected sine and SIN selected triangle; the layer buttons map through `jamWaveIndex` and the summary glyph through `jamOscTile`. 883 frontend tests, tauri 168, clippy clean, svelte-check 0/0, production build clean | ✅ engine + first UI pass committed `d691bea`; layers rework in tree |
| 11z-10 | **Jam layer v2: constrained patch synth + drum pads, engine + integration** (contract: `docs/INTERFACES.md` §12; security boundary: `docs/THREAT-MODEL.md`). *Engine* (commit `d64f77e`, pure `jam-*.ts` + 8 test files, 60 tests): `jam-contract.ts` (every limit/type, single source), `jam-wire.ts` `JamFrameDecoder` (1024B pre-parse cap + all-frame bucket charged BEFORE `JSON.parse`, 200B post-parse cap for non-patch frames, sustained-abuse auto-mute), `jam-patch.ts` (one strict validator for wire/storage/import/playback; full 64-hex SHA-256 content ids), `jam-channel.ts` (opaque per-generation `JamSourceChannel` capability: handlers close over it, authority is NEVER rebuilt from a frame field), `jam-allocator.ts` (64 global/16 held voices; steals own releasing tails first, then global; NEVER a held voice: rejects instead), `jam-engine.ts` (fixed-topology WebAudio renderer: ≤3 osc + env + filter + bounded LFO + sends into receiver-owned room chorus/delay/reverb buses, master limiter, Deafen hard-gate, 30s hold watchdog, deterministic seeded drums with source-scoped chokes, full node teardown on steal), plus engine-ready-but-unwired `jam-clock.ts` (metronome/NTP-offset) and `jam-recorder.ts` (bounded group-bound takes). Drums are `t:"d"` (an old build IGNORES them; a pad as `t:"n"` would strand held MIDI notes 0-9 forever). *Integration* (App.svelte, in tree): `createPeer` mints a fresh capability + decoder per inst-channel generation and the handler closes over both; sends carry `q` sequences + optional `p` patch id with the legacy `w` always present; `t:"p"` announces (also how drums learn the sender nonce) go per-edge on open, broadcast on sound change, debounced 400ms under the receiver's 1-per-2s cap; drawer gained KEYS/PADS mode (local presentation: the wire is one event stream), LANDING/DUSK preset patches + a full EDIT fold (osc stack/env/filter/LFO/sends: every edit mints a new id), a receive-side LEGACY ONLY chip (signals nothing), pad-hit colour attribution, and flood-mute visibility (unmuting the peer forgives it). **Two long-standing bugs fixed as contract preconditions**: Deafen now hard-gates instrument rendering via `engine.setDeafened` (it previously only muted `<audio>` elements: the synth played on), and `members-changed` now reconciles `callPeers` against the server-scoped roster (`reconcileCallRoster` → `removePeer`), so a revoked member's established voice/video/inst edges die with their membership. *Metronome wired* (same pass, follow-up): drawer transport row (start = you anchor; BPM ±5 with a 300ms coalesce + the clock's own 2s revision floor honoured by self-deferring pushes; bar signature cycle 2/3/4/6/8; beat pips + bar count; SYNCED/LOCAL ONLY chip) on `jam-clock.ts`: anchor state machine + best-RTT NTP offset (`JamClockSync`) + audio-clock click planning (`plan()` from a 50ms lookahead tick, so timer throttling only gaps, never drifts). Probes go 1/s (self-limited) toward the anchor only; probe replies echo `tx` and are correlation-checked; `t:"c"` probes answered per-edge. Anchor's grid replays to late joiners on dc open (`jamMetHello`); anchor leaving stops the grid (dumb failover: anyone restarts); Deafen silences the click but the pips keep counting. The click is receiver-local synthesis outside the engine's voice budget. *Takes wired + owner-feedback round* (same pass): TAKES is a folded strip under the board (recording is loud, machinery quiet): REC arms a `JamTakeRecorder` whose consent gate rides the state heartbeat: new `PeerState.rec` (0/1 asking/2 recording) + `rc` (consent), both FRESH claims per message (absence = neither, so an old build can never be recorded: it cannot consent), fed to `setConsent` from both state paths; the take starts only when EVERY participant allows it, pauses on any membership change (`membershipChanged` from createPeer/removePeer), auto-stops at the 10-min cap, and both call surfaces render a room-wide REC/RECORD? banner with the ALLOW toggle whether or not any drawer is open. Kept takes are ephemeral by contract (die with the call); rows show duration/bpm/players and an honest "N lost" chip from sequence gaps; playback replays the validated event log through the SAME engine as synthetic per-lane sources (`take\u0000<lane>` ids, lane nonces via begin(Legacy)SourceSession, patches re-hashed + re-installed, nothing re-broadcast). Peer patch descriptors shadow-cached (same LRU depth as the engine) so recorded notes embed what the room heard. Feedback fixes: tile/head rows wrap (the register `+` could scroll off), the slider editor became drag KNOBS (up/right raises, Shift fine, wheel + arrow keys, `role="slider"`), editor grid auto-fits narrow docks, custom patches SAVE as named local tiles (12 cap, same validator, ✕ forgets), and piano/pad keys REMAP per device (click slot, press key; steals the key from its old slot; RESET; the vault melody's fixed keys are deliberately untouchable). GOTCHA fixed en route: a raw U+0000 typed into a template literal tripped the source-hygiene test and Bash-side node writes to App.svelte silently failed to stick: the byte swap needed PowerShell. 877 frontend tests, svelte-check 0/0, production build clean | ✅ engine `d64f77e` + integration in tree |
| 11z-9 | **Title-bar notification ticker + layered sound policy (frontend-local).** The ticker lane spans Forward→Minimise; stable receipt ids admit each announcement/wiki/event/notified-message once per unlocked UI session and are cleared on lock because wiki ids can name content. Message notifications fetch the stable row, enqueue a clickable server/channel/message target, and share the receipt with their audible alert so duplicate `channel-updated` events cannot ring twice. The built-in news cue is a generated three-pip square-wave phrase retuned from an operator-supplied reference (no game recording bundled). `notification-sounds.ts` centralizes device master → per-category global → per-server inherit/on/off and tone precedence for message/mention/news. Global and per-server custom audio is MIME/data-URL validated, ≤384 KiB/8s, localStorage-only, never gossiped; corrupt settings fall back safely. Server voice banners keep their own gate and use that server's effective mention tone. | ✅ in tree |
| 12a–12c | **operations + accountable moderation.** `DocType::Moderation=14` adds group-bound signed warning snapshots, kick cases/resolutions and one-origin-per-case advisory votes; owner/admins receive a per-user lane graph above the full evidence scroll, while members receive only focused chat vote cards. Shift-range warning/delete, warned-post collapse/expand, evidence picking and owner-only MLS removal remain. R7 is explicit: signatures prove attribution/field integrity, not historical role or append-only completeness. Durable local history moves drafts/read marks into bounded vault-sealed state. Storage health verifies seals/CIDs/file keys once per server/process, caches the report, and adds deduplicated category/pin/largest-file inventory; explicit repair re-fetches through authenticated content-addressed responses and replaces the cache. Sidebar utilities now form one profile-colored bottom stack; Transfers repeats health. Designs: [`design-moderation-plane.md`](design-moderation-plane.md), [`design-operations-recovery.md`](design-operations-recovery.md). | ✅ full suites + gates passed 2026-08-20 |
| 12d | **Backup & Recovery centre (export + secret rotation).** `create_backup` snapshots then copies the opaque sealed vault under the store lock into a fresh non-overwriting Downloads directory, refusing links/special files. UI/docs disclose offline guessing, filesystem metadata, historical-state retention and old-backup non-revocation. `change_vault_passphrase` authenticates the current secret and atomically rewraps the same DEK under a fresh salt/nonce, supporting passphrase/sigil/melody without bulk data re-encryption. Automated import remains deferred until locked staged verification/atomic-swap/rollback exists. Later ideas/gates: [`feature-implementation-tracker.md`](feature-implementation-tracker.md). | partial recovery; export + rotation verified, restore deferred |
| 11z-13 | **Jam/native release hardening after PR #19 adversarial review.** Patch publication is causal across WebCrypto on receive and per outbound edge; wire/local-renderer/recorder use one immutable sender-paced recipe, with a 256-operation inbound bound. Unopened edges drop historical musical events and establish only the current patch, preventing a reconnect from collapsing paced history into the receiver's burst budgets; gestures queued before the first digest are local/recording-only even if an edge opens meanwhile. Recorder recipe lookup uses the engine's exact validated LRU rather than a duplicate cache. Takes validate/hash their <=64 patches once into an engine-owned archival table, cap identity strings and overdue work per pass, preserve shared-deck remote provenance, scale through receiver-owned lane gains, release unmatched held notes at end-of-log, and retain legal tails up to 8 s. Drum hashing has one active job per opaque channel generation and 32 globally, with 256/performer and 512/global pending bounds; causal backpressure preserves valid dense hits and event/choke order, while stale lanes cannot block replacements. Every completion rechecks channel/session/Deafen/audio state plus monotonic mute/Deafen generations before allocating a voice. App admission generations advance on both gate edges and local input captures its epoch before patch publication, so neither an outer queued frame nor an engine digest can revive across an on→off cycle. Local queue overflow retires old closures and engine work before fresh input. Jukebox take reads preempt stale JavaScript loads through four process-wide native cancellation registrations; any submitted lower request keeps its charge until response/failure/timeout, so at most four withholding debts exist and new loads pause at that bound. Take progress is exact call/server/token-bound. Metronome stop bypasses tempo throttling; suspended contexts create no click nodes, cancel lookahead work and catch up without a burst. Deafen gates local previews too. Instrument auto-mute retains only an exact 2/s `t:"s"` control lane so consent cannot freeze. Native close calls serialize behind a separate sticky continuity-debt latch. Sheet durations floor honestly and native export accepts only the versioned inert renderer SVG grammar (no links, scripts, handlers, foreign content or external assets). | ✅ full suites + gates passed 2026-09-03; manual two-client smoke pending |
| 12h | **Desktop performance + IPC hardening (first slice).** Live profile message frames are rollout-gated without deleting their studio/config. Chat mounts a bounded 320-row tail with anchored paging/jumps, caches only bounded sanitized HTML, resolves rich placeholders per row, coalesces event snapshots, avoids forced bottom scroll and schedules cross-server inbox scans at idle. Feedback/Wiki Help are lazy Svelte components with feature CSS; QR codecs are dynamic chunks, reducing the measured App chunk from 881.49 kB to 709.33 kB minified. The 100-command Tauri surface has an executable review ledger; explicit lock now atomically saves UI continuity, rejects every non-bootstrap command, suppresses actor events and re-checks long downloads while native actors stay online. Full plan/audit: [`PERFORMANCE-SECURITY-HARDENING.md`](PERFORMANCE-SECURITY-HARDENING.md). Native paged history, larger Settings/operations extraction, remote-media consent and worker search remain queued. | active; first slice fully test-gated |
| 12i | **MIDI controllers in Settings → Devices; reliable hot-plug.** Web MIDI moves out of a one-shot lazy request into `midi.ts` (pure, unit-tested: parsing, pedal-aware routing, device rows, status diagnosis) plus retryable browser plumbing. Every connected input is wired rather than only the last in the map, disconnected ports are skipped, an already-granted permission reconnects at startup without prompting, and a lost device lifts the notes it was sounding. Settings → Devices gains a live controller panel: status verdict with one honest reason, device list with routed/unplugged/filtered state, per-port input routing persisted locally (matched by id then name), an always-on message monitor (velocity + channel; clock/sensing counted separately so a live-but-silent cable is distinguishable), a stuck-note release, and setup + troubleshooting help. Sustain (CC64) is honoured for the call instrument **only**: deferring note-offs on the melody lock would change the secret an identical performance encodes, and that vault has no recovery path. Velocity is parsed and displayed but not yet mapped to loudness; per-instrument receive controls remain queued. | ✅ added 2026-08-21 |
| 12j | **Unread indicators become state, and the jukebox stops lying about playback.** From an adversarial review of `P-fixes`. `AppEvent::ChannelUpdated` grows a typed `ChannelChange` (an arrival is "a message id never seen before", not a count that grew), so a reaction, a topic edit or a jukebox add can no longer read as an unread chat message. Read marks are no longer advanced by a refresh: `unread.ts` (pure, unit-tested) owns one observation predicate (chat surface active, no takeover/call-focus over it, window focused, document visible, pinned to the newest row) and a selected-but-unobserved channel goes unread exactly like an inactive one; a failed `get_messages` no longer clears the badge it was navigating to. New `get_channel_heads(server)` rebuilds unread from durable read marks at unlock, resume and once each server's directory settles, which is the only path that survives an explicit lock or a restart; mention badges come back the same way from the inbox scan. Sender-clock timestamps are clamped to a plausible ceiling before any read decision, so one wrong clock can neither hide later messages nor stick as a permanent unread row, and the server rail/orbit/DM dots all derive from the one `unread` list. Jukebox: a `hello` is answered with the current transport immediately instead of waiting up to five seconds for the next re-announce, transport revisions are bounded (`1e308` passed `Number.isInteger` and could not be incremented past, wedging the deck), a blocked `play()` surfaces as a clickable ENABLE PLAYBACK chip instead of silence under a "SYNCED" label, a listener that cannot fetch or decode says so, and queue reads are generation-guarded and no longer turn every error into an empty queue | ✅ |
| 12k | **Profile editor tabs + the arrival catalogue** (owner: "really cramped", sliders "look like dev art", wants PowerPoint-style arrivals; Livery tabs deliberately NOT done, frames expected to be dropped). The shared `profileEditor` snippet is one draft behind tabs **Identity** (avatar via `upload-btn`, banner, name, colour, bio) / **Name style** (studio, font, typography with pill toggles, effects, master, readability) / **Arrival**; a **Frame** tab exists only while `CHAT_MESSAGE_FRAMES_ENABLED`. `profileTab` state, `profileDirty` derived per tab against `profiles[myFp]` (gold dot on the tab, named in the sticky `.psave` bar with Discard + Save), `.profile-tab` widened 420→620px, every range in the editor wears the `.lv-range` look (`rangeFills` container action paints `--pct`, watches for sliders that unfold later). **Arrival catalogue**: `MESSAGE_FRAME_MOTIONS` 5→22 (wipe, split, blinds, checker, bars, wheel, dissolve; blackout, newsflash, swivel, flip, spiral, crawl; bounce, boomerang, slam, quake) with `messageFrameMotionTraits()` saying what the one distance slider means per motion (travel/depth/grain/amplitude/spin/none) and whether fade and the entry vector apply; the picker renders four families. **Compat**: `parseMessageFrame` now maps an UNKNOWN motion id to Still instead of rejecting the whole frame, so an older build keeps a newer peer's surface. CSS: one keyframe set per motion (0% → 25% then hold) shared by live rows (`animation-iteration-count: 0.25`, so a settled row stops being a stacking context) and the looping preview; mask reveals ride one registered `--arr-p` (initial 1: a frozen animation never hides a row); the log row and preview carry a dynamic `arrival-{id}` class instead of five `class:` directives. Fixes found on the way: arrival ids were pruned at 900ms while durations reach 1200ms (now 1500ms); `.messages` clips horizontal overflow so a fly-in cannot flash a scrollbar; effect tiles clip their own sparkles instead of spilling over the neighbour's border. Gotcha: the visual fixture zeroes every animation for determinism, so arrival screenshots need that rule removed over CDP first. **Round 2** (owner: "the arrivals don't actually work"): the own-send path was the bug. The optimistic `pending:` row is marked and animates, but the send acknowledgement's `refresh()` swaps it for the server-assigned id within a few frames, and that id was deliberately excluded from marking, so an own message never visibly animated; `refresh` now carries the mark over to new own rows while the pending mark is still live (peers' rows keep animating only on `refresh(true)`, which `channel-updated` already requests through the coalescer). Verified in the fixture by saving Newsflash, sending, and reading the acknowledged row's classes. **Identity library** (owner ask): whole-draft snapshots (avatar, banner, name + style, bio, arrival) saved under a label in `localStorage` (`catcoms.identities.v1`, ten max, base64 length caps, quota failure surfaces as a warning), tiles at the top of the Identity tab load a snapshot into the draft (Save profile still publishes), saving under an existing label replaces it. **Round 3** (owner: Newsflash "pushes the scroll bar up/down"): a row spun or scaled as a whole grows the scroller's scrollable overflow mid-flight. Moving arrivals now animate the row's CHILDREN (`li > *`: gutter and body) inside a row that clips at its own edge (`overflow: clip`); reveals, wipes and the blackout stay on the row since a mask, clip-path or filter never changes its box. Measured in the fixture: overflow beyond the viewport 101 → 158 when the 58px row lands and constant through the spin while the body's bounding box peaks at 582px. Side effect, deliberate: Glide/Slam no longer nudge the scrollbar either, and each row's motion is confined to its own band. **Round 4** (owner: centre the animation on the message's own size): `arrivalOrigin` row action measures the message (a Range over `.text` unioned with the `.author-link` box; body as fallback) with the animation momentarily switched off for the read, then writes the SAME absolute point into both children's `transform-origin` (centre, or the content's left/right/top/bottom edge for swivel/flip/crawl per `--message-arrival-origin`), so gutter and body turn as one piece around the message rather than each about its own box, and a short message spins small. Verified: both parts resolve to the text's left edge for a swivel. **Appearance** gained its own "Message arrivals" section (PLAY / STILL seg, copy says it is separate from text effects and tells nobody); the old toggle buried in Interface is gone, and the Arrival tab warns when the device has arrivals off. | ✅ in tree |
| 13 | Android (Tauri 2 mobile): JNI keystore, foreground service, two-tier keys | planned |
| 14 | hardening: cover traffic, supply-chain attestation, metadata-index aging, recovery import, **security review** (deeper adversarial scenarios land here) | planned |

Jam release-hardening addendum (PR #19 re-review): the call-epoch abuse budget now survives peer
reconnects; publication and take preparation retain only one running plus one latest crypto task
and are replaced on leave; old patch-digest lanes cannot block replacement channels. A fresh
channel may reinstall one byte-identical recipe previously hash-verified in that peer/call epoch
without spending another patch token/digest, while distinct recipes retain the persistent limit.
Deafen
destroys buffered room effects and cancels bounded, limited call cues. Local consent withdrawal
closes recorder admission immediately, and every musical frame retains receipt-time recorder
provenance before the App causal queue. Recorder leases bind an uninterrupted recording generation,
so withdrawal/restart invalidates old queued/digest work; disconnect revokes that edge's consent,
and the original take clock survives consent pause/resume. Unopened instrument edges retain no
historical notes or drums; they announce the current patch once and start at fresh live traffic.
Local causal overflow retires its old queue generation plus active/queued engine render work.
Jukebox takes are rejected above 512 KiB before fetch/base64 decode and use an eight-entry LRU;
loads are one-running/latest, exact-call-leased, and natively cancellable inside the actor chunk
await. Their process-wide cap of four remains charged through any submitted transport request's
response/failure/timeout; prompt UI preemption never converts withholding work into unbounded
lower requests. Caller ids carry a per-WebView random nonce so remount-reset counters cannot alias
an active predecessor. Current listing/trust admission is rerun after fetch before cache or playback.
Drum replay restores the stable channel id and original performer used live; device-local server
handles are excluded. Reconnect lanes for one take participant share fairness, digest and hat-choke
budgets under an engine-minted owner domain, so an unsigned performer label cannot collide with a
live member's authority. Sheet durations floor after a declared one-sixteenth visible minimum, and
the native SVG writer holds the exact UI-generation guard through write and reveal. Native close
also binds continuity completion to the close request's generation. Durable phase-6 attribution
remains blocked until the frontend receives a stable group id; current take `group` is local scope
metadata only. Automated full release gates passed; the real two-client smoke test remains manual.

The historical 8… row still ends with “AddressCache persistence” in its original remaining list.
That item is now complete: the desktop seals an integrity-tagged cache beside the server snapshot
and re-verifies it on load. Ordinary interface refresh is also live: one process-wide native
route/interface monitor debounces platform events and wakes every server to poll route-selected
IPv4/IPv6 sources, reconcile per-source address ownership, update libp2p external addresses, and
publish one newer signed record before PEX. The roughly-minute discovery poll remains the repair
path when native monitoring is unavailable or misses an event. The first two reciprocal-dial review
prerequisites are now complete: peer/invite/switchboard discovery routes use one canonical grammar
with a mandatory terminal id matching the signed/discovered Phase-0 peer, root-path-only WebSocket
forms, and no network-actor bare-address fallback. `DiscoveryPolicy` meters every address and a
single desktop-owned `EndpointDialScheduler` bounds attempt/prefix, canonical Phase-0 peer, server,
and process submissions across untrusted member-discovery paths plus pre-join rendezvous/direct/
switchboard, two-way reply, and companion-grant paths. The endpoint carries its parser-derived
principal, so cache/device/raw-libp2p aliases cannot mint parallel peer buckets. Direct routes share
a physical-socket key; relay routes use a relay/target circuit key so distinct targets at one relay
do not starve one another, with the shared relay host still bounded by prefix/process caps. Two-way
proof retries use a live-connected-only actor command and cannot redial from `recent_peers`. The
default process window is 32 endpoints per minute; counters are bounded, monotonic and session-only.
Discovery calls transfer generation-bound, non-cloneable permits into the network actor. The
scheduler's injected monotonic clock is authoritative at both reservation and commit, so a queued
permit cannot cross the end of its accounting window even when no newer reservation has rolled the
generation. Duplicate, already-connected, and actor-tracked already-dialling infrastructure
suppression refund before commit; pending/socket submission commits, so a cancelled caller cannot
reclaim queued work and an old permit cannot refund a replacement window. Pending infrastructure
also covers member routes reclassified during an actor drain. Constructor TCP/QUIC routes are
grouped per terminal peer into one known-peer race and seed the pending state before actor startup;
state is released on immediate refusal, connection, or outgoing failure. Post-commit failures are
conservatively spent. Trusted operator-infrastructure connections are not
all mediated by this scheduler. It still has no separate exact relay-outer-socket lease or
process-wide in-flight lease, and endpoint signatures do not prove IP/port ownership.

Pairwise path evidence and typed claimed-peer health/actions are now complete. The transport tracks
concurrent connection ids so relay-to-direct upgrades and partial closes remain truthful; the sync
layer bounds transient detail/history, expires historical success, and exposes safe action ids to
both Connectivity and the debug console. The UI no longer translates this device's missing
connection into “offline.” The explicit `self_asserted` binding remains because signed descriptors
do not yet prove device-key control of their claimed transport key. Dial counts are scheduler
submissions rather than asserted transport failures; IPv6-only candidates are diagnostic clues,
not proof this host lacks an outbound IPv6 route; and admission-only switchboards are not offered
as post-join repair.

The post-join resilience backlog is complete. The actor now has a fail-closed direct-only
peer-bound batch, connected-only helper sends, exact-descriptor/replay/rate/expiry-bounded
reciprocal repair, two-helper SWIM-style evidence, derived HyParView-like active/passive views,
local CYCLON age/source sampling, a manual anti-click-bounded redial, and TTL-aware rendezvous
renewal. Probe/result/forward/delivery are separate connected-only authenticated pushes, so no
remote repair timeout is held inside the sole-owner actor; helpers never carry application
traffic. Newer route withdrawal removes old sealed-cache addresses.
The reported same-LAN close/reopen gap has two narrow repairs. After a successful direct
join, the joiner seals the exact outbound IP route that Noise authenticated for the named inviter
in `ServerNet` v3 under an explicit `AuthorizedPeer` policy. Direct admission also attempts bounded
PEX before the first post-join snapshot so the inviter's signed descriptor is present for the
roster check even if the user closes immediately. Reload and the discovery cadence retry the route
even with no rendezvous, after canonical peer binding, unique current-roster claim, raw-TCP/QUIC
host-shape and shared scheduler checks; reconnect logs retain only route shape. New
helper/reply/switchboard admissions persist `Disabled`. Pre-v3 records decode as `LegacyPending`
and may migrate once only after a successful overlap in an unambiguous two-member group, using a
private/loopback route. A real TCP regression snapshots the joiner immediately after direct
admission, closes both clients, rebinds/restores the inviter, then restores the joiner with the same
transport identities; they reconnect without a fresh invite and exchange messages and a file.
For a changed listener or an already-isolated pair, a current member can now send a ten-minute
`mewtual-reconnect-v1:` code out of band. The receiver verifies the server, current device
membership, signature, expiry and terminal peer binding, then submits at most four literal-IP
TCP/QUIC routes through the process-wide scheduler. Applying a code seals only consent for that
peer; the pasted route becomes durable only after an ordinary Noise-authenticated connection.
The real-TCP regression now also changes the listener while the old one remains bound, applies the
code, reconnects and exchanges another message. A Tauri-bridge lifecycle regression seals a DM
server and continuity state, rejects a wrong password, performs two independent vault
open/restore/actor/shutdown cycles through the production restore seam, and verifies messages,
DM metadata and reconnect consent each time.
This does not provide mDNS/first contact or prove the device key controls its self-asserted
transport key.
The remaining honest limits are architectural: no repair can heal complete isolation, submitted
dials cannot be recalled through the current seam, registration grants have no request id, and
the reciprocal control protocol is not a dual-key device↔transport ownership proof. See
`design-postjoin-discovery.md`.

> **Connectivity terminology note:** current UI and diagnostics report a member's self-asserted
> peer as **claimed path connected / no claimed path**, never proof that the device key controls
> that transport, that a person is online, or that they are globally reachable. Operational file
> and delivery availability is stricter: the live peer must previously have served a
> roster-verified, request-bound catch-up. This supersedes the older roadmap row's “online
> dots/count”, “no peers online”, and “Online / Last seen” wording.

### 2026-08-28 field-report triage

- **Message close/reopen:** fixed for a direct join to a stable listener by the sealed authenticated
  route described above. Changed listeners and already-isolated current members have the manual,
  signed recovery-code path. Real TCP covers stable and changed-address recovery; the desktop
  bridge covers two complete sealed-vault open/actor/shutdown cycles.
- **Delivery indicator:** a send accepted by the local actor now says `sent · awaiting
  confirmation`, not `sending…`. A newly applied remote op now generates a connected-only,
  authenticated kind-18 receipt for the exact bounded document/change target, so a quiet recipient
  can confirm without replying. Actor queries and `delivery-changed` events share one monotonic
  snapshot revision, and the webview refuses an older completion, so a delayed query cannot replace
  fresher receipt evidence. It is a delivery receipt, never a read receipt.
- **DM first contact:** the frontend now surfaces the native `join-reply-ready` code while Add
  friend or Accept is still waiting, explains its deadline, and gives the inviter an in-DM paste/
  replacement flow. An expired reply is not revived: retry Connect to create a fresh overlapping
  60-second window. Member recovery remains only for an already-established server; it does not
  grant membership or repair an expired first-contact reply.
- **File trust policy:** the found/join onboarding now requires a local on-demand (default),
  specific-member, or everyone choice before the new server view can issue passive media requests.
  The per-server policy and exact trusted full device identities are bounded and vault-sealed; the
  short 32-bit fingerprint is display-only. Server Settings -> File Trust edits them. Passive shared
  media trust changes bypass the ordinary read-position/draft debounce, and a normal window close
  waits for the native final continuity snapshot before destroying the WebView. A hard process or
  operating-system failure can still interrupt the disk write.
  Passive shared
  embeds, custom emoji, event images, card thumbnails and call-jukebox tracks obey the policy,
  while an explicit Load/Play/Open/Download click is always an override. Third-party HTTP(S)
  images stay click-only in every mode because they can disclose the client address or target
  loopback/private services. File-index
  materialization/publication is capped at 256 rows with bounded signed fields. Fetched chunks
  already occupy one authenticated XChaCha20-Poly1305-sealed
  vault copy, so keeping a distrusted file packaged does not inherently double storage; explicit
  export adds a separate plaintext Downloads copy. Storage now inventories each complete encrypted
  local file and offers that export as “Unlock copy”; partial files remain listed only in aggregate
  until all chunks are present. Inline media and exports compare an exact inert MIME allow-list with
  a bounded common-container signature. SVG, mismatches and unknown formats receive a bodyless
  inline-scheme denial (not an octet-stream body the WebView may sniff), while exports disclose
  matched/mismatched/unrecognized evidence. Inline head/chunk caches are bound to the complete
  current compatible manifest set rather than the member-claimed plaintext CID. This blocks simple
  type disguise but is not full bitstream validation or a decoder sandbox. Full
  trust-everyone/specific automatic
  **whole-share mirroring** remains deferred until the sealing store has a disk quota, otherwise a
  trusted member can exhaust local storage. Native saves sanitize the filename, use non-overwriting
  verified staging, and reveal rather than execute, which blocks direct command/path execution.
  The staging rename/reveal is committed only for the exact unlock generation that began it.
  Storage verification opens at most four exact references per ciphertext CID; overflow fails the
  CID and dependent manifests closed instead of turning one held large blob into thousands of
  actor-blocking decryptions. Native storage cache rows are tied to a process-local server
  incarnation and frontend promise results to the exact still-unlocked view, closing late
  publication across lock, leave and same-id reinstall.
  Content and platform decoders remain untrusted parser/zero-day surfaces. See `THREAT-MODEL.md`.
- **4K screen streaming:** screen capture now has a live settings panel for 720p through 4K,
  15--60 fps, quality priority and a per-full-resolution-viewer Mbps cap. Each receiver advertises
  only the nearest display bucket (720/1080/1440/2160), dynamically derived from its Mewtual window
  with hysteresis or fixed by the user; the sender independently scales/caps each peer and shows
  per-peer plus aggregate upload estimates. A screen edge stays parked until its resolution,
  bitrate and frame-rate cap applies; rejection pauses that edge or stops sharing if it cannot be
  parked. Resize bursts are bounded to one active plus one latest pending mutation per peer. The
  WebView performs the compressed WebRTC encode. The same installation-wide preset is now visible
  in Settings -> Voice & Calls and behind the cog beside the call-stage IN/OUT selectors; numeric
  resolution/FPS choices explicitly select their persisted defaults. Shared audio defaults off.
  Surface audio can ride the initial picker; separate mode lets the user add multiple freshly
  granted application/window sources while sharing, discards those sources' video, mixes them into
  one audio track with a requested 160-kbps-per-peer cap (shown as planned until the WebView
  confirms it), and clears every grant on stop/camera swap/leave. The shared-audio track has a
  persisted 0--200% client-wide master plus ephemeral 0--200% faders for each explicitly granted
  source; 100% is unity and the UI warns that overlapping boosts can clip. At most eight source
  grants can be live, and removing the last source stops the mixed output and closes its Web Audio
  graph. The normal call
  microphone remains a separate sender and is intentionally not relabelled as part of that mix.
  H.265/HEVC is preferred when both runtimes expose it, with AV1/VP9/H.264/VP8 fallbacks, and the UI
  reports the codec actually observed in stats. There is still no application-owned offline media
  transcoder or guarantee that a particular platform WebView offers H.265.
- **Regression coverage added in this round:** real-TCP close/reopen and changed-route recovery;
  recovery-code signature/group/device→peer/expiry/address limits; pending-consent persistence,
  lost-update and establish+close evidence; restored-server capture-worker installation; explicit
  receipt quiet-recipient/catch-up/error-prefix paths; current-roster and omitted-snapshot delivery
  replacement; two independent sealed-vault restore/shutdown cycles with wrong-passphrase refusal;
  lock-vs-deferred-command ordering; and pure streaming tests for receiver rounding/hysteresis,
  actual-source scaling, burst coalescing, frame-rate/bitrate caps, rejected-cap parking, detached
  first attachment and screen→camera invalidation, permission-prompt stop/leave/supersession,
  stream-preset sanitization/defaults and bounded mixer gain, file-trust fail-closed parsing/full-identity decisions,
  mixer source-count/graph teardown, exact-container inline-media admission, exact-manifest and
  exact-`FileRef` inventory/cache binding and per-CID work bounds, stale media/storage publication
  after lock or server-id reincarnation, exact-generation plaintext export publication,
  jukebox/cross-server gates/index bounds, and sealed continuity/lock ordering of trust policy.
- **Native/Linux integration coverage:** `process_recovery_e2e` now crosses actual process,
  snapshot and TCP boundaries: Bob joins, exits, restores in a second process on a deliberately
  different listener, reloads an explicitly persisted transport seed, atomically publishes the
  signed recovery code through a file standing in for copy/paste, and proves two-way messaging
  after Alice applies it. (The seed file stands in for `ServerNet`; its vault-sealed encoding has
  separate store coverage.) Linux-only store tests abort a subprocess
  after the staged record is synced and immediately after rename; readers observe either the
  complete previous or complete replacement record. The persistence primitive uses unique
  create-new siblings, rejects staging symlinks, syncs the staged file and, on Unix, its parent
  directory; a post-rename sync failure is explicitly classified as committed-but-not-durable.
  The root `vault.bin` now uses the same durable staging shape plus an OS-backed interprocess lock;
  it fails lock contention promptly as `VaultBusy`, and real child-process tests prove concurrent
  first creation cannot return mismatched DEKs or hang and conflicting rewraps cannot both succeed.
  A separate OS session lock is now owned for each `ServerStore` lifetime, preventing two desktop
  processes from running divergent actors over one vault; a production-constructor child-process
  regression covers prompt contention, normal release and abort release. Already-mounted UI
  re-unlock authenticates through a verify-only vault transaction, preserving wrong-passphrase
  refusal without self-contending on that lifetime lock.
  New/replacement vault secrets are capped at 4096 bytes. A legacy v1 wrapper using a
  4097..65536-byte secret is accepted once and atomically migrated to v2 with a fixed,
  domain-separated prehash before Argon2; larger inputs are rejected. The v2 migration is
  forward-only, so a v1-only older build cannot reopen the migrated profile. Both wrapper versions
  are fixed at 89 bytes and hostile lengths are rejected without file-sized allocation.
  `compose.linux-test.yml` provides a non-root Debian full-suite/process lane plus explicitly
  root/privileged opt-in netns NAT/relay
  lanes; ordinary CI also compiles/tests the frontend and separate Tauri workspace on Ubuntu.
- **Remaining high-value integration coverage:** the recovery test exercises the native core, not
  two visible Tauri/WebView applications and their IPC/copy-paste controls. WebView-level tests are
  still needed against each supported platform's real `RTCRtpSender`, capture portal and codec
  implementation (including H.265 availability/rejection and application-audio choices). The
  Linux abort test pins the shared atomic-write boundary, but a kill matrix at each Tauri recovery
  state-machine transition remains. Decoder sandboxing/transcoding also remains architectural work;
  a headless Docker container cannot prove graphical portal, hardware-codec or decoder isolation.
  See `LINUX-TESTING.md`.

Status posts now use distinct `status_post/<random-id>` root maps rather than requiring concurrent
authors to share/create one Automerge list. Readers and all mutations enumerate every legacy list
conflict, so already-split old feeds recover while simultaneous new posts converge. IDs must be one
canonical 128-bit lowercase-hex value resolving to exactly one object across both layouts;
ambiguous/malformed rows are hidden and non-actionable. Older clients do not understand newly keyed
posts, so status-feed participants must upgrade rather than relying on mixed-version visibility.
See `INTERFACES.md` §8 for the persistence contract.

### Earlier blocks (history)
6d-1b (missed-commit recovery + past-epoch key window) and 6d-2 (fork resolution +
the convergence-safe single-serializer membership model) are complete; their details
live in the commit messages and [`design-6d2.md`](design-6d2.md). The default config
stays **single designated committer**; the concurrent-committer / fork path is OFF
by default (`max_committer_rank=0`) pending **I1** (a wall-clock contest window can
converge honest nodes differently under async timing); re-read `design-6d2.md` before
touching it.

## 6e-3d; COMPLETE (rendezvous discovery + eclipse-resistance)

**Read [`design-6e-rendezvous.md`](design-6e-rendezvous.md)**; the 9-slice contract
from a 7-agent design+review workflow, plus the per-slice adversarial-review outcomes
(2b, 3d-5, and 3d-6…9 are all recorded there). All 9 slices are done.

**Phase 7** (end-to-end local integration over real sockets + consolidated security
suite) is **COMPLETE**. Every networking + NAT-traversal path is proven end-to-end over
**real TCP loopback sockets** (not just the libp2p memory transport): **direct**
(`tcp_e2e.rs`, 7a), **rendezvous-discovered** with no hard-coded address
(`tcp_rendezvous_e2e.rs`, 7c), **relayed** NAT-traversal (`tcp_relay_e2e.rs`, 7d), and a
**relayed→direct DCUtR upgrade** driven through a full join (`tcp_dcutr_e2e.rs`, 7e).
The consolidated security suite (`security.rs`, 7b) maps the threat model to where each
property is proven and adds the cross-layer scenarios (eclipse-never-gates-a-removal;
removed-member-excluded-from-the-rotated-namespace). Deeper adversarial scenarios are
deferred to the dedicated hardening + security-review phase (**14** in the table above;
it was numbered "10" before the UI overhaul took that number).

**Goal:** members find each other with no hard-coded bootstrap addresses, and an
attacker cannot isolate (eclipse) a member. Everything is built on a per-removal
routing secret `ns_secret_L`:

- **Foundation (3d-1/2a/2b, done).** `ns_secret_L` is snapshotted at each member
  **removal** (counter `L`), retained `{L-2,L-1,L}` in `ChannelSync`. The blinded
  gossip topics **and** rendezvous namespaces both derive from it (keyed BLAKE3), so a
  non-member can't compute them and they **rotate on removal** (forward secrecy for
  routing metadata). Because the secret is epoch-specific, the **join handshake
  transfers it** (sealed, bound into the inviter signature) so every member; founder,
  joiner, *post-removal* joiner; derives identical topics. Re-keying the topics
  **closed the pre-existing A1 CRITICAL**.
- **Discovery (3d-3/4, done).** A zero-knowledge rendezvous **server** (`catcomsctl
  rendezvous`) and a **client** in `MeshBehaviour`: register a signed peer record under
  a blinded namespace, discover others. Discovered records are **surfaced, never
  auto-dialed**; the dial decision (and eclipse-resistance) lives a layer up.
- **Source trust (3d-5, done).** Commit catch-up responses are now **signed** by the
  responder's MLS leaf key, bound to the request; a **two-pool** model separates
  untrusted candidates from verified `member_peers`. Each session proof retains the
  signing roster `DeviceId`, so removal drops only departed devices' proofs and does not
  strand unaffected continuously-connected members. **Closed Sybil-C1.**

**Eclipse-resistance (3d-6…9, done, each adversarially reviewed):**
- **3d-6**; pure `catcoms-discovery` `DiscoveryPolicy` (rank candidates → bounded,
  Clock-paced/RNG-jittered dial plan; ≤1 root/rendezvous; roster clamp; seq-freshness;
  the only thing that decides what to dial). Plus the pre-dial **membership tag** and
  the catch-up **nonce/epoch anti-replay** (closing the 3d-5 deferred items).
- **3d-7**; **member PEX** (`KIND_PEX`): members supply each other dialable
  `PeerDescriptor`s without a rendezvous; responder-signed, members-only, capped +
  rate-limited; entries are discovery candidates (never auto-promoted to the trusted
  catch-up pool).
- **3d-8**; advisory **`EclipseDetector`** (D/R/S + hysteresis; never gates) +
  cross-session **`AddressCache`** (proven members, tamper-detected load).
- **3d-9**; invite rewiring (signature-bound `rendezvous` vector, `INVITE_DOMAIN`
  v2), pre-join **`join_namespace`**, and `serve --rendezvous`/`join`
  **discover→dial→join** (DiscoveryPolicy-mediated); verified by a memory end-to-end
  test (no hard-coded server address).

## Voice (group calls)

Shipped `bd483b5` → `7492f92`; contract in [`design-voice.md`](design-voice.md)
(design phases 1–3 are in, phase 4 is not). **All media-plane code is frontend**
(`apps/desktop/src/App.svelte`); the Rust core only derives a key and relays opaque,
authenticated signalling.

**How it works today**
- **Rooms are per channel.** The channel id doubles as the call id *and* the media-key
  id, so "join #general's voice" is unambiguous. Presence heartbeats give each channel a
  live "🔊 N in voice" pill; a room going active raises a per-server-gated banner + chime.
- **Media plane:** a full **WebRTC mesh** in the webview; one `RTCPeerConnection` per
  other participant, no server in the path, so DTLS-SRTP is genuinely end-to-end. Mesh
  economics cap the useful size at **~8** (uplink = (n−1) × ~32 kbit/s Opus).
- **Signalling:** SDP/ICE over `KIND_CALL_SIGNAL`; members-only, signed,
  freshness-bound, `from` = the verified signer. Because signalling is authenticated,
  the **DTLS fingerprints can't be MITM'd**. Payload is opaque to the core; not deduped
  (every candidate must land); FIFO-bounded. **Signalling rides the existing mesh**, so
  two members must already be mesh-connected (i.e. chat works between them) before a
  call can be set up; STUN/TURN only fixes the *media* path.
- **Media key:** `media_secret(call_id)` off the MLS exporter at the current epoch;
  every member derives the identical key locally, it is **never sent on the wire**, and
  distinct calls are domain-separated. Test-pinned (`members_derive_the_same_e2e_media_key`).
- **NAT:** STUN by default, optional personal TURN (Settings → Calls), plus a
  **server-provided TURN** the operator sets once and every invitee inherits.

**Pending / honest gaps**
1. **The MLS-keyed frame layer (SFrame / Encoded Transform) is NOT implemented.** The key
   is derived and exposed to the webview (`call_media_key`), and the frontend does not yet
   call it; today's E2E property comes from mesh DTLS-SRTP + un-MITM-able signalling.
   That holds while media is peer-to-peer or TURN-relayed (a TURN sees only SRTP
   ciphertext); it would **not** hold behind an SFU, which is exactly what the frame layer
   is for. Don't describe voice as "MLS-encrypted media" until this lands.
2. **Design phase 3 remainder:** re-derive the media key on an **MLS epoch change**
   (using the bounded past-epoch window for in-flight frames) and VAD/DTX. A long call
   spanning a membership change currently keeps its original epoch's key.
3. **Design phase 4:** move media onto the libp2p relay/DCUtR fabric so calls need no
   third-party STUN/TURN (the ethos-consistent transport).
4. **Ring-at-start only**; a member who comes online mid-call isn't rung (the channel
   presence pill covers most of this in practice).
5. **No adversarial-review workflow is recorded for the voice slices.** 11a added
   protocol surface (a new `KIND` + an MLS exporter label); the working conventions call
   for a hostile review on that class of change. Worth running before voice is "done".

## Known limitations / deferred (the security-relevant ones)

- **Desktop networking: the transport paths are wired, but public infrastructure is not
  deployed by the app.** The `apps/desktop` bridge binds all interfaces and the
  founder advertises a reachable address (LAN/public IP, `host:port`, or a relay-circuit
  multiaddr); joining dials every bootstrap address. So **same-machine** (blank), **LAN**
  (founder's LAN IP), and **internet via a port-forwarded public IP** all work;
  **relay-circuit NAT traversal** (8q) needs no port-forward on either side;
  **rendezvous auto-discovery** works in the UI (join with *no* address in the invite) and
  **post-join steady-state discovery** re-finds the group after a restart with no fresh
  invite; an established direct join also retains a vault-sealed, roster-rechecked same-LAN route
  to the named inviter for unchanged-address close/reopen; **UPnP/PCP/NAT-PMP** (11n/11n++) can make
  a node directly reachable with no relay at all.
  Successful joiners retain their own mapping/AutoNAT background results too; on a failed join
  the temporary listener is dropped, so its mapping cannot help reach the inviter. **AutoNAT v2**
  can verify one candidate from a connected relay/rendezvous, but does not create a route and has
  no default public server. Residual: automatic mapping is router/upstream-dependent (double NAT,
  CGNAT, or a disabled gateway can still defeat it), relay fallback needs a deployed relay, and
  DCUtR hole-punching still needs a relay to
  coordinate, and a **`cargo build` (debug) exe is a dev build** that loads the UI from the
  Vite dev server (`localhost:1420`) and shows "can't reach the page" on any machine
  without it; to distribute, build a release exe with the frontend embedded
  (`npm run build && npm run tauri build -- --no-bundle`; needs WebView2 on the target).
- **Blob store: persistent + sealed, but not yet last-copy-safe (8l–8s, 9h).** The
  desktop attaches a per-server on-disk **`SealingBlobStore`** once the vault is unlocked
  (9h-a), so files/avatars survive a restart and are encrypted at rest under `blob_key`;
  deleting a file reclaims its orphaned chunk blobs while keeping any chunk another file
  references (dedup-safe GC). `MemoryBlobStore` remains the pre-unlock/test path and is
  size-bounded (`DEFAULT_BLOB_BUDGET` 128 MiB, **FIFO**, 8s). What's still missing is the
  `catcoms-storage` **retention engine**: eviction is not holder-probe-aware, so it can
  drop the **last copy** of a blob (re-fetchable only while some holder is online), and
  there is no disk quota/expiry enforcement on the sealing store.
- **Persistence + encryption-at-rest are DONE (Phase 9); the residuals are recovery
  ergonomics.** [`design-persistence.md`](design-persistence.md) is the design; it shipped
  as 9a–9h: a passphrase-sealed key vault, per-server snapshots (MLS + docs + routing +
  ledger + commit log + peer records) written atomically under the vault, an on-disk
  sealing blob store, and a stable per-group **file-wrap key** minted at founding +
  transferred in the join handshake (so files are e2e ciphertext keyed by ciphertext CID).
  Vault-secret rotation is implemented: `change_vault_passphrase` authenticates the current
  secret and atomically rewraps the same DEK under a fresh salt/nonce. Residual: **no secret-reset
  or recovery path** (lose every valid secret and the servers are unreadable; there is no escrow
  by design), and a
  corrupted/partial snapshot surfaces as a load failure rather than a repair.
- **Network admission is single-committer-only** (only the lowest-leaf-index member
  admits). Concurrent admits / fork resolution + cross-member single-use = 6d-2.
- **Commit catch-up needs a peer that still holds the commit.** A member behind by
  more than a serving peer's `max_commit_log` window can't recover via commit
  catch-up; a full snapshot rejoin (deferred) is required; the gap is logged and a
  bad source is excluded, but exhausting all sources surfaces only a warning (a
  recovery *event* to the app is a follow-up).
- **Catch-up auth is now nonce-bound (6e-3d-6).** Catch-up *responses* are signed by a
  current member and bound to `(group_id, requester pubkey, req_ts, **nonce**, **epoch**,
  bundle)`, and requests carry a fresh signed timestamp + per-request RNG nonce; so a
  captured response cannot be replayed against a different request and the same-ms `ts`
  collision window is closed. Residual: there is still no *server-side* seen-nonce log,
  so a captured *request* can be re-sent within `MAX_REQUEST_AGE_MS` (60s); harmless
  (the member just re-serves a freshly-signed bundle; the Noise transport confines it to
  the peer's own session). A full snapshot rejoin for a too-far-behind member is the
  remaining recovery gap.
- **6e-3d discovery is built (eclipse-resistance complete).** `DiscoveryPolicy`
  (ranked, budgeted dial plan), the pre-dial membership tag, member PEX, the advisory
  eclipse detector, and the cross-session cache are all in (6e-3d-6…9), and a joiner can
  bootstrap via `join_ns` with no hard-coded address. The desktop seals the cross-session cache
  beside the server snapshot, re-verifies it on load, and periodically retries its current signed
  candidates with bounded backoff; SQLCipher remains a storage-engine refinement rather than an
  availability blocker. Raw interface changes, pairwise path evidence, reciprocal repair,
  SWIM-style helper observations, topology maintenance, manual redial and TTL-aware desktop
  registration renewal are implemented. Residual / deferred: the CLI's standalone
  `catcomsctl serve --rendezvous` flow is not the desktop's recurring actor loop; the `join` path
  uses the **first** rendezvous only (no multi-rendezvous fall-through yet);
  and `--host` must be a raw IP. The net Actor never auto-dials; the dial decision and
  all bounds live in `catcoms-discovery`.
- **A forged future `CommitRecord`** on the control topic is bounded (gap +
  buffer caps, deduped catch-up) and fails MLS verification at apply time. Per-peer
  rate limiting + exponential catch-up backoff are a hardening follow-up.
- **Persistent sealed MLS storage** landed in 9c (`snapshot_server`/`restore_server`
  over openmls's in-memory provider, the whole sync state sealed under the vault). What
  remains from the original note: **SQLCipher** backing (for the address cache + a local
  metadata index) rather than sealed flat files.
- **Metadata** is the dominant residual: who-talks-to-whom, timing, group sizes, the
  member IPs a DCUtR upgrade reveals to the peer, and; now; a **rendezvous** node
  learning `namespace ↔ IP ↔ timing` for the registration TTL (a higher-value target
  than a relay; querying ≥2 rendezvous doubles the operators who see it). An AutoNAT server also
  learns the requesting peer, candidate address and probe timing. A malicious server can refuse
  or induce a false negative, while a v2 positive requires the nonce-bearing callback.
  Per-rendezvous namespace diversification removes the cross-operator join key; rotation-on-removal
  limits long-term linkage but leaks a removal-cadence signal. **Voice widens this**: ICE
  reveals each participant's IP to every other participant (inherent to a P2P mesh), and
  a **STUN/TURN operator** learns `IP ↔ call timing ↔ duration` (a TURN sees only SRTP
  ciphertext, but it sees *that* you called and for how long); the default public STUN
  is a third party the rest of the system deliberately avoids, so privacy-sensitive
  deployments should point Settings → Calls at their own (or blank it for LAN-only).
  Mitigated, not eliminated (≥2 relays/rendezvous, cover traffic, staying relayed).
  Relays/rendezvous only ever see Noise+MLS ciphertext / opaque namespaces.
  See ARCHITECTURE §3.
- **Per-peer rate limiting / off-actor offload** of join work: a hardening follow-up.
- **`tracing` retrofit** for the earlier crypto/storage crates: deferred (user OK'd).

## Where the design/review outputs live

Design passes and adversarial reviews run as background `Workflow`s; their structured
output is under the session's `tasks/<id>.output`. The load-bearing conclusions are
distilled into the design docs and the memory files; **read the design doc for the
block you're touching**:
- `ARCHITECTURE.md` §1–§2; the four locked decisions + the initial corrections;
  §4a/§4b; join + commit propagation; §3; honest residual risks.
- [`design-6d2.md`](design-6d2.md); fork resolution / single-serializer membership
  (committer = lowest **leaf index**; the **I1** gate keeping concurrent committers off).
- [`design-6e-relay.md`](design-6e-relay.md); relay-v2 + DCUtR.
- [`design-6e-rendezvous.md`](design-6e-rendezvous.md); rendezvous discovery +
  eclipse-resistance: the 9-slice contract and the recorded per-slice adversarial-review
  outcomes (A1/2b and Sybil-C1/3d-5) with their deferred follow-ups.
- [`design-rendezvous-ui.md`](design-rendezvous-ui.md) ·
  [`design-postjoin-discovery.md`](design-postjoin-discovery.md); the same discovery
  machinery wired into the desktop client (found/join with no address; steady-state
  re-registration after a restart).
- [`design-persistence.md`](design-persistence.md); the Phase 9 vault/snapshot/at-rest
  slice plan (9a–9h).
- [`design-chunked-transfer.md`](design-chunked-transfer.md); chunked large-file
  transfer (`FileManifest`, per-requester bytes budget, whole-file CID verification).
- [`design-dms-friends.md`](design-dms-friends.md); DMs as 2-person servers + the
  in-band friend-request path (`KIND_DM_INVITE`).
- [`design-admin-invites.md`](design-admin-invites.md) ·
  [`design-grant-revocation.md`](design-grant-revocation.md); owner-serialized admin
  invites and the owner-local authoritative admin set.
- [`design-voice.md`](design-voice.md); **the active block**: E2E group voice
  (media key, `KIND_CALL_SIGNAL`, the WebRTC mesh, and the phase-4 transport plan).
  Cross-check it against [§ Voice](#voice-group-calls); the doc describes the frame
  layer that is **not** built yet.
