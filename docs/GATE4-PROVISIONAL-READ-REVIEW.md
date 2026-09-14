# Gate 4: post-succession newcomer review

**Current Gate 4 position (2026-09-14).** The user accepts the combined scheduling checkpoint
`6b71d96` with no production or test changes required. This closes block 1 of the four remaining
work blocks. Three remain: (2) durable Closing overlays and repeated-owner tenure, (3) runtime
signed fault repair, and (4) combined Gate 4 acceptance. These are unequal work areas, not a
percentage or time estimate.

The [Closing overlay foundation design](GATE4-CLOSING-OVERLAY-REVIEW.md) passed user review at
`d576af2` on 2026-09-14 with no required changes. Its core/store implementation now distinguishes
explicit local acceptance from an ordinary failed Save, preserves sequence and timestamps across
restart, shares existing storage/reference accounting, and holds annotated entries out of ordinary
Apply and retirement. The 131 affected local tests, Clippy and three isolated mutations pass;
implementation review passed and OVERLAY-TEST-001 is closed by the re-review of `65db6ac`.
The user accepts the corrected [atomic handoff design](GATE4-OVERLAY-HANDOFF-REVIEW.md) at
`dd2fbc0` and closes HANDOFF-001. The [core/store implementation](GATE4-OVERLAY-HANDOFF-IMPLEMENTATION-REVIEW.md)
is pushed at `bf37cc4`; 202 local Studio tests and the dedicated handoff/mutation workflows pass.
The user requests HANDOFF-002 (P2): reference inventory must honor required intent metadata before
installing a complete pixel pin set. The correction at `62f06d4` passes 24 focused local tests,
Clippy and its isolated mutation/restored regression; GitHub checks and user re-review are pending.
The other reviewed handoff boundaries have no additional finding. Evidence is in [HANDOVER](HANDOVER.md).
It persists Prepared, the whole signed source, then Completed before releasing the overlay hold;
ordinary intents remain pending for receipt settlement. A local source dependency also prevents
missing metadata from exposing an unfinished batch after restart. This is still block 2. No native overlay command,
overlay replay/disposition, provisional-preview writes or new tenure authority is enabled.

**Combined scheduling checkpoint `6b71d96` (2026-09-13; user review PASS).** The three-member actor
fixture retains three preview reservations: three real deliveries, or two deliveries plus a
cancelled parser or a cancelled transport request. It restores only owner transport reachability and
requires both Registry and Studio checkpoints, exact content and durable reopen within 40 seconds
of simulated scheduler time. Preview custodians remain retained throughout; no restart, unwatch,
additional user Read or expiry of all original seeds enables the installations.

This exposed a fast-retry defect: the provider's shared head rail allows a two-request burst, spent
by Registry and Studio discovery. An immediate provisional head can repeatedly fail before a seed
is fetched. The bounded preview queue now waits one second before its fresh head attempt, without
holding a reservation or resetting that deadline on duplicate Hints. Queued targets defer new
same-target page work so repeated Reads cannot bypass that wait. Authoritative work retains
priority. Both new conditions have isolated mutation failures and restored-source passes.
The broad run on `487cb0e` passes 165 tests (one opt-in profile ignored); final `6b71d96` passes
11 targeted actor tests, the shared native fixture, Clippy and 19 native Studio tests on GitHub,
including both existing native mutation checks. The native abort regression now waits for actual
final guard release after its unchanged held-while-paused assertions. Native/UI response shapes
are unchanged. Exact validation is recorded in HANDOVER. The user's review accepts this bounded
checkpoint with no actionable production/test finding or required changes. The reviewer inspected
source and actual GitHub native logs, but did not rerun Cargo locally; local actor/mutation evidence
remains the recorded execution evidence. The broad 165-test run was on `487cb0e`, not final `6b71d96`.
The combined authoritative target is Index plus its Registry bucket; Index/Flipnote previews supply
pressure. Cancelled transport is a controlled lower-layer model, and 40 seconds is simulated scheduler
time. Priority covers pending authoritative work, not preemption or a universal latency guarantee.
Closing overlays/repeated tenure, signed repair and full Gate 4 acceptance remain outstanding.

**Actor/native preview checkpoint (2026-09-13, implementation PASS at `a89bde6`).** The user's
source review found no blocking production defect and raised NATIVE-TEST-001 (P3): the existing
native test only exercised an ordinary Index. The test-only correction at `c60de4e` requires real
actor-produced Index/Flipnote previews through `invoke_custody`/`read_view`, full content and
explicit preview flags without phase/publication. Expiry after successful conversion must fail
only the preview-delivery fence while the other native checks stay valid.
[Native validation](https://github.com/Thalpy/Mewtual/actions/runs/34779683539) passes all 19 Studio tests;
changing the trust flag and removing only the final preview-validity condition each fails at
its intended assertion. Byte-for-byte restoration and both reruns pass. The user re-review of
`a89bde6...134394e` closes NATIVE-TEST-001 without further changes. The reviewer also inspected
the GitHub job logs (PR merge checkout for `c60de4e`), without independently rerunning Cargo.
This does not establish the combined authoritative-progress acceptance case.
TAIL-TEST-001
is closed by the user's PASS of `2a1814e` against `47bf098`. The existing receiver now schedules
provisional discovery, seed fetching and finite signed tails outside vault custody; cold workers
hold their process permits through cancellation and release them on completion. Ready results
and native delivery share the original seed reservation. Native Read/List expose a separate
`awaitingTenureReceipt: true` result with `provisional: true`; installed sources take precedence,
and ordinary Apply cannot use a preview epoch. Native uses request/session/instance checks plus
a bounded actor handoff for final conversion. Lock clears the volatile cache. No frontend layout
or editor implementation is changed.

The two previously ignored newcomer preview cases now pass, together with both known-CID PIX
fetch/reopen cases. Runtime custody tests cover three real ready previews, native delivery retaining
a slot, authoritative Hint release, and handoff cancellation/expiry. Exact final checks are in
HANDOVER. This does not yet close the combined owner-reachability/competing-installation regression
or all of block 1. Blocks 2-4 remain durable Closing overlays/repeated tenure, runtime signed repair,
and combined Gate 4 acceptance. Earlier statements below describing missing actor/native delivery
record the boundaries of those earlier checkpoints, not the current implementation.


Status: user re-review of `7393165` passed. TEST-001 and TEST-002 are closed for the tests and
instrumentation; PR-001 is closed as a design finding. No further closure changes were requested.
The reviewer inspected source and did not execute Cargo. The approved provisional-read path
remains unfinished. The user accepted the capacity foundation in `1c90c41` without requested
changes. Published checkpoint `0b32bad` connects real provisional head discovery and candidate
custody; the user adversarial review passes both this checkpoint and the separate interrupted-
successor tests in `2f5a8a0`. Neither requires code changes. The reviewer inspected source without
running Cargo and suggested a stronger delayed-completion expiry test, included in the current
seed slice. That slice adds one-shot transport and detached typed validation, with unconfirmed
results retaining their original capacity and deadline. It does not yet connect the actor's
scheduler, authenticated tail or native preview delivery. Focused regressions, three deliberate
mutations, formatting and Clippy pass. The user review accepts seed checkpoint `b3e54dc` against
`2f5a8a0` with no blocking implementation defect. Its non-blocking P3 follow-up is now implemented:
the real raw-change and typed-schema gates accept a correctly signed seed with noncanonical root
operation order, while the final comparison rejects it. Both target types pass; disabling that
comparison fails the test and restored source passes. The reviewer did not run Cargo or mutations.
The user review passes signed-tail checkpoint `47bf098` against `b3e54dc`, with no blocking
production defect. It closes the prior canonical-encoding P3 and requests one P3 test correction,
TAIL-TEST-001, described below. The review was source-only; Cargo and mutations were not run by
the reviewer. Actor/native integration remains pending.

## Signed-tail checkpoint (`47bf098`; bounded implementation passed review)

TAIL-TEST-001 identified a false-positive inner-document fixture: signing and sealing for B,
then changing outer routing to A, exercised failed decryption with A's key. The new dedicated
regression covers both Index and Flipnote. It manually pads the correctly signed B operation,
encrypts under A's key, and supplies A's valid outer envelope. Before receiver preparation it
requires successful open, byte-equivalent decoded operation, valid signature and a mismatched
inner ID. It then requires `ReplError::EpochScope` directly from `prepare_tail`. Removing only
the inner-ID guard makes the regression fail; production source is restored byte-for-byte and
all six provisional core tests pass. This is a test/documentation correction, not a production
logic change or a claim that actor/native integration is accepted.

The seed now retains its private parsed graph and bounded volatile operation metadata. It never
constructs an `EncryptedDoc`, `CheckpointOrigin`, epoch gate or verified receipt. At most 32
operations / 512 KiB enter each page, with ciphertext checked before encoding copies. Current MLS
sealing, exact admitted author key and inner signature are checked before cold parsing. The
parser requires physical/logical/channel scope, matching Automerge actor, known predecessors
descending from the seed, typed operation semantics and the exact domain marker. Its aggregate
limits are 20,000 distinct operations / 4 MiB of signed content, with the existing exact Studio
checkpoint/recovery preflight. These are bounded preview checks, not owner quota/admission grants.
Any rejected page consumes the candidate; no partially checked page becomes a ready result.

Sync owns fixed-prefix cursors and limits aggregate page traffic to 20,001 pages, 20,000 transmitted
operations and 16 MiB. Restart, checkpoint-required and historical-authorization-required outcomes
discard the pass. A repeated cursor is refused. `tail_complete()` means the supplied finite prefix
ended; the future runtime must require it before exposing a read fallback. Scoped seed inspection
remains a backend data seam and is not a native preview. No renderer hook has been added.

The distinct page binding preserves the original installed/absent-epoch watch while requesting
the candidate's concrete successor epoch. It cannot complete through ordinary Studio page APIs.
The lower transport retains both outbound and provisional seed capacity. Completion, detached
parsing and inspection preserve the original deadline and hint generation; app checks add mount,
numeric server and channel. Tests exercise genuine Index/Flipnote multi-page providers, stale
membership/watch/attempt/provider/runtime, deadlines, cancelled lower custody, and the app's
unchanged canonical reads, refused Apply and empty documents/journals after reopen.
The reviewer accepted these bounded paths by source inspection. The existing tests exercise
per-page operation-count and retained-byte guards directly and real multi-page continuation, but do not drive
every aggregate traffic counter to its limit. Those counters' acceptance is inspection evidence.

This remains block 1 of the four remaining Gate 4 blocks. The scheduler, process worker permits,
reserved-lane fairness, periodic revalidation, native delivery custody and ignored newcomer reads
are still outstanding. The allocator tests here do not establish runtime install progress.

## Reproduced boundary

`studio_exchange/tests/succession/joining.rs` starts with a real Alice-authored frame and
PIX blob. Bob receives the signed frame and fetches the bytes over the network. The existing
eligible-history fixture fills Bob's source; the Closing variant additionally arranges an
old-owner seal. The staged-Remove fixture supplies an observed Alice-to-Bob transition and
restores strict single-committer policy before any Studio actor runs.

Bob's ordinary actor worker issues and installs its own epoch-one receipt and Registry pointer.
An independently checked tail edit follows. Bob then shuts down and reopens its sealed vault
on a fresh network without Alice. A genuinely new device joins through Bob's invite.

That join reuses Alice's low MLS leaf: **Bob ceases to be owner and the newcomer becomes owner**.
Bob observes the new transition. The newcomer has Unknown tenure because Welcome does not
prove when that tenure began. This behavior is already specified in
[`design-epoch-close.md` section 11](design-epoch-close.md#11-owner-succession-adoption-inside-the-first-receipt).
Treating the join as preserving Bob's tenure would make the acceptance fixture incorrect.

The restarted Bob still has its receipt, checkpoint, tail and pixels. The newcomer can fetch
the exact PIX bytes when the fixture supplies the CID, including a subsequent offline fetch
from its reopened vault. This is **byte availability with a known CID**, not discovery or a
displayable Flipnote. The current receiver's `Hint` branch does not fetch/render unconfirmed
Studio history. No newcomer Studio source is installed, and `Read` returns `None`.

The two ordinary tests require byte availability and preserve the authority boundary. Two
explicitly ignored acceptance tests additionally require a provisional read of the same
history. They are expected failures, not completed acceptance coverage. They must gain the
explicit confirmation-state assertions below when that result exists; a generic provisional
edit flag alone cannot establish whether an owner has confirmed the history.

The revised fixture captures Registry state before discovery and compares it after discovery,
Read, a refused Apply and vault reopen (TEST-001). Its single-value test observation is emitted
only at the real completed Studio `Hint` branch, after watch/mount/request/member authentication.
It requires the expected target, endpoint, full responding identity and exact receipt before
checking the missing preview (TEST-002). No hint or selection is injected. The final offline
fetch still restores identity from an in-memory snapshot: it proves blob persistence after
vault reopen, not persistence/recovery of the newcomer's complete identity/session state.

Validation of accepted `7393165`: all ten active succession cases and both ordinary newcomer discovery
regressions pass. Explicitly enabling the two preview cases still fails at the missing-read
assertion, after the new hint/Registry/Apply guards and byte checks pass. Three temporary
counterexamples fail at the intended assertions: suppressing Studio head requests, writing a
signed Registry pointer from the hint and persisting an orphan intent before the correct Apply
refusal. Source restoration is byte-for-byte. Formatting and library/test clippy pass; commands
and logs are recorded in [HANDOVER](HANDOVER.md). No preview allocator or capacity-progress
implementation is included or claimed to pass by this revision.

The refused-Apply guards establish unchanged specified Studio/Registry document and journal
state, not literally zero filesystem writes: the app transaction may persist its server snapshot
before store-level rejection. The observed Hint is historical test evidence, never a current
lifecycle capability. Explicit preview/trust assertions and the lifecycle scenarios below remain
required when the runtime result exists.

## Capacity foundation (`1c90c41`; user review passed)

`registry_seed/capacity.rs` adds an opaque provisional memory reservation within the existing
four-slot sync pool. Provisional reservations can use only three slots; ordinary authoritative
discovery uses the same allocator and can still use all four. This reservation authenticates
nothing and exposes no seed, preview, read or installation operation. Existing discovery retains
its reservation through head/seed jobs, completions and transport cancellation. A raw Hint still
releases that reservation on completion.

Tests exercise both authenticated Studio and Registry seed paths while three provisional
reservations remain held, a real Hint in the fourth slot, failed preparation, completed-result
custody and lower-transport custody after cancellation/expiry. An allocator test models cloned
parser/delivery keepalives; that checkpoint added no provisional parser or transport. Those are allocation
tests, not the full actor scheduling/install regression required below. Preview fetching, ready
preview ownership, priority/fairness, lifecycle fencing and native delivery remain pending.

Capacity validation: all 24 shared seed tests pass (6 new), all 10 active succession cases pass,
and both ordinary newcomer regressions pass. The 2 provisional-read cases remain ignored; their
explicit expected failures were last run at `7393165`. The new cross-class seed test rejects two
temporary mutations: a fourth provisional reservation and unaccounted authoritative discovery.
Source restoration and local toolchain/disk workarounds are recorded in [HANDOVER](HANDOVER.md).
Root formatting and sync/app library/test Clippy pass. The reviewer inspected source without
running Cargo and accepted this allocation-only scope with no new finding or requested change.

## Provisional head discovery (`0b32bad`; user review passed)

Distinct sync and app prepare/fetch/complete types now acquire provisional capacity before a
real head request, pass its keepalive to the existing cancellation-aware transport and retain it
in completed responses and opaque candidates. No raw receipt or authoritative Hint can construct
the candidate context. The shared head authentication checks request transcript, response
signature, bounded canonical encoding, logical scope, member/provider identity, sync instance,
membership epoch and attempt generation. This proves member delivery only: the candidate receipt's
own signature and claimed current/historical ownership remain unverified at this metadata stage.

An owner-proof, repair or absent-receipt response produces no candidate and releases its custody.
This branch neither creates nor supersedes an owner selection; the future scheduler must use
fresh authoritative discovery when appropriate. Existing selected passes retain their authority
under the normal rules. Candidate expiry is fixed at 60 seconds from preparation; a head response
must still complete within its original 10-second deadline. Retrying cannot renew a candidate.

The copied Studio watch is checked at prepare, completion and inspection. Same-key rewatch and
any later head preparation revoke the old candidate, even if a replacement request is dropped.
That attempt binding also prevents a candidate from surviving a later owner selection; existing
authoritative selections retain their separate generation and are not revoked by hint attempts.
Current membership, proven endpoint identity and sync instance are rechecked on inspection.
The app wrapper adds mount/server/channel checks without holding a Server/store borrow across
the network wait. Eligibility can disappear while capacity remains charged to lower owners.

These adapters are exercised directly; the actor receiver does not schedule them yet.
This checkpoint stops at scoped, unconfirmed candidate metadata. It implements no seed/tail
fetch, parser, ready preview, actor fairness, native result/event or late-delivery fence. It does
not make the two ignored provisional-read cases pass or satisfy the full competing-class actor
progress regression. Those obligations, including receipt/seed verification before preview,
remain below.

Discovery validation: 24 head tests (7 new), 24 shared seed regressions, 2 new app candidate tests,
7 existing app discovery tests, 10 active succession cases and 2 ordinary newcomer regressions
pass on restored source (69 total). The 2 preview cases remain ignored; this checkpoint does not
claim a new explicit run of those known failures. Three temporary mutations fail at the intended
guards: skipped response-signature checking, candidate use after same-key rewatch and unaccounted
lower transport custody. Sources were restored byte-for-byte. Commands and logs are in HANDOVER.
Root formatting and sync/app library/test Clippy pass. User adversarial review passes this
discovery/custody boundary. Seed validation and preview integration can continue.
The two head paths share latest-attempt state: newer provisional discovery can supersede an
in-flight authoritative head for the same target. The future scheduler must coordinate them;
reserved capacity alone does not prevent this interference.

## Approved implementation boundary

Implement a bounded **read-only provisional fallback** for a known Studio logical key through
the existing receiver and native read seam. This is the first part of the existing design's
provisional reading/editing requirement, not a replacement for overlays or historical authority.

1. Retain a private, authenticated hint context from a completed head response: mount/server,
   logical target, requester, responding current member, membership epoch, receipt bytes,
   request lifetime and the existing copied watch/attempt/selection generation bindings.
   Unwatch followed by rewatch of the same key invalidates the old completion even when all
   visible identifiers and the membership epoch still match. Arbitrary caller-supplied receipts
   cannot construct this context. A hint
   supplies a candidate history; it supplies no evidence that its signer was ever owner.
2. Fetch its expected seed and bounded current tail through a distinct provisional-fetch type.
   Reuse the existing transport framing, request/retained-slot budgets, cancellation and detached
   preparation workers. Validate signature encoding, document/channel scope, seed hash, typed
   payload/count/byte bounds and tail binding before producing any preview. Do not produce a
   `HeadSelection`, `VerifiedCheckpoint`, adoption plan or receipt-publication permit.
3. Keep the resulting `StudioPreview` separate from `EpochStudioState`. It is content supplied
   by an authenticated member with **unconfirmed historical attribution**. Seed attribution
   claims must not be presented as independently verified authorship. Current-member signed
   tail operations still need normal cryptographic validation; removed-author historical
   operations need separate historical-authority evidence and cannot be admitted by inference.
4. Expose the fallback through native read with an explicit `awaitingTenureReceipt: true`
   indication and `provisional: true`. Do not introduce a new "confirmed" state for ordinary
   views in this slice. Keep the existing installed-source read preferred. An internal distinct
   preview result must survive the native conversion; returning an ordinary current epoch with
   an indistinguishable flag is insufficient. UI implementation remains user-owned.
5. The preview owns no canonical source, intent, recovery or owner-journal write and cannot
   authorize pruning, settlement, Registry publication or a normal Apply. Use the existing
   bounded retained slots with the **three-preview / one-authoritative reservation** below;
   clear on lock, unwatch, mount/server replacement or membership change. Reopen can refetch.
   Pending preview bytes must count against those limits before allocation, and work must not
   hold an actor/vault lease across a network wait or cold parse. Cancellation invalidates
   eligibility immediately, but the actual transport/parse/native-delivery owners retain their
   capacity keepalives until they release the corresponding memory. Expiry is not a refund.
6. A later fresh current-owner selection enters the existing recovery-first installation path.
   Replacing displayed content with authoritative history waits for successful installation.
   **Evicting an unconfirmed preview does not:** capacity pressure, expiry and lifecycle changes
   may remove it without any successful installation. Emit invalidation and let Read report
   no preview; do not retain an unaccounted display/cache copy to disguise eviction. Failed
   fetch, malformed data, cancellation or expired provenance leaves any installed source and
   durable journals intact. Fence late native delivery with the current request/view/session
   generation; backend invalidation alone cannot retract an object already handed to a renderer.

### PR-001: reserve capacity and scheduling for authoritative progress

The four retained seed slots are shared across Studio and Registry, and discovery reserves one
**before sending a head request**. The original retain-until-installation wording could fill all
four with previews and prevent the discovery needed to replace them. Use this concrete policy:

- At most **three** slots per sync instance may belong to provisional work in total: pending
  hint discovery, fetches, parsing/results, ready previews and cancelled work still held by a
  lower layer all count. Additional watched targets wait within the existing bounded watch rail.
  This is a quota inside the four shared slots, not three extra slots or a separate byte cache.
- The remaining slot is unavailable to provisional work. Reserve it for authoritative Studio
  and Registry head discovery, seed retrieval and installation, keeping its reservation across
  those stages until completion/failure and actual keepalive release. Authoritative work can
  also use any other free shared slot; all existing total slot/byte ceilings still apply.
- A head request in the reserved capacity may return a hint. That result cannot become a
  retained preview or start a provisional seed fetch there. Release it and queue only a bounded
  target retry; any provisional discovery must separately acquire preview-eligible capacity.
  Never wait for a preview slot while holding the authoritative reservation.
- Revalidate watched previews on the existing bounded discovery/retry cadence, including when
  a current owner becomes reachable without a membership change. Pending authoritative Studio
  and Registry work gets priority over new/refilled previews. Alternate eligible Studio and
  Registry work in that reserved capacity, rotating bounded watched targets within each class;
  failures/timeouts advance the turn so one unavailable target cannot monopolize it. Duplicate
  hints cannot reset that turn or enqueue unbounded work.
- Dropping/expiring a preview makes it ineligible immediately. Its slot becomes reusable only
  when all transport, parser, result and delivery keepalives actually release their memory.
  Preserve lower-transport cancellation ownership. A ready preview retains its accounted seed
  slot, but does not retain a completed cold-parser worker permit. Bounded parser jobs release
  their existing process permits on completion; new preview work cannot jump ahead of queued
  authoritative preparation.

Required runtime regression: fill all three preview-eligible slots, including variants with a
cancelled transport or cold parse still holding a slot; make a valid current owner reachable
**without changing membership**; queue competing Studio and Registry discovery. Require both
authoritative installations to complete within the bounded worker schedule without unwatch,
restart or expiry of every ready preview. Assert no fourth preview is admitted, no capacity is
refunded while a lower owner still holds it, hint replies in the reserved capacity release it,
and preview refills cannot starve either authoritative class. This is an implementation
obligation, not a regression claimed to pass before the allocator/runtime slice exists.

This first slice deliberately cannot complete the whole newcomer contract: durable overlay
edits/replay, restart of those overlays, historical-authority transfer and the broader
tenure/repair observations still need subsequent integration. In particular, a newly joined
lowest-leaf owner with Unknown tenure remains unable to issue a receipt. No step may derive
tenure from Welcome's current epoch, a hint, a receipt's claimed tenure or one peer's statement.
Supplying new independent tenure evidence would require a separate reviewed protocol change.

## Acceptance and adversarial review focus

- The two ignored provisional-read cases should pass with exact frame CID/length, original
  claimed attribution and the successor-authored tail, plus explicit unconfirmed-history
  state. Canonical source/journals must remain untouched until actual owner confirmation.
- Wrong key/channel, wrong seed hash, malformed or oversized seed/tail, removed-author input
  without evidence, stale membership, expired request and replaced mount must fail before
  preview exposure. Cancellation/lock must release custody and never revive a stale preview.
- Exercise unwatch/rewatch of the same key before completion, membership change during cold
  parsing and late native delivery. Reject stale generations even if IDs/epoch still match;
  invalidate authority immediately while retaining capacity until actual resource release.
- A malicious authenticated member's hint can influence its provisional preview, never current
  history or verified attribution. Review the parser/result types for accidental promotion into
  current-owner authority or ordinary Apply; review aggregate limits for a parallel shadow cache.
- Preserve the eight user-accepted succession cases and ordinary unopened-provider discovery.
  Re-run the current-byte tests and the explicit provisional acceptance tests. Neither this
  proposal nor those eventual passes closes Gate 4, signed repair or full-suite acceptance.
