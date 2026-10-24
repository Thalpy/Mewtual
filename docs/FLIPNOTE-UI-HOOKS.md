# Flipnote UI hook guide

Last checked: 2026-09-15. **Gate 4 is incomplete; Gate 5 has not started.** The user closes
HANDOFF-002 and accepts the bounded Closing-overlay core/store handoff, explicitly excluding
actor/native activation and full Gate 4 acceptance. Finish Gate 4 before starting Gate 5.
Durable overlay Save is still unavailable in native. The next integration checkpoint addresses
bounded preparation/signing and the overlay lifecycle. The user accepts the
[detached inspection proposal](GATE4-OVERLAY-RUNTIME-REVIEW.md) at `0b28f06`, with no findings.
Its read-only `studio_overlay_read` implementation is registered at `d5ca2ff`; the
[implementation review](GATE4-INSPECTION-IMPLEMENTATION-REVIEW.md) of `0b28f06...c47ae0b` now
passes. The user closes P3 INSPECTION-TEST-001 after accepting the digest-specific regression
and size-only mutation. Its correction at `d38df93` passes the local regression and mutation/
restored test, plus the complete GitHub inspection/mutation workflow. No production correction
is required. The command/result contract
is unchanged. Durable overlay Save is still unavailable, and Gate 4 remains incomplete.
The current [handoff signing checkpoint](GATE4-HANDOFF-SIGNING-REVIEW.md) splits core work into
detached typed preparation, one-operation signing turns and detached complete-source assembly.
It awaits adversarial review and adds no native command or UI write capability.
The [four implementation handoffs](GATE4-AGENT-HANDOFFS.md) now divide the remaining work into
runtime Save/handoff, manual/provisional lifecycle and repeated tenure, signed repair, and final
integration/required suites. [Review preambles](GATE4-REVIEW-PREAMBLES.md) cover each scope.
Native Save must wait for both reviewed runtime custody and manual lifecycle; Agent 4 maintains
this guide as the branches integrate. This documentation checkpoint enables no new UI control.
GitHub passes all 25 native Studio tests and both new inspection mutations with restored
passes for the inspection correction at `d38df93`; these runs predate the core signing split.
Exact checkout and execution scope are in [HANDOVER](HANDOVER.md).

Earlier accepted scheduling/preview evidence: combined scheduling at `6b71d96` passed user
review without required changes. That earlier block 1 is accepted. The new four-agent split above
subdivides what remains and does not reopen that scheduling acceptance.
Actor scheduling and native preview implementation passed review
at `a89bde6`. The user's re-review of `a89bde6...134394e` closes NATIVE-TEST-001 with no further
changes required, for both Index and Flipnote. The reviewer inspected source and the GitHub job
logs: 19 native tests passed, both mutations failed at their intended assertions, and restored
regressions passed. Execution used the PR merge checkout for `c60de4e`; `134394e` adds only docs.
The reviewer did not independently rerun Cargo. TAIL-TEST-001 also remains closed.

## Handoff to the UI agent

The existing mockup now has a native integration implementation (`0b6e870`).
The typed adapter is [studio-native.ts](../apps/desktop/src/studio-native.ts);
request/session/event coordination is [studio-session.ts](../apps/desktop/src/studio-session.ts).
Continue from those files and the connected surface. The [original UI builder prompt](FLIPNOTE-UI-BUILDER-PROMPT.md)
records the handoff scope; do not introduce a second adapter or restart the mockup.

Use the command/result and event tables below as the backend contract and
`design-creative-suite.md` for visual behavior. Preserve the separation between native views
and the editor model in `studio-store.ts`. Reuse the existing PIX codec and canonical operation
encoding. Backend review acceptance does not independently accept the UI implementation or
establish live two-client UI acceptance. Gate 4 backend work continues separately; coordinate
Rust/native and shared-document edits.

Connect list/open/create, frame and header edits, Index edits, pixel publication/bounded fetching,
refresh events, recovery list/read/preview/apply/backup export and eviction acknowledgement.
Preserve unsaved editor work and same-request retry identity. Keep channel/epoch identifiers
lossless, fence obsolete async results, and preserve conflicts/overflow/deletions in the adapter.

`awaitingTenureReceipt: true` identifies a read-only history preview. `provisional: true` alone
also appears on ordinary editable local views; it is not the read-only discriminator. Do not
invent a phase, settlement receipt, author confirmation or publication claim for a preview.

Leave backend-dependent actions unavailable for durable Closing overlays, signed repair,
claims/Ask/Pass (Gate 5), and sound/linked Music/`.pixa` export (Gate 6). Recovery backup export
is already available and uses a different format. These gaps and full Gate 4 acceptance do not
block the core UI hookup, but full-suite behavior is not ready for release acceptance yet.

The actor now schedules the reviewed provisional head, seed and signed-tail adapters through
its detached job queue. A ready preview owns its shared seed reservation, releases its parser
permit, and remains separate from installed Studio history. The two former-owner newcomer
preview cases are enabled and pass alongside the two known-CID PIX fetch/reopen cases.
Local native checking requires uncached dependencies. The focused
[Studio native workflow](https://github.com/Thalpy/Mewtual/actions/runs/34767022661) passes all 17
native Studio tests on `febbd70`; two-client acceptance also passes. Broader strict CI remains
blocked by existing unused security-intent APIs, so these results do not mean the whole PR is green.
See [HANDOVER](HANDOVER.md) for exact commands and current validation status.

Gate 4 remains open. The combined actor regression now exercises competing authoritative
Studio/Registry installations under retained-preview, cancelled-parser and cancelled-transport
pressure, with owner reachability returning without MLS churn. The final checkpoint `6b71d96`
passes its 11 focused actor/preview/newcomer tests, shared native fixture, Clippy and all 19 native
Studio tests plus native mutation checks on GitHub; the broad run on `487cb0e` passed 165 tests.
Both scheduler changes have isolated mutation failures and restored passes. User adversarial
review accepts this bounded checkpoint with no required corrections. One-second provisional-head
pacing and preservation of queued retries across repeated Reads change no native commands/result shapes. Durable Closing overlays/repeated
tenure, signed repair and full acceptance remain.
The [Closing overlay design](GATE4-CLOSING-OVERLAY-REVIEW.md) passed user review at `d576af2`
on 2026-09-14. Its internal core/store implementation at `b1b0ec9` also passed review, with
source-version finding OVERLAY-TEST-001 closed by the re-review of `65db6ac`. The accepted
[handoff design](GATE4-OVERLAY-HANDOFF-REVIEW.md) now covers transfer into shared pending intents;
HANDOFF-001 adds a retained complete target so completed retries remain bound to their channel
after base removal. The user closes HANDOFF-001 at `dd2fbc0`. The internal handoff now has a
three-step store transaction and distinct local-draft/completed results. The
[pushed implementation](GATE4-OVERLAY-HANDOFF-IMPLEMENTATION-REVIEW.md) at `bf37cc4` passes 202 local
Studio tests and the dedicated handoff/mutation workflows. The user accepts the corrected
bounded implementation at `62f06d4` / evidence head `aa0a81f` and closes HANDOFF-002: reference
inventory enforces the linked source's required metadata before enabling deletion. The reviewer
inspected 24 passing handoff tests, ten detected mutations and ten restored passes in GitHub
run 34903404377. This changes no UI command or result shape.
Those accepted write adapters supply no native Save command. Continue to disable durable overlay
Save in Closing/Fault and awaiting-tenure previews; preserve unsaved editor work without claiming
it is vault-saved. The separate read-only inspection command is documented below.
The accepted foundations and earlier review closures are recorded in
[the provisional review note](GATE4-PROVISIONAL-READ-REVIEW.md) and HANDOVER.

This guide describes native contracts. Frontend implementation and visual design remain
independently owned; backend work changes command security registration, not the visual surface.

## Available now

`studio_overlay_read({ server, channel, object? })` reads a retained local Closing draft.
Use canonical decimal `channel` and optional 32-character lowercase hexadecimal `object`;
omit `object` for Index. It returns a separate result, never an ordinary `StudioRead`:

```ts
type OverlayInspection =
  | { v: 1; kind: "absent"; channel: string; object: string | null }
  | { v: 1; kind: "local-draft"; channel: string; object: string | null;
      basis: string; accepted: number; transferState: "active" | "prepared";
      readOnly: true; content: StudioContent };
```

`StudioContent` is the existing full Index/Flipnote content representation below, including
conflicts and deletions. `basis` is a 64-character local identity, not an append capability.
Prepared is a retained draft awaiting transfer resolution; reading does not resolve it.
Absent does not imply no pending ordinary edits, no completed transfer or settled content.
There are no `epochId`, `epoch`, `phase`, publication, receipt or provisional-preview flags.
Keep this separate from the editor's unsaved work and from awaiting-tenure history previews.
Durable overlay Save, transfer, copy/export and disposition still have no native command.

Run ordinary and overlay reads sequentially for the same target: they share the latest-view
request fence, so starting another read supersedes the older result.
The original native request and session span capture and detached reconstruction. Changed
records, membership/context, actor, mount or a newer request reject delivery; refresh from
fresh context. Capacity exhaustion returns a retryable error. The shared four preparation
slots remain owned until actual worker/result destruction, including cancellation. Complete
native JSON is capped at 32 MiB; overflow returns an error with no partial result. Neither
this encoded limit nor the bounded input is a measured heap or latency guarantee. The
frontend adapter/layout remains separately owned; this adds the backend contract only.

The native entry points are [studio.rs](../apps/desktop/src-tauri/src/studio.rs),
[studio/inspection.rs](../apps/desktop/src-tauri/src/studio/inspection.rs) (local-draft reads),
[studio/recovery.rs](../apps/desktop/src-tauri/src/studio/recovery.rs) (all seven
`studio_recovery_*` commands) and
[creative_blobs.rs](../apps/desktop/src-tauri/src/creative_blobs.rs); registration and event
forwarding are in [lib.rs](../apps/desktop/src-tauri/src/lib.rs). These are real actor/vault
paths. The frontend adapter/session modules above implement the existing document and recovery
flows. Hooking up the new local-draft read remains with the UI agent.

`studio_list` and `studio_read` can now return a distinct unconfirmed-history result:

```json
{
  "v": 1,
  "epochId": "<32 lowercase hex>",
  "epoch": "<decimal>",
  "channel": "<decimal>",
  "provisional": true,
  "awaitingTenureReceipt": true,
  "content": { "kind": "flipnote" }
}
```

`content` uses the existing complete Index/Flipnote projection schema, including conflicts,
claimed authors, frame CIDs and declared byte lengths. The example abbreviates that content.
This result has no current `phase` or `publication` claim. Display it as read-only and awaiting
a tenure receipt. Its `epochId` cannot authorize ordinary Apply; durable overlays remain pending.
An installed source takes precedence. An absent Index's synthetic empty epoch zero does not hide
a ready preview. Ordinary view fields remain unchanged; there is no new `confirmed` flag.

A preview is eligible only after a finite authenticated tail finishes. That does not establish
owner tenure, historical attribution or completeness beyond the returned provider prefix. Reads
never create canonical sources, Registry pointers or durable intent/recovery/owner journals.
PIX fetching still uses the existing bounded CID API and does not imply local possession.

The original discovery lifetime remains 60 seconds. The receiver retries authoritative discovery
on its existing paced schedule, with a ready preview yielding for refresh after 30 seconds.
`studio-updated` also invalidates a preview when it becomes ready, expires or is evicted. Re-read
and accept absence after eviction. Do not retain an undisclosed projection cache to hide it.
Lock signals the actor to clear volatile previews; mount, server, watch and membership changes
also invalidate use. Keep renderer request/navigation/session generations and discard stale
results even after IPC has delivered them.

Native additionally suppresses a superseded request for the same target after JSON conversion.
For a preview it holds a cancellable, at-most-five-second actor handoff across conversion and
final native fences, after releasing the vault lease. This keeps the checked actor state stable
without a lease across network or cold parsing. Timeout suppresses delivery; the seed reservation
stays charged until every actual native/worker/transport owner releases its data.

Tauri invoke argument names are camelCase. `server` is the existing local numeric server id,
not the MLS group id. `channel` is a canonical decimal string; do not convert a channel u128
through a JavaScript Number. Object, frame and nonce ids are 32 lowercase hex characters;
`epochId` is also 32 lowercase hex, supplied by the last read. CIDs and full author identities
are 64 lowercase hex. A four-byte display fingerprint is never an authority key.

| UI action | Native command and invoke arguments | Result |
|---|---|---|
| Load sidebar | `studio_list({server, channel})` | Installed Index view, awaiting-tenure preview, or an empty epoch-zero Index view if neither exists |
| Open flipnote | `studio_read({server, channel, object})` | Installed Flipnote view, awaiting-tenure preview, or `null` if neither exists |
| New flipnote | `studio_create({server, channel, object, nonce, title, createdAtMs})` | Flipnote view after durable local create/index work |
| Edit art/metadata | `studio_apply({server, channel, object, epochId, nonce, body})` | Updated flipnote view |
| Edit sidebar entry | `studio_apply_index({server, channel, epochId, nonce, body})` | Updated Index view |
| Publish pixels | `publish_pix({server, bytesB64})` | `{cid: string, bytes: number}` after validated PIX persistence/promotion |
| Fetch referenced blob | `request_blob_bounded({server, cid, maxBytes})` | `{bytes_b64: string, bytes: number}` or `null` if unavailable |
| List recovery versions | `studio_recovery_list({server, channel, object?})` | Metadata for at most two retained versions plus one staged version; omit `object` for the Index |
| Inspect one recovery version | `studio_recovery_read({server, channel, object?, snapshot})` | Historical typed content, not a current Studio view |
| Export a recovery backup | `studio_recovery_export({server, channel, object?, snapshot})` | Bounded `{format:"p1-recovery-v1", bytes, bytesB64, snapshot}` |
| Accept an eviction warning | `studio_recovery_acknowledge({server, channel, object?, oldestSnapshot, stagedSnapshot})` | Updated recovery listing after exact-pair durable acknowledgement |
| Preview a recovery choice | `studio_recovery_preview({server, channel, object?, snapshot, choice, mode})` | One bounded proposed domain edit or an explicit conflict/hold; saves nothing |
| Apply that exact choice | `studio_recovery_apply({server, channel, object?, edit})` | Ordinary provisional content Save; see the retry contract below |
| Restore discoverability | `studio_recovery_restore_pointer({server, channel, object?})` | Separately retryable Registry pointer step for an actually saved document; no caller-supplied epoch |

`body` is a **canonical JSON string**, not an object or a full signed P1 envelope. Use
`canonicalJson` from [studio-contract.ts](../apps/desktop/src/studio-contract.ts).
[Shared wire vectors](../crates/catcoms-replication/tests/fixtures/studio-ops-v1.json) and
[studio-wire.test.ts](../apps/desktop/src/studio-wire.test.ts) pin its bytes. Backend derives
the author and operation id; the UI must not supply a purported sender's signature/identity.
`createdAtMs` must be a non-negative, JS-safe integer timestamp.

Implemented bodies:

- Index: `put_object` for flipnotes, `set_title`, `set_expiry`, `tombstone_object`.
- Flipnote: `insert_frame`, `replace_frame`, `remove_frame`, and `set_header` for `title`/`fps`.
- Audio, linked-score and export bodies in the wider contract are **not yet accepted** by these
  materializers. Recognized codec fields are not proof of backend support.

## Save and retry

1. Encode flattened PIX1 bytes. Editor layers remain local; no layer document is stored.
2. Call `publish_pix` and keep its actual `{cid, bytes}`. A placeholder/frontend hash is not a CID.
3. Call `studio_apply` with that exact declaration, the last view's `epochId`, a fresh random
   nonce and the canonical insert/replace body. Publish alone does not save a frame record.
4. Replace the displayed materialization with the returned view. Keep unsaved editor work
   separate; do not discard it on timeout, cancellation, capacity refusal or epoch mismatch.

Example for an existing frame (names such as `savedView` are caller-owned state):

```ts
const blob = await invoke<{cid: string; bytes: number}>("publish_pix", {
  server, bytesB64: encodedPixBase64,
});
const body = canonicalJson({op: "replace_frame", frame: frameId, ...blob});
const updated = await invoke("studio_apply", {
  server, channel, object, epochId: savedView.epochId, nonce: editNonce, body,
});
```

A retry of an uncertain operation keeps the **same complete request**, including nonce, body
and epoch id. Do not mint a new nonce automatically after an error: the local save may already
have committed. If the epoch changed, re-read and preserve the unsaved work; the backend's
conservative replay and recovery controls are described below. Do not silently reauthor into a new
epoch. A failed Create may have persisted part of the two-document work; retry the same Create
identity/nonce rather than creating another object. For `studio_read`, `null` is local absence,
not a deletion or proof no other member has the document. `studio_list` instead synthesizes an
empty epoch-zero Index view without creating a file. Ordinary reads establish the bounded
background watch.

Success means durable **local, provisional** metadata, not remote delivery or owner settlement.
Native busy/locked errors are ordinary refusal paths; do not spin or force-unlock storage.

## Read model: adapt it, do not cast it to the fixture root

Every non-null Studio response has the common fields below and one of two trust states.
Check `awaitingTenureReceipt` before using `epochId` to prepare an ordinary edit:

```ts
type StudioReadResult = {
  v: 1;
  epochId: string; // 32 hex; only an ordinary editable view can supply a write epoch
  epoch: string;   // u64 decimal, preserve losslessly
  channel: string;
  provisional: true;
  content: {kind: "index" | "flipnote"; /* complete typed projection */};
} & (
  | {awaitingTenureReceipt: true; phase?: never; publication?: never}
  | {awaitingTenureReceipt?: never; publication: "local";
     phase: "open" | "closing" | "settled" | "fault"}
);
```

Index content has `objects`, `overflow`, `deletedObjects`, and `tombstones`, keyed by object id.
Each entry preserves all `creations` and selected/conflicting title/expiry values. Flipnote
content has `title`, `fps`, `timeline`, `frames`, `declaredFrameBytes`, `overCap`, and `tombstones`.
Use `timeline` order, not map enumeration. Registers are `{selected: {value, source}, conflicts:
[{value, source}]}`. Sources carry `opId`, full `author`, `nonce`, and (for frame sources) `ts`.
Frame `pixels` registers select `{cid, bytes}`; `insertions` retain ordering/provenance evidence.
Do not silently throw away conflicts, deleted content or overflow when adapting the view.

Expiry reads as `{kind:"unrecorded"}`, `{kind:"never"}`, or `{kind:"at", ms:number}`.
They are different states; zero milliseconds is not “never”. For operation bodies use the wire
codec's omitted / `null` / timestamp forms, not the view's discriminated object.

The frame cap is 999 and selected declared frame sizes sum to at most 8 MiB for the playable
prefix. Concurrent overflow stays visible via `overCap`; skip flagged frames during playback
and do not label them a successful accepted local addition. Backend is the authority on caps.
See [INTERFACES](INTERFACES.md#native-studio-saveload-gate-2-indexart-only) for the full read contract.

## Fetching and event hooks

Pass the frame record's **declared bytes** as `maxBytes`. Require exact returned length, then
validate/decode PIX1 before rendering. `null` is unavailable/fetching; oversize, invalid CID and
authentication failures reject. This command does not retry providers. If the selected CID,
object, server, view/session or frame changes while fetching, discard the obsolete result.
Do not start an unbounded fetch for every conflict/frame on every repaint; prioritize visible
frames and keep the UI's pending fetches bounded. PIX input is capped at 64 KiB; the generic
bounded fetch ceiling is 9 MiB, not permission to fetch a 9-MiB frame.

| Event | Payload | UI response |
|---|---|---|
| `studio-updated` | `{server: number, channel: string, object: string | null}` | Invalidate/re-read the matching Index and affected open object. `null` names the Index. Coalesce refreshes; an event is not a new projection. |
| `studio-receive-paused` | `{server: number}` | Show a receive/storage warning without claiming settlement failure or deleting local work. Successful explicit Read/Save resumes the paused receiver; Read alone may not warm unrelated large histories. |
| `settlement-changed` | `{server: number, docType: 15 | 16, logicalKey: string, channel: string, object: string | null, state: string}` | Invalidate matching recovery/phase observations and re-read `studio_recovery_list`. This is not a receipt or a new projection. |

Settlement states currently emitted are `open`, `closing`, `settled`, `fault`,
`recoveryAvailable`, `recoveryEvictionPending`, and `refreshRequired`. Phase and recovery
observations are independent: a recovery notification does not replace the document phase.
`logicalKey` is 32 lowercase hex characters; `channel` is a decimal u128 string. List/read
commands do not emit invalidations, so listening and re-reading must not create a refresh loop.

Install listeners with normal component/session cleanup. Fence late reads by current server,
channel, object and local request generation. Native already rejects stale actor/session
results, but frontend navigation can still supersede a legitimate returned view.
The backend handles recent-target watches, missed-edit catch-up, checkpoint discovery and
installation; the UI should not implement a second catch-up scheduler or derive receipt proofs.

## Waiting on Gate 4 and later

| Canonical UI surface | Backend availability / next hook |
|---|---|
| Settlement chip / rotation progress | `settlement-changed` invalidates the actual phase/recovery listing. `open` does **not** mean the current edits are receipted. Current responses always say `provisional:true`. Do not synthesize receipt author/time or “settled” from epoch alone. |
| Current owner has not confirmed history | Native Read/List return the distinct `awaitingTenureReceipt: true` preview documented above. Actor/native implementation and NATIVE-TEST-001 passed review; combined scheduling at `6b71d96` is now accepted. The frontend adapter handles this read-only result; live UI acceptance remains separate. |
| History fault / repair progress | Actual `phase:"fault"` and `fault` invalidations are available; signed repair has no actor/native command or `repairing` event yet. Restore/Copy saves ordinary content in an Open target and cannot clear Fault. Historical Read/Export remain available. |
| Local overlay while rotating | Preserve unsaved editor work. The [core/store foundation](GATE4-CLOSING-OVERLAY-REVIEW.md) passed review and OVERLAY-TEST-001 is closed. The [handoff design](GATE4-OVERLAY-HANDOFF-REVIEW.md) is accepted at `dd2fbc0`; HANDOFF-001 is closed. The [core/store implementation](GATE4-OVERLAY-HANDOFF-IMPLEMENTATION-REVIEW.md) is pushed at `bf37cc4`, with normal/mutation checks passing; the correction at `62f06d4` is accepted and HANDOFF-002 is closed. No native durable overlay or replay command exists. Shared Apply refuses Closing/Fault. |
| Recovery rail: Restore / Copy / Export | List/inspect/backup export, per-item Restore/Copy and conservative own-intent replay are connected. Unsafe replay is manual recovery, never settlement. Final Gate 4 acceptance remains pending; backup export is not `.pixa`. |
| Eviction warning / countdown | Use the listing's actual warning pair/deadline and `studio_recovery_acknowledge`. Refresh after the action and matching `settlement-changed` events. |
| Claims, Ask, Pass, countdown | Pending Gate 5. Local fixture claims are not peer claims and never locks. |
| Sound, linked Music, `.pixa` export | Pending Gate 6. Do not infer availability from the contract's type definitions. |

Implementation progress and reuse evidence live in [BACKEND-IMPLEMENTATION](BACKEND-IMPLEMENTATION.md).
UI hooks will be marked callable here only once their actor/native path and tests exist.

## Recovery read/export/ack contract (Gate 4 worktree)

The backend automatically considers this device's saved intents after checkpoint changes. It
keeps the original author/nonce, requires complete retained recovery evidence and rechecks the
current projection before each paced edit. It does not overwrite a newer header or frame value,
resurrect a deleted id, infer causal order from hashes, or replay another member's edits.
Conflicting or unsafe choices can leave the pending queue only after their full envelopes are
verified in a durably flushed recovery snapshot. Show **needs recovery**, never **settled**.
The existing two retained snapshots, staged warning, Restore/Copy/Export and eviction limits
apply. An intent with no such evidence remains pending. Cold large referenced documents may
require explicit recovery. Current-log accepted edits remain pending until covered by a receipt.

Snapshot ids are 64 lowercase hex characters. Listing returns `kind:"recoveryList"`, `v:1`,
`channel`, `object`, `source`, `versions`, `evictionPending` and `pendingIntents`.
`source` is null for local absence, otherwise `{epochId, epoch, phase, provisional:true}`.
Each version is `{snapshot, epoch, staged, bytes, reason}`; retained entries are newest-first,
then the optional staged entry. Reasons are `excluded`, `rewound`, `conflictOverflow`, `repair`.
Epochs are decimal strings. A pending intent can be an ordinary provisional Save; the count is
**not** a count of lost or excluded edits. An unreadable live source currently makes List fail;
Read/Export of a known retained snapshot do not depend on the live source being readable.

`evictionPending` is null or `{oldestSnapshot, stagedSnapshot, deadlineMs}`. The deadline is a
lossless decimal u64 string from the persisted receiver clock, not a locally invented countdown.
Show the warning before acknowledging. The exact pair is retryable; a changed/stale pair refuses.
The countdown is enforced, not decorative: acknowledgement only brings the eviction forward. Once
`deadlineMs` passes, the next ordinary owner settlement pass promotes the staged version and drops
the named oldest one with no acknowledgement at all, so offer Export while it is still running.
Until then no settlement pass rewrites the record, so the ids and deadline survive restarts
unchanged rather than restarting the grace.
Acknowledgement returns `kind:"recoveryAcknowledged"` with the same listing fields. It changes
local recovery slots only: it neither retires intents nor installs/prunes a document. Ordinary
background settlement must still finish its own barriers afterward.

Read returns `kind:"recoveryVersion"`, `historical:true`, `version` metadata and `content` in the
same conflict-preserving projection shape as current views. Never replace the current view's
epoch/phase with this historical projection. Inspect and export remain usable in Closing/Fault.
Every retained/staged slot is authenticated and type/channel checked before an action; a corrupt
unselected slot fails rather than being quietly discarded.

Export returns `kind:"recoveryExport"`, `v:1`, `snapshot`, `format:"p1-recovery-v1"`, `bytes` and
`bytesB64`. This is the existing canonical P1 recovery envelope: private historical content and
operation evidence, **not** playable media, a published share, an archive of PIX blobs, or an
implemented import command. All controls reuse Save's native operation cap, actor lease and
session/incarnation fences. Do not retry on navigation/lock as though the original session survived.

## Restore / Copy choices (current worktree)

Whole-version recovery is a sequence of explicit domain edits, not an atomic replacement of the
document. Use the historical Read projection to enumerate choices, preview each, apply ready
choices, then re-read/re-preview before the next step. A partial sequence is saved content and
must not be labelled an all-or-nothing Restore. Caps and missing local PIX bytes can refuse Save.
Fetch missing frames through `request_blob_bounded` before applying; never invent a CID or size.

`mode` is exactly `restore` (add only) or `copy` (user-confirmed mutable replacement/deletion).
`choice` has an exact key set:

| Choice | Fields besides `kind` | Meaning |
|---|---|---|
| `frame` | `id`, `value` | Stable frame id and original pixel value's `source.opId` |
| `frameDeletion` | `id` | Explicitly apply a deletion recorded on that fork; requires Copy confirmation |
| `title`, `fps` | `value` | Original register value's `source.opId`; headers never automatically Restore |
| `object` | `id` | Add a missing Index entry, only if its same-channel object exists locally |
| `objectTitle`, `objectExpiry` | `id`, `value` | Copy a specific historical mutable Index value |
| `objectDeletion` | `id` | Explicit fork deletion; requires Copy confirmation |

Element ids are 32 lowercase hex; `value` and snapshot ids are 64 lowercase hex. For a whole-version
Restore use selected pixel values, not the initial insertion blob. A missing frame's predecessor
resolves to its recorded live predecessor, otherwise the current end. Deleted ids are not silently
resurrected, even in Copy mode. Immutable Index kind/creator/time are never overwritten. A restored
Index insertion is authored by the restoring member; `originalAuthor` is historical attribution,
not an authorization claim or the new operation's signer.

Preview returns `kind:"recoveryPreview"`, `snapshot`, `epochId`, `expectedProjection`,
`disposition`, `body` and `originalAuthor`. Dispositions: `ready`, `unchanged`, `conflict`,
`deleted`, `full`, `missingTarget`. Only Ready carries a canonical JSON body string. A preview is
not a storage reservation. `expectedProjection` covers all current conflict/deletion/provenance
data, not just visible pixels; Apply also rechecks every retained/staged recovery slot.

Apply's `edit` is `{snapshot, choice, mode, epochId, expectedProjection, nonce, body}`. Echo the
preview exactly and choose a fresh 32-hex nonce. Retry that *same whole payload* after uncertain
failure. Never rewrite its predecessor/body under the same nonce. A stale preview requires a
new preview and new nonce. An exact own operation already saved in the current Open epoch can
still retry after later edits or snapshot eviction; a pending intent alone cannot bypass checks.
Success is `{v:1, kind:"recoveryApplied", contentSaved:true, alreadySaved, provisional:true,
pointerRestored:false}`. Re-read current state. No remote delivery or owner settlement is implied.

After content recovery, restore the affected document's pointer separately. For an Index `object`
choice, call pointer restoration with that object's id (not just the Index). Omit `object` to
restore the Index's own pointer when needed. Success is `kind:"recoveryPointerRestored"` with
`checkpointEpoch` (decimal string), `registryEpochId` (32-hex), channel/object and `provisional:true`.
Historical Registry deletion requires this explicit action; ordinary idle refresh still refuses
it. A current tombstone waits for Registry rotation; Closing/Fault, a full bucket, missing source
or a pointer to a newer checkpoint refuses. Content already saved stays saved: show the pointer
step as pending/blocked and retry it separately, not "Restore completed".
