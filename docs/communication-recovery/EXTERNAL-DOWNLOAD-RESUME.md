# External download resume

Design checkpoint, 21 October 2026. Status: **revision 3 after read-only design review and
re-review (no blocker or high remains); implementation in progress.** Dispositions are at the end.

Baseline: `dbb8d36b919372737178fd56c1e301dde8fd423c` (pushed tip of `chat-update-implementation`,
"Record complete validation of the protected handoff checkpoint"). Branch
`communication-external-download`, worktree `target/communication-external-download-worktree`.

## Goal and scope

Close the remaining Package 4 boundary in
[CHAT-FILES-VOICE-PLAN](CHAT-FILES-VOICE-PLAN.md): a save of a shared file to the user's
Downloads folder continues the partly written file after pause, crash or restart, instead of
abandoning it and starting a fresh numbered copy. The plan's rule applies throughout: resuming
"revalidates its destination, current local write authority and no-overwrite policy; a saved
path is not permission to replace a newly created file."

"User-selected file" means the file the user explicitly chose to save. A **fresh** save goes to the
operating system's Downloads folder, resolved by native on that invocation, as today. A resumed
save continues in the folder recorded when it started, which must still be the same directory
object. There is no save dialog; one needs a new Tauri plugin dependency and is out of scope.

Out of scope: voice, live V2, media relay, six-client topology, acquisition/network changes,
automatic (unrequested) saves, and any dependency other than the `winapi-util` edge below.

## Current behavior (verified at the baseline)

- `save_group_file` first acquires the whole encrypted file into a private Download record
  (`crates/catcoms-storage/src/kept/downloads.rs`, sealed `mewtual/download-transfer/v1`) and
  verifies the whole plaintext CID. Restart keeps verified encrypted chunks and spent allowance.
- It then exports: `stream_download_to_disk` (`apps/desktop/src-tauri/src/lib.rs`) reads each
  chunk through `ReadDownloadChunk`, writes plaintext into a freshly created exclusive
  `<name>.part`, fsyncs, rechecks the whole CID, hard-links the final name (no replace) and
  unlinks the staging file. `finish_download_export` then releases the private record.
- Any export failure, pause, lock or cancellation deletes the staging `.part`. A crash leaves an
  orphan `.part` that nothing tracks. Resume always creates a new staging/final name pair.
- No external path, identity or offset is persisted anywhere.

## Design overview

The two phases stay separate. Encrypted acquisition is unchanged and remains the only network
phase. Plaintext is written to Downloads only after the encrypted copy is complete and verified.

The export phase gains a durable **export journal** inside the same sealed Download record.
Responsibilities are split so that no Downloads I/O ever runs on the server actor:

- **Storage (on the actor), vault-only:** typed journal transitions with compare-and-swap fences
  (lease generation, intent, exact journal). These write only the sealed record in the vault.
- **Storage crate, plain functions:** an owned `ExportFile` value (file handle, cursor, plaintext
  hasher, expected sizes and CID bound from the `DownloadReady` manifest) and the platform module
  that creates, reopens, verifies, writes, publishes and removes external files. These hold no store
  borrow and are called by native inside `spawn_blocking`.
- **Native:** resolves the Downloads folder for fresh saves, drives one plaintext chunk at a time
  through the existing `ReadDownloadChunk` command, runs every external effect in a blocking
  worker, emits progress, and asks the actor for journal transitions.

A stalled Downloads volume (dropped SMB share, unplugged USB drive) therefore pins at most that
save's blocking worker. Chat, MLS, sync and Lock do not wait on per-chunk external work.

If a **fresh** export cannot be journaled (no stable identity or birth time on this filesystem, an
encoded folder path over the bound, or an unsupported platform), native keeps today's
non-resumable streaming export unchanged. Storage enforces that a record with a journal never
completes through that path (see Fences).

## State machine

Durable record dimensions (existing): `phase` Resumable/Completed/Deleting, `user_paused`,
`blocked`, `exhausted`, `intent`, `grant`. New dimension, Download records only:

```
export: None | Writing(Journal) | Published(Journal)
Journal { folder, folder_id, staging_leaf, final_leaf, file_id,
          written_chunks, written_bytes, halt: None | Unavailable | Changed }
```

Runtime (never persisted): `checked`, the active acquisition run, and an **export presence**: a
weak reference to a token owned by the native export task, registered with the journal edge that
starts or reopens a session. "Saving" is shown only while that token is alive and the lease
generation that registered it is current. A panicked or abandoned task drops the token.

Allowed journal transitions (all checked in `validate_successor`):

| From | To | Edge |
| --- | --- | --- |
| None | Writing(0) | Fresh export start. Phase Completed on both sides, current export lease, no halt. This is the only edge that upgrades DownloadV1 to DownloadV2. |
| Writing | Writing | Same folder, folder identity, staging leaf and file identity. Progress checkpoint, a new final leaf before a link attempt, or setting a halt. |
| Writing (halt) | Writing (no halt) | Only in the same publication as an explicit intent change (Resume). |
| Writing | Published | Every chunk written; native proved the final name holds the file identity. |
| Writing, Published | None | Explicit "Save new copy" or "Save another copy" (intent change, phase Completed), or retirement in phase Deleting after cleanup. |
| Published | Writing | Refused. |
| any | Deleting gains a journal | Refused. Retry (network grant) requires identical export state and encoding. Revision 1 records have no journal. |

Pause, Resume, Retry, exhaustion and blocked keep their existing meaning. Pausing a Published
record is refused.

Projected renderer status, highest precedence first (closed set; never inferred from paths):

| Status | Condition | Label | Actions |
| --- | --- | --- | --- |
| `removing` | phase Deleting | Removing | Remove, Keep partly saved file |
| `saved` | export Published | Saved | Save another copy, Clear |
| `destination-changed` | halt Changed | Partly saved file changed | Save new copy, Remove |
| `destination-unavailable` | halt Unavailable | Folder or file unavailable | Try again, Save new copy, Remove |
| `exhausted` | network allowance exhausted | Retry needed | Retry download, Remove |
| `save-paused` | user Pause with export Writing | Saving paused | Resume saving, Remove |
| `paused` | user Pause otherwise | Paused | Resume download, Remove |
| `blocked` | deterministic verification failure | Verification failed | Resume download, Remove |
| `receiving` | acquisition run active | Receiving | Pause, Remove |
| `saving` | export presence alive | Saving | Pause, Remove |
| `save-interrupted` | export Writing, not running | Saving interrupted | Resume saving, Remove |
| `ready` | Completed and checked, no journal | Ready to save | Save to Downloads, Remove |
| `incomplete` | otherwise | Interrupted | Resume download, Remove |

The projection adds `savedChunks` (journal checkpoint, 0 without a journal) next to the existing
`verifiedChunks`. Saving a file from the chat file list routes by status: `ready`, `incomplete`,
`paused`, `blocked`, `save-interrupted` and `save-paused` resume; every other status opens
Transfers instead of acting.

## Record format and versioning

New sealed domain `mewtual/download-transfer/v2` (`RecordEncoding::DownloadV2`). Its body is the
exact DownloadV1 body followed by:

```
u8   export kind      0 None, 1 Writing, 2 Published
if kind != 0:
  bytes folder        platform-encoded absolute path, 1..=16384 bytes
  bytes folder_id     25 bytes: platform u8, volume u64 (LE), object id [u8; 16]
  bytes staging_leaf  UTF-8, 1..=1024 bytes, one normal path component
  bytes final_leaf    UTF-8, 1..=1024 bytes, one normal path component
  bytes file_id       25 bytes, same layout; object id = file index or inode, then birth time
  u32  written_chunks <= plan chunk count
  u64  written_bytes  <= 1 GiB
  u8   halt           0 None, 1 Unavailable, 2 Changed
```

The folder path is UTF-16LE on Windows and raw bytes on Unix; the identity's platform byte selects
the decoding. A platform mismatch is reported as Unavailable, never reinterpreted.

The worst-case journal adds about 18.2 KiB. The V4/Download body stays below 360 KiB, so the
existing 400 KiB slot and its quota charge are unchanged. A regression encodes a maximum
four-variant record with a maximum journal under the slot limit.

Encoding predicates become exhaustive matches with a single `is_download()` helper:
`owner()`, `public_retained()`, `has_variants()`, variant minimums and initial-field rules. A
DownloadV2 record can never be mistaken for a Keep, be served publicly, or be rewritten as V3.

Compatibility:

- DownloadV1 records keep their exact canonical bytes and predecessor hashes and decode with
  `export = None`. A V1 successor stays V1 unless it is the None-to-Writing edge.
- V2-to-V1 is refused. Keep records can never carry a journal.
- Records without a journal remain readable by the released reader. While any V2 record exists,
  an older reader refuses its unknown domain during `configure_keep_scope`: the retained store
  faults without deleting anything, including the external partial. This matches the documented
  V4 and DownloadV1 behavior. Returning to a current reader restores the journal.
- An unknown future domain is refused by this reader in the same non-destructive way.
- Legacy `pending` stays cleanup-only and is untouched.

## Destination identity and verification

A platform module (`crates/catcoms-storage/src/kept/external.rs`) is the only code that touches
external paths. The workspace denies `unsafe_code`, so Windows identity comes from the safe
`winapi-util` wrapper of `GetFileInformationByHandle` (`Unlicense OR MIT`; already in the native
lockfile; reuses the root's `windows-sys` 0.61.2). It is added as a `cfg(windows)` dependency of
`catcoms-storage`; `cargo deny check` runs with the change.

- **File identity** (from the open handle): platform, volume (Windows volume serial; Unix `dev`),
  object id (Windows 64-bit file index, which carries an NTFS reuse sequence; Unix `ino`) and the
  **birth time** from `Metadata::created()` (Unix `statx`). A zero volume or id, or a missing birth
  time, makes identity Unsupported. Birth time defeats inode-reuse collisions; NTFS creation-time
  tunneling cannot forge it because the file index still differs.
- **Folder identity**: volume and id of the opened directory, following a Downloads folder that is
  itself a junction or symlink (the user's configured location).
- **Leaf open without following a link**: `symlink_metadata` must show a regular file first.
  Windows then opens with `FILE_FLAG_OPEN_REPARSE_POINT` and refuses a non-regular handle; Unix
  compares the opened handle's identity with `lstat`. FIFOs and devices are refused before open.
- **Staging suffix** `.mewtual-part`, distinct from browsers' `.part`, so another program's
  in-progress download is never at our name by convention. The fallback writer keeps `.part`.

Rules:

1. **Creation (fresh only).** In the freshly resolved Downloads folder (created if missing, as
   today), choose `name`, `name (1).ext`, ... (at most 1,000) whose final name is absent, then create
   `<final>.mewtual-part` with `create_new`. Capture identity, then **prove hard links work here** by
   linking the new file to an absent probe name, checking the probe has the same identity and
   removing only the probe name. FAT and exFAT report a usable-looking index and creation time but
   cannot link, so without this probe a journal there could never publish. Attempt a folder flush,
   then the actor persists Writing(0). If identity or linking is Unsupported, or the Writing(0)
   publication fails, the file just created is removed after its identity is rechecked, and a fresh
   save falls back to the non-resumable writer.
2. **Reopen and preflight.** Every journaled operation uses the **sealed folder path** and requires
   that folder to open with identity `folder_id`; nothing is created there. The staging leaf must
   open without following a link with identity `file_id` and link count 1. Native runs this check as
   a preflight before an explicit Resume spends any allowance on re-verifying the encrypted copy,
   and again to obtain the write handle.
3. **Prefix verification.** The session compares the partial's bytes with plaintext regenerated
   from the retained encrypted chunks, one whole chunk at a time, from chunk zero.
   - Inside the checkpointed range `[0, written_bytes)`, any byte mismatch halts as **Changed**
     and nothing is modified. A file shorter than the checkpoint rewinds to its last whole verified
     chunk (a torn or lost tail).
   - Beyond the checkpoint (our own unacknowledged tail, identity-proven), matching whole chunks
     are kept; the first short or mismatching chunk is truncated and rewritten from there.
   - Every chunk, verified or written, feeds the plaintext CID hasher; chunk lengths must match the
     manifest. No unkeyed plaintext digest is persisted.
4. **Writes and checkpoints.** Each written chunk is followed by `sync_all`. The journal
   checkpoint (`written_chunks/bytes`) is persisted only at fsynced boundaries, every 8 chunks, when
   a session stops for a reason other than disk full, and before publication. It is never ahead of
   fsynced bytes. Verification recovers anything written after the last checkpoint.
5. **Publication.** Requires every chunk processed, hasher equal to the plaintext CID and file
   length equal to the declared size (a longer file is trimmed first, then fsynced). If the final
   name already holds our identity (crash row 6), the save is already published. Otherwise, if the
   final name is occupied, the next absent numbered candidate is persisted as `final_leaf` before
   any link attempt. Under the native session commit guard, with the original invocation and lease
   still current, the staging identity is rechecked and `hard_link(staging, final)` runs (refuses an
   existing destination on both platforms; no overwriting fallback). The final name is reopened
   without following a link and must have our identity, and the actor persists Published. The guard
   is then released; only after that is a folder flush attempted and the staging name unlinked if
   it still has our identity. A link failure reporting that links are unsupported is a halt, not a
   transient failure.
6. **Removal.** Deletes only the staging leaf, after rule 2's folder and identity checks prove it is
   ours. When staging and final both have our identity (link count 2), only the staging name is
   removed. The final name is never deleted.

Halt classification: folder missing, volume gone, platform mismatch, permission denied, or the
staging leaf missing with no published final name of ours is **Unavailable**. A folder identity
mismatch, a staging leaf that is a link, directory or another file, an unexpected link count, or a
byte mismatch inside the checkpoint is **Changed**. Halts are persisted (journal CAS) and keep the
encrypted progress. Disk full and transient write failures are not halts; see Budgets.

## Crash table

| # | Crash point | Recovered state | Next action |
| --- | --- | --- | --- |
| 1 | After exclusive create, before Writing(0) is durable | export None; an empty `.mewtual-part` orphan may exist | A fresh export skips the occupied name. The orphan is never adopted or deleted. |
| 2 | After Writing(0), file lost (power loss) or deleted | Writing(0); staging missing | Preflight reports Unavailable; Save new copy or Remove. |
| 3 | After chunk writes, before the next checkpoint | Writing(k); file holds more, possibly torn, bytes | Verification keeps matching whole chunks after k, truncates the first torn one, continues. |
| 4 | Checkpoint claims more than the file holds (torn write, lying flush) | Writing(k); file shorter | Rewind to the last whole verified chunk and continue. A mismatch inside the checkpoint halts Changed. |
| 5 | After persisting a new `final_leaf`, before linking | Writing(all); new final name | Resume verifies, then publishes at that name. |
| 6 | After `hard_link`, before Published | Writing(all); final has our identity; staging link count 2 | Preflight/reopen sees the published final name, persists Published, reports Saved. No second link or copy. |
| 7 | After Published, before staging unlink or record release | Published; both names may exist | Saved. Clear unlinks the staging name if still ours (never the final name), then releases. |
| 8 | After Deleting (Remove/Clear), before external cleanup | Deleting with journal | Mount leaves it in place; Removing row. Remove retries cleanup then releases; Keep partly saved file releases without deleting. Never resumes fetching or export, never serves. |
| 9 | After external cleanup, before journal retirement | Deleting with journal; staging gone | Remove is idempotent (missing staging is done), then releases. |
| 10 | After the move to `deleting-v2` | Existing cleanup-only directory | Existing bounded `finish_deleting`. |
| 11 | During Save new copy, after removing our partial, before the journal clears | Writing; staging missing | Preflight reports Unavailable; Save new copy again. |

External cleanup that cannot prove ownership (folder or partial missing, identity changed,
platform mismatch) finishes without deleting anything it finds. A proven partial whose deletion
fails keeps the record in Deleting with its journal, conservatively charged; Remove retries it and
"Keep partly saved file" releases the record while leaving the file for the user. Cleanup failure
never refunds quota or creates a second reservation.

## Budgets

- Acquisition, including re-verification of a saved encrypted copy on Resume, keeps the existing
  durable execution allowance unchanged. Restart, providers and route changes do not refill it.
  Explicit network Retry is the existing persisted CAS grant. Export performs no network work and
  never debits or refills this allowance. Retry never changes the journal. The Resume preflight
  avoids spending allowance when the destination is already known to be unusable.
- Local destination failures have a separate in-session bound: a transient write, flush, reopen or
  link failure may be retried automatically at most 3 times per chunk and 8 times per export
  session, with backoff awaited in native through the sanctioned `catcoms-rt` clock (never a sleep
  on the actor). When the bound is reached the session stops, checkpoints if it can, and the journal
  stays Writing (Saving interrupted). Exports never start without an explicit invocation, so each
  cross-session retry is a user action; no durable local counter is needed.
- Disk full is not retried automatically. The session stops with "Not enough space in Downloads",
  keeps the partial and its last checkpoint, and writes no record.
- Record publication on a full vault volume: a record write whose `create_new`, `write_all` or
  `sync_all` fails with disk full (kind captured before the error is stringified) removes the torn
  `next` this call created and flushes the directory, instead of poisoning the transfer. For the
  first record of a new transfer it also removes the then-empty transfer directory and does not
  fault the store. A failure at or after the rename, or a failed cleanup, keeps the existing poison
  and fault path; process crashes keep their existing fail-closed recovery. This covers progress,
  checkpoint and admission records for Keep and Download alike. It does not cover the one-time
  retained-store format marker, which deliberately keeps an incomplete file and still faults.

## Fences and races

- Every journal transition checks the original export lease (mount and transfer read epochs) or,
  for lease-free preflight edges (halt, recovered Published), the exact saved journal by CAS. Lock
  does not retire the lease, so **every journal command also carries the original invocation
  cancellation** (session generation and registry entry) and the actor refuses it once cancelled,
  as `ReadDownloadChunk` already does. A blocking write that finishes after Lock therefore cannot
  persist its checkpoint, and a stale preflight cannot persist a halt. Lock, account switch, actor
  freeze/thaw and remount release stale work; an in-flight blocking write can finish only into the
  identity-proven partial, which is crash row 3.
- Publication and Remove/Clear/Pause are serialized by the native session commit guard:
  publication holds it across the link and the Published transition, as `commit_kept_command`
  already does for actor commands. Either Published commits first (Remove then clears the record and
  the staging name only) or Deleting/Pause commits first (the lease is retired and the link never
  runs). One outcome, reflected truthfully in the native response.
- Save new copy retires the transfer's read epoch in the same transition that clears the journal,
  so any in-flight export of the old journal stops at its next actor call.
- `finish_download_export` is allowed only with export None (the non-resumable path) or Published.
  A record with a Writing journal cannot be released through the fallback path. An already
  released record makes finish a no-op, so a Remove that won does not cause a false cleanup warning.
- Replacement transfer of the same file: a new Download ID or run cannot use the old lease, presence
  or journal.

## Deleting lifecycle

Remove and Clear publish Deleting (keeping the journal, retiring the read epoch) on the actor, then
native performs external cleanup in a blocking worker, then the actor retires the journal and
releases the encrypted record. Mount never performs external cleanup: it releases Deleting records
without a journal as today, and leaves Deleting records with a journal untouched, in a single
non-fatal pass. A stuck cleanup therefore cannot loop, fault the store or stall mount on an
unavailable folder; it remains visible as Removing with Remove and "Keep partly saved file".

## Privacy

The destination folder path, both leaf names and the file identity are new persisted metadata.
They live only inside the sealed record (purpose-separated metadata key) and in native memory.
Debug output for journals, identities and open results is redacted. They never appear in renderer
projections, logs, diagnostics or sidecars, and error strings never include paths (pinned by a
test). The renderer still receives the final path once, in the successful save result, as today.

New exposure: a paused or interrupted export now deliberately keeps partial plaintext in the
destination folder as `<name>.mewtual-part` across lock and restart, until it is resumed,
completed, replaced by Save new copy, or removed. Previously a failed export deleted it.
THREAT-MODEL records this.

## UI

Logic lives in `apps/desktop/src/private-downloads.ts` (status labels, actions, chat Save routing,
detail text) with tests; `App.svelte` changes are limited to the Transfers list rendering through
that module and a small action dispatcher. Copy uses no "·" separators or em dashes and makes no
durability absolutes. Section note: "Resume continues a partly saved file only if it is still the
file Mewtual created. Save new copy starts a separate file. Remove deletes saved progress and a
partly saved file Mewtual created."

No new native command: `save_group_file` gains optional `freshCopy` (requires `resume` and
`expectedIntent`), and `cancel_private_download` gains optional `keepPartial`. Command policy,
capability, ledger and fixture lists are unchanged.

## Platform abstraction and the Windows blind spot

All platform code is in `external.rs`. The Unix branch uses only stable `std` and is
**compile-unverified on this Windows host**; the Linux CI job is its first compiler. Windows
by-handle delete and no-replace rename would need `unsafe` FFI, which the workspace denies, so
hard link plus unlink is used on both platforms.

## Tests planned

- Storage: identity, birth time and no-follow primitives on Windows; encoding predicates keep a
  journaled V2 Completed record out of every public read and listing; every crash-table row;
  destination deleted, moved, replaced by another file, replaced by a link or junction, folder
  removed, folder identity changed, permission denied, disk full (injected); an identity collision
  over foreign content halts Changed and Remove leaves the bytes unchanged; budget exhaustion
  during re-verification then restart stays exhausted until Retry, and Retry preserves the journal;
  V1 read and V1-to-V2 upgrade; unknown domain refused without deleting bytes or the external
  partial; maximum record size; disk full on a record write leaves no torn `next` and no poison;
  mount leaves Deleting-with-journal records and does not loop; finish refused while Writing;
  error strings carry no paths.
- Actor: journal commands through the real actor; pause mid-export; cancel racing publication in
  both orders; freeze/thaw and remount fences; replacement transfer; Save new copy retires an
  in-flight export.
- Native: projection of every status; export driver retry bound and Full stop with a scripted
  step source; fresh/fallback selection.
- Frontend: projection validation, labels, actions, chat Save routing and detail text; lifecycle
  handler tests for Save new copy and Keep partly saved file.

Each new guard is confirmed by breaking its invariant and watching the test fail.

## Honest limits

- A fresh save goes to the OS Downloads folder; there is no save dialog.
- Resume avoids duplicate copies and orphan partials. It is not faster than a fresh save on a
  local disk: it re-verifies the encrypted copy and reads the plaintext prefix again.
- Identity checks and path-based link/unlink have a small race window against a same-user process
  that swaps files at exactly that moment. That process already has equivalent authority.
- On filesystems without stable file IDs or birth times (some SMB, FUSE, FAT), saves stay
  non-resumable as today. Volumes whose device number changes across reboots report Changed.
- Folder flushes on the destination are attempted but not required; on a filesystem that ignores
  them, power loss can lose a just-published name while the journal says Saved. Process crashes
  are covered.
- Hard links are required, as today; a FAT or exFAT Downloads folder cannot publish.
- Publication holds the session commit guard across the staging identity recheck, one hard link,
  the final-name reopen and the Published actor transition (the folder flush and staging unlink run
  after release). A destination that hangs during those steps can delay Lock, as the existing
  writer's guarded link already can.
- Orphans from crash row 1, or `.part` files from before this change, are never adopted.
- Old readers refuse the retained store while a journaled record exists, without deleting it.
- Unix code is compile-unverified locally; native and physical acceptance remain separate.

## Design review dispositions (revision 2)

| Finding | Disposition |
| --- | --- |
| H1 weak Unix identity; mismatch truncates | Birth time added to identity, zero/unsupported values make identity Unsupported; mismatch inside the checkpoint halts Changed; distinct `.mewtual-part` suffix; collision regression planned. |
| H2 actor-thread external I/O | All external I/O moved to native blocking workers using storage-crate functions; the actor only performs sealed journal transitions; backoff awaited in native. |
| H3 disk full can poison the store | Per-chunk record writes removed (checkpoint every 8 chunks and at stop/publish); pre-rename disk-full record failures remove the torn `next` instead of poisoning; doc claim corrected. |
| H4 encoding predicates | Exhaustive `is_download()` matches; serving/listing regression for journaled V2. |
| M1 Deleting lifecycle | Explicit Remove performs external cleanup; mount single pass, non-fatal, never external; "Keep partly saved file" escape; link-count-2 staging removal defined. |
| M2 folder source | Sealed folder path plus identity for every journaled operation; no directory creation there. |
| M3 unreachable/misleading states | Chat Save routed by status; destination preflight before re-verification; Published offers Save another copy and Clear; Save new copy retires the read epoch. |
| M4 fallback not enforced | `finish_download_export` refused while Writing; no-op once released. |
| Low: Unix FIFO/link race | Regular-file metadata check precedes open; residual race runs on a native worker, not the actor. |
| Low: by-handle Windows ops | Not available without `unsafe`; final-name identity checked before choosing a new numbered name. |
| Low: lock/freeze drop session | Session is owned by the native task and dropped with it; presence is weak. |
| Low: Writing(0) failure | Just-created file removed after an identity recheck. |
| Low: size/CID binding | `ExportFile` binds sizes and CID from the `DownloadReady` manifest. |
| Low: error strings | Path-free messages, pinned by a test. |

## Re-review dispositions (revision 3)

| Finding | Disposition |
| --- | --- |
| M-1 disk-full fix misses new-transfer creation | Create path also removes the empty transfer directory and skips the fault; only `create_new`/`write_all`/`sync_all` failures qualify; only a `next` this call created is removed; kind captured before stringification. Format-marker case documented as unchanged. Regression: first record on a full disk, then remount. |
| M-2 FAT/exFAT pass identity but cannot link | Hard-link probe before Writing(0) falls back to the non-resumable writer; "unsupported" at publication halts. Regressions for both. |
| M-3 journal writes after Lock | Every journal command carries the original invocation cancellation, checked by the actor before the transition. |
| M-4 guard scope | Folder flush and staging unlink moved after the guard is released; honest limit corrected. |
| Low: Save another copy can orphan a row-7 staging name | Native removes a proven staging name before forgetting the journal; a failure leaves a second name for the saved file, documented. |
| `QuotaExceeded` alongside `StorageFull` | Not adopted: only `StorageFull` is classified, keeping the non-poisoning exception narrow. |
