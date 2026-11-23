# Ordinary member text without the founder online

Native founder and joined-member text saves use the same durable outbox. The joined
member can save and recover the original local receipt with the founder offline, then
author a new message after restarting with the same registered source. Focused Windows
and Linux CI covers both paths. Chat V2 stays disabled pending peer delivery, shared
projection and the remaining acceptance gates.
[Joined storage and cold authoring CI](https://github.com/Thalpy/Mewtual/actions/runs/45078038029)
[Native join and local save CI](https://github.com/Thalpy/Mewtual/actions/runs/45077351739)

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
The stable `group_nonce` is the authenticated selected
`SignedOwnerProfile.spec().creation`, not the live local Scope incarnation returned
by `scope_nonce_data()`. Cold activation and same-Profile rejoin preserve this value.

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
The general member wire envelope uses a domain-separated channel exporter under the
selected group, Profile, governing Header, epoch and channel. It wraps the exact signed
action and claimed Admission in authenticated padding. Exporting this key leaves the MLS
application sender ratchet unchanged. Receivers still need the original selected Core
prefix to validate the claimed Admission before projection. The joined runtime derives the
blinded topic from the distinct MLS metadata exporter and checks the actual selected
verifier and received topic before opening the envelope. The mounted row can subscribe on
its original native mesh and open bounded frames from that same live registration. The
driver reports receive pressure so a caller can recover missing originals. The receiver can import into an
original channel ledger with exact dependencies. The resulting effect is provisional and
requires the receiver's actual view and current sender Admission. An original opened by
one source cannot be applied through another source with equal cryptographic bytes.
The receiver resolves parents and targets from that ledger's Valid originals. Missing
dependencies return their exact IDs without importing the child or making it visible.
The mounted sources can exchange one sealed original per dependency request over an
existing authenticated TCP connection. The request binds both peers, the selected MLS
channel and its one ActionId. The exact encrypted request binds the reply. An unavailable
reply describes only that provider's selected source. It cannot clear another member's
history or justify importing a child with missing parents.
Frame-triggered recovery can request a signed direct dependency on the exact original
TCP carrier. When an authenticated reply itself needs a signed parent or target, the
joined receiver can follow that edge on the same carrier while the reply and its
ancestors are still current. Explicit standalone requests supply no follow-up route.
The desktop attaches a provider to the original joined row after Join or authenticated
reopen. The founder attaches one on its original actor after the first Add, including
after cold reopen. Each incoming request captures that same installed row again and
can read only an exact original in its own selected outbox and Admission.
That row also attaches an exact native gossip listener. It can attempt up to four
dependency recoveries per frame through selected local storage or the original carrier,
with one network request in flight. For each requested ActionId, local replay uses up to
eight lookups in the exact row's outbox and selected archive to recover signed parents
and targets. Unresolved originals stay outside the ledger.
The receiver retries its subscription while the row remains installed.
The mounted joined row can prepare a paid channel ledger and apply frames from its own
receiver. A final native callback checks the original receiving scope and the prepared
Core view before revealing a body. The ledger holds up to 64 originals or 1 MiB of
canonical action bytes. Durable replay and catch-up must address pressure at this limit.
An exact Valid original can be prepared as encrypted action chunks and a manifest
bound to the local vault, Profile, initial tenure and channel. The manifest preserves
the action's original Admission and signed bytes across transport epochs. Native
storage must select the manifest before cold replay can treat it as saved history.
Semantic replay restores dependency knowledge only after the original signature and
governing Admission verify. A retired author's dependency also needs the exact frozen
Admission from the same selected prefix. Replay issues no provisional projection.
A reopened joined row can already replay an exact original from its own outbox by ActionId.
The history disk implementation verifies a complete WAL, keeps the original file
identities through the barriers and selects the exact manifest under a keyed ActionId
locator. Recovery completes the same bytes after a torn bundle or selector. Native
retries retain the presealed WAL and its original file so they can write the missing
suffix. Cold recovery preserves a partial WAL it cannot authenticate. The versioned
catalog declaration binds the exact Current, joined pin and native registry descriptors
and leaves every earlier namespace entry byte-identical. Earlier catalog grammars
refuse it. Its encrypted restart seed preserves both catalog frames and the exact
history root marker. Seed validation also requires the original governance and outbox
markers, joined pin index and native registry leaves.
The empty joined layout can stage and select these physical files under the existing declaration claim.
Native startup reopens an existing joined layout and retains the original marker when the
vault has a selected archive declaration. On that layout, the joined receiver can archive each exact
verified original and use saved originals for semantic dependency replay. The mounted archive
supports a single registered channel and enforces 2,048 records and 256 MiB of live file bytes
plus one bounded WAL. The startup claim becomes visible to earlier readers only after its
separate staged file is complete and durable. A partial first stage leaves the joined layout
readable. A complete declaration seed can finish exact torn output and retire its claim after
a power cut. Fresh desktop vaults select the archive on the same owning session before
exposing native rows. Migration of populated vaults remains open. The disk owner checks a bounded
physical inventory against separate record and byte limits before admitting a new WAL.
The quota check charges partially written files at their final size. At capacity,
existing originals and idempotent retries remain readable and new writes are refused.
The native reader can page exact archived Create IDs or find actions for a single
Create in byte order under the held directory. New encrypted selectors bind the
target found in the signed original. The reader opens the full bundle to derive
the target from an older selector. Both versions require the same physical bundle
inventory, and an unsettled WAL refuses the page. Each candidate still needs its
full selected bundle and governing replay before a message body can be shown.
For one archived Create, the joined source opens its exact selected actions
and resolves their required originals within a paid 64-action, 1 MiB ledger.
The source checks current governing Admission and real signatures again. The
projection remains provisional and is invalidated by an intervening archive
write or retirement of the governing or native view. An incomplete target or
dependency set refuses projection.
The mounted joined source prepares one exact debt item for its original mesh and releases
the metadata slot before awaiting the driver. The caller, original Core Admission and
selected MLS fence must still authorize it at the driver's submission boundary. A real
`Submitted` result produces a receipt that a fresh capture of the same registered row can
checkpoint. A successful desktop joined send or exact saved retry resolves the local receipt,
then wakes the single worker for that installed row. The worker retries unchanged debt after
interruption, polls for debt after an authenticated reopen and stops when the row retires.
Foreground reads and saves share the row's local work permit with publication preparation
and checkpointing. Transport submission runs after that permit is released.
The main renderer receives an action ID notification and reads the exact live projection
through a fresh authenticated invocation. Provisional bodies are labelled separately from
local receipts. **Saved received messages** pages the selected joined archive after
restart. For each of at most eight Create IDs, a separate native invocation enters the
guarded projection before returning a body. A live update in the active view retires the
older saved body and cancels a pending archive read. The renderer labels a partial page
when any individual original cannot be replayed. Recovery beyond the live worker's
bounded direct dependency window remains open.
The native row serializes live application, archive selection and body publication.
A failed archive write for a live mutation holds that target out of archive replies
for the installed row. An exact selected retry clears its entry. This bounded in-session
state stays with the row through receiver reattachment. Durable protection after
unsettled mutations and a process or vault restart still needs a selected negative record.

The installed founder Sync node can now prepare and open ordinary encrypted frames on
the joined member's selected topic after the first Add. Its exact Core Admission is
captured during the original Core and MLS installation. An equal replay from a
different verifier, a changed selection, or a retired founder source refuses the
transport operation. The mounted founder source now reads its exact outbox debt,
rechecks the original held governing Head, and releases the runtime loan before
the mesh wait. Only a submission acknowledged by the original driver can
checkpoint that debt on a fresh capture of the same row. The desktop worker
wakes after the saved receipt and the first governed Join. It shares the row's
local work permit with saves, history, and the founder's Join handler and releases
it before awaiting the driver. The founder attaches one receiver to the original
actor after the Add. It verifies each signed original under its current governing
view and holds a paid live ledger for that subscription. The main window reads
that projection under the same native publication guard as a joined row.
For an authenticated incoming action that names at most four missing direct
dependencies, the founder can look up those exact IDs in its own saved outbox.
An ID absent there can be read from the same channel's selected received
archive. It uses the same eight-lookup limit across the frame's whole local
causal slice, including the saved parents of archived replies. The current
Core verifier replays each matching original for dependency validation.
The founder saves exact received originals in the mounted
history namespace and can read their projection after its live receiver
retires. Both desktop roles use the same bounded archive replay and recheck
the archive selection and active Core view before returning a body.
The native founder can reopen a selected first Add after authenticated startup.
The registered joined row can reconnect using an explicit retry of the same
invitation and approved IP. A joined member can request the founder's own saved
original on that connection. The founder can likewise recover a missing direct
dependency from the frame's original connected carrier and retry the signed child.
It attempts up to four recoveries per frame, one in flight at a time. Each reply and
archive write requires the same live subscription and selected founder state.

The desktop reply composer seals the exact target ActionId with the existing
Create token and body. Native checks the original durable receipt first. A
new reply resolves its Create parent from the selected local outbox or
received archive in the same channel and checks the exact signature and
Admission through Core. A changed target conflicts with an already saved
token. A missing parent leaves the request unsigned.
The native Edit and Delete commands author only against an exact Create and
explicit revision in the member's own outbox. These commands retain signed
originals and exact retry receipts. Display of the current own-message
projection and its editor controls still need to use that revision evidence.
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
