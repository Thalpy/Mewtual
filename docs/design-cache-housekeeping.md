# Cache housekeeping

Status: design agreed on 24 October 2026 (owner answers and design-review fixes folded in);
implemented on 26 October 2026 (see `HANDOVER.md`); the complete suites run in CI. This PR is stacked on PR #51 (`communication-external-download`). The
saved-files list PR is stacked on this one.

## Why

A review of how files are stored found five problems in the encrypted blob cache
(`<vault>/blobs/<group>/`).

1. **The cache only grows.** There is no size limit and no eviction. The 30-day file expiry is
   recorded and displayed but never enforced, although `USER_GUIDE.md` implies more, and the
   storage crate's docs describe retention eviction that does not exist.
2. **Leaving a server leaves its files behind.** `remove_server` deletes the snapshot, network
   and cache-address files, but not `blobs/<group>/` or its Keep copies.
3. **Cache writes are not crash-safe.** A chunk is written with a plain `fs::write`, with no sync
   and no atomic rename. "Held" only checks that the path exists, so a truncated chunk can count as
   cached until a read fails its check and fetches it again.
4. **Saving a file does not fill the cache.** A private Download acquires into its own store and is
   deleted after the export, so viewing the same file in chat later fetches it again.
5. **The docs overstate expiry and retention**, as described in item 1.

## Owner decisions

First round:

- **Display the file we have.** A file held locally and verified is shown from disk and never
  fetched again. Only missing or corrupt chunks are fetched.
- **Saved files propagate.** When a file is saved out of the app, its verified chunks also go into
  the shared cache, so the device can serve them to members as it does for files it has viewed.
  Only once the save has published (section 10): an unfinished or removed Download never serves.
- **Crash-safe writes.** Temporary name, sync, rename. A chunk counts as held only once verified.
- **Leaving a server cleans up** its cached and Keep files, reports the space freed, and never
  removes anything Studio still references.
- **A per-server cache limit.** When it is reached, newer files take priority and the oldest are
  dropped first, ordered by listing time, not last viewed. Never evicted: Keep, Studio references,
  files in use right now, in-flight Downloads, and whatever the saved-files list PR agrees.
- **Footer warning** for the selected server: a bar of the limit used and a count of files "leaving
  soon", shown before the cache is full.
- **Truthful docs.** Expiry stays a displayed date.

Second round (answers to the design questions):

1. **Defaults confirmed.** On by default at 5 GiB per server, settable from 1 GiB to 1 TiB or No
   limit. An existing cache starts at `max(5 GiB, usage / 0.8)` so the upgrade drops nothing.
   Leaving soon = the files the next 10% of the limit would push out; the bar turns amber at 90%.
2. **Never evict this device's own uploads**, so the original source survives.
3. **"Keep forever" becomes "Never expires".** It is a server-wide listing flag, not a cache pin.
   Keep stays the per-device pin.
4. **Leaving asks for confirmation** that names the Keep copies and unfinished Downloads it will
   permanently delete, at every Leave entry point.
5. **Drop the cache's duplicate of a file with a completed Keep copy.** Reads fall back to the Keep
   chunks; Storage health and Repair must not treat those chunks as missing.

## Design

**Display the file we have** needs no new mechanism. Every read already tries the local copy first
(`get_blob` in the read job, the held check in `request_blob_tracked_cancellable`, and the Keep
fallback in the composite store) and fetches only a missing chunk or one that fails its check. A
regression test pins it: a held, valid chunk is displayed with no network request.

### Terms

- **Cache**: the sealed files directly inside `blobs/<group>/`, each named by its ciphertext CID.
  `kept/` (Keep and private Downloads), `staging/` (unpublished uploads) and `incoming/` (writes in
  progress) are not cache.
- **Usage**: the sum of the cache files' on-disk lengths (sealed bytes, about plaintext plus 40),
  from directory metadata. Nothing is decrypted to measure it.
- **File**: one plaintext CID listed in the server's file index. Its chunks are the ciphertext CIDs
  of every live manifest variant listed for it.
- **Listing position**: a row's position in the replicated file index. Rows are appended at the end
  (`write_file_entry`); conflicting first lists are projected in stable object order
  (`file_index_lists`); the index is capped at 256 rows. **A chunk's position is the newest
  position of any live row naming it**, and a file's position is the newest of its chunks. So a
  hostile old row naming a newer file's chunks cannot drag that file out early, and relisting moves
  a file forward, as a new share should.

### 1. Listing order, the mount scan and eviction order

**Listing time is listing position.** Rows carry no timestamp, and a claimed one would be
member-supplied: a far-future value would let a member's files outlive every honest file. Position
needs no clock and is the same on every device. A member who can rewrite rows at the CRDT level can
reorder files or unlist them outright; position adds no power beyond that. Rows appended on both
sides of a partition land in the earlier-sorting list, so order across such a merge is only
approximate.

**No persisted listing index.** The planner reads the index each pass. A second copy on disk would
only add a source of truth that can disagree after a crash.

**The index must be readable, or nothing is evicted.** The planner receives the index as a result,
not a possibly empty list. It pauses entirely when the index document is not open or its
projection fails (more than 256 conflicting `ROOT/FILES` values). When the raw row count exceeds
the 256-row cap (`raw_file_index_row_count`; a hostile write or an honest partition merge can do
this), rows past the cap are invisible, so **class 1 is skipped** but class 2 still runs: a
hidden row's chunks are never offered in class 2, so dropping visible files stays safe. An
unreadable profile document likewise skips class 1 only, because its references only protect
unlisted blobs. One member therefore cannot switch off everyone's limit.

**Mount scan.** `attach_blob_store` opens the primary cache, scans `blobs/<group>/` on a blocking
worker (CID-named regular files and their lengths), recovers leftover temporary files (section 9),
removes CID-named files too short to be a sealed record, and keeps the result in the store: a size
index and the **mount set** (CIDs present at the scan). The store keeps the size index current for
its own writes and deletes. A CID written, promoted or deleted through this handle leaves the mount
set, so a re-published blob (for example a Studio frame whose CID happened to be present) never
counts as unclaimed. The scan reads the directory itself because the composite store's `cids()`
also lists Keep copies. If any entry cannot be read, the scan is marked incomplete and eviction
stays off for that mount.

**Order of eviction**, run only while usage is over the limit and stopping when it is not:

0. **Completed Keep duplicates** (owner decision 5): a cache chunk of a completed Keep copy's
   committed plan is dropped on every pass, over the limit or not, unless another pin holds it,
   but only when that copy passed its full check in this mount (`KeptFile.checked`), is not
   poisoned and is bound. After a restart a Keep copy is unchecked, so its cache duplicate stays
   until a check succeeds: a damaged Keep chunk is never left without the copy repair would read.
   The actor re-checks these conditions before each batch. Reads, serving and Storage health fall
   back to the Keep copy through the composite store.
1. **Unclaimed blobs**: CIDs in the mount set that no live listing names, no profile names as an
   avatar or banner, and no pin set holds. Mostly chunks of listings someone else deleted (the
   delete-time GC runs only on the deleting device) and superseded profile images. They go first,
   in CID order, and are never counted as files. Livery images are inline in the livery document
   and event images are listed files, so neither is a separate blob reference.
2. **Listed files, oldest position first.** A file is dropped as a whole: every held chunk except
   those a pin set holds. Partly cached files count only their held chunks. Profile images are
   excluded here too, so a hostile old row naming an avatar cannot drag it out first.

The planner is a pure function of (size index, mount set, index snapshot, profile references, pin
sets, limit), unit-tested without a disk.

### 2. Pins

Every pin is a **set of chunk addresses taken from its owner's own saved plans**, applied to every
class above, including unclaimed blobs and Keep duplicates:

- **Keep in progress**: every chunk of every saved plan of each Keep transfer that is not completed
  (`kept_files().transfers`, `kept_saved_plans`). A completed Keep is not a pin: its cache duplicate
  is dropped (class 0).
- **Downloads**: every chunk of every saved plan of each unfinished Download record, running or
  paused (`downloads()`, `download_saved_plans`).
- **In use**: the chunks of every manifest an in-flight read, range read, Keep run, Download or
  export is using, plus the chunks of files read in the last 10 minutes by the monotonic Clock. The
  actor keeps this as a bounded recency set (64 files); when it is full the oldest entry leaves.
- **Own uploads** (owner decision 2): the chunks of every live listing row whose attestation
  verifies under this device's own key (`author_verified` and `author_identity` equal to this
  device). Only verified rows count, so a member cannot exempt a file from this device's limit by
  writing this device's short fingerprint into a row. Legacy unsigned uploads from before
  attestation are not pinned. Own uploads that are deleted from the index are no longer listed and
  become ordinary unclaimed blobs.
- **Studio**: the group's protected set from `ProtectedBlobs` (durable references plus live job
  holds). The deletion guard also re-checks it under its mutex through each unlink.
- **Saved-files list**: added by PR #56 to the same input.

**Not pins: wiki embeds and "Never expires".** Both are circulation rules members set for every
device: any member can embed a listed file in a wiki page (even a pending edit pins it in
`wiki_pinned_cids`), and the uploader, an admin or the owner can mark a listing "Never expires".
As cache pins they would let one member exempt chosen files from everyone's limit. Expiry stays
unenforced; the `wiki_pinned_cids` comment is narrowed to expiry. The Properties button is renamed
from "Keep forever" to "Never expires" (owner decision 3), so it no longer reads as a local promise.

**Unknown pins pause eviction.** If the Keep or Download inventory reports an error (including Keep
not yet bound to the group and device), a Keep or Download plan cannot be read, or Studio
protection is unknown, the pass deletes nothing and the footer says "cleanup paused". A pin set that
cannot be read never reads as empty.

**One evicting owner per directory.** One vault can attach a group under two native server numbers.
Only the handle that holds the namespace's Keep lease evicts; any other attachment fails that lease,
reports a Keep error and is paused by the rule above. A Studio per-request handle that wins the
lease during an actor reload leaves eviction paused for that mount, which fails safe.

Pinned bytes count in usage, so usage can sit above the limit when pins alone exceed it. The footer
then says so ("held by pinned files") instead of a count nothing will act on.

### 3. The Studio-reference service

The actor cannot run `creative_pinned_cids`, which needs the mounted `ServerStore`. A native service
in the desktop bridge runs it on a blocking worker under the store lock:

- once after each unlock, once after a server's blob store attaches, and when an actor asks;
- at most once per 60 seconds by the Clock, coalescing requests that arrive meanwhile;
- then it tells every actor that protection may have changed, and they replan.

An actor asks when a pass would delete and its store reports protection unknown. The actor reads
known or unknown and the group's protected set through a new `BlobStore::protected_cids()`
(`None` = unknown), which `ProtectedBlobs` answers from its shared state. Durable writes that make
protection unknown again are followed by a new request, still bounded by the 60-second rate. A
scan that fails backs off (doubling, up to 30 minutes) instead of retrying every minute, and the
footer says cleanup is paused because Studio references could not be checked.

Studio's own save path promotes through its own handle and takes its hold after promoting; a frame
published in an earlier mount and saved after a restart can therefore be class 1 in between. That
gives the existing "publish the frame first" refusal, not data loss, and relies on `publish_pix`
running on the actor handle.

### 4. When eviction runs, and concurrency

- **Triggers**: after the mount scan; after a store write takes usage over the limit; after the
  Studio service reports; when the user lowers the limit. At most one pass per 30 seconds by the
  Clock, except an explicit limit change.
- **Where**: planning and deletes run on the server actor, which owns the index, the pin inputs and
  the blob handle. Deletes must use the actor's handle, because only it issues background read plans
  whose epoch a delete has to retire. At most 64 deletes per actor turn, so commands and sync keep
  flowing. Every batch is planned from fresh inputs (pins, rows, Keep state), so a change between
  turns is always seen before the next delete; a pass ends when the plan has nothing left or a
  batch deleted nothing.
- **Numbers**: the store's size index covers its own writes. Studio and the desktop open other
  handles on the same directory, so before a pass that would delete, the actor refreshes the index
  with a metadata-only rescan (no recovery mid-session; Windows directory listings carry sizes, so
  this is cheap). The rescan is bounded; if it is incomplete the pass deletes nothing.
- **Local reads and fetches**: display reads, whole-chunk and page serving, and chunk acceptance all
  run synchronously on the actor, so eviction never lands mid-read or mid-write. Between two chunks
  an evicted chunk is fetched again.
- **Background readers**: only file-source serving reads off the actor (two per process). A delete
  retires the store's read epoch, revoking every outstanding plan; a revoked read answers Absent or
  Unavailable, never stale bytes. That is why passes are batched and rate-limited, and why `delete`
  is never called for a CID the size index does not hold.
- **Windows**: our own readers open with delete sharing. Antivirus, indexers or sync clients can
  block an unlink (error 32); that delete counts as failed and is retried next pass.
- **Crash mid-pass**: deletes are independent unlinks; a half-dropped file is a partly cached file.

### 5. What a peer gets for an evicted chunk

Exactly what it gets for a chunk this device never held: an empty reply to a whole-chunk request, a
signed Absent to a page or file-source request, or Unavailable if the delete revoked a read already
under way. The requester tries its next provider and then the bridge catalog. Nothing advertises
holdings, and there is no eviction marker or wire change. A completed Keep copy keeps serving
through the composite store.

### 6. Default limit and migration

- **On by default at 5 GiB per server; 1 GiB to 1 TiB, or No limit.** 5 GiB holds 20 files at the
  default 256 MiB server file limit and is never smaller than the 1 GiB largest legal file.
- **Migration**: a server with no saved setting (or a record naming another group, which reads as
  missing) gets `max(5 GiB, usage / 0.8 rounded up to a whole GiB)`. If that exceeds 1 TiB it gets
  No limit. If the mount scan was incomplete, no record is written and eviction stays off until a
  complete scan.
- **Unreadable record**: eviction stays off and the settings page asks for a limit again.
- Without a holder probe the limit can drop the last copy anywhere; own uploads are pinned
  (decision 2), which covers the most common sole holder.

### 7. Leaving soon and nearly full

- **Leaving soon** = the listed files the planner would drop if the next **10% of the limit**
  arrived: walk classes 1 and 2, skipping pins, until `usage + 10% of limit - freed <= limit`, and
  count the distinct listed files touched. Unclaimed blobs count toward the bytes, not the number.
- **Nearly full** = usage above **90%** of the limit; with no unclaimed blobs the count turns
  non-zero exactly when the bar turns amber.
- Housekeeping, including class 0, runs only on the persistent primary cache, after a complete
  mount scan and once a limit is known (saved or migrated); No limit still runs class 0. The
  in-memory fallback store keeps its own FIFO budget.
- **Over** = usage above the limit: while pins hold it there, or between a write and the next pass.
  One large write can jump straight past 90%, so the warning comes before the cache is full in the
  usual case, not in every case.

### 8. The setting, commands and UI

- **Record**: `servers/<id>.storage`, sealed under the vault `db_key` like `.net`, written with the
  existing atomic record helper. Version 1: group id, limit in bytes (0 = No limit), origin
  (migrated or user). `remove_server` deletes it with the other per-server records.
- **Commands**: `get_cache_status(server)` (cheap: the actor keeps the last plan's totals and
  recomputes at most once a second) and `set_cache_limit(server, bytes|null)`, which refuses values
  outside 1 GiB to 1 TiB natively. Both are registered in the command policy, the capability and the
  frontend security ledger.
- **Settings page**: Server settings → **Storage** ("Storage on this device"), for servers and DMs:
  the same bar, a limit picker (1, 2, 5, 10, 20, 50, 100 GiB, 1 TiB, No limit), what is pinned, and
  the leaving-soon count. Lowering below current usage confirms with what will go.
- **Footer**: a segment on the right of the status bar, after the spacer and before the fingerprint,
  clear of the Transfers button PR #51 is changing. `CACHE`, a slim bar (`role="progressbar"`) and
  the percentage; amber at 90% with a "N leaving soon" chip; red over the limit; "held by pinned
  files" or "cleanup paused" when those apply; "CACHE 3.2 GB" with no bar for No limit. Title text
  such as "2.8 GB of 5 GB used on this device. The oldest files leave first. Kept files, files you
  shared from this device, Studio work and files in use stay." (Legacy unsigned uploads are not
  covered; the guide says so.) Clicking opens the Storage page. Hidden when no server is selected.
  Data: `get_cache_status` on server switch and a native `cache-status` event, at most once a
  second. No animation.
- **Remove from Keep**: once a checked Keep copy has replaced the cache duplicate, releasing the
  Keep copy deletes the device's last local copy; the Release button's explanation says so.

### 9. Crash-safe writes

- **Primary cache `put`**: seal, write `incoming/<cid hex>.<process id>.<mount generation>.<seq>`
  with `create_new`, `sync_all`, then rename onto `<cid hex>`. No directory flush per write: the
  guarantee is "never torn", and a rename lost to power failure only loses an entry that is fetched
  again. The mount generation is a process-wide counter assigned when a primary handle opens and
  kept in a process-wide live set until that handle drops. An existing record is still replaced
  only after a bounded read proved it absent or invalid.
- **Only the primary cache changes.** Keep reuses `SealingBlobStore` for each `kept/<cid>/` and
  `transfers-v2/<name>/` directory, and those loaders reject any unexpected entry, so an `incoming/`
  folder there would fault the Keep store. Keep already flushes each chunk and re-verifies after a
  crash. The atomic mode is a constructor option used only by `ServerStore::blob_store`.
- **Staging and promotion**: `put_staged` syncs before close; promotion keeps its existing flush and
  directory barrier (`promote_staged_bounded`), and `promote_staged` gains the same best-effort
  barrier.
- **Held means verified**: `put` derives the CID from the bytes it writes, and a CID name appears
  only after a complete synced write. `has()` stays a cheap hint; legacy torn files still fail their
  read check and are repaired by `put`.
- **Mount recovery**: delete `incoming/` files whose process id is not this process, or whose mount
  generation is not live in this process. In the sealing store, delete CID-named files shorter than
  40 bytes (nonce plus tag, the size of a sealed empty blob). The plain test store holds an empty
  blob as an empty file and skips that rule.

### 10. Saved files into the cache

Contract (corrected after CI on `0c32f7ae`, which had copied at acquisition, more broadly than
decided): a private Download's chunks reach the shared cache only once a save of that Download has
**published** outside the app. An in-progress, paused, failed, cancelled or removed Download never
makes this device a source.

- **Acquisition copies nothing.** Network fetches and the recheck from the private store after a
  restart write only to the private store, which is never served.
- **Export reads stage.** Each `ReadDownloadChunk` puts the ciphertext it read (whose CID the
  private store's read re-verified) into the primary cache's `staging/`, unless the cache already
  holds it. Staged blobs are invisible to `has`/`get`, serving, `file_available` and cache usage.
- **Publication promotes.** Native calls `FinishDownloadExport` only after the save published. The
  actor releases the private record and then promotes the staged chunks (one rename each), but only
  if the export lease was still current and the release succeeded. Otherwise (a Remove won the
  race, or the release was refused) it drops them.
- **Remove drops.** `CancelDownload` drops whatever that Download's exports staged. A failed export
  leaves staged blobs that are never served; they go at the next attach (`clear_staged_uploads`),
  or when the Download is saved or removed.

Staging during the export, rather than copying everything at its end, keeps the end to renames: a
1 GiB file re-sealed in one actor turn would stall the server's other commands for seconds. A failed
stage or promotion is logged and never fails the read, save or removal. Keep runs do not copy (their
completed copy serves). Afterwards this device serves those chunks like any file it has displayed,
so a member asking can learn it holds the file. Limitation: a journaled save whose temporary second
name could not be removed keeps its record (native skips `FinishDownloadExport`), so that save does
not propagate, and Clear later drops what it staged. Tests that pinned "a Download never becomes a
source" keep that check through acquisition and export and now also check that the published save
makes the file available and that a removed Download does not.

### 11. Storage health and Repair

- A referenced chunk that is neither in the cache nor in a completed Keep copy is **not cached**
  (new `not_cached_chunks`), unless it belongs to one of this device's own uploads, where it stays
  **missing**. Completed Keep chunks verify through the composite store's fallback, so dropped
  duplicates are not missing.
- **Repair** fetches only unreadable chunks (held but failing their checks) and missing own-upload
  chunks. It no longer fetches every not-cached chunk back, which would fight the limit.
- The storage pane shows "not cached on this device" as normal, not as damage.

### 12. Leave cleanup

`leave_server` keeps today's order (stop the actor, drain persistence, remove the registry row,
`remove_server`, persist the registry) with two additions: the `leaving` marker is written in
`blobs/<group>/` (and flushed) **before** `remove_server`, and cleanup runs only if the registry
persist succeeded. `remove_server` now also deletes `.storage`. Then:

1. **Claim the group.** An in-process cleanup claim per group lives in the bridge state from here
   to the end. Found, join and blob-store attach for that group wait for the claim; the resume
   sweep checks claims and running servers under the same lock. So no live server can attach the
   namespace while it is being emptied.
2. Under the store lock: run the Studio scan (`creative_pinned_cids`), open a cache-only handle
   (the protected primary without the Keep store, so it does not take the `kept/` lease) and
   enumerate the CIDs to delete. If the scan fails, delete nothing from the cache and keep the
   marker. Release the store lock.
3. Without the store lock, delete exactly the enumerated CIDs through `ProtectedBlobs::delete` in
   batches, checking between batches that the unlocked session is still current. Studio pins stay.
4. Remove `staging/` and `incoming/`, then delete `kept/` wholesale: take its lease, remove every
   entry except the lease file without following links, release the lease, remove the lease file
   and the directory. No per-record release and no device binding are needed, so this also works
   on resume. Exported files outside the vault are untouched.
5. Remove the marker only when every delete succeeded, then the namespace if it is empty.
   Studio-pinned blobs keep it, without the marker. Release the claim.
6. Return `{freed_bytes, kept_bytes, failed}`; the UI shows a toast such as "Left Lantern Room.
   Freed 1.2 GB on this device." and names any kept or failed amount.

**Resume**: after unlock, once every registry server has restored, each namespace with a marker
whose group no running server uses is cleaned the same way, under a claim. If a registry server
failed to restore, the sweep waits and logs which one.

**Rejoin over a marker.** Attaching a group whose namespace still carries a marker first finishes
the non-cache steps (removes `staging/`, `incoming/` and `kept/`), keeps the cache (it is the same
group's content-addressed data), removes the marker, and only then opens the Keep store. These
steps run under the exclusive attach hold described next; if another handle still has the
namespace open, or the removal fails, the marker stays and Keep is disabled for that mount
(eviction pauses with it), so a live handle's scratch is never removed under it, a half-deleted
`kept/` is never loaded as whole and confirmed deletions are not resurrected.

**An attach that came first** (review H1, corrected in re-review). A claim only stops attaches
that start after it; a rejoin, or a join of the same group still being registered while the old
server runs, may already hold the namespace. The Keep lease cannot prove that: only the first
attach of a group gets it, and a second attach runs with a faulted Keep store and no lease. So
every primary cache handle (`SealingBlobStore::open_cache`, used by every attach and every Studio
per-request handle) holds `<namespace>/attach.lock` **shared** for its lifetime, and phase 1
requires the marker to still be present (a rejoin removes it), then takes `attach.lock`
**exclusively**, then the Keep lease, and holds both to the end. The cleanup's own handle opens
under that exclusive hold (`open_cache_under`) rather than taking a shared one. All attempts are
try-locks, and the lock file is unlinked (while still held) only as the last step before the
namespace is removed, so a racing opener either fails its try-lock or creates a fresh lock file
that keeps the directory.

`leave_server` takes its claim **before** it stops the server, while that server's handle still
holds the Keep lease. From then on no attach of the group can start, so any handle still open at
cleanup time attached while the lease was held and has a faulted Keep store with no copies of its
own. (A claim taken after the stop would let a join land in between with a healthy Keep.) The
claim is taken even when an alias row shares the group, and dropped after the row's removal if an
alias still remains: a join that attached before the claim and registered during the stop is such
an alias, and two alias rows left together run one after the other, so the last one cleans
instead of each seeing the other and neither marking the namespace. Residual: the argument
assumes the leaving server owns the Keep lease. If it attached second in its mount and the owner
has since gone, a third attach of the group could get a healthy Keep before the claim and, if
still unregistered at cleanup, sit under the kept marker. Accepted as a follow-up (a found or
join attach could disable retention whenever a registered entry already uses the group).

A busy lock means some handle is open: such an earlier attach of the same group, or the leaving
server's own handle still being dropped (the actor can take longer than the wait when its event
channel is full). The two cannot be told apart, so nothing is deleted, the marker is **kept** and
the leave reports a failure. That is safe either way: a live server's next attach treats the
marker as a rejoin (the cache stays and the confirmed-deleted Keep copies go; by the precondition
above it holds no copies of its own), the sweep skips groups a registered server uses, and
otherwise the next unlock finishes the leave. A rejoin whose marker cannot be removed also keeps
Keep off for that mount, so no new copies are made under a marker. A leave first waits up to
three seconds for the just-stopped actor's handle to drop. A cleanup stopped by a session change
skips scratch and `kept/` too, and keeps the marker. The leave report says `cleaned: false`
whenever cleanup did not run, with `failed > 0` when its marker stays for a later attempt (the
namespace was busy, or the registry change was not durable yet).

Residual: Linux emulates `flock` on NFS with per-process POSIX locks, under which a second lock in
the same process succeeds; like the vault and Keep locks, this proof assumes a local filesystem.

**Confirmation** (owner decision 4): every Leave entry point (the rail menu and the settings page)
asks first, naming the Keep copies and unfinished Downloads it will permanently delete. If either
inventory cannot be read, it says it could not check rather than that nothing will be lost. Caches of
servers left before this change carry no marker and are not touched.

### 13. Docs to correct

- `USER_GUIDE.md` §6: the circulation date is displayed only; this device keeps cached files up to
  its limit and drops the oldest listings first; Keep, own uploads and files in use stay; "Never
  expires" and wiki embeds keep circulating but can leave this device's cache; leaving deletes the
  server's cache, Keep copies and unfinished Downloads after confirming.
- `catcoms-storage` docs (`lib.rs`, `retention.rs`): the retention engine is not used by the app;
  there is no SQLCipher persistence.
- `catcoms-app` `FileExpiry` and `wiki_pinned_cids` docs.
- `INTERFACES.md`, `ARCHITECTURE.md` files row, `THREAT-MODEL.md` (the no-quota note, the save
  privacy note, the Download source rule, eviction without holder proof), `HANDOVER.md`, and
  `design-file-reliability.md` remaining limits.

### Out of scope, honest limits

- No holder probe, remote possession acknowledgement or automatic replication; expiry is not
  enforced.
- Caches of servers left before this change are not cleaned automatically.
- Usage measures sealed file lengths, not filesystem allocation.
- A hostile listing can name other blobs as chunks of an old fake file. Profile images, pinned
  chunks and newer listings' chunks are protected; anything else it names can leave early and is
  fetched again on demand.
- Unclaimed blobs written during the current mount wait for the next mount.

## Verification

- Unit tests for eviction order, chunk positions, pins on every class, pause rules, thresholds and
  footer counts.
- Torn-write and recovery tests, including the live mount-generation filter.
- Leave-cleanup tests with Studio references, failed deletes and resume.
- The save-fills-cache test; Storage health and Repair with evicted and Keep-held chunks.
- Footer, settings and leave-confirmation UI tests; visual-fixture screenshots at 900 x 640.
- The full suites, `test:startup` and `test:flows`, and an adversarial review of the diff.
