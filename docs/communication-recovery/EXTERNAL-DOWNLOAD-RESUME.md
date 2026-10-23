# External download resume

Design checkpoint, 21 October 2026, updated 22 October. Status: **revision 3 after read-only
design review and re-review; implemented; then "Save as" (addendum) and the fixes for two later
reviews of `27ffa183` (one independent, one by the user) on top. Focused checks only; complete
suites and native gates pending.** Dispositions for every review are at the end.

## Implementation map

| Concern | Code | Tests |
| --- | --- | --- |
| Platform identity, no-follow opens, name budget, codecs, fault seams | `crates/catcoms-storage/src/kept/external.rs` | `kept/external_tests.rs` |
| Journal format, transition rules, `ExportFile`, saved-content verification, store transitions, preflight observation | `crates/catcoms-storage/src/kept/export.rs` | `kept/export_tests.rs` |
| "Save as" destination rules and the remembered (planned) destination | `export.rs` (`ChosenDestination`, `PlannedDestination`) | `kept/export_chosen_tests.rs` |
| DownloadV2 domain (journal plus planned destination), exhaustive encoding predicates | `crates/catcoms-storage/src/kept/record.rs`, `variants.rs` | `export_tests.rs`, `export_chosen_tests.rs` (successor rules, max size, unknown domain) |
| Disk-full record writes, presence field, admission | `crates/catcoms-storage/src/kept/persistence.rs` | `export_tests.rs` (disk full) |
| Two-step Remove, retire-before-delete, finish rules, status projection, mount pass | `kept/downloads.rs`, `kept.rs` | `export_tests.rs` (removal, finish) |
| Sync forwarding | `crates/catcoms-sync/src/downloads.rs` | through actor tests |
| Actor transitions, preflight observation, remembered choice, fences | `crates/catcoms-app/src/actor.rs`, `actor/file_transfers.rs` | `actor/file_transfers/tests/export.rs`, `actor/file_transfers/tests.rs` |
| Native export sequence over `ExportPorts`, preflight, cleanup claim, Remove, chooser seam, projection | `apps/desktop/src-tauri/src/private_downloads.rs`, `save_group_file` in `lib.rs` | `private_downloads.rs` tests, `private_downloads/{cleanup,export,chooser}_tests.rs`, `keep_session.rs` (recovered save) |
| Renderer states and actions, Save as entry points | `apps/desktop/src/private-downloads.ts`, `native-download.ts`, `App.svelte`, `visual-fixture.ts` | `private-downloads.test.ts`, `private-download-lifecycle.test.ts`, `native-download.test.ts`, `visual-fixture.test.ts`, `tauri-command-security.test.ts` |

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

"User-selected file" means the file the user explicitly chose to save. A one-click save goes to the
operating system's Downloads folder, resolved by native on that invocation, as today; "Save as"
(see the addendum) lets the user pick a folder and name in a native dialog. A resumed save
continues in the folder recorded when it started, which must still be the same directory object.

Out of scope: voice, live V2, media relay, six-client topology, acquisition/network changes,
automatic (unrequested) saves, and any dependency other than the `winapi-util` (storage, Windows),
`libc` (storage, Unix, open-flag constants only) and `rfd` (native dialog) edges described here.

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

A stalled destination volume (dropped SMB share, unplugged USB drive) therefore pins at most that
save's blocking worker. Chat, MLS, sync and Lock do not wait on per-chunk external work. Since
picker review M2 this holds for the non-resumable writer as well: its staging, each chunk write,
the sync, failure cleanup and publication also run on the blocking pool, with the liveness checks
between them on the runtime and again when each queued staging or write worker starts. Before
that fix it did its file I/O directly on a tokio worker, which Save as made worse by sending
exactly the stall-prone identity-less shares to it. The media check of the published file runs on
the blocking pool too, before the process-wide session commit guard is taken (re-review
MEDIUM 1). Closing a session's handle is destination I/O as well: on the async side the session
lives in an `OnPool` wrapper, so every early return or error (a refused transition, a failed
checkpoint, Lock at the commit guard, a refused Publish) closes it on the blocking pool, and the
fallback writer closes its handle together with the staging removal (re-review 2 LOW 7).

If a **fresh** export cannot be journaled (no stable identity or birth time on this filesystem, no
OS file lock, an encoded folder path over the bound, or an unsupported platform), native keeps
today's non-resumable streaming export. A folder whose filesystem cannot hard link (FAT, exFAT)
is refused instead, before any plaintext exists, because every writer publishes with a no-replace
link. Storage enforces that a record with a journal never completes through the fallback path
(see Fences).

The native sequence runs over an `ExportPorts` seam (journal transitions, chunk reads, the commit
guard, liveness, progress, backoff); production wires it to the actor and the session, and tests
drive the real sequence with real files.

## State machine

Durable record dimensions (existing): `phase` Resumable/Completed/Deleting, `user_paused`,
`blocked`, `exhausted`, `intent`, `grant`. New dimension, Download records only:

```
export: None | Writing(Journal) | Published(Journal)
Journal { folder, folder_id, staging_leaf, final_leaf, file_id,
          written_chunks, written_bytes, halt: None | Unavailable | Changed }
planned: None | Some(PlannedDestination { platform, folder, folder_id?, name })
         (only while export is None; see the Save as addendum)
```

Runtime (never persisted): `checked`, the active acquisition run, and an **export presence**: a
weak reference to a token owned by the native export task, registered with the journal edge that
starts or reopens a session. "Saving" is shown only while that token is alive and the lease
generation that registered it is current. A panicked or abandoned task drops the token.

Allowed journal transitions (all checked in `validate_successor`):

| From | To | Edge |
| --- | --- | --- |
| None | Writing(0) | Fresh export start. Phase Completed on both sides, current export lease, no halt. With a planned destination, the journal must be the one made from it, and the plan is retired in the same publication. This edge and the one that remembers a plan are the only edges that upgrade DownloadV1 to DownloadV2. |
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
| `saved` | export Published | Saved | Save another copy, Save as, Clear |
| `destination-changed` | halt Changed | Partly saved file changed | Save new copy, Save as, Remove |
| `destination-unavailable` | halt Unavailable | Folder or file unavailable | Try again, Save new copy, Save as, Remove |
| `exhausted` | network allowance exhausted | Retry needed | Retry download, Remove |
| `save-paused` | user Pause with export Writing | Saving paused | Resume saving, Remove |
| `paused` | user Pause otherwise | Paused | Resume download, Save as, Remove |
| `blocked` | deterministic verification failure | Verification failed | Resume download, Remove |
| `receiving` | acquisition run active | Receiving | Pause, Remove |
| `saving` | export presence alive | Saving | Pause, Remove |
| `save-interrupted` | export Writing, not running | Saving interrupted | Resume saving, Remove |
| `ready` | Completed and checked, no journal | Ready to save | Save to Downloads (or Save to chosen place), Save as, Remove |
| `incomplete` | otherwise | Interrupted | Resume download, Save as, Remove |

The projection adds `savedChunks` (journal checkpoint, 0 without a journal) next to the existing
`verifiedChunks`, and `chosenPlace` (a boolean: a "Save as" choice is remembered; never the place
itself). Saving a file from the chat file list routes by status: `ready`, `incomplete`, `paused`,
`blocked`, `save-interrupted` and `save-paused` resume; every other status opens Transfers instead
of acting. Storage also refuses a plain (chat) save of a halted journal before any re-verification;
only the explicit Resume, whose intent CAS clears the halt, or Save new copy continue it.

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
u8   planned          0 None, 1 present (amendment, see the Save as addendum)
if planned == 1:
  u8    platform      1 Windows, 2 Unix
  bytes folder        platform-encoded absolute path, 1..=16384 bytes
  u8    has_id        0 no folder identity, 1 identity follows
  bytes folder_id     25 bytes, platform must match (only when has_id)
  bytes name          UTF-8 chosen leaf, valid with room for every derived suffix
```

The folder path is UTF-16LE on Windows and raw bytes on Unix; the identity's platform byte selects
the decoding. A platform mismatch is reported as Unavailable, never reinterpreted.

An empty file is one empty chunk in the manifest format; its complete checkpoint is
`written_chunks = 1, written_bytes = 0`. Journal validation accepts chunks without bytes for that
case only in effect: the reopen's plan-prefix check refuses the shape for any non-empty manifest.

**v2 was amended, not superseded.** The planned destination byte was added to the v2 body after
the first implementation commit, before `mewtual/download-transfer/v2` was ever pushed or released,
so there is no v3. A record written by the first v2 implementation on this branch does not decode
with the amended reader (development vaults only).

The worst-case journal adds about 18.2 KiB. The V4/Download body stays below 360 KiB, so the
existing 400 KiB slot and its quota charge are unchanged. A regression encodes a maximum
four-variant record with a maximum journal under the slot limit.

Encoding predicates become exhaustive matches with a single `is_download()` helper:
`owner()`, `public_retained()`, `has_variants()`, variant minimums and initial-field rules. A
DownloadV2 record can never be mistaken for a Keep, be served publicly, or be rewritten as V3.

Compatibility:

- DownloadV1 records keep their exact canonical bytes and predecessor hashes and decode with
  `export = None`. A V1 successor stays V1 unless it starts an export or remembers a choice.
- V2-to-V1 is refused. Keep records can never carry a journal or a planned destination.
- Records that never had a journal or a planned destination stay V1 and readable by the released
  reader. A record that once had one stays V2 for good (Save new copy forgets the journal but
  does not downgrade), so it stays unreadable to the old reader. While any V2 record exists, an
  older reader refuses its unknown domain during `configure_keep_scope`: the retained store faults
  without deleting anything, including the external partial. Returning to a current reader
  restores the journal.
- Evidence (22 October): fixtures written by this code (V2 `current` with a journal; V1 `current`
  with its V2 successor in `next`; Deleting with a journal; V2 with only a planned destination)
  were opened by the actual `dbb8d36b` reader in a scratch checkout. Each was refused
  (`configure_keep_scope` returned Malformed and the store reported "retained storage needs a
  storage check"), and every byte under the vault was unchanged after two opens. See the ledger.
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
  ReFS and some other filesystems do not guarantee that the 64-bit index from this API is unique or
  never reused; birth time makes an accidental match unlikely but not impossible there. Detecting
  ReFS needs a volume-information call that the safe wrapper does not expose, so it is not
  detected; this is a documented limit, not an ownership guarantee.
- **Folder identity**: volume and id of the opened directory, following a Downloads folder that is
  itself a junction or symlink (the user's configured location). On Unix the open is
  directory-only and non-blocking (`O_DIRECTORY | O_NONBLOCK`), so a FIFO put at a saved folder
  path fails at once instead of hanging Resume, Remove or the folder flush.
- **Leaf open without following a link**: `symlink_metadata` must show a regular file first.
  Windows then opens with `FILE_FLAG_OPEN_REPARSE_POINT` and refuses only name-surrogate reparse
  points (symlinks, junctions, mount points). A cloud files placeholder, WOF compression or
  deduplication tag is still the same plain file; its data is then read through a normal handle
  whose identity must match. Unix opens with `O_NOFOLLOW | O_NONBLOCK` and compares the opened
  handle's identity with `lstat`; a link (`ELOOP`) or reader-less FIFO (`ENXIO`) swapped in after
  the `lstat` is foreign at once.
- **Staging suffix** `.mewtual-part`, distinct from browsers' `.part`, so another program's
  in-progress download is never at our name by convention. The fallback writer keeps `.part`.
- **Name budget**: every name derived from a final name (a collision number up to " (999)", the
  staging suffix and the "-probe3" link probe) must fit the filesystem's component limit in its own
  units (255 UTF-16 units on Windows, 255 bytes elsewhere). Downloads names are shortened to fit,
  keeping the extension; a "Save as" name that does not fit is refused, never shortened.
- **Single writer**: every `ExportFile` holds an exclusive OS lock on its handle (`File::try_lock`:
  `flock` on Unix, `LockFileEx` on Windows) from create or reopen until it is dropped. A second
  reopen in this process, after a remount, or in another process is refused as Busy. On Windows the
  lock does not block deleting or hard-linking the file, but it does make other handles' reads of
  it fail while a session is open.

Rules:

1. **Creation (fresh only).** In the freshly resolved Downloads folder (created if missing, as
   today), with the name fitted to the name budget, choose `name`, `name (1).ext`, ... (at most
   1,000) whose final name is absent, then create `<final>.mewtual-part` with `create_new`. Take
   the exclusive lock and capture identity, then **prove hard links work here** by linking the new
   file to an absent probe name, checking the probe has the same identity and removing only the
   probe name. FAT and exFAT report a usable-looking index and creation time but cannot link: when
   the link call itself fails, the file just created is removed and the save is refused
   ("This drive cannot finish a save safely"), because the non-resumable writer would also fail at
   its final link. When identity or locking is unavailable, or the probe could not be confirmed or
   removed, the file is removed and a fresh save falls back to the non-resumable writer. Attempt a
   folder flush, then the actor persists Writing(0). If that publication fails, the file is removed
   after its identity is rechecked and the save reports the error.
2. **Reopen and preflight.** Every journaled operation uses the **sealed folder path** and requires
   that folder to open with identity `folder_id`; nothing is created there. The staging leaf must
   open without following a link with identity `file_id` and link count 1. Native runs this check as
   a preflight before an explicit Resume spends any allowance on re-verifying the encrypted copy,
   and again to obtain the write handle, which must then take the exclusive lock. A final name that
   already holds our identity is only a candidate: it is reported saved after its content is read
   and verified against the plan (exact length, whole plaintext CID), never on identity alone; a
   mismatch halts Changed and keeps both the bytes and the private record.
3. **Prefix verification.** The session compares the partial's bytes with plaintext regenerated
   from the retained encrypted chunks, one whole chunk at a time, from chunk zero.
   - Inside the checkpointed range `[0, written_bytes)`, any byte mismatch halts as **Changed**
     and nothing is modified. A file shorter than the checkpoint rewinds to its last whole verified
     chunk (a torn or lost tail), but the present bytes of a chunk that straddles the file's end
     must match inside the checkpoint first. With checkpoint 0 nothing is acknowledged, so identity
     alone proves ownership of the bytes.
   - Before any write, the partial must still have exactly one name and no link may have been
     attempted; otherwise the session halts Changed without modifying anything. A session that
     failed verification never produces a checkpoint again.
   - Beyond the checkpoint (our own unacknowledged tail, identity-proven), matching whole chunks
     are kept; the first short or mismatching chunk is truncated and rewritten from there.
   - Every chunk, verified or written, feeds the plaintext CID hasher; chunk lengths must match the
     manifest. No unkeyed plaintext digest is persisted.
   - The session only ever shrinks the file. If it finds the file shorter than its own verified
     offset (another writer truncated it while the handle was open), it stops with Conflict, writes
     no checkpoint, and the next Resume re-verifies from chunk zero. It never zero-extends, because
     the stream CID check covers the plaintext fed, not the file contents.
   - A verify-only session never lowers the saved checkpoint; only a session that has started
     writing after a rewind may lower it.
4. **Writes and checkpoints.** Each written chunk is followed by `sync_all`. The journal
   checkpoint (`written_chunks/bytes`) is persisted only at fsynced boundaries, every 8 chunks, when
   a session stops for a reason other than disk full, and before publication. It is never ahead of
   fsynced bytes. Verification recovers anything written after the last checkpoint.
5. **Publication.** Requires every chunk processed, hasher equal to the plaintext CID and file
   length equal to the declared size. Before any mutation it checks whether the file may already be
   visible: if a link was attempted, the final name holds our identity, or the partial has another
   name, nothing is trimmed or written. Our own final name is then verified read-only (exact length
   and CID through the session's handle) and reported already published; anything else halts
   Changed. Otherwise a longer file is trimmed and fsynced and, if the final name is occupied
   (including by something that cannot be opened), the next absent numbered candidate is persisted
   as `final_leaf` before any link attempt. Under the native session commit guard, with the original
   invocation and lease still current, the staging identity is rechecked on a handle that (on
   Windows) denies delete sharing and is held across `hard_link(staging, final)` and the final-name
   check, so the staging file cannot be renamed, replaced or deleted, nor its folder renamed, in
   between. The link refuses an existing destination on both platforms (no overwriting fallback).
   A definite "name taken" refusal leaves the file private for a numbered retry; any other link
   outcome marks it possibly visible. The final name is reopened without following a link and must
   have our identity, and the actor persists Published. The guard is then released; only after that
   is a folder flush attempted and the staging name unlinked if it still has our identity. When
   that unlink cannot be settled, the Published record is kept (and the result says so) so the
   second name stays visible until Clear. A link failure reporting that links are unsupported is a
   halt, not a transient failure.
6. **Removal.** Deletes only the staging leaf, after rule 2's folder and identity checks prove it is
   ours. When staging and final both have our identity (link count 2), only the staging name is
   removed. The final name is never deleted. A read-only staging name is made writable for deletion
   only when it has no other name, because hard links share attributes.

Halt classification: folder missing, volume gone, platform mismatch, permission denied, or the
staging leaf missing with no published final name of ours is **Unavailable**. A folder identity
mismatch, a staging leaf that is a link, directory or another file, an unexpected link count, or a
byte mismatch inside the checkpoint is **Changed**. Halts are persisted (journal CAS) and keep the
encrypted progress. Disk full and transient write failures are not halts; see Budgets.

## Crash table

| # | Crash point | Recovered state | Next action |
| --- | --- | --- | --- |
| 1 | After exclusive create, before Writing(0) is durable | export None; an empty `.mewtual-part` orphan may exist, and (if the crash fell inside the link probe) an empty `<staging>-probeN` second name for it | In Downloads, a fresh export skips the occupied names. At a chosen place the exact name is required, so the remembered choice is refused with `PartialExists` ("...for example from an earlier interrupted save. Delete it from the folder, or choose another name.") until the user deletes the orphan or picks another name (picker review L3). Orphans are never adopted or deleted; both are empty, so they disclose nothing. |
| 2 | After Writing(0), file lost (power loss) or deleted | Writing(0); staging missing | Preflight reports Unavailable; Save new copy or Remove. |
| 3 | After chunk writes, before the next checkpoint | Writing(k); file holds more, possibly torn, bytes | Verification keeps matching whole chunks after k, truncates the first torn one, continues. |
| 4 | Checkpoint claims more than the file holds (torn write, lying flush) | Writing(k); file shorter | Rewind to the last whole verified chunk and continue. A mismatch inside the checkpoint halts Changed. |
| 5 | After persisting a new `final_leaf`, before linking | Writing(all); new final name | Resume verifies, then publishes at that name. |
| 6 | After `hard_link`, before Published | Writing(all); final has our identity; staging link count 2 | Preflight (against the saved manifest's plan) or reopen verifies the final name's content read-only, persists Published, reports Saved. No second link or copy. A changed file there halts Changed. |
| 7 | After Published, before staging unlink or record release | Published; both names may exist | Saved. Clear unlinks the staging name if still ours (never the final name), then releases. |
| 8 | After Deleting (Remove/Clear), before external cleanup | Deleting with journal | Mount leaves it in place; Removing row. Remove retries cleanup then releases; Keep partly saved file releases without deleting. Never resumes fetching or export, never serves. |
| 9 | After external cleanup, before journal retirement | Deleting with journal; staging gone | Remove is idempotent (missing staging is done), then releases. |
| 10 | After the move to `deleting-v2` | Existing cleanup-only directory | Existing bounded `finish_deleting`. |
| 11 | During Save new copy, after the retire step, before or after removing our partial | Writing with a new intent; staging present or missing | The old click is stale; the row offers Save new copy again (preflight reports Unavailable when the staging is gone). |
| 12 | After a planned destination is remembered, before Begin | export None, planned Some | Resume (any later one, after restart too) saves to the remembered place; Save as again replaces it. If the crash also left a row-1 orphan at that place, Resume is refused as in row 1. |

External cleanup reports a typed outcome. Only `Removed` means nothing of this save stays on disk.
`KeptSaved` (our staging name is gone; the same file stays under its final name), `OtherNameRemains`
(another name for the partial, not made by Mewtual, still holds its plaintext), `LeftInPlace`
(something at our name cannot be proven ours) and `FolderChanged` (the saved folder path names a
different directory now) also let the removal finish, but each carries a notice that Remove and
Save new copy show, so a removal is never reported complete while plaintext remains unmentioned
(clearing a saved row keeps the saved file by design and says nothing). `FolderUnreachable`
(missing folder or volume, other platform) and `Unchecked` (the partial exists but could not be
opened, for example held without sharing), and a proven partial whose deletion fails, keep the
record in Deleting with its journal, conservatively charged; Remove retries and "Keep partly saved
file" releases the record while leaving the file for the user. Save new copy also proceeds on
`FolderUnreachable` (an unreachable folder is why the user wants a new copy, and the user is told
the old file stayed) but not on `Unchecked`. Cleanup failure never refunds quota or creates a
second reservation.

## Budgets

- Acquisition, including re-verification of a saved encrypted copy on Resume, keeps the existing
  durable execution allowance unchanged. Restart, providers and route changes do not refill it.
  Explicit network Retry is the existing persisted CAS grant. Export performs no network work and
  never debits or refills this allowance. Retry never changes the journal. The Resume preflight
  avoids spending allowance when the destination is already known to be unusable.
- Local destination failures have a separate in-session bound: a transient write, flush, reopen or
  link failure may be retried automatically at most 3 times per chunk and 8 times per export
  session, with backoff awaited in native through the sanctioned `catcoms-rt` clock (never a sleep
  on the actor). Publication retries (a name taken just before the link, a final-name check that
  failed transiently) are charged to the same per-session budget and wait on the same clock. When
  the bound is reached the session stops, checkpoints if it can (only after transient local
  failures: never after a verification failure, a conflict, disk full or an inactive session), and
  the journal stays Writing (Saving interrupted). Exports never start without an explicit
  invocation, so each cross-session retry is a user action; no durable local counter is needed.
- Cleanup workers (Remove, Keep's presence check, Save new copy's delete) take one of four
  dedicated slots, so a compromised renderer cannot pin unlimited blocking threads on a stalled
  volume.
- Disk full is not retried automatically. The session stops with "Not enough space in the
  destination folder", keeps the partial and its last checkpoint, and writes no record.
- Record publication on a full vault volume: a record write whose `create_new`, `write_all` or
  `sync_all` fails with disk full (kind captured before the error is stringified) removes the torn
  `next` this call created and flushes the directory, instead of poisoning the transfer. For the
  first record of a new transfer it also removes the then-empty transfer directory and does not
  fault the store; a full volume refusing the new transfer directory itself is the same clean
  outcome. A failure at or after the rename, or a cleanup that cannot remove the torn `next` or
  flush its removal, keeps the existing poison and fault path; process crashes keep their existing
  fail-closed recovery. This covers progress, checkpoint and admission records for Keep and
  Download alike (each exempted step and both failed-cleanup paths have a regression). It does not
  cover the one-time retained-store format marker or the sealed blob store's first open, which
  still fault on a full volume.

## Fences and races

- Every in-session journal transition (including a halt or a recovered publication found by the
  reopen) checks the original export lease (mount and transfer read epochs). The Resume preflight
  runs before re-verification, so it first takes a native-only **preflight observation** from the
  actor: the saved state, the saved manifest's plaintext layout, and guards on the actor lifetime
  (freeze/thaw), Root authority, the storage mount and the transfer read epoch. Its halt and
  recovered-Published edges, and the response path after a recovery's cleanup, require that
  observation to be current as well as the exact saved journal by CAS. Lock does not retire the
  lease, so **every journal command also carries the original invocation cancellation** (session
  generation and registry entry) and the actor refuses it once cancelled, as `ReadDownloadChunk`
  already does. A blocking write that finishes after Lock therefore cannot persist its checkpoint,
  and a stale preflight cannot persist a halt or a publication. Lock, account switch, actor
  freeze/thaw and remount release stale work.
- A blocking chunk step re-checks liveness when its worker actually starts, not only when it is
  dispatched, so a step queued behind a busy blocking pool writes nothing after retirement. A step
  that has started runs to completion: its write is fsynced, and only a lease-fenced checkpoint
  could ever describe it (crash row 3).
- Publication and Remove/Clear/Pause are serialized by the native session commit guard:
  publication holds it across the link and the Published transition, as `commit_kept_command`
  already does for actor commands. Either Published commits first (Remove then clears the record and
  the staging name only) or Deleting/Pause commits first (the lease is retired and the link never
  runs). One outcome, reflected truthfully in the native response.
- Save new copy first retires every observer of the old journal under the explicit intent CAS
  (read epoch and a new intent, journal kept durable), then deletes the old partial, then forgets
  the journal with the retired intent. A delayed click, a concurrent Resume or a live writer of the
  old journal cannot act on it while it is being deleted, and a failed delete leaves the journal
  naming the partial.
- Destructive cleanup (Remove, Save new copy) and the preserve decision (Keep partly saved file)
  hold a process-wide claim per (server, Download ID). The deleting blocking worker owns a clone
  of the claim, so it outlives a dropped invocation (Lock, a closed window) until the delete has
  ended; a second claimant gets Busy. Keep reassesses under the claim and says when the file was
  already removed instead of promising it. That check is best effort (re-review 2 MEDIUM): with
  every cleanup slot busy it answers "unknown" without touching the disk, and Keep stops waiting
  for it after a few seconds on the sanctioned clock, so Keep never waits on the destination and
  stays the escape from a removal stuck on a dead share; "unknown" promises nothing. Keep's other
  two steps (publishing Deleting, releasing the record) are actor commands under the session
  commit guard, so like every guarded command they wait while another holder of that guard
  (for example a publication whose link hangs) holds it. Plain Resume takes the same claim only
  around its intent change (re-review 2 LOW 2, narrowed by re-review 3), so it is Busy while a
  cleanup owns the Download; its destination preflight runs without the claim, so a Resume
  hung on a dead share never blocks a cleanup, and that preflight's halt and recovery edges are
  refused once a cleanup has started, because Remove's Deleting step and Save new copy's retire
  both invalidate its observation. When that preflight recovered a save (crash after the link),
  its cleanup of the record is conditional too (final review L1): it stands down, without a
  warning, if another cleanup holds the claim or its observation is no longer current, so it can
  never release a record that a later Save new copy is re-saving.
- Native checks the original invocation and lease before **every** write attempt, including each
  retry after a backoff, so a session retired by Lock, Pause, Remove or replacement writes no more
  plaintext.
- One live writer per partial: within a mount, Begin and Resume are refused while an earlier
  session's presence token is alive (it is not cleared by Pause, halts, a failed Remove write or
  Save new copy, only by its own drop); across mounts and processes, the OS lock every
  `ExportFile` holds refuses a second handle onto the partial. A halted journal is never shown as
  actively saving.
- After a successful link, a failure to *check* the final name is transient: the retry trims and
  writes nothing, verifies our own final name's content read-only, and only then reports it saved;
  a different identity, a foreign object or changed content there is Changed. A link call that
  failed uncertainly (a network timeout) but left our file with one name and a final name that is
  absent or opens as another file made no link, so the retry publishes normally instead of
  halting Changed (re-review 2 LOW 1). A final name that merely cannot be opened proves nothing
  (our link may have made it); the session then stays "possibly visible" and trims nothing
  (re-review 3 LOW 1). Recording a proven publication also retires an earlier halt and clears
  Pause with an intent change.
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

The destination folder path, both leaf names, the file identity and a remembered "Save as" folder
and name are new persisted metadata.
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
`expectedIntent`) and optional `saveAs` (a flag; its parameter list is pinned by a command-security
test so no path, folder or destination parameter can be added), and `cancel_private_download`
gains optional `keepPartial` and now returns an optional notice when a file of the save stays on
disk. Command policy and capability lists are unchanged; the review ledger comment and the visual
fixture (Save as behaves as a closed dialog and any path argument is refused) are updated.

Notices name the Downloads folder only when a save provably went there (a first one-click save);
saves to a chosen or remembered place, and resumed partials, name no folder. Closing the "Save as"
dialog is quiet: no error, no notice, no leftover row.

## Platform abstraction and the Windows blind spot

All platform code is in `external.rs`. The Unix branch uses stable `std` plus `libc` open-flag
constants passed through std's safe `custom_flags`. It cannot run on this Windows host, but since
22 October the storage crate (all targets, including the Unix-only FIFO tests) compiles and passes
Clippy with `-D warnings` for `x86_64-linux-android`, a Unix-family target that is installed here;
Linux-only `target_os` code would still be unchecked, and the Linux CI job remains the first runner.
Superseded by the PR51 review (see that section): deletion now goes through the verified handle on
Windows (`fs_at`) and through rename-aside, verify and `unlinkat` relative to a pinned folder on
Unix (`rustix`), and publication names the held object (Linux `/proc/self/fd`, Windows pin, macOS
path link plus post-check only). Before that, by-handle delete would have needed `unsafe` FFI in
our code, which the workspace denies, so removal and the Unix publication link were path calls
made after a handle check.

Creating a new partial is path-based too (picker review L4): the folder identity check and the
exclusive create are separate calls. Every creation (`create`, `create_chosen`, and
`create_unjournaled` when an identity was captured) now reads the folder identity again after
creating; on a mismatch it removes its own partial, only while that name's handle identity still
proves it is ours, and refuses (`FolderChanged` at a chosen place, a local failure in Downloads).
A swap and swap-back that both fall between the two identity reads still leaves an empty partial
in the other folder; creation then refuses, because it requires the partial's name to hold our own
handle's object whenever the filesystem reports identities (re-review LOW 2 added this to
`create_unjournaled`), so no plaintext is written there even if a file was planted at our name. On
Windows the link probe makes that check; on Unix, where creation makes no link (PR51 M3), a
read-only no-follow check of the name does (restored after the Linux CI failures at 516ab8d0, see
below). An identity-less chosen folder has no swap check at all (as before). The
proven removal leaves a just-created partial in place when its name cannot be opened right then
(for example another program holding it); that empty file then blocks a remembered choice with
`PartialExists` until the user deletes it.

## Tests

- Storage: identity, birth time and no-follow primitives; name-surrogate classification; the
  Windows pin; the name budget; encoding predicates keep a journaled V2 Completed record out of
  every public read and listing; every crash-table row; destination deleted, moved, replaced by
  another file, replaced by a link or junction, folder removed, folder identity changed, permission
  denied, disk full at every exempted step and both failed-cleanup paths; an identity collision
  over foreign content halts Changed without writing; a straddling chunk inside the checkpoint;
  a second writer refused by the OS lock; a retry after an unconfirmed link never trims or claims a
  changed final file; tampered partials linked or renamed to the final name are not saved; a
  stream that fails verification never checkpoints; removal outcomes for every name left behind;
  retire-before-delete; preflight observations retire with re-verification and remount; empty
  files; long multibyte names; budget exhaustion, Retry, V1 read and upgrade, unknown domain,
  maximum record size, mount behavior, finish rules, path-free errors; all Save as rules (see the
  addendum).
- Actor: journal commands through the real actor; pause mid-export; cancel racing publication in
  both orders; freeze/thaw and remount fences, including stale preflight edges; replacement
  transfer; Save new copy; an empty file acquired and saved; a Save as choice surviving an
  interrupted acquisition and remount; a Save as refused before it can interrupt a live journaled
  save; the undo for a new record whose choice cannot be written.
- Native: projection of every status; retry bound; the real export sequence over `ExportPorts`
  (guard placement, Publish failure after the link and its recovery, paced and bounded publication
  retries, stop checkpoints, verification failure, a step queued before retirement, a staging name
  that cannot be removed); the real Remove, Keep and Save new copy sequences with a barrier inside
  the deleting worker; the recovered-save response after Lock, entry retirement or an actor change;
  the chooser seam (cancel, existing name, valid choice, one dialog at a time, no Downloads
  folder); destination selection; the export sequence at a chosen place; the fallback writer's
  staging off a single-threaded runtime; a failed cleanup keeping earlier warnings; peer device
  names with trailing spaces; message mapping.
- Frontend: projection validation, labels, actions, chat Save routing and detail text; lifecycle
  handler tests for Save new copy, Keep partly saved file, removal notices and every Save as path;
  the save_group_file signature pin; the visual fixture.

Each new guard was confirmed by breaking its invariant and watching its test fail, except where
the ledger says the guard cannot run on this host.

## Honest limits

- A one-click save goes to the OS Downloads folder; "Save as" lets the user choose a folder and
  name for one save (see the Save as addendum).
- Resume avoids duplicate copies and orphan partials. It is not faster than a fresh save on a
  local disk: it re-verifies the encrypted copy and reads the plaintext prefix again.
- PR51: automatic deletion deletes only the verified object; on Windows against every process, on
  Unix only against processes running as other accounts (folders other accounts can modify get no
  automatic deletion; a same-account process could swap the random aside name before the unlink).
  macOS publication guarantees only that a foreign object is never reported as saved. Creation is
  rechecked afterwards, so a swapped folder gets at most an empty partial.
- Identity is the only ownership proof for deletion: a real identity collision (infeasible with
  birth time on NTFS, ext4 and APFS; possible in principle on ReFS) would let Remove delete a
  foreign file at our staging name.
- On filesystems without stable file IDs, birth times or file locks (some SMB and FUSE mounts),
  saves stay non-resumable as today. Volumes whose device number changes across reboots report
  Changed. A OneDrive placeholder whose data is not on the device is downloaded when Mewtual reads
  it back to verify or resume.
- Folder flushes on the destination are attempted (since PR51 M2, after the link and before
  Published) but not required; on a filesystem that ignores them, power loss can still lose a
  just-published name while the journal says Saved. Clearing such a row keeps the staging name if it
  is the only copy and says so. Process crashes are covered.
- Hard links are required, as today; a FAT or exFAT folder (Downloads or chosen) is refused before
  any plaintext is written, with a message pointing to Save as.
- Publication holds the session commit guard across the staging identity recheck, one hard link
  and the final-name reopen; the folder flush then runs without the guard, and the Published actor
  transition takes it again (PR51 M2; the staging removal runs after release). A destination that
  hangs during the guarded steps can delay Lock, as the existing
  writer's guarded link already can, and, because the guard is process-wide, every other command
  that takes it (other saves' checks among them) until the operating system gives up.
- Orphans from crash row 1, or `.part` files from before this change, are never adopted. At a
  chosen place a row-1 orphan blocks the remembered choice until the user deletes it from the
  folder or chooses another name; the refusal says so.
- Old readers refuse the retained store while any V2 record exists, without deleting it.
- Unix code is compile- and Clippy-checked locally for an Android target but never run here;
  native and physical acceptance remain separate.
- While a save session is open on Windows, other programs cannot read the partial (the session's
  lock); this ends when the session ends.

## Save as: a user-chosen destination (addendum, implemented)

Added on top of `27ffa183`. The path authority moves from the fixed Downloads folder to a folder the
user picks in a native dialog, for that one save.

- **Command.** `save_group_file` gains an optional `saveAs` flag. It carries no path: native opens
  the dialog itself. A command-security test pins the command's exact parameter list so no path,
  folder or destination parameter can be added unnoticed.
- **Dialog.** `rfd` 0.16 (MIT) used directly from Rust with `default-features = false` and
  `common-controls-v6` plus `gtk3`, the same backend configuration `tauri-plugin-dialog` 2.7 uses.
  The dialog is constructed on the main thread through Tauri core's
  `AppHandle::run_on_main_thread` and awaited on a dedicated thread; the async command only awaits
  a oneshot. Neither the async runtime nor the main thread blocks while it is open. The official
  plugin was not chosen because it adds `tauri-plugin-fs`, a plugin IPC command surface and a
  builder registration for no capability we need; with `rfd` there is no `dialog:*` permission, no
  plugin registration and no builder or `setup` change. The dialog sits behind a chooser trait so
  tests never open one. It starts in Downloads, or at the OS default when no Downloads folder can
  be resolved (picker review L6: Save as is the remedy offered when Downloads cannot be used, so it
  must not depend on one). `rfd` logs only in backends this build does not compile (the XDG portal
  and the macOS message box); the Windows common-controls and GTK save dialogs log nothing, so no
  chosen path reaches a log through it.
- **Order.** The invocation captures its session generation, server instance and native origin
  first; then the dialog opens (at most one at a time process-wide); then the origin and server
  instance are rechecked, so a Lock or unlock while the dialog is open discards the choice. Only
  then is the export slot taken (an open dialog holds no slot), and Save new copy, the intent CAS,
  acquisition and export follow. Cancelling, or a dialog that closes without a path (for example a
  virtual or phone location the dialog cannot return as a path), writes no record, starts no
  acquisition and spends no allowance.
- **Selection checks** (storage, `ChosenDestination::validate`): the path is absolute; its leaf
  passes the journal's leaf rules (no Windows device names such as `CON`, `NUL` or `COM1`) and the
  name budget (room for " (999)", `.mewtual-part` and the probe suffix; a longer name is refused as
  too long, never shortened); its folder path fits the sealed bound; its parent opens as a
  directory and its identity is captured when the filesystem provides one; the chosen name does
  not exist (refused even when the OS dialog already asked to replace it, and that file is left
  untouched); `<name>.mewtual-part` does not exist. Nothing is created by validation.
- **Remembered choice (record change).** The encrypted copy usually is not complete when the user
  chooses, and that network phase is where interruptions happen. The choice therefore persists in
  the sealed Download record as a *planned destination* (folder path, optional folder identity,
  chosen name), written by the actor in the same synchronous step that starts or resumes the
  acquisition run, before any network work (if that write is refused, the run just started is
  released: a record this step created is removed outright, an existing one is paused; picker
  review L2). A Save as on a file whose record already has a journal is refused before the run is
  resumed, so it never interrupts a live save of the same file. Every later Resume, including
  one-click Resume after a restart, saves to that planned place: an explicit Resume first checks
  it read-only (folder identity, names free) before spending allowance on re-verification
  (re-review 2 LOW 3; only for a Completed record, so an incomplete download's network Resume is
  never refused over its destination, re-review 3 LOW 2), the verified copy carries the record's planned destination, the export
  validates it again (identity, name absent) and Begin consumes it, refusing a journal that does
  not match it. Save as again replaces it (the latest choice wins); a Save as invocation whose
  choice another save already consumed is refused, never redirected to Downloads. A remembered
  choice this platform cannot decode (a vault moved to another OS) is refused. Save new copy and
  Remove behave as before. This amends the unreleased v2 encoding (a planned field after the
  export section) rather than adding v3; v2 has never been pushed or released. A planned
  destination may exist only while there is no journal, and only a V1 record that gains a journal
  or a planned destination is upgraded.
- **Builds at `27ffa183` (re-review 2 LOW 6).** Records written by that intermediate build carry
  v2 without the planned-destination byte, so this build refuses them and the retained store
  faults ("needs a storage check") without destroying anything. No dogfood or installed build of
  `27ffa183` exists; it was only ever exercised by tests, so no real vault is affected.
- **Creation** (`ExportFile::create_chosen`): the folder must still have the captured identity (a
  parent swapped for a link, junction or another folder, or one that no longer reports any
  identity, is refused with "The chosen folder changed"); the chosen name must still be absent; the
  partial is created exclusively beside it and the folder identity is read again afterwards (a
  swap between the two reads removes our partial and refuses; a swap and swap-back inside that
  window is the residual named under Platform); the lock, identity and hard-link probe run as for
  Downloads. Permission denied means "Mewtual cannot write to that folder". No directory is ever
  created there. An existing `<name>.mewtual-part` (for example an orphan from crash row 1) is
  refused, never adopted.
- **Overwrite policy.** Never replace. A name that becomes occupied after the partial exists gets
  the next free numbered name at publication, derived from the sealed name this save used (the
  staging name without `.mewtual-part`), never from today's listed file name. The success result
  shows the actual final path.
- **Journal.** Unchanged shape. Every resume uses the sealed folder (never `download_dir()`), so
  resume at a chosen folder runs the same identity, link-count and prefix checks.
- **Save new copy** from any row still writes to Downloads (the one-click default). "Save as" on a
  row whose journal is halted or saved means "save a new copy to a chosen location": dialog first,
  then the old journal is forgotten exactly as for Save new copy. Native refuses a chosen
  destination when a journal exists unless `freshCopy` is set, and re-checks "no journal" at export
  time so a concurrent one-click save cannot hand its journal to the Save as invocation. Choosing
  the same folder and name as a halted partial is refused (its `.mewtual-part` still exists when the
  choice is checked); choose another name or Remove first.
- **Unsupported locations.** Two cases are kept apart:
  - No hard links (FAT, exFAT, typical USB sticks): no writer can publish there without risking a
    replace, so the save is refused before any plaintext is written ("This drive cannot finish a
    save safely..."). This also applies to one-click saves into such a Downloads folder, including
    the non-resumable one-click writer for a Downloads folder without identities, which probes
    links on its fresh `.part` with the same storage function (re-review 2 LOW 4).
  - Hard links but no stable identity (some network shares, FUSE mounts without birth time such as
    the document portal): the non-resumable writer runs in the chosen folder with a chosen-location
    variant (`ChosenDestination::create_unjournaled`) that never creates folders, uses the exact
    chosen name, refuses an existing name or `.part` instead of numbering, requires the folder to
    still be a directory, and probes hard links on its fresh `.part` first (refusing a no-link
    folder before writing). The result and any error say that an interrupted save there starts
    over. Without an identity there is no parent-swap check (documented residual). Like the
    journaled path, this writer does all its file work on the blocking pool (picker review M2).
- **Authority and privacy.** The chosen path authorizes saves of that one Download, never of any
  other file. It stays the destination of every save of that Download until a save begins there
  (Begin consumes it into the journal), a newer Save as replaces it, or the record is released.
  So a plain one-click Download of the same file from the chat, made meanwhile, also saves there,
  by design (the choice survives interruption), and a folder without identity, which never
  begins a journal, keeps the choice until its non-resumable save completes and releases the
  record. Re-review 2 LOW 7 asked whether a plain click should stop inheriting an older choice;
  it was kept, because that is exactly the interrupted-save path the choice exists for, and the
  result's `inDownloads` flag keeps the notice truthful. It never comes from the renderer, is
  never logged, and persists only inside the sealed record (planned destination, then journal). The renderer receives the final path once, on success, plus a
  boolean "saving to a chosen place" for the row. The app writes only the chosen leaf, its
  `.mewtual-part` (or `.part` for the non-resumable writer) and a transient link probe there;
  operating-system permissions still apply. Lock cannot close an open OS dialog, so the suggested
  name and folder stay on screen until the user closes it (the choice is then discarded). Lock,
  unlock or a server replacement does end the invocation's wait and releases the one-dialog turn,
  so a dialog that never answers (a GTK future that never completes) cannot wedge Save as until
  restart; that orphaned window's answer is discarded and a new Save as may open beside it. An
  invocation already cancelled when it reaches the dialog never opens one (re-review 3 LOW 3).
  Cloud-synced or shared folders may upload the partial plaintext and the probe name.
- **UI.** "Save as" sits next to Download in the file menu, embed menu, file details and image
  viewer, and in Transfers rows that are ready, incomplete, paused, unavailable, changed or saved.
  It is not offered on running, removing, exhausted, save-interrupted or save-paused rows (continue
  or Remove those first). Rows with a remembered choice say so in their detail text (", saves to
  the chosen place"); only the ready row's action label changes, to "Save to chosen place". Resume
  labels are unchanged. The flipnote viewer and inline chips keep only their existing action
  (follow-up). Copy that said "Downloads" for every save now names no folder, including a media
  file whose bytes matched its declared type (picker review M1). Native's save result carries
  `inDownloads` only when the file sits directly in the Downloads folder, and the notice names
  Downloads only with it (re-review LOW 1b, made fail-closed in its re-check); a failed reveal
  says "its folder could not be opened".
- **Tests.** Scripted chooser: cancel writes nothing; an existing name is refused and left
  byte-identical; a chosen folder resumes after restart; the chosen folder removed or renamed while
  paused halts; no-link folders are refused before writing; identity-less folders fall back with
  the chosen-location rules; a swapped parent is refused, before and during creation; an orphan
  partial is refused and left alone; the remembered choice survives an interrupted acquisition and
  restart; a Save as never interrupts a live journaled save; a new record whose choice cannot be
  written leaves no row; one dialog at a time, also without a Downloads folder; the destination
  selection; the export sequence at a chosen place (exact name, refused Begin, existing journal);
  the fallback writer off the async runtime; the command signature cannot carry a path; the visual
  fixture mock; frontend projection, actions and folder-free notices.

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

## Post-implementation review dispositions

Read-only review of `dbb8d36b..1bca1620` (no blocker). All items below are fixed on this branch
unless marked as a follow-up.

| Finding | Disposition |
| --- | --- |
| HIGH: `unused_mut` in a cfg-gated branch breaks the Linux `-D warnings` build | `remove_proven` helper with no `mut`; the Windows read-only retry is inside `#[cfg(windows)]`. Still compile-unverified locally. |
| HIGH: an old session could write after Lock/Pause/replacement; a later write could zero-extend a hole | Per-attempt liveness check (`accept_with_retries`); only-shrink with `Conflict` in `accept` and `prepare_publication`; one live writer per partial. Regressions: native retry/liveness tests, storage truncation and second-writer tests. |
| MEDIUM: `remove_partial` treated "cannot check" as done | Typed `PartialRemoval`; Remove keeps Removing on `FolderUnreachable`/`Unchecked`; Save new copy refuses `Unchecked`. Windows regression with a non-sharing handle. |
| MEDIUM: a failed post-link check became a permanent Changed halt | Post-link check errors are transient; recovery retires a halt. Regression with a seam on that check. |
| LOW: Writing(0) failure text vs code | Design text corrected (error, no fallback). |
| LOW: opt-in private-download browser flow stale | Rows carry `savedChunks`, new note text, Resume absent while receiving, plus a changed-destination action check. Not run here (`test:flows` reserved). |
| LOW: recovered publication while paused | `mark_published` clears Pause with an intent change. |
| LOW: stale comment; Debug of path-carrying results | Comment updated; `ExportProbe`, `ExportReopen` and `Publication` Debug redacted. |
| Residual: no native end-to-end test of `export_journaled` | Follow-up. The retry/liveness policy is unit-tested; the sequence is covered at the actor layer. |
| Residual: intermittent `actor_freeze_and_thaw_release_stale_export_transitions` | One unreproduced failure right after a rebuild; later passed in about 25 targeted reruns and in the complete per-crate app run (1,216 passed). Follow-up to root-cause if it recurs. |
| Residual: Save new copy stale click | Intent pre-check before deleting anything; the abandon repeats the exact CAS. |
| Residual: ReFS 64-bit file index, Unix CI birth time and non-root assumptions | Documented limits. |

Re-review of `591ba423`: no blocker, high or medium remains. Low follow-ups, not fixed here:

| Follow-up | Note |
| --- | --- |
| Single writer across remount | Presence is per mounted transfer; key the exclusion process-wide by Download ID in native if a stalled-volume race across remount matters. Qualified above and in THREAT-MODEL. |
| Clear on a saved row with an unplugged folder | Stays Removing and offers "Keep partly saved file", although what remains is a second name for the complete file; wording could be specialized. |
| Older session still alive | Resume spends one re-verification before the presence refusal; Save new copy can forget the journal and then fail at Begin. Bounded by clicks and that session's lifetime. |
| Remove on a hung share | `privateDownloadBusy` stays set until the blocking removal returns (OS I/O timeout); pre-existing pattern. |

Later status of these follow-ups: "single writer across remount" is resolved by R-M1 (the OS lock);
"older session still alive" no longer lets Save new copy forget the journal before its delete
(U-H1's retire-before-delete); the other two are unchanged.

## Picker design review dispositions (Save as)

Read-only review of the Save as design before implementation (two HIGH, four MEDIUM, no blocker).

| Finding | Disposition |
| --- | --- |
| H1: FAT/exFAT pass identity but cannot link, so the fallback writer fails at the very end | Fixed in `551f54a3`. A failed link probe is `NoLinks`: the fresh partial is removed and the save refused before any plaintext, for chosen and Downloads folders alike; only identity-less folders that can link fall back, and the chosen fallback probes links on its `.part` first. Tests: `a_folder_without_links_is_refused_at_creation_and_halts_at_publication`, `a_chosen_folder_without_links_is_refused_before_any_plaintext_exists`. |
| H2: the choice was lost if acquisition was interrupted | Fixed by the v2 amendment (planned destination) in `551f54a3` and its actor plumbing in `54638f5a`: remembered before any network work, survives restart, consumed by a matching Begin. Tests: `a_remembered_choice_survives_an_interrupted_acquisition_and_restart` (storage), `a_save_as_choice_survives_an_interrupted_acquisition_and_remount` (actor), plus Begin binding, replacement and successor-rule tests. |
| M1: the fallback writer broke the chosen-location rules | Fixed in `551f54a3`: `ChosenDestination::create_unjournaled` (no folder creation, exact name, refuses an existing name or `.part`, folder must be a directory and keep any captured identity, link probe). Test: `an_identityless_chosen_folder_falls_back_with_the_chosen_location_rules`. |
| M2: collision numbering after resume used the listed name | Fixed in `551f54a3`: `reopen` no longer takes a name; numbering derives from the journal's own staging name. Test: `a_collision_after_resume_is_numbered_from_the_chosen_name_not_the_listed_one`. |
| M3: a Save as could pick up a Downloads journal made by another click | Fixed: a chosen destination refuses an existing journal at entry and at export time, Begin refuses a journal that does not match the remembered choice, and a Save as whose choice was consumed is refused (`PLACE_CHANGED`). Tests: `begin_refuses_a_journal_that_is_not_the_remembered_choice`, `a_choice_can_be_replaced_but_never_redirects_an_existing_journal`. |
| M4: ordering, duplicate dialogs, parent, dropped sender | Fixed in `54638f5a`: generation and origin captured before the dialog, export slot taken after it, origin, server instance and generation rechecked once it closes; one dialog at a time process-wide; the main window is the parent; a dropped sender is Cancel. Test: `save_as_selection_through_the_chooser_seam` (one dialog at a time). The recheck itself is not unit-tested (it needs a real AppHandle and session); see the ledger. |
| LOW: copy still said Downloads | Fixed: disk-full, local-failure and busy messages name no folder; notices name Downloads only when a save provably went there. |
| LOW: protected folders | Fixed: permission denied at a chosen place is `NotWritable`, "Mewtual cannot write to that folder." |
| LOW: Windows device names | Fixed: `valid_leaf` rejects `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9` with any extension. |
| LOW: privacy notes | Recorded in THREAT-MODEL: the dialog stays on screen after Lock, cloud-synced folders upload the partial and the probe name, and the app data folder is not refused (documented, not enforced). |
| LOW: doc drift, same name as a halted partial, virtual locations | Addendum corrected (identity may be absent, save-paused listed); choosing the same folder and name as a halted partial is refused (documented); a virtual or phone location the dialog cannot return as a path behaves as Cancel. |

## Review of `dbb8d36b..27ffa183` (independent reviewer): dispositions

| Finding | Disposition |
| --- | --- |
| M1: a writer stalled across remount truncates a file already reported Saved | Fixed in `0cb7f46a`: every `ExportFile` holds `File::try_lock` from create or reopen until drop. Test: `a_second_writer_is_refused_by_the_file_lock_even_without_store_presence` (break-checked). |
| M2: a non-symlink reparse point made our own partial foreign | Fixed in `7c6bdb4f`: only name surrogates are foreign; non-surrogate tags are read through a normal handle with the identity rechecked; removal outcomes are typed and every file left in place is reported. Tests: `only_name_surrogate_reparse_points_make_a_file_foreign` (break-checked), `removal_reports_every_name_it_leaves_behind`, the lifecycle notice test. The suggested `compact /exe` reproduction does not produce a visible reparse point on this Windows 11 build (fsutil agrees), so those two tests skip here; see the ledger. |
| M3: the "no path-based race" invariant did not hold | Fixed in `18b13ae7`/`0692b80c`: on Windows the staging file is pinned by a delete-denying handle across the hard link and the final-name check; `symlink_metadata` replaces `fs::metadata` in the read-only retry; the remaining path-based steps are named in the module doc, here and in THREAT-MODEL, which no longer claims handle-only deletion. Tests: `a_pinned_leaf_cannot_be_swapped_while_our_own_hard_link_still_works`, `publication_pins_the_proven_staging_file_across_the_link` (break-checked). |
| L1: straddling chunk rewritten without comparison | Fixed in `cc5d6e00`. Test: `a_straddling_chunk_inside_the_checkpoint_must_match_before_any_rewrite`. With checkpoint 0, identity alone is the proof (documented; our own torn tail must stay rewritable). |
| L2: Remove reported success while files remained | Fixed in `7c6bdb4f` (typed outcomes and notices). Tests: `removal_reports_every_name_it_leaves_behind`, `destination_changes_while_paused_become_typed_halts_without_touching_other_files`. |
| L3: untracked second plaintext name after publication | Fixed in `e93191e4`: `finish()` reports whether the staging name is settled and the Published record is kept otherwise. Test: `a_staging_name_that_cannot_be_removed_keeps_the_saved_record`. |
| L4: checkpoint advanced after a CID mismatch | Fixed in `14ddba4b`: a session that failed verification never checkpoints and `stop()` checkpoints only after transient local failures. Tests: `a_stream_that_fails_verification_never_reaches_a_publishable_checkpoint`, `a_stream_that_fails_verification_stops_without_a_checkpoint`. |
| L5: garbled messages | Fixed in `14ddba4b`: no doubled prefix; storage refusals map to user messages. Test: `storage_refusals_become_user_messages_without_doubled_prefixes`. |
| L6: admission on a full vault faulted the store | Fixed in `f7201068` for the transfer directory. Test: `admission_on_a_full_vault_fails_cleanly_without_faulting_the_store`. The format marker and blob-store open still fault (documented). |
| L7: `cancel_download` cleared presence too early | Fixed in `f7201068`. Test: `a_failed_removal_write_keeps_the_live_session_exclusive`. |
| L8: an occupied final name that cannot be opened halted forever | Fixed in `cc5d6e00`: numbered around. Test: `an_occupied_final_name_that_cannot_be_opened_is_numbered_around` (Windows). |
| L9: Remove cleanup took no slot | Fixed in `3dfe1538`: four dedicated cleanup slots, the permit moves into the worker. |
| L10: chat save of a halted journal | Fixed in `f7201068`: refused in `begin_owned` before any re-verification. Test: `a_chat_save_of_a_halted_journal_is_refused_before_any_reverification`. |
| L11: Unix leaf open could hang on a FIFO | Fixed in `e46503ac` with U-M6 (`O_NOFOLLOW | O_NONBLOCK`, ELOOP/ENXIO are Foreign). Unix-only test; compiles and passes Clippy for `x86_64-linux-android`, cannot run here. |
| L12: doc drift | Fixed in this document and THREAT-MODEL: V2 without a journal stays unreadable to the old reader; crash row 1 names the probe orphan; the misnamed test is now `identity_collision_over_foreign_content_halts_without_writing` and documents that a real collision would let Remove delete; the "only through a handle" wording is gone. |
| L13: nits | `Leaf` Debug no longer prints a path (fixed). FAT's link error kind: the creation probe now refuses any link failure, so the publication-time Unsupported branch is defensive only (documented). Publication retries now back off (U-L1). Not changed: an `ExportFile` dropped on an async thread in an error path (closing a handle on a hung share can stall that task; follow-up), and "0 of N chunks saved" showing checkpoint granularity (copy follow-up). |
| Also: native end-to-end coverage of `export_journaled` | Added in `e93191e4`: the sequence runs over `ExportPorts`; seven tests in `private_downloads/export_tests.rs` (plus `7ab98819`'s worker-start test). They found a real defect (link marked visible before a definite "name taken" refusal), fixed in the same commit. |
| Also: root cause of the one-off freeze/thaw failure | Not reproduced in six more module runs (and about 25 earlier reruns). The test wraps a full sealed vault snapshot save (`save_and_freeze`, with fsync) in a 5 s wall-clock `bounded`; the one failure came on the first run after a rebuild, with parallel tests, a cold binary and fresh files to scan. That timeout is the most plausible cause, but it is unconfirmed and the test is unchanged. |

## User review of `27ffa183` (U-): dispositions

| Finding | Disposition |
| --- | --- |
| U-H1: Keep partly saved file could report success just before an older Remove deleted the file | Fixed in `3dfe1538`: process-wide cleanup claim per (server, Download ID) held by the deleting worker; Keep reassesses under it; Save new copy retires every observer under the intent CAS before the delete and keeps the journal until it is settled. Tests (real sequences, barrier inside the deleting worker): `keep_partial_cannot_promise_a_file_while_an_older_remove_may_still_delete_it`, `a_dropped_remove_still_owns_its_delete_until_the_worker_ends` (the Lock/unlock case), `save_new_copy_retires_the_old_journal_before_deleting_and_excludes_keep`, `a_stale_save_new_copy_deletes_nothing`; storage `save_new_copy_retires_every_observer_before_the_delete_and_keeps_the_journal`. Each guard break-checked. |
| U-M1: a publication retry truncated a file visible under its final name | Fixed in `cc5d6e00`: after a link attempt, or with any other name, nothing is trimmed or written; the final name is verified read-only. Tests: `a_retry_after_an_unconfirmed_link_never_trims_or_claims_a_changed_final_file` (append and same-length cases), `a_partial_that_gained_another_name_is_never_written_again`. |
| U-M2: a matching final identity skipped content verification | Fixed in `cc5d6e00` (`SavedCandidate::verify`) and `14ddba4b` (the preflight verifies against the saved manifest's plan). Test: `a_matching_final_identity_is_not_saved_until_its_content_is_verified` (same length, shortened, rename); the genuine crash-after-link tests still pass. |
| U-M3: preflight edges skipped actor-generation retirement | Fixed in `14ddba4b`: `DownloadPreflight` observation (actor lifetime, Root, mount, transfer epoch) required by preflight edges and recovery; in-session edges are lease-fenced. Tests: `preflight_edges_from_before_a_freeze_and_thaw_record_nothing` (actor), `preflight_edges_need_a_current_observation_of_this_mount_and_transfer` (mount and transfer epoch). The Root-generation variant is not tested directly: changing Root in an actor test needs an epoch change; the same predicate `DownloadReady` uses checks it. |
| U-M4: recovered success could return a path after Lock or retirement | Fixed in `14ddba4b`: `finish_recovered_save` runs the guarded finalization after cleanup. Test: `a_recovered_save_returns_its_path_only_if_still_authorized_after_cleanup` (Lock, entry retirement, actor change, live cleanup failure with warning). |
| U-M5: empty files could not be saved | Fixed in `e1622234`. Tests: `an_empty_file_saves_as_one_empty_chunk_across_restart`, `only_the_canonical_empty_manifest_may_carry_an_empty_chunk`, actor `an_empty_file_is_acquired_and_saved_as_one_empty_chunk`. |
| U-M6: the FIFO fix must cover directory opens | Fixed in `e46503ac` (folder identity and directory flush use `O_DIRECTORY | O_NONBLOCK`). Tests (Unix-only, bounded by a thread with a timeout so a regression fails instead of hanging): `a_fifo_at_a_leaf_or_folder_path_never_blocks_an_open`, `a_fifo_in_place_of_the_saved_folder_fails_resume_and_remove_promptly`. Compile- and Clippy-checked for Android; cannot run here. |
| U-L1: publication retries exceeded the session budget | Fixed in `e93191e4`. Test: `publication_retries_are_bounded_and_the_stop_checkpoints_what_is_durable`. |
| U-L2: clearing a staging link removed the final file's read-only attribute | Fixed in `18b13ae7`. Test: `clearing_a_saved_row_never_changes_the_saved_files_attributes`. On Rust 1.89 `remove_file` deletes a read-only name without touching attributes, so the guard is exercised with an injected refusal. |
| U-L3: the staging suffix broke names the old writer could save | Fixed in `aa6ae216`. Tests: `long_multibyte_names_are_shortened_to_leave_room_for_every_suffix`, `a_long_multibyte_name_saves_with_its_extension_and_numbering_room`, and the chosen-name refusal. |
| Design: ReFS identity | Documented, not detected: the safe wrapper exposes no volume information, so detecting ReFS would need `unsafe`. Docs no longer present the file index as a universal guarantee. |
| Design: queued but unstarted workers | Fixed in `7ab98819`: liveness rechecked when the worker starts; a started step completes (contract documented under Fences). Test: `a_step_queued_before_retirement_writes_nothing_when_its_worker_starts`. |
| Design: compatibility evidence | Done: real fixtures opened by the `dbb8d36b` reader; see Record format and the ledger. |
| Design: disk-full coverage | Done in `cbdfee43`: each exempted step, both failed-cleanup paths, and a Keep record write. Test: `every_exempted_pre_rename_step_refuses_cleanly_and_a_failed_cleanup_still_fails_closed`, `a_keep_record_write_on_a_full_vault_refuses_cleanly`. |
| Design: native sequence across awaits | Done: `ExportPorts` and `CleanupPorts` run the real sequences in tests (export, cleanup, chooser), plus the recovered-save response test. Still not covered: `save_group_file` itself end to end (it needs a real AppHandle, actor and dialog). |

## Picker implementation review dispositions (Save as, `27ffa183..50d15dab`)

Read-only review of the implemented picker and both review fix sets: no blocker or high, two
medium, five low, plus coverage gaps and residual risks. Every guard below was confirmed by
breaking it and watching its test fail.

| Finding | Disposition |
| --- | --- |
| M1: a Save as of a recognized media file still said "Saved X to Downloads" | Fixed in `fa51f5b1`: a "matched" verdict takes the folder-free wording when the save may not be in Downloads. Test: `a save that may not be in Downloads names no folder` (matched, with and without a warning and reveal). |
| M2: the fallback writer at a chosen place did blocking file I/O on the async runtime | Fixed in `19e1f486`: staging, every chunk write, the sync, failure cleanup and publication run through the shared blocking-pool helper; liveness checks stay on the runtime between steps; the session guard is still held across the blocking link. Test: `the_fallback_writer_stages_off_the_async_runtime` (single-threaded runtime, staging parked on a barrier only another runtime task can release). Docs corrected (Design overview, Unsupported locations, HANDOVER, INTERFACES). |
| L1: a failed cleanup wiped earlier warnings | Fixed in `88869f39`: the cleanup notice is appended. Test: `a_failed_cleanup_keeps_the_warnings_the_save_already_carried`. |
| L2: a chat Save as on a file with a journal interrupted that save before being refused; a new record could be left after a failed plan write | Fixed in `325220b9`: `start_saved` refuses a choice over a journal before `resume_download`, with storage's text; a record created in that step is released when its plan write fails, an existing one is paused. Tests: `a_save_as_choice_never_interrupts_a_live_journaled_save` (actor, through the real command), `a_new_download_whose_choice_cannot_be_written_leaves_no_row`. The failing plan write itself cannot be injected from the app crate (storage keeps fault injection private), so the second test pins the undo the error branch calls. |
| L3: an orphan from a crash before Begin blocked the remembered choice forever | Documented and the message fixed in `5588a256`: the orphan is still never adopted (nothing proves it is ours); `PartialExists` now names an earlier interrupted save as a likely source and says to delete the file from the folder or choose another name (wording revised for re-review LOW 4 in `4a71bf25`); crash rows 1 and 12 and Honest limits say so. Test: `an_orphan_partial_from_a_crash_before_begin_is_refused_and_left_alone`. |
| L4: a folder swap between the identity check and creation was not handled or documented | Fixed in `5588a256`: every creation re-reads the folder identity after creating, removes its own partial only while handle identity proves the name is still ours, and refuses; `adopt_created`'s cleanup uses the same proven removal. Creation is added to the path-based residual list here, in `external.rs` and in THREAT-MODEL. Test (new `BEFORE_CREATE` hook): `a_folder_swapped_between_the_identity_check_and_creation_is_refused` (chosen journaled, chosen non-resumable, Downloads; each call site break-checked separately). |
| L5: a peer name such as "CON .txt" failed as "malformed data" | Fixed in `88869f39`: the sanitizer trims trailing spaces before its device check, matching storage. Test: `a_device_name_with_trailing_spaces_is_prefixed_like_storage_requires`, which also runs each sanitized name through storage's leaf rule. Design question (Unix Save as names such as `aux.c` are refused): kept, so a journal never names a Windows device if the vault moves. |
| L6: Save as failed without a resolvable Downloads folder | Fixed in `3a93448d`: the dialog then starts at the OS default. Test: the chooser seam with no starting folder. The `save_group_file` call site itself needs an AppHandle and is not unit-tested; the `Option` in the seam's signature makes the earlier `?` a type error. |
| L7: doc mismatches | Fixed in this document (only the ready row's label changes) and in HANDOVER and INTERFACES (the boundedness wording now covers the fallback writer). |
| Coverage: `export_sequence` with a chosen destination | Added in `29836f10`: `a_save_as_choice_saves_at_the_exact_chosen_name_through_the_same_sequence`, `a_refused_begin_at_a_chosen_place_discards_the_new_partial`, `a_chosen_place_never_redirects_an_existing_journal` (the last two break-checked). `Unsupported` leading to `Ok(None)` needs an identity-less folder, which only storage's private injection can model; storage covers `create_chosen` returning it. |
| Coverage: the inline destination selection | Moved into `select_destination` in `29836f10`. Test: `the_record_not_the_invocation_selects_where_a_fresh_save_goes` (every arm, `PLACE_CHANGED` break-checked). Still untested: `prepare_resume`'s chosen-plus-journal refusal through a real AppHandle, `stream_download_to_chosen` end to end, and the post-dialog recheck. |
| Residual: Windows device names with superscript digits, `COM0`/`LPT0`, `CONIN$` | Fixed in `0db5498f` in storage's leaf rule and native's sanitizer together. Tests: the selection-checks table and the sanitizer test above. |
| Residual: `%` in a suggested name | Fixed in `0db5498f`: the Windows dialog expands `%VAR%` in its name box, so the suggestion replaces `%`. Test: the chooser seam. |
| Residual: `rfd` logging | Checked: `rfd` 0.16 logs only in its XDG portal and macOS message-box backends, neither compiled with `common-controls-v6` plus `gtk3`. |
| Residual: no real dialog has run | Open; see the ledger. |
| Residual: the one-dialog turn ends when the invocation future drops while an OS dialog stays open | Accepted: Tauri does not drop a running command future except at process exit. |
| Residual: a dialog that fails to open looks like Cancel | Partly: a refused `run_on_main_thread` already returns "The Save as window could not be opened."; only a dialog task that ends without a result is treated as Cancel. Follow-up if it is seen in practice. |
| Residual: any hard-link probe failure is reported as "FAT or exFAT" | Accepted wording residual ("for example"); copy follow-up. |
| Residual: a remembered choice silently redirects the chat "Download" item | Documented; label follow-up. |
| Residual: remembered choices without folder identity have no swap check | Documented (unchanged). |
| Follow-on to M2 (found while fixing it) | With file work on the pool, a staging or write step could pass the async session check and start after Lock. Fixed in `d747c0d2`: `SaveSource::liveness` is evaluated when each queued staging or write worker starts, as on the journaled path. Test: `a_fallback_step_whose_worker_starts_after_retirement_does_nothing` (both checks break-checked). |

### Re-review of the fixes (`50d15dab..0db5498f`)

Read-only re-review: no blocker or high, one medium, four low. All fixed and break-checked except
where stated.

| Finding | Disposition |
| --- | --- |
| MEDIUM 1: the media check after publication read the destination on a tokio worker while holding the process-wide commit guard | Fixed in `d747c0d2`: `inspect_published` runs it on the blocking pool, before `read_commit`; a retired invocation is refused before it starts; the result is returned only after the guarded recheck. Test: `the_saved_file_check_runs_off_the_runtime_and_outside_the_commit_guard` (single-threaded runtime; the releasing task must also take the guard; both the order and the pool break-checked). The existing retirement-edges test still passes. |
| LOW 1a: a failed reveal said "Downloads could not be opened" for any save | Fixed in `d747c0d2`: "its folder could not be opened". |
| LOW 1b: a one-click save that resumed a record with a remembered choice was announced as "to Downloads" | Fixed in `e1b0cc39` (a `chosenPlace` flag), extended in `9d975719` to resumed journals, then replaced in `680184d1` after the re-check found it failed open for recovered journals: `SavedFileResult.inDownloads` (omitted when false) is set only when the saved file sits directly in the resolved Downloads folder, on every return path including the recovered-save one, and the notice requires it. Tests: `only_an_asserted_downloads_save_carries_the_in_downloads_flag`, `a_save_counts_as_in_downloads_only_directly_inside_the_downloads_folder`, `a notice names Downloads only when native asserts the file is there` (each break-checked); the visual fixture mirrors the field. |
| LOW 2: `create_unjournaled` probed links without identity, so a swap and swap-back plus a planted file defeated the residual claim | Fixed in `4a71bf25`: the probe uses the new partial's handle identity when available and cleanup uses the proven removal. Test (new `AFTER_CREATE` hook, the swap through a directory link because Windows will not rename a folder holding our open file): `a_swap_and_swap_back_around_creation_with_a_planted_file_writes_nothing` (break-checked). |
| LOW 3: `remove_created`'s proof branch was untested | Test added in `4a71bf25`: `removing_a_just_created_partial_needs_its_identity_to_still_match` (break-checked). |
| LOW 4: "Remove it" in `PartialExists` read as the Transfers action | Fixed in `4a71bf25`: "Delete it from the folder, or choose another name." The orphan test asserts the wording. |
| Design question: a stale-list one-click start can still resume a record whose journaled save is live, interrupting it | Not changed: this predates the picker (the renderer routes rows by status, so it needs a stale list) and refusing it changes the one-click contract. Follow-up: refuse a non-resume start when the found record shows an active Writing export. |
| Residual: a hung guarded link stalls every command that takes the process-wide guard, not only Lock | Documented here and in THREAT-MODEL. |
| Residual: a just-created partial whose name cannot be opened at cleanup time stays | Documented under Platform; it is empty and the refusal tells the user. |
| Residual: a release that fails after Deleting is durable (inside the L2 undo) needs a storage check | Same failure mode as `cancel_download`; unchanged. |
| Test gap: only staging and the media check are pinned off the runtime; writes, sync, cleanup and `ActorSaveSource::publish_verified` are not | Open: std's file writes and the production source cannot be parked from a test. The code runs each through the same helper. |
| Doc drift: INTERFACES said every creation re-reads the folder identity | Fixed: "every creation with a captured folder identity". |

Short re-check of those fixes (`0db5498f..e1b0cc39`): no blocker, high or medium; MEDIUM 1, LOW
1a, 2, 3 and 4 confirmed fixed. Two lows:

| Finding | Disposition |
| --- | --- |
| LOW A: `chosenPlace` failed open for resumed and recovered journals at a place chosen earlier | Fixed in `9d975719` and `680184d1` (the fail-closed `inDownloads`; see LOW 1b above). |
| LOW B: the unjournaled reservation now refuses when the probe name is not provably ours, with a "could not be removed" message | Accepted as the safe direction (it only arises on unusual filesystems or a transient hold); the message now says "could not be confirmed or removed" (`680184d1`). No retry added. |

## Re-review 2 (`27ffa183..fe18048b`): dispositions

Read-only re-review of the whole range after the merge of `chat-update-implementation`: no
blocker. Its HIGH (Windows-only test fields dead on Linux under strict Clippy) was resolved by
`5e04f5c0`. Every guard below was broken on purpose and seen to fail.

| Finding | Disposition |
| --- | --- |
| MEDIUM: "Keep partly saved file" was no longer an escape from a stuck removal (Busy when four stuck Removes held every cleanup slot; an unbounded presence check held the claim, record and quota while a dead share hung) | Fixed in `81541158`: with no free slot the check answers Unknown without touching the disk; a failed check is Unknown; Keep stops waiting after `KEEP_PRESENCE_WAIT` on the sanctioned clock (a new `CleanupPorts::wait`). Unknown promises nothing and the claim alone excludes a concurrent delete. Tests: `keep_partial_still_releases_when_its_presence_check_hangs`, `keep_presence_is_unknown_without_touching_the_disk_when_every_slot_is_busy`. |
| LOW 1: an uncertain link error became a permanent, false "changed" | Fixed in `9fe145f5`: with one name on our file and a final name that is not ours, the link did not happen; the session is private again and publication retries within the budget. Test: `an_uncertain_link_error_that_made_no_link_retries_instead_of_halting_changed` (red before the fix). |
| LOW 2: Resume was not excluded by the cleanup claim | Fixed in `7857aaeb`: plain Resume held the claim across its preflight and intent CAS and was Busy while a cleanup owned the Download. Re-review 3 found the preflight part harmful and narrowed it to the intent CAS (`44c72957`, below); that test became `resume_takes_the_cleanup_claim_only_around_its_intent_change`. |
| LOW 3: a remembered Save as choice had no preflight | Fixed in `2d9290cf`: `ChosenDestination::preflight` (read-only folder identity and free names), carried by `DownloadPreflight::planned` and checked by the Resume preflight unless the invocation brings its own new choice. Tests: `a_remembered_choice_is_preflighted_without_creating_anything` (storage), `the_resume_preflight_carries_the_remembered_choice_until_begin` (actor), `a_remembered_choice_that_cannot_be_used_is_refused_before_reverification` (native). A plain chat click with a stale list (no row, so no Resume preflight) still learns at creation. |
| LOW 4: the Downloads fallback writer had no link probe | Fixed in `7caeedec`: the shared `confirm_staging_links` (identity-checked probe, proven removal) now also guards native's one-click `.part`. Test: `a_downloads_reservation_without_links_is_refused_before_any_plaintext` (storage; the native call site has no failure injection, so only its success path runs natively). |
| LOW 5: the Save as success toast named the listed file | Fixed in `2abd8649`: the notice names the final component of the saved path. Test: `a notice names the file that was written, not the listed name`. |
| LOW 6: vaults written at `27ffa183` now fault | Agreed, no code change: no dogfood or installed build of `27ffa183` exists. Stated under Remembered choice above and in HANDOVER. |
| LOW 7a: export handles were dropped on the runtime in about six error paths | Fixed in `7b4f29ef` rather than documented: `OnPool` holds the session on the async side and closes it on the blocking pool on every early return; the fallback writer closes its handle with the staging removal. Test: `a_session_dropped_on_the_async_side_closes_on_the_blocking_pool`. The INTERFACES claim is now accurate. |
| LOW 7b: the docs said a choice authorizes "that save and its resumes only" | Wording corrected here, in THREAT-MODEL and INTERFACES: the choice is the destination of every save of that Download until Begin consumes it, a newer Save as replaces it or the record is released. Inheritance by a plain click was kept on purpose (it is the interrupted-save path); the `inDownloads` flag keeps the notice truthful. |
| Residual: a GTK dialog future that never completes kept the one-dialog flag set until restart | The turn already was an RAII guard, but Tauri does not drop running commands. Fixed in `e1ad2103`: the dialog wait also ends when the invocation's origin is cancelled (Lock, unlock, a server replacement), releasing the turn; the orphaned window's answer is discarded. Test: the chooser seam's never-answering dialog (bounded, break-checked). Without a Lock or replacement a hung dialog still holds the turn for that session; documented. |
| PR #51 Windows root failure (`accepted_crossing_range_chunk_renews_quiet_deadline_before_fetching_the_next_chunk`) | Not worked on, as asked. That test drives `range_cancellable`. In the actor, Re-review 2 changed only `download_preflight` (an extra read of the planned destination); `start_saved` changed earlier (picker L2). Neither is on that test's path, and nothing here changes range chunks, quiet deadlines or chunk stepping. |

## Re-review 3 (`cb1bc3e0..867ae8f5`): dispositions

Short read-only re-review of the Re-review 2 fixes: no blocker or high. Confirmed closed: H1, the
Keep MEDIUM, LOW 1-5, LOW 7a, `a8f959c8` and the docs commit, with the caveats fixed below. Each
guard was broken on purpose and seen to fail.

| Finding | Disposition |
| --- | --- |
| MEDIUM: plain Resume held the cleanup claim across destination reads that never time out, so a Resume hung on a dead share made Remove, Save new copy and Keep report "still being removed" | Fixed in `44c72957` by narrowing, not by bounding: the preflight runs without the claim and `plain_resume_commit` takes it only around the intent CAS. Narrowing was chosen because bounding the preflight would still hold the claim for the bound on every Resume and needs a timeout on a full read of up to 1 GiB, while the preflight's only effects are already fenced: its halt and recovery edges need its observation, which Remove's Deleting step and Save new copy's retire both invalidate. Tests: `resume_takes_the_cleanup_claim_only_around_its_intent_change` (native: Busy only at the intent change, and the preflight is reached with the claim held by a cleanup; both guards break-checked), `a_preflight_from_before_a_cleanup_cannot_act_on_its_journal` (actor, Remove and Save new copy; break-checked by removing the retire's epoch change). The native test's contract changed on purpose: the preflight is no longer excluded, the intent change is. |
| LOW 1: the uncertain-link reset could treat a visible file as private when the final name existed but could not be opened | Fixed in `1590bd56`: the reset needs the final name absent or opening as another identity, never an open error. Test (new `LinkReport` effect: the link succeeds and reports an error): `an_uncertain_link_error_that_did_link_never_trims_a_final_file_it_cannot_open` (red before the fix). |
| LOW 2: the plan preflight also blocked network-phase resumes | Fixed in `f9537e97`: `DownloadPreflight::planned()` reports a choice only for a Completed record. Test: `an_incomplete_download_resumes_although_its_chosen_place_is_gone` (break-checked). |
| LOW 3: a Save as dialog could open for an already-cancelled invocation | Fixed in `6c75dfaa`: the cancellation branch is polled before the chooser. Test: the chooser seam with a pre-cancelled invocation asks for no dialog (red before the fix); the hung-dialog case now cancels only after the dialog opened. |
| Nit: docs said Keep "always releases" | Qualified: Keep never waits on the destination, but its two actor steps wait on the session commit guard like every guarded command. |
| Nit: the busy-slot Keep test depended on the process-wide pool | Fixed in `01cea85b`: `presence_in` takes a pool and the test owns one (break-checked). |

## Final review (`867ae8f5..724a7228`): dispositions

Short read-only review: no blocker, high or medium; all six Re-review 3 commits close their
findings. Two lows:

| Finding | Disposition |
| --- | --- |
| L1: a recovered save's cleanup could release a record a later Save new copy was re-saving (dating from `14ddba4b`) | Fixed in `cd0d2aa9`: `recovered_cleanup` stands down without a warning when another cleanup holds the claim or the recovering preflight's observation is no longer current; under its own claim it clears the record as before. Tests: `a_recovered_save_clears_only_the_record_it_recovered` (native, over the cleanup ports: the moved-on record and a held claim both leave the record and run untouched with no error, the current record is cleared; both guards break-checked), `a_recovered_publication_keeps_its_observation_until_save_new_copy_retires_it` (actor: recording the recovery keeps the observation current, the retire ends it). The literal end-to-end sequence (recover, Save new copy acquiring, then the late cleanup) is not one test; it is covered by those two halves. |
| L2: the rewritten native test did not pin that Resume takes the claim | Fixed in `8de5641a`: `a_real_resume_commits_its_intent_change_only_without_a_cleanup_claim` builds a real local Completed record whose preflight passes; with a cleanup holding the claim `prepare_resume` is Busy and the intent unchanged, without it the intent advances and the claim is released. Bypassing `plain_resume_commit` makes it fail. |

## PR51 review (user, at `86664d94`): design note

The user's review requests changes: H1 (blocker), M1, M2, M3. Policy: no `unsafe` in our code;
operating-system calls come from vetted crates with safe APIs; where none fits, the guarantee is
narrowed honestly instead.

### Crate evaluation

| Need | Candidate | Verdict |
| --- | --- | --- |
| Windows: delete exactly the object whose identity was verified, through that handle | `fs_at` 0.2.1 (Robert Collins, the `remove_dir_all`/rustup maintainer; Apache-2.0; deps `windows-sys` 0.52 (already in the root lock), `aligned` 0.4.3 (with `as-slice` 0.2.1 and `stable_deref_trait`) and `cvt` 0.1.2, all permissive; its Unix-only dependency `nix` 0.29 (MIT) enters both lock files but is never compiled, because `fs_at` is a Windows-only dependency for us, so Linux CI gains nothing) | **Chosen.** `fs_at::os::windows::FileExt::delete_by_handle(File)` is a safe trait method: `SetFileInformationByHandle(FileDispositionInfoEx)` with POSIX semantics and IGNORE_READONLY on the handle we pass, falling back to `FileDispositionInfo` and, for read-only files, clearing and restoring the attribute through that same handle. No name is re-resolved. |
| Windows: the same, std-only | `FILE_FLAG_DELETE_ON_CLOSE` via `OpenOptionsExt` | Rejected: the disposition is set at open, before the identity can be checked, and cannot be cancelled without `SetFileInformationByHandle`, so a foreign object opened by mistake would be deleted. |
| Windows: 128-bit file id / filesystem name / remote volume | `file-id` 0.2.3 (path-only API that opens the path itself; useless for a handle we hold), `winapi-util` (`GetFileType` only), `sysinfo` (mount-point prefix matching, rejected as a path guess), `windows`/`windows-sys` (functions are `unsafe` to call) | **None fits: narrowed** (below). |
| Unix: directory-relative rename, stat, unlink, link, statfs, euid | `rustix` 1.1.4 (Bytecode Alliance; Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT; already in both lock files through `tempfile`) | **Chosen**, as a direct `cfg(unix)` dependency with the `fs` and `process` features. `renameat_with(NOREPLACE)` exists on Linux and Apple. |

Empirical checks on this host (Windows 11, NTFS): while a file is held open with `DELETE` access
and share mode READ|WRITE (no DELETE), renaming its name fails (sharing violation), renaming its
parent or grandparent folder fails (access denied), and `delete_by_handle` removes it even when it
is read-only.

### Contracts (as implemented; the design review's recommendations are noted where taken)

Which path each finding took: **H1** crate on both platforms (`fs_at` on Windows, `rustix` on
Unix), with the Unix guarantee narrowed to other accounts because Unix has no delete-by-inode.
**M1** crate on Linux and Android (`rustix`); Windows needs no new crate (std share modes); macOS is
narrowed (no link by descriptor exists there). **M2** needs no new dependency. **M3** crate on Unix
(`rustix::fs::fstatfs`); narrowed on Windows (no vetted crate reports the filesystem name).

**H1, every automatic delete of an external name.** Covered: Remove, Save new copy, Clear of a
saved row's staging name, the staging name after a journaled publication, failed-creation
cleanup, the link-probe name (`-probeN`), the fallback writer's staging file on failure and after
publication, the GIF export's staging file, and a small download (`write_download`) whose write
failed. All go through `external::remove_verified`:

- Windows: open the name with `DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE`, share READ|WRITE (no
  `FILE_SHARE_DELETE`), `FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS`; verify the
  handle's identity (or in-session number); delete through that handle with
  `fs_at::os::windows::FileExt::delete_by_handle` (POSIX semantics with IGNORE_READONLY where
  available, else classic disposition). While the handle is held the name cannot be renamed,
  replaced or deleted and its folders cannot be renamed, so the deleted object is exactly the one
  verified. The handle asks for no `FILE_WRITE_ATTRIBUTES`, so the classic fallback for a read-only
  file (FAT, SMB) fails instead of clearing the attribute: no attribute is ever changed, through a
  path or a handle. Long-lived handles keep std's default share mode, so this short-lived open is
  allowed (design review H-A).
- Unix (no delete-by-inode exists): pin the folder by a descriptor and check it against the journal
  when one exists. If other accounts can modify it (owner neither us nor root, or group- or
  other-writable without the sticky bit), or may (adversarial review M-3: a network, FUSE or
  unrecognized filesystem, where mode bits do not say who can change the folder; ACLs are not
  read), delete nothing and say so, but only after a read-only check that the name still holds our
  file (combined review M-A; a missing name is `Missing`, a foreign one `NotOurs`, as elsewhere):
  `SharedFolder`, "This folder may be changed by other accounts, so Mewtual deletes nothing there
  automatically and left the partly saved file in place. Delete it yourself if you no longer need
  it." (for a saved row: "... left the temporary .mewtual-part copy of the saved file in place.
  Delete that copy yourself if you no longer need it.").
  Otherwise verify the object where it is (review M-2: a foreign object is never renamed), then
  `renameat` the name to `mewtual-removing-<128 random bits>` (no-replace where the platform has
  it; the random name comes from the `catcoms-rt` RNG seam), verify again there, and `unlinkat` it
  if it is still ours; else put it back without replacing anything (no-replace rename, or a hard
  link back plus unlink of the aside name where no-replace rename is missing) and report "not
  ours". If it cannot be put back, the outcome is an error (the save stays tracked as
  `Unchecked`), never "left in place". A crash between the rename and the unlink leaves our file
  at a removing name; for a journaled save Keep and the next Remove find it by identity, never by
  name (Keep then releases the record and leaves the file at that name; review L-4); for a writer
  without a journal nothing records it, and the user would have to delete it. Kept despite the
  design review's advice to drop the rename (it is what makes the swapped-in file survive on Unix).
- Identity-less destinations (design review H-D, taken): a destination without birth times gets no
  journal (the save falls back), and the fallback writer and GIF export verify every removal
  against the in-session number (`SessionId`: volume and file index, or device and inode) of the
  handle they created the file with and still hold, so that number cannot be reused meanwhile (only
  after a worker panic is the number checked without the handle; review L-5). A destination that
  reports no file number at all is refused before any plaintext is written (the Windows link probe
  and the fallback writer's staging check), because nothing there could ever be proven ours.
- Messages (review M-4, M-5): native says "delete it yourself" only after a read-only look proves
  the leftover name still holds Mewtual's own file ("This folder may be changed by other accounts,
  so the partly saved file "<name>" was left in place; delete it yourself.", "The partly saved
  file "<name>" could not be removed; delete it yourself.", and the same for "the temporary copy
  "<name>" of the saved file"); nothing when the name is gone or holds another file; "Mewtual could
  not check "<name>", so it was left in place." when it cannot look. A successful save reports a
  temporary copy that had to stay with its result: the journaled writer keeps the record (and says
  Clear can remove it) only when the name could not be checked or deleted right now; in a shared
  folder it releases the record with the saved-row `SharedFolder` notice above. "May be changed"
  (combined review LOW 1) covers network and FUSE folders, where Mewtual cannot tell who can.
- Guarantee wording, changed: "the deleted object is exactly the one verified" holds on Windows
  against every process. On Unix: "Automatic deletion is guaranteed only against processes running
  as other accounts; in folders other accounts can modify, and on network or FUSE filesystems,
  Mewtual deletes nothing automatically. A process running as the same account could swap the
  random name in the instant before the unlink; such a process can already delete the file
  itself."
- User-private groups (review M-6; user decision, 23 October, see "User decisions" below): a
  group-writable folder without the sticky bit counts as private only when it is owned by our
  effective uid, its group is our effective primary gid, and the local account files prove that
  group has no other member; anything unprovable stays shared. Folders Mewtual creates itself (a
  missing Downloads) get 0755 explicitly, and the test fixtures pin their modes.

**M1, publication link.** Two guarantees: (a) never report a save that did not happen; (b) never
give the final name to an object other than the one verified. Both writers now publish exactly the
object behind the handle they hold (`ExportFile::link`; `publish_unjournaled` for the fallback
writer and the GIF export):

- Linux and Android: (a) and (b). `linkat` of `/proc/self/fd/N` with `AT_SYMLINK_FOLLOW` into the
  pinned folder. Without a usable `/proc` no journal is started and the fallback writer refuses
  before any plaintext is written (fail closed, "this system does not provide /proc, which Mewtual
  needs to publish a save safely"). `/proc` disappearing between creation and a later Resume makes
  the link fail definitively (`ENOENT`/`EXDEV` become `Unsupported`), which halts the journal as
  unavailable without treating the save as possibly published (review L-3). Since the Linux CI
  follow-up, `EPERM`, `EMLINK` and `EOPNOTSUPP` (`ENOTSUP`) are definite refusals too
  (`external::linkat_error`): before, `EPERM` halted with the save marked as possibly linked, and
  `EMLINK` and `EOPNOTSUPP` were retryable failures, so Resume retried a link that could not
  succeed. The folder flush is not done inside the guarded link (review L-1).
- Windows: (a) and (b). A delete-denying pin on the staging name, checked to be our object, is held
  across the path-based link (the existing test also pins that the folder cannot be renamed).
- macOS: (a) only (narrowed). The link is by path; the post-link check fails the save when the
  final name holds another object, and both names are left alone. A post-link check that cannot be
  made fails the save on macOS, but not on Linux, Android or Windows, where the link itself already
  named our object.

**M2, durability.** The destination folder is flushed after the link and before Published is
recorded, for a fresh link, for the export sequence's reopen that finds an interrupted session's
link, and for the Resume preflight's recovery (all three record Published or RecoverPublished). The
flush runs outside the commit guard (design review), and Published takes the guard again; a Lock
that wins that gap leaves the every-chunk journal with our final name linked (crash-table row 6),
which the next Resume recovers without a second copy. A Remove landing in that gap sees the same
state and reports the saved copy as kept, while the save command reports its error (no test pins
this interleaving; review L-7). Writers without a journal flush the folder when they settle their
staging name, after the guard. A Published record whose final name is not proven ours is never
re-published automatically (the design review's resurrection hazard: the user may have deleted the
saved file on purpose), and its Clear deletes nothing (review M-1): the staging name, when it is
the only copy, is kept (`KeptOnlyCopy`, "The saved file was no longer in its folder, so its copy
under the temporary .mewtual-part name was kept. Rename that file to keep it."); when another file
now has the saved name (an editor's atomic save), the original copy is kept and the notice says
exactly that (`SavedNameReplaced`); both names gone is `SavedFileMissing` ("The saved file was no
longer in its folder."); a final or staging name that cannot be checked right now (an indexer
holding it) is `Unchecked` and the row stays until a retry; a staging name already gone with
someone else's file at the saved name is simply `Removed`. Since the user's decision of 23 October
the row no longer reads "Saved" until it is cleared: Transfers checks the saved file and says when
it is missing or replaced, offering Save again (see "User decisions" below).

**M3, filesystem support (fail closed).** See the support matrix below. Unix decides with
`fstatfs` (Apple: `f_fstypename` plus `MNT_LOCAL`). Windows has no vetted crate that reports the
filesystem name or the remote flag, so: FAT and exFAT are refused by the link probe; network
folders are recognized without guessing by canonicalizing the folder with std (a `\\?\UNC\`
result, which mapped drives also produce) and get no journal; every other local volume is
journaled on the 64-bit file index plus birth time, which is verified on NTFS only. The 64-bit file
index is not presented as universally unique.

### Support matrix

| Filesystem | Journaled (resumable) writer | Fallback writer | Automatic cleanup | Notes |
| --- | --- | --- | --- | --- |
| NTFS (Windows) | Yes | Yes | Yes, by verified handle | Verified on the development host. |
| ReFS (Windows) | Yes, unverified | Yes | Yes, by verified handle | ReFS has 128-bit file ids; Mewtual reads the 64-bit index, which is not guaranteed unique there. No vetted crate exposes the 128-bit id. |
| SMB, mapped drives (Windows) | No | Yes, if the server gives file numbers; otherwise refused before writing | Yes, by verified handle | Recognized by the `\\?\UNC\` canonical path. |
| FAT, exFAT (Windows) | No | No (refused before writing) | n/a | No hard links, so publication is impossible. |
| OneDrive and other cloud folders (Windows) | Yes, on NTFS | Yes | Yes | The provider uploads partial plaintext as it is written. Placeholders are the same file. |
| ext2/3/4, xfs, btrfs, f2fs, tmpfs (Linux) | Yes, when the filesystem reports birth times (tmpfs on older kernels and ext4 with 128-byte inodes may not; the save then falls back) | Yes | Yes, except in folders other accounts can modify (mode bits; a group-writable folder without an extended ACL, of a user-private group proven from the local account files outside any container, counts as private) | Needs `/proc/self/fd`. |
| overlayfs, FUSE, NFS, SMB/CIFS, 9p (Linux) | No | Yes if the mount supports hard links; otherwise the save fails at publication after writing, and the partial is left for the user | No (counted as shared, review M-3) | Container `/tmp` on overlayfs gets no journal. Unix does not probe links here: the probe name could not be removed. |
| vfat, exFAT (Linux) | No | No (refused before writing) | n/a | |
| APFS, HFS+ (macOS, local) | Yes, unverified | Yes | As on Linux | Not built in CI; publication guarantee (a) only. |
| Network volumes (macOS) | No | As on Linux | No (counted as shared) | Not `MNT_LOCAL`. |

### Adversarial review of the PR51 changes (at `838bef9b`) and dispositions

Provenance of the fixes so far: the user's PR51 review (H1, M1, M2, M3) produced `90de8c03` (code)
and `838bef9b` (this design note and the support matrix); this adversarial review of those two
commits, plus two issues the implementer found while it ran (marked "Self"), produced
`5d630cea` (code) and `fc2f99c9` (dispositions below); the user's second review (R2-) is in the
next section.

No blocker or high. Two issues found by the implementer while the review ran are listed first.

| Finding | Disposition |
| --- | --- |
| Self: Clear of a saved row treated a final name merely held by another program as gone (`KeptOnlyCopy`, false notice, second name kept) | Fixed: a final name that cannot be opened right now is `Unchecked`. Test `a_saved_rows_final_name_held_by_another_program_is_not_treated_as_gone` (Windows; break-checked). |
| Self: the Resume preflight recorded RecoverPublished without the M2 flush | Fixed: flush on the pool before the edge. No unit test (the preflight runs against the real actor and a flush is not observable there); covered by reading and by the export sequence's own ordering test. |
| M-1: published-row removal read "not provably ours" as "gone"; false `SavedFileMissing`/`KeptOnlyCopy`; the only copy could be deleted when the staging check errored | Fixed: tri-state name checks; a published row's staging name is deleted only once the final name is proven ours, and nothing at all is deleted otherwise; new `SavedNameReplaced`; crash leftovers are only looked at, not deleted, on that path. Tests: `clearing_a_saved_row_whose_final_name_is_gone_keeps_the_only_copy` (gone and replaced), `..._whose_names_are_both_gone_never_says_removed`, `..._whose_saved_name_was_replaced_says_nothing_false`, the held-name test above. |
| M-2: Unix renamed before verifying; rename-back failures ignored and reported as "not ours"; check-then-rename fallback could replace | Fixed: verify at the original name first; a failed put-back is an error (`Unchecked`); the no-noreplace fallback is a hard link back plus unlink. Tests (Linux CI): `unix_never_renames_an_object_that_is_not_ours`, `unix_a_name_that_cannot_be_put_back_is_never_reported_as_left_in_place` (new `AFTER_ASIDE` seam). |
| M-3: mode bits do not describe network/FUSE folders | Fixed (fail closed): a pinned folder whose class is not local `Journal` counts as shared, so nothing there is deleted automatically on Unix. ACLs are documented as not read. Test (Linux CI): `unix_never_deletes_automatically_on_a_network_or_fuse_filesystem` (new `FsClass` seam). |
| M-4: "delete it yourself" could name a file that is not ours | Fixed: native looks read-only (`session_at`) before saying it; nothing for a gone or foreign name, a hedged sentence when it cannot look. Tests: `download_link_commit_survives_failed_staging_cleanup_and_unsupported_link_refuses` (foreign name with a failed and a shared removal), `a_failed_fallback_save_never_deletes_a_file_swapped_in_at_its_staging_name`. |
| M-5: a successful save's leftover temporary copy was silent (fallback, GIF) or promised a Clear that could not remove it (journaled) | Fixed: the fallback writer and the GIF export settle the staging name after the guard and append a note; `ExportFile::finish` returns the `PartialRemoval`, and native keeps the record (with "Clear ... to remove it") only when Clear can still remove it, else releases it with a truthful note. Tests: `a_fallback_save_in_a_shared_folder_says_its_temporary_copy_stayed` (Linux CI), the settle seam test, `studio_export_link_is_committed_when_staging_cleanup_fails`, `fresh_export_publishes_without_replacement_and_releases_only_private_progress` (finish outcome). |
| M-6 (design question): user-private groups (umask 002) make folders 0775, counted as shared | Kept the rule (the user's own definition of "shared"), documented as a decision for the user; Mewtual-created folders get 0755 (`create_folder`); fixtures pinned. |
| L-1: folder fsyncs inside guards or on async workers | Fixed: no fsync in `link_open`; the unjournaled writers flush and settle after the guard; the GIF export settles on the blocking pool after the registry lock. |
| L-2: Unix no longer probes links on non-FAT mounts | Documented in the matrix: link-less network/FUSE mounts fail at publication and leave the partial (no automatic deletion there). A probe would leave a second name in exactly those folders. |
| L-3: `/proc` loss after creation became a "possibly published" error | Fixed: `ENOENT`/`EXDEV` from the descriptor link are `Unsupported`, a definite no-link. |
| L-4: crash leftovers at removing names | Documented (journaled: found by identity; unjournaled: nothing records them; Keep leaves the file at that name). Follow-up: let Keep put it back. |
| L-5: the panic path checks the number without the handle | Documented in code and here. |
| L-6: doc drift | Fixed: module docs, `open_leaf_pinned`, the crate table (`as-slice`, `nix` 0.29), f2fs/ext2/3 and birth-time caveats, SMB and identity-less wording, `SavedFileMissing`. |
| L-7: test gaps | `/proc` seam added (`without_proc_no_save_starts_that_could_not_finish`), rename-back and replaced-final tests added. Still open: Remove or Pause landing in the flush gap; Linux CI is the first runner of the Unix tests and macOS is never compiled. |

## Second user review of `86664d94` (R2-): dispositions

Three mediums and one low, all at the seams between native operations, external filesystem waits
and the renderer. Line numbers in the findings are at `86664d94`.

| Finding | Effect of the PR51 changes | Disposition |
| --- | --- | --- |
| R2-M1: one global renderer token (`privateDownloadBusy`) held across `cancel_private_download`, so a Remove stalled on an external folder disabled every Transfers button and refused Pause, Remove, Keep, Resume, Save new copy and Save as on unrelated rows | Not fixed by PR51, and more likely since: verified removal (and Unix rename-aside) still waits on the folder. The native cleanup claim was already per download. | Fixed in `App.svelte`: the token is now a map keyed by download ID; a row refuses only a second action on itself, buttons are disabled per row, and the pre-acquisition window of Resume uses the same per-row owner. Test (production text of `changePrivateDownload`, `resumePrivateDownload` and `runPrivateDownloadAction` extracted from `App.svelte`): `a Remove stalled on its folder holds only its own row; every other row keeps its actions` (B's Pause, Remove, Keep and Resume dispatch while A's removal is held; every second action on A is refused; A is free once native answers). Break-checked (a global check turns it red). |
| R2-M2: `choose_destination` held the process-wide one-dialog turn across validation of the picked path and stopped watching cancellation, so a validation stalled on a dead share refused every later Save as until the filesystem answered | Not affected, except that validation now also reads the filesystem class on Unix (R2-M3), one more call that can stall. | Fixed: the turn is scoped to the chooser and ends when the dialog answers; validation runs on the blocking pool while cancellation is still observed, and a cancelled invocation stops waiting at once (its late result is dropped with the future, so it can never start a save). A validation worker left running holds one of four process-wide validation slots, taken before the dialog opens, until the filesystem answers; with all four held, Save as is refused before any dialog ("Earlier Save as choices are still being checked. Try again shortly."). Test: the R2-M2 part of `save_as_selection_through_the_chooser_seam` (the chooser returns a path, validation waits on a barrier, a second Save as opens while the first is still live, the invocation is cancelled and returns at once, a third Save as through a one-slot pool is refused before any dialog, then the barrier is released, the slot returns and nothing was created). Break-checked (holding the turn across validation turns it red). |
| R2-M3: when Save new copy could not reach the old folder, the old journal was abandoned and the "left in place" notice lived only in a local variable attached to a successful result; a replacement that then failed (for example a FAT drive refused at the link probe) lost it | More notices can come out of the abandon since PR51 (`SharedFolder`, `SavedNameReplaced`, `KeptOnlyCopy`); all are carried the same way. | Fixed with the minimum option: every result after the abandon passes through `report_left_behind`, a warning on success and an extra sentence on every failure (including the intent commit inside `prepare_resume` right after the abandon). Plus a read-only check before the abandon where possible: on Unix `ChosenDestination::validate` refuses a FAT, vfat or exFAT folder (`ChoiceRefusal::NoLinks`), so Save as there never abandons the old journal; Windows still learns it only at the creation probe (no vetted crate reads the filesystem name; validation creates nothing), and then the error carries the notice. Not chosen: a durable "unresolved old partial" in the sealed record (a second amendment of the unreleased v2 format, with its own UI and Remove path), and deferring the abandon (the record holds one journal, and the new save needs that slot from Begin on). Tests: `a_failed_new_copy_still_reports_the_old_partial_it_left_behind` (the real `abandon_sequence` reports FolderUnreachable and abandons the journal, then the replacement fails; the final error starts with the replacement's error and contains the notice; a success carries it after earlier warnings), `a_choice_on_a_drive_without_links_is_refused_before_anything_starts` (Linux CI). Coverage gap: `save_group_file` itself needs a live `AppHandle`; that every path after the abandon goes through `report_left_behind` is structural (one block) and was checked by reading. |
| R2-L1: `OnPool::drop` closed the session on the blocking pool without the export permit, so after an early error more than four stalled closes could pile up | PR51 keeps the fallback writer's staging handle open in `OnPool` until it is settled, so more closes take this path. | Fixed: `OnPool::with_custody` moves the save's export custody (`DownloadReady::export_custody`, a new read accessor in `actor.rs`) into the deferred disposal and drops it only after the close; the journaled session (both creation and reopen) and the fallback writer's handle carry it (`ExportPorts::custody`, `SaveSource::custody`). Test: `a_session_dropped_on_the_async_side_closes_on_the_blocking_pool` now holds a close behind a barrier and asserts the export slot stays occupied until the close finishes. Break-checked. |

### Adversarial review of the combined diff (`86664d94..e3db72ea`) and dispositions

No blocker or high.

| Finding | Disposition |
| --- | --- |
| M-A: on Unix a journaled removal in a shared folder answered `SharedFolder` ("delete it yourself") before checking the name, so a name the user had already deleted, or another account's file, produced that instruction | Fixed: the read-only check at the original name comes first; `Shared` only for a name verified ours, `Missing`/`NotOurs` otherwise. Test (Linux CI): `unix_a_shared_folder_tells_the_user_to_delete_only_its_own_file` (missing, foreign, saved row with its staging name gone). |
| LOW 1: saved rows and network/FUSE folders worded as "the partly saved file" and "Other accounts can change" | Fixed: `SharedFolder.notice(saved)` names the temporary copy for a saved row (and `STAGING_SHARED` is gone in favour of it); Save another copy of an unreachable saved row says "The folder of the earlier saved copy is unavailable, so its files there were left in place."; all shared-folder sentences say "This folder may be changed by other accounts". |
| LOW 2: `abandon_sequence` read `published` before its retire | Fixed: from the read after the retire. Test: `save_new_copy_reads_published_after_its_retire`. |
| LOW 3: the jam-sheet export cleaned up a failed write under the session commit guard on an async worker | Fixed: `write_download_held` plus `FailedWrite::clean_up`, which the jam-sheet export runs after dropping the guard (the success path keeps the guard across the write and the reveal as before). The write itself still runs on the async worker, as it did before PR51. Coverage gap: no unit test, because a write into a freshly created local file cannot be made to fail deterministically without a writer seam; checked by reading. |
| LOW 4: panic paths after publication (silent fallback settle; `?` after Published in the export sequence) | Fixed: a hedged sentence for a panicked settle; `after_publication` takes the worker result and keeps the record instead of failing a committed save. Test: `after_publication_keeps_the_record_only_when_clear_can_still_help`. |
| LOW 5: doc drift (`cleared_staging`, `publish_session` errors, "with all four held") | Fixed in code docs, INTERFACES and THREAT-MODEL. |
| LOW 6: empty files left on Windows SMB folders without file numbers | The refusal now says "empty temporary files may remain there". No plaintext. |
| Design: Linux confinement (AppArmor, SELinux, snap, seccomp) can deny `linkat` through `/proc/self/fd` although the read-only availability check passes, and the Flatpak document portal or Android FUSE storage behave alike; such a save writes everything and then halts at publication | Documented residual (the halt is truthful and nothing is reported saved). Follow-up: a real descriptor-link probe at creation, where the probe name could be removed. |
| Design: on Windows the pin blocks renaming the folder's ancestors but not retargeting an ancestor junction, so guarantee (b) holds only without such a writable junction (the save halts as Changed, never reported saved) | Documented residual (THREAT-MODEL). |
| Design: four stalled validations or cleanups on a hard NFS mount disable Save as or Remove until they return; R2-M1 lets one window start concurrent Removes, the fifth reports busy | Documented; the messages are truthful and a retry recovers. |
| Design: Unix writers without a journal: a failed unlink plus a failed put-back leave plaintext at a `mewtual-removing-` name that no record or message mentions | Documented residual (same-account or kernel failure paths only). |

Short re-review of those fixes (`e3db72ea..f38d01ea`): no blocker, high or medium.

| Finding | Disposition |
| --- | --- |
| LOW 1: in a shared folder, another account's file we may not read made Remove retry forever (and a saved row promise a Clear that cannot help) | Fixed: permission denied at the name in a shared folder is `NotOurs` (nothing is deleted there anyway). Test: a mode-0 "foreign-unreadable" case in `unix_a_shared_folder_tells_the_user_to_delete_only_its_own_file` (Linux CI, skipped as root). |
| LOW 2: a crash leftover at a `mewtual-removing-` name, found by identity after the folder became shared, is reported with the staging name's wording | Documented residual: the file is verified ours; only the name in the message is wrong (crash during a removal, then a permission change). |
| LOW 3: code comments described the old order | Fixed (`remove_verified`, `Verified::Shared`, `PartialRemoval::SharedFolder`). |
| LOW 4: test gaps | Added `save_another_copy_of_an_unreachable_saved_row_says_what_was_left`, missing and foreign names in a shared folder for `remove_if_session` (Linux CI), and a notice check that matches both shared-folder instructions; the LOW 3 coverage gap is stated above. |
| LOW 5: the junction sentence in THREAT-MODEL said "Changed" only | Fixed: "halts, as Changed or Unavailable". |

## User decisions (23 October)

Implemented:

- **Group-writable folders: trust a proven user-private group.** A group-writable folder without
  the sticky bit counts as private (so Remove and the writers' cleanup may delete there) only when
  all of these hold: it is owned by our effective uid; its group is our effective primary gid;
  it carries no extended POSIX ACL (with one, the group bits are the ACL mask and may be another
  account's write access; `system.posix_acl_access` read through `rustix`, an error other than
  "no such attribute" or "no ACL support" counts as an ACL); and that group provably has no other
  member. The proof (`group_is_private`) comes only from the local system, read bounded with std
  (1 MiB per file, userdb directories only up to their first entry): this process is in the
  initial user namespace (`/proc/self/uid_map` and `gid_map` are the identity map) and not in a
  container (`/run/.containerenv`, `/.dockerenv`); `nsswitch.conf` lists only `files`, `compat` or
  `systemd` for `passwd` and `group`, and for `initgroups` when present, with `[...]` action groups
  removed character by character as glibc does (or there is no `nsswitch.conf`); when `systemd`
  is listed, no systemd userdb record directory has an entry and systemd-homed is not running;
  `passwd` and `group` have no NIS `+`/`-` or malformed lines and list our own account; no other
  uid has that group as its primary group; exactly one `group` entry has that gid, and its
  supplementary members name nobody but us. A group name equal to the user name is not required
  (only a supporting signal; the membership is the proof). Anything unprovable stays shared
  (directory services such as `sss`, `ldap`, `nis`; unreadable files; a group that resolves only
  through NSS; containers and user namespaces). macOS keeps its account databases in Directory
  Services, so membership is never provable there and `staff` (gid 20) stays shared; Android has
  no `/etc/passwd`, so it fails closed too. World-writable stays shared unless sticky. Not readable
  (residual): `gshadow` members and group passwords, group credentials granted by privileged
  launchers (systemd `SupplementaryGroups=`, set-group-id programs), and sessions that kept a
  group after an account left it. Classification (and its reads) happens only where deletion is
  decided, never under the session commit guard of the publication link.
  Tests inject the account files (`ACCOUNT_FILES` in tests; production reads the real files):
  `a_group_writable_folder_is_private_only_for_a_proven_user_private_group`,
  `a_user_private_group_is_proven_only_from_the_local_files`, `unprovable_membership_fails_closed`,
  `other_accounts_rule_follows_owner_and_mode_bits`, and on Linux CI
  `unix_removes_in_a_group_writable_folder_only_for_a_user_private_group`. Seven guards were broken
  one at a time and each turned a test red (owner, primary gid, other primary, other member,
  unresolvable group, NSS sources, world-writable).
- **Lost saved file: check and say so.** Replaces the "Saved until cleared" residual. When
  Transfers is shown (once per visit) and after acting on a Saved row, the renderer asks
  `check_saved_download(server, download, expectedIntent)`. Native reads the record (revision and
  Published journal), runs `saved_presence` (read-only: the folder must still be the identified one,
  then the final name is opened without following links and compared by identity) on the blocking
  pool with one of two check slots, and waits at most `SAVED_CHECK_WAIT` (3 s, Keep's bound) on
  the sanctioned clock for a free slot and the check together, holding no session commit guard
  while it waits (the renderer asks for one row at a time). The answer is returned only while the
  record is the same Saved record
  (revision and journal) before and after and the UI session and server entry are unchanged; the
  renderer keeps it only for the same view and row revision. Outcomes: `present` keeps "Saved";
  `missing` shows "The saved file is no longer there"; `replaced` shows "The saved file was changed
  or replaced" (never claimed as ours); both offer **Save again**, the existing explicit fresh save
  from the private encrypted copy (no-replace, fresh or numbered name, never automatic);
  `unknown` (timeout, folder unreachable or moved, busy, a refused or unreadable answer) keeps
  "Saved" with the quiet hint "Saved, could not check the file". Clear is unchanged. No path in any
  answer, error or log. The new command is `account-read` in `security/command-policy.json`,
  granted in `capabilities/main.json`, listed in `tauri-command-security.ts` and mocked in the
  visual fixture. Tests: storage
  `a_saved_file_check_says_what_is_at_the_saved_name_without_changing_anything`; native
  `saved_check_tests` (each outcome, the timeout, stale record before and after, session change,
  the bounded read-only production check); frontend `a saved file check says what it found...`,
  `a saved file check answer is a closed word...`, and the production-text App.svelte tests
  `Transfers checks each Saved row's file once per visit...` and `a saved file check answered after
  the row or view changed never relabels anything` (both renderer fences and the native
  after-check fence break-checked). Residual: an edit made in place keeps the file's identity and
  reads "present"; a file renamed within its folder reads "missing"; on Windows the check opens the
  saved file with read access, which antivirus or another program's share mode can delay (the
  bounded wait then answers "unknown").

Adversarial review of these two changes (`516ab8d0..d291cb88`) and dispositions:

| Finding | Disposition |
| --- | --- |
| HIGH: an extended POSIX ACL can give another account write access while the mode shows only group write, so the new rule would call such a folder private | Fixed: the group proof requires the absence of `system.posix_acl_access` (read through `rustix::fs::fgetxattr`, size only); any read error other than `ENODATA`/`EOPNOTSUPP` counts as an ACL. Tests: `an_extended_acl_is_never_ruled_out_without_proof` (Linux/Android), and on Linux CI an ACL case in `unix_removes_in_a_group_writable_folder_only_for_a_user_private_group` (a named user with write, set with `fsetxattr`, gives `SharedFolder`). Both run only on Linux; compiled and Clippy-checked for Android here. |
| M1: all Saved rows were checked at once while native ran two workers and answered "unknown" for the rest | Fixed: native waits for a slot within its 3 s bound (no thread held while waiting); the renderer asks one row at a time and claims its batch up front. Tests: the production check test (a held slot, then a real answer), `Saved rows are checked one at a time...` (break-checked). |
| M2: `nsswitch.conf` parsing dropped services glued to `[...]` and ignored `initgroups` | Fixed: action groups removed character by character (unbalanced brackets fail closed); `initgroups`, when present, must use the same sources. Test: `directory_services_cannot_hide_behind_nsswitch_syntax` (initgroups break-checked). |
| M3: the `systemd` source also serves userdb records and systemd-homed | Fixed: with `systemd` listed, the userdb record directories must be empty or absent and the homed socket absent, else (or if unreadable) shared. Test: `systemd_accounts_and_containers_fail_closed` (break-checked). |
| M4: containers and user namespaces see an `/etc` that does not describe the host | Fixed: the identity uid/gid maps and no container marker are required. Tests: the same, plus `the_account_files_are_read_bounded_from_the_root` (Linux CI; break-checked on the pure rule). |
| L1: unused items on macOS | Gated to Linux/Android or allowed elsewhere. |
| L2: the publication link classified the folder under the commit guard | Fixed: `pin_folder(.., classify: false)` for the link reads no account files or ACLs. |
| L3: acting on a Saved row briefly showed plain "Saved" | Fixed: the last answer stays until the re-check replaces it. |
| L4: INTERFACES error wording | Fixed. |
| L5: test gaps | Added the call-site test, the bounded loader test on a prepared root, and the set-group-id case now changes the folder's group instead of skipping. `ActorSavedCheck` itself (actor-backed) stays untested; its fences are covered through the sequence. |
| Re-review LOW 1: containers without a user namespace (nspawn without `-U`, privileged LXC) keep identity maps and no Docker/Podman marker | Fixed: `/run/systemd/container` and `/run/host/container-manager` are markers too. Test: the loader test checks every marker. |
| Re-review LOW 2: a row that changed during a batch was skipped by the refresh meanwhile and stayed unchecked | Fixed: one re-run of the batch when an answer was dropped as stale (never more, so a row that keeps changing cannot loop). Tests: the stale-row case now expects the new revision's own check; a row that keeps changing gets exactly one re-run (re-run break-checked). |

Recorded, no change:

- "Warning once is enough": the R2-M3 minimum option stays (the old-partial notice travels with
  every result of the replacement save; no durable record of an abandoned partial).
- The Unix rename-aside before unlink, and Windows FAT/exFAT detection only at the creation probe,
  are accepted as implemented.

## Linux CI failures at `516ab8d0` (reproduced at `c7fcaca1`)

Four `catcoms-storage` export tests failed on Linux. They run only in Linux CI, and the Windows
host cannot reproduce them; WSL Ubuntu (Rust 1.89) did.

- **`chosen::a_swap_and_swap_back_around_creation_with_a_planted_file_writes_nothing`: a product
  defect, fixed.** At `86664d94` the Unix branch of `links_supported` hard-linked the staging
  *name* to a probe name and required the probe to be our handle's object, which is what defeated
  a folder swapped before the exclusive create and swapped back with a file planted at our name
  (picker re-review LOW 2). PR51 M3 (`90de8c03`) replaced the Unix probe with `fstatfs` and dropped
  that check with it. On Linux the folder identity recheck then passed (the original folder is
  back) and `create_unjournaled`, and the journaled `create` and `create_chosen` too, kept a handle
  on our partial in the swapped-in folder, so the plaintext would have been written there and
  linked under the final name in that folder. The test's hooks sit in `create_leaf` around the real
  path-based create, so the seam matched production; the test was right. Fix: Unix again requires
  the staging name to hold our handle's object, now with a read-only, no-follow open compared by
  in-session number (`session_id` and `holds_session`), which creates no second name. A failed
  check is `Inconclusive`: the journaled creation removes nothing it cannot prove ours and returns
  `Unsupported` (native then uses the non-resumable writer, which runs the same check against the
  folder as it is now); the non-resumable reservation refuses. Regression tests: the original test
  (non-resumable writer) and the new `a_swap_and_swap_back_around_a_journaled_creation_starts_no_journal`
  (journaled `create_chosen`); both fail on Linux without the check and pass with it. Residual, as
  before PR51: like the Windows probe and the Unix probe it replaces, the check resolves the folder
  by path, so a third swap timed between the folder identity recheck and the name check could still
  pass it; reading both through one pinned folder descriptor would close that window (follow-up).
- **`a_folder_without_links_is_refused_at_creation_and_halts_at_publication`,
  `chosen::a_chosen_folder_without_links_is_refused_before_any_plaintext_exists`,
  `chosen::a_downloads_reservation_without_links_is_refused_before_any_plaintext`: a test-seam
  mismatch.** They injected a failure of `Effect::Link` to model FAT, but Unix makes no link at
  creation since PR51 M3, so there was no link to fail and creation succeeded. The guess that the
  `/proc/self/fd` `linkat` path skips the hook is refuted: `link_open` calls
  `inject(Effect::Link)` on every platform, and the publication half of the first test relies on
  it. Fix: the tests model a no-links folder where each platform decides it (`without_links`: the
  link probe's failure on Windows, `fstatfs` reporting vfat or exFAT through `Effect::FsClass` on
  Unix). Production is unchanged for these three.
- **Real Linux link failures at publication.** `linkat` of `/proc/self/fd/N` reported only
  `ENOENT` and `EXDEV` as a definite refusal. `EPERM` (vfat, `protected_hardlinks`, a security
  module), `EMLINK` and `EOPNOTSUPP` (`ENOTSUP`) now are too (`external::linkat_error`), so the
  journaled save halts as unavailable (persisted; Save new copy or Save as elsewhere) without being
  marked possibly linked, and the writers without a journal fail and clean up their staging name by
  verified removal. A no-links folder is still refused before any plaintext exists where Mewtual
  can tell (`NoLinks` from `fstatfs` on Unix, the probe on Windows); a mount that refuses links
  although its type suggests otherwise is found only at publication, after writing, as the support
  matrix says. Test: `a_refused_descriptor_link_is_reported_as_no_link_made` (Linux and Android).

Each fixed guard was broken on purpose and its test went red on Linux: the Unix name check (both
swap tests), `journal_capable` accepting a no-links filesystem (the first and second seam tests),
`links_supported` ignoring a no-links filesystem (the second and third), and `EPERM` dropped from
`linkat_error` (the mapping test).

## Ledger (23 October, PR51 review rounds)

Focused commands at the final PR51 commit (Windows host), all passing unless noted:

- `cargo test -p catcoms-storage --lib kept::`: 142 passed (the Unix-only tests run in Linux CI).
- `cargo test -p catcoms-app --lib actor::file_transfers`: 62 passed alone and with
  `--test-threads=1`. In parallel runs one export test sometimes fails with "file transfers are
  busy" (the process-wide `process_jobs` pool is shared by the parallel tests of one binary); the
  app crate only gained a read accessor and test call-site updates in this work.
- Native `--lib` filters `private_downloads studio download streamed_save fallback
  stops_the_save lock publish in_downloads_flag device_name keep_session jam_sheet`: 234 passed.
  (A loaded earlier run timed out once in the Node-subprocess studio acceptance test, which passes
  alone in 9 s.)
- The complete frontend suite: 2,366 passed, 0 failed; `npm run check`: 0 errors, 0 warnings.
- Strict native Clippy (all targets, all features, `-D warnings`), root Clippy for storage, sync and
  app, Clippy for `x86_64-linux-android` and `armv7-linux-androideabi` on storage, `cargo fmt`
  (root and native), `scripts/check-no-ambient.sh`, `scripts/check-store-raw-fs.sh`, root and native
  `cargo check --locked`: clean.
- `cargo deny check`: root advisories, bans, licenses and sources ok; native 12 advisory errors,
  the same as the baseline, none in `fs_at`, `rustix`, `aligned`, `as-slice`, `cvt` or `nix`.

Not run (reserved or manual): the complete root and native suites, `npm run build`,
`test:startup`, `test:flows`, real Save as dialogs, Pause or Lock during real external writes,
restart after an interrupted save, FAT/exFAT and network volumes, ReFS, OneDrive, power loss, and
any Unix or macOS execution (Linux CI runs the Unix tests; macOS is never compiled).

## Ledger (22 October)

Focused commands, all passing at `8de5641a` (after the final review):

- `cargo test -p catcoms-storage --lib kept::`: 131 passed.
- `cargo test -p catcoms-app --lib actor::file_transfers`: 62 passed (includes the export tests).
- Native `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib` with filters
  `private_downloads` (33 passed), `keep_session` (14 passed), `download` (52 passed), and the
  set `streamed_save fallback stops_the_save lock download_to_disk publish in_downloads_flag
  device_name` (95 passed); native `cargo check --locked`.
- The complete frontend suite, `npm --prefix apps/desktop test`: 2,365 passed, 0 failed (last
  run at `724a7228`; the final review changed no frontend file).
  `npm run check`: 0 errors, 0 warnings.
- Strict native Clippy, `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml
  --all-targets --all-features -- -D warnings` (with `apps/desktop/dist` built): clean on
  Windows. (The baseline findings noted below are gone since the merge of
  `chat-update-implementation`.)
- `cargo fmt --all -- --check` (root and native), `cargo clippy -p catcoms-storage -p catcoms-sync
  -p catcoms-app --all-targets --all-features -- -D warnings`, `cargo clippy --target
  x86_64-linux-android -p catcoms-storage --all-targets -- -D warnings`,
  `bash scripts/check-no-ambient.sh`: clean.
- `cargo deny check` (root): advisories, bans, licenses and sources ok. Native `cargo deny check`:
  12 advisory errors, the same count as the baseline, none in `libc`, `rfd` or `winapi-util`.
  (Run at `b36737ba`; no dependency changed afterwards.)
- Before the merge, native Clippy was not clean at the baseline: a voice join-key variant size
  and `too_many_arguments` on `save_group_file` (8 parameters at `dbb8d36b`, 10 now with
  `fresh_copy` and `save_as`) and `finish_saved_read_if`. The merge brought upstream's allowances,
  and strict native Clippy is now clean (above).

Compatibility run: fixtures written by a temporary generator (never committed) into
`M:/cargo-isolated/compat-fixtures`; a scratch `git worktree` at `dbb8d36b` with its own target dir
ran a temporary test that opened each fixture twice. All four were refused
(`configure_keep_scope` = Malformed, store error "retained storage needs a storage check") and every
file under each vault except the lock file was byte-identical afterwards. The scratch worktree was
removed.

Not run (reserved or outside this session): `cargo test --all --all-features`, the full native and
frontend suites, whole-workspace Clippy, `npm run build`, `test:startup`, `test:flows` (whose
private-download flow script was updated for the rows' `chosenPlace` but not run), physical FAT, SMB, ReFS,
OneDrive and document-portal checks, a real dialog on each OS, and any Unix execution. The two WOF
tests skip on this host.
