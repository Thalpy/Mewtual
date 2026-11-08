# Ordinary member text without the founder online

Design checkpoint, 7 November. This is the next integration boundary, not an enabled
feature or a completed implementation. The general Core verifier's ten focused tests
pass; member source/signing, independently durable outbox and native routing remain open.

## Two independent paths

The Profile founder owns the governing Header key. Other devices retain their own
MlsDevice keys. A new founder Append can certify an exact action, but ordinary member
Create/reply, own Edit/Delete and causal Join must work without that Append. Signed
originals merge additively. Certification may later upgrade presentation evidence;
it cannot replace original action signatures, erase branches or gate local sending.

The current founder A/I/S/B/WC path remains a separate certification/control path.
Its founder/genesis-only context, one pending child and physical closed inventory are
not a general member outbox. Do not relax those guards, share the founder key or use
a legacy/v1 joined checkpoint as V2 provenance.

## Signing boundary

1. An actual warm-member owner retains the SAME whole Server, MlsDevice, native binding,
   original source generation and independently trusted V2 pin/selected Admission.
   The joined device currently lives inside ChannelSync. No graph clone or extracted
   raw device becomes a new source. Cold 20/50-member graph restore is still separate.
2. Reserve an independent signing/dependency payment BEFORE producing or copying the
   draft, its canonical bytes, dependencies, plan and result buffers. Clock/RNG flow
   through existing runtime seams once and their results stay with the original draft.
3. Core validates an immutable unsigned ActionDraft using the unchanged canonical action
   grammar and the same exact delegation/minimal signed dependency rules as ingestion.
   It issues a non-Clone opaque plan only for a current actual OPEN Admission. Neither
   Admission DATA labels nor a dummy zero-signature SignedAction supply authority.
4. The actual native/UI/source guard is outer to the Core original-group reader. Inside
   that reader, recheck the exact draft/plan/Admission/OPEN/source and full device key.
   Hold it through the concrete bounded device key call and caching raw64. No await,
   filesystem access, payment wait, exporter or SDK lookup occurs after final admission.
5. Cache raw64 BEFORE every late callback/negative. Materialize and verify the SignedAction
   outside writers from the SAME raw; completed retries call neither signer nor RNG.
   The opaque plan, packet and payment move whole. Matching bytes from another source,
   generation or rejoined A2 cannot adopt the earlier A1's pending owner.

A Remove or governance fault closes flags immediately. An already admitted key operation
may complete and retain historical DATA; the original group reader delays CLOSE
COMPLETION until raw caching ends. It does not prevent removal selection or reopen the
old Admission. New signing and active projection refuse after closure. Root must also
order lock/shutdown against its actual native admission; a Core guard alone is insufficient.

Signing itself uses MlsDevice's retained SignatureKeyPair and needs neither channel-secret
export nor sender-ratchet mutation. Transport encryption is a separate later operation.
Across a crash, reusing exact stored raw is straightforward; ambiguous pre-raw signing
requires an explicit durable uncertainty policy and must not be claimed solved here.

## Independently durable outbox

Use a separately versioned member namespace. Its stable context binds vault/layout,
trusted pin, Profile/tenure, original group nonce and full device; each immutable entry
also binds its original Admission. Transport epoch belongs to envelope attempts, never
to the original action identity or stable request-token namespace.

A selected local Head covers a paged exact-request receipt index, immutable original
Action chunks and paged publication debt. Lookup exact token/request/context before new
authoring. Different text/kind/target/parents is a conflict; an IO/decode error never means
absence. An exact saved historical receipt remains readable after removal without granting
new OPEN, signing or publication rights.

Prepare one complete, bounded, presealed old-local-Head-to-new-local-Head plan. A distinct
actual member-source save admission covers WAL plus parent barrier, exact immutable
outputs plus barriers, fixed selector CAS and parent barrier. It invokes no signer, SDK,
projection or network. Once admitted it may finish after cancellation while retaining its
actual Files/session lease/End and payments. Cache SavedLocal before late failure. Only
then release this request's local semantic pending turn. Fresh active projection and
network submission require separate current guards; no founder B/WC wait is involved.

Cold completion authenticates the complete typed WAL, original signature/delegation,
dependencies, parent/candidate roots and quotas before finishing exact stored bytes.
Already selected output can finish barriers idempotently. Torn pre-MAIN scratch and
partial outputs need distinct narrow recovery roles. Unknown, unsupported, foreign and
nonmatching state stays preserved. No cold re-signing or source/OPEN resurrection follows.

Publication reads bounded debt pages and reseals original bytes under a separately
admitted current envelope. Persist any mutated sender state before export. Record local
submission completion through its own durable checkpoint; a crash may resend the same
signed original. A transport submission is not a peer durability receipt or certification.
Original audit chunks and request receipts remain retained after scheduling debt clears.
Removal changes active view, never deletes the audit. Unproven late removed actions stay
Held pending the separately authorized moderation path; ordinary concurrent edits resolve
automatically without a user repair prompt.

## Resource and acceptance gates

Reuse the general cold Action chunk codec and private-page crypto/native hold/barrier
patterns. The current founder Retain format's 66,057-byte/two-chunk Create profile cannot
silently cover the general 128KiB/64-parent action grammar. Do not add member files to its
closed inventory or reuse Automerge durable_chat/oldest-evicting managed outbox as V2 state.

Independent signing, dependency, outbox work, metadata and reader classes require real
compiled simultaneous invoices before activation. Proposed caps are not measured fits.
Keep fee-last tails, bounded active ownership, paged durable debt, persistent disk counters
and original reader/IO Ends; pressure must preserve receipts instead of evicting them.

Required regressions include actual member signing with founder absent; wrong key/source/
Admission/plan/draft; both Remove/sign orderings with real close drain; late raw-cache errors
and retries; exact once-Clock/RNG behavior; outbox crashes at every barrier/selector cut;
reordered dependencies/duplicates; transport epoch changes; A1/A2 isolation; bounded debt
and memory pressure; original File/session/payment destruction; real 20/50-member bubbles,
restart and native dispatch. Complete root/native/frontend and adversarial review remain
mandatory. Two-device physical acceptance is unavailable on the current single desktop.
