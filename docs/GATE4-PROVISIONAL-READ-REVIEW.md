# Gate 4: post-succession newcomer review

Status: user re-review of `7393165` passed. TEST-001 and TEST-002 are closed for the tests and
instrumentation; PR-001 is closed as a design finding. No further closure changes were requested.
The reviewer inspected source and did not execute Cargo. The approved provisional-read path
remains unimplemented; the next bounded checkpoint adds its shared capacity foundation below.

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

## Capacity foundation (checkpoint; awaiting user review)

`registry_seed/capacity.rs` adds an opaque provisional memory reservation within the existing
four-slot sync pool. Provisional reservations can use only three slots; ordinary authoritative
discovery uses the same allocator and can still use all four. This reservation authenticates
nothing and exposes no seed, preview, read or installation operation. Existing discovery retains
its reservation through head/seed jobs, completions and transport cancellation. A raw Hint still
releases that reservation on completion.

Tests exercise both authenticated Studio and Registry seed paths while three provisional
reservations remain held, a real Hint in the fourth slot, failed preparation, completed-result
custody and lower-transport custody after cancellation/expiry. An allocator test models cloned
parser/delivery keepalives; no provisional parser or transport exists yet. These are allocation
tests, not the full actor scheduling/install regression required below. Preview fetching, ready
preview ownership, priority/fairness, lifecycle fencing and native delivery remain pending.

Capacity validation: all 24 shared seed tests pass (6 new), all 10 active succession cases pass,
and both ordinary newcomer regressions pass. The 2 provisional-read cases remain ignored; their
explicit expected failures were last run at `7393165`. The new cross-class seed test rejects two
temporary mutations: a fourth provisional reservation and unaccounted authoritative discovery.
Source restoration and local toolchain/disk workarounds are recorded in [HANDOVER](HANDOVER.md).
Root formatting and sync/app library/test Clippy pass. The checkpoint awaits user adversarial
review before the next provisional discovery/fetch integration slice.

## Proposed implementation boundary

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
