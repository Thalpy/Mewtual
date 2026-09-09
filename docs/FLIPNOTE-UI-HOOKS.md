# Flipnote UI hook guide

Last checked: 2026-09-10, Gate 4 integration worktree (not yet committed/accepted).
This is the maintained frontend integration map, not a replacement UI design. The user's
canonical HTML/mockups remain authoritative for layout and interaction. Update this guide in
the same slice that adds or changes a native command, event or returned state.

## Available now

The native entry points are [studio.rs](../apps/desktop/src-tauri/src/studio.rs) and
[creative_blobs.rs](../apps/desktop/src-tauri/src/creative_blobs.rs); registration and event
forwarding are in [lib.rs](../apps/desktop/src-tauri/src/lib.rs). These are real actor/vault
paths, not fixture functions. The current frontend `studio-store.ts` is still an in-memory
editor model; its presence does not mean it calls these commands.

Tauri invoke argument names are camelCase. `server` is the existing local numeric server id,
not the MLS group id. `channel` is a canonical decimal string; do not convert a channel u128
through a JavaScript Number. Object, frame and nonce ids are 32 lowercase hex characters;
`epochId` is also 32 lowercase hex, supplied by the last read. CIDs and full author identities
are 64 lowercase hex. A four-byte display fingerprint is never an authority key.

| UI action | Native command and invoke arguments | Result |
|---|---|---|
| Load sidebar | `studio_list({server, channel})` | Index view below; a missing local file yields an empty epoch-zero Index view |
| Open flipnote | `studio_read({server, channel, object})` | Flipnote view, or `null` for locally absent epoch zero |
| New flipnote | `studio_create({server, channel, object, nonce, title, createdAtMs})` | Flipnote view after durable local create/index work |
| Edit art/metadata | `studio_apply({server, channel, object, epochId, nonce, body})` | Updated flipnote view |
| Edit sidebar entry | `studio_apply_index({server, channel, epochId, nonce, body})` | Updated Index view |
| Publish pixels | `publish_pix({server, bytesB64})` | `{cid: string, bytes: number}` after validated PIX persistence/promotion |
| Fetch referenced blob | `request_blob_bounded({server, cid, maxBytes})` | `{bytes_b64: string, bytes: number}` or `null` if unavailable |
| List recovery versions | `studio_recovery_list({server, channel, object?})` | Metadata for at most two retained versions plus one staged version; omit `object` for the Index |
| Inspect one recovery version | `studio_recovery_read({server, channel, object?, snapshot})` | Historical typed content, not a current Studio view |
| Export a recovery backup | `studio_recovery_export({server, channel, object?, snapshot})` | Bounded `{format:"p1-recovery-v1", bytes, bytesB64, snapshot}` |
| Accept an eviction warning | `studio_recovery_acknowledge({server, channel, object?, oldestSnapshot, stagedSnapshot})` | Updated recovery listing after exact-pair durable acknowledgement |

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
have committed. If the epoch changed, re-read and preserve the unsaved work; automatic intent
replay and user-facing recovery controls are still Gate 4. Do not silently reauthor into a new
epoch. A failed Create may have persisted part of the two-document work; retry the same Create
identity/nonce rather than creating another object. For `studio_read`, `null` is local absence,
not a deletion or proof no other member has the document. `studio_list` instead synthesizes an
empty epoch-zero Index view without creating a file. Ordinary reads establish the bounded
background watch.

Success means durable **local, provisional** metadata, not remote delivery or owner settlement.
Native busy/locked errors are ordinary refusal paths; do not spin or force-unlock storage.

## Read model: adapt it, do not cast it to the fixture root

Every non-null Studio response currently has:

```ts
{
  v: 1,
  epochId: string, // 32 hex, reuse on writes
  epoch: string,   // u64 decimal, preserve losslessly
  channel: string,
  publication: "local",
  provisional: true,
  phase: "open" | "closing" | "settled" | "fault",
  content: {kind: "index" | "flipnote", /* typed projection */}
}
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

Install listeners with normal component/session cleanup. Fence late reads by current server,
channel, object and local request generation. Native already rejects stale actor/session
results, but frontend navigation can still supersede a legitimate returned view.
The backend handles recent-target watches, missed-edit catch-up, checkpoint discovery and
installation; the UI should not implement a second catch-up scheduler or derive receipt proofs.

## Waiting on Gate 4 and later

| Canonical UI surface | Backend availability / next hook |
|---|---|
| Settlement chip / rotation progress | `phase` is available; a dedicated settlement-status event/view is not. `open` does **not** mean the current edits are receipted. Current responses always say `provisional:true`. Do not synthesize receipt author/time or “settled” from epoch alone. |
| Local overlay while rotating | Keep editor work separately. Persisted overlay/replay orchestration is not yet a native command. Shared apply may refuse Closing/Fault. |
| Recovery rail: Restore / Copy / Export | List, inspect and recovery-backup export are callable above. Restore/Copy application and automatic intent replay are still pending Gate 4; backup export is not `.pixa`. |
| Eviction warning / countdown | Use the listing's actual warning pair/deadline and `studio_recovery_acknowledge`. Dedicated settlement-change events are still pending; refresh after the action. |
| Claims, Ask, Pass, countdown | Pending Gate 5. Local fixture claims are not peer claims and never locks. |
| Sound, linked Music, `.pixa` export | Pending Gate 6. Do not infer availability from the contract's type definitions. |

Implementation progress and reuse evidence live in [BACKEND-IMPLEMENTATION](BACKEND-IMPLEMENTATION.md).
UI hooks will be marked callable here only once their actor/native path and tests exist.

## Recovery read/export/ack contract (Gate 4 worktree)

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
