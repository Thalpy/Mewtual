# Gate 4: post-succession newcomer review

Status: proposed next runtime slice; user-provided adversarial review requested before
implementation. The associated checkpoint changes tests and documentation only.

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

## Proposed implementation boundary

Implement a bounded **read-only provisional fallback** for a known Studio logical key through
the existing receiver and native read seam. This is the first part of the existing design's
provisional reading/editing requirement, not a replacement for overlays or historical authority.

1. Retain a private, authenticated hint context from a completed head response: mount/server,
   logical target, requester, responding current member, membership epoch, receipt bytes and
   request lifetime. Arbitrary caller-supplied receipts cannot construct this context. A hint
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
   bounded retained slots for volatile preview custody; clear on lock, unwatch, mount/server
   replacement or membership change. Reopen can refetch. Pending preview bytes must count
   against those limits before allocation, and work must not hold an actor/vault lease across
   a network wait or cold parse.
6. A later fresh current-owner selection enters the existing recovery-first installation path.
   Discard/supersede the preview only once that path succeeds. Failed fetch, malformed data,
   cancellation or expired provenance leaves any installed source and durable journals intact.

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
- A malicious authenticated member's hint can influence its provisional preview, never current
  history or verified attribution. Review the parser/result types for accidental promotion into
  current-owner authority or ordinary Apply; review aggregate limits for a parallel shadow cache.
- Preserve the eight user-accepted succession cases and ordinary unopened-provider discovery.
  Re-run the current-byte tests and the explicit provisional acceptance tests. Neither this
  proposal nor those eventual passes closes Gate 4, signed repair or full-suite acceptance.
