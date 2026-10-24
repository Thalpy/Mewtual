# Cache housekeeping

Status: design proposed on 24 October 2026, waiting for the owner to answer the questions at the
end (default limit, "leaving soon" rule and three smaller points). Implementation and the diff
review are still to come. This PR is stacked on PR #51
(`communication-external-download`). The saved-files list PR is stacked on this one.

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

- **Display the file we have.** A file held locally and verified is shown from disk and never
  fetched again. Files are addressed by a hash of their encrypted bytes, so a fetch can only return
  identical bytes; a changed file would be a new listing with a new address. Only missing or
  corrupt chunks are fetched.
- **Saved files propagate.** When a file is saved out of the app, its verified chunks also go into
  the shared cache. The device can then serve them to other members, as it already does for files
  it has viewed.
- **Crash-safe writes.** Cache chunks are written to a temporary name, synced, then renamed. A chunk
  counts as held only after it has been verified.
- **Leaving a server cleans up.** Leaving removes that server's cached and Keep files and reports
  the space freed. It never removes anything Studio still references.
- **A per-server cache limit, set by the user for each server.** When the limit is reached, newer
  files take priority and the oldest files are dropped first, ordered by the file's listing time,
  not by when it was last viewed. Never evicted:
  - files pinned with Keep;
  - files Studio references;
  - files being displayed or transferred right now;
  - in-flight Downloads;
  - files the saved-files list relies on, if any, as agreed in that PR.
- **Footer warning.** The status bar shows, for the currently selected server, a bar of how much of
  its cache limit is used, and an alert with the number of files "leaving soon", meaning the files
  next in line to be dropped. Users are warned before the cache is full.
- **Truthful docs.** The guide and the storage crate's docs must match what the code does. Expiry
  stays a displayed date unless a later decision changes that.

## Proposed design

**Display the file we have** needs no new mechanism. Every read already tries the local copy first
(`get_blob` in the read job, the held check in `request_blob_tracked_cancellable`, and the Keep
fallback in the composite store) and fetches only a missing chunk or one that fails its check. A
regression test pins it: a held, valid chunk is displayed with no network request.

### Terms

- **Cache**: the sealed files directly inside `blobs/<group>/`, each named by its ciphertext CID.
  `kept/` (Keep and private Downloads), `staging/` (unpublished uploads) and the new `incoming/`
  (writes in progress) are not cache.
- **Usage**: the sum of the cache files' on-disk lengths (sealed bytes, about plaintext plus 40),
  measured from directory metadata. Nothing is decrypted to measure it.
- **File**: one plaintext CID listed in the server's file index. Its chunks are the ciphertext CIDs
  of every live manifest variant listed for that plaintext CID.
- **Listing order**: a row's position in the replicated file index. Rows are appended at the end
  (`write_file_entry`), conflicting first lists are projected in stable object order
  (`file_index_lists`), and the whole index is capped at 256 rows. A file listed more than once
  (an upload deduplicated under a new name or folder) takes its newest row's position.

### 1. Listing time and eviction order

**Listing time is the listing order.** The index is append-only in the order files are shared,
identical on every device, and needs no clock. Rows carry no timestamp today, and a new claimed
timestamp would be member-supplied: a far-future value would make a member's files outlive every
honest file. Position cannot be pushed forward that way. A member able to delete and re-insert
rows at the CRDT level could move a file to the front, making it leave sooner everywhere, but that
member can already unlist files outright, so this adds no new power.

**No persisted listing index, so nothing to migrate in format.** The planner reads listing order
from the replicated index each pass (256 rows, at most 128 chunks each). Keeping a second copy on
disk would only add a source of truth that can disagree with the index after a crash. What existing
caches do need, on every mount rather than once, is a **mount scan**: list `blobs/<group>/` on a
blocking worker, record each CID-named regular file's length, recover leftover temporary files
(section 9) and remove CID-named files too short to be a sealed record. It runs in
`attach_blob_store`, between `blob_store()` and `set_blob_store`, where nothing else holds the new
handle and no read is in flight; that step moves into `spawn_blocking` with the existing staging
sweep, and also runs the Studio reference scan so protection starts known. The scan enumerates the
directory itself: the composite store's `cids()` also lists Keep copies. The one-time migration is
the starting limit for caches that predate the setting (section 5).

**Order of eviction**, run only while usage is over the limit and stopping as soon as it is not:

1. **Unclaimed blobs**: blobs already present at the mount scan that are not a chunk of any live
   listing, not a known non-file reference (every avatar and banner named in the profile document,
   including departed members' still shown beside old messages, and this device's own) and not
   Studio-protected. Livery images are inline in the livery document and event images are listed
   files, so neither is a separate blob reference. These are
   mostly chunks of listings someone else deleted (the delete-time GC runs only on the deleting
   device) and superseded profile images. They go first, in CID order, and are never counted as
   "files" in the UI. Requiring presence at the mount scan keeps anything written during this mount,
   by any handle, out of this class. That covers Studio's publish-then-reference window:
   `publish_pix` stores a frame before any record names it.
2. **Listed files, oldest listing first.** A file is dropped whole: every held chunk of every
   variant, except chunks pinned for another reason. Partly cached files count their held chunks.

The planner is a pure function of (scan sizes, index snapshot, pin sets) so order and pins are
unit-testable without a disk.

### 2. Pins (never evicted)

- **Keep**: any file with a Keep record, completed, partial or paused (`kept_files()`,
  `kept_saved_plans`). Its cache chunks stay. The Keep copy itself lives in `kept/`, has its own
  1 GiB / 32-file quota and is not counted in usage. Keeping the cache duplicate is the literal
  reading of the decision; dropping it while a completed Keep copy exists would save space and is
  listed as a question below.
- **Studio**: every eviction delete goes through `ProtectedBlobs::delete`, which checks durable
  pins and live job holds under its mutex through the unlink. The planner also excludes Studio
  pins up front so it does not count bytes it cannot free. Protection starts unknown on a vault
  with epoch files and becomes unknown again after some durable writes, and the guard then refuses
  every delete. A pass that finds it unknown first runs the same complete scan Studio runs
  (`creative_pinned_cids`, on a blocking worker under the store's existing custody). Only if that
  scan fails does eviction pause, and the footer says so.
- **Displayed or transferring now**: files with an in-flight read job, media range read, upload,
  Keep run, Download or export, plus files read in the last 10 minutes by the monotonic Clock. The
  actor keeps this as a bounded recency set (64 files). Displaying an image or playing a video
  refreshes it. It is a short pin, never an ordering input.
- **In-flight Downloads**: any unfinished Download record, whether running or paused
  (`downloads()`, `download_saved_plans`).
- **Saved-files list**: whatever PR #56 agrees, added to the same pin input.

**Not pins: wiki embeds and "Keep forever".** Both are circulation rules that members set for
every device. Any member can embed any listed file in a wiki page, and even a pending edit pins it
(`wiki_pinned_cids`); any uploader can mark their own files "Keep forever" (circulation expiry
never). Treating either as a cache pin would let one member exempt chosen files from everyone's
limit. The older retention contract pinned both, but for expiry, which stays unenforced. The code
comment on `wiki_pinned_cids` that says every retention pass must consult it is narrowed to expiry.
The "Keep forever" label now reads as a local promise it does not make, so it should change (for
example "Circulate forever"). Both are listed as a question below.

**Unknown pins pause eviction.** If the Keep or Download inventory reports an error, or Studio
protection cannot be made known, the pass deletes nothing and the footer shows "check paused". A
pin set that cannot be read is never treated as empty. This includes Keep before it is bound to
the group and device: its inventory then reads empty but carries an error, which pauses the pass.
Eviction runs only on the persistent sealed store; the in-memory fallback keeps its own FIFO
budget.

**One evicting owner per directory.** One vault can attach the same group under two native server
numbers (an alias). Only the handle that holds the Keep lease for the namespace evicts; a second
attachment fails that lease, reports a Keep inventory error and is paused by the rule above. Its
setting is ignored while it does not own the lease.

Pinned bytes still count in usage, so usage can sit above the limit when pins alone exceed it. The
footer then says the cache is over its limit because of pinned files, rather than showing a count
that nothing will act on.

### 3. When eviction runs, and concurrency

- **Triggers**: after the mount scan; after a store write takes usage over the limit (coalesced,
  at most one pass per 30 seconds by the Clock); immediately when the user lowers the limit.
- **Where**: planning runs on the server actor, which owns the index, the pin inputs and the blob
  handle. Deletes must use the actor's handle, because only it issues background read plans whose
  epoch a delete has to retire. Deletes run in batches of at most 64 per actor turn, so commands
  and sync keep flowing. Before each batch the actor rechecks the recency set, the Keep and
  Download inventories and the index revision; a changed index discards the rest of the plan and
  replans on the next turn.
- **Fresh numbers**: Studio and the desktop open extra handles on the same directory, so the actor's
  running total can drift. Each pass starts from a fresh measurement scan on a blocking worker
  (metadata only; recovery happens at mount, never mid-session). The running total only feeds the
  footer between scans. Class 1 stays limited to CIDs the mount scan saw.
- **Local reads and fetches**: display reads, whole-chunk and page serving, and chunk acceptance
  all run synchronously on the actor, so eviction never lands mid-read or mid-write. Between two
  chunks an evicted chunk is simply fetched again. A file being fetched is in the recency set. A
  fetch that lands after its file was dropped stores the chunk again; the file stays old and can
  leave again later.
- **Background readers**: only file-source serving reads off the actor (at most two per process).
  `delete` retires the store's read epoch, which revokes every outstanding plan, not only the
  deleted CID's: a revoked read answers Absent or Unavailable, never stale bytes. That is a second
  reason to batch and rate-limit passes, and `delete` is never called for a CID the scan did not
  see, because it retires the epoch even when nothing is removed.
- **Windows**: our own readers open with delete sharing, so they cannot block an unlink. Antivirus,
  indexers or sync clients can (error 32); such a delete is counted as failed and retried next
  pass. On a filesystem without POSIX delete semantics a refetch of the same CID during the brief
  delete-pending window can fail once and retries.
- **Crash mid-pass**: deletes are independent unlinks. A file left half-dropped is a partly cached
  file; the next pass continues. Nothing needs reconciling.

### 4. What a peer gets for an evicted chunk

Exactly what it gets for a chunk this device never held: an empty reply to a whole-chunk request,
a signed Absent to a page or file-source request, or Unavailable if the delete revoked a read
already under way. The requester then tries its next provider and, after that, the bridge catalog.
Nothing advertises holdings, so there is no stale advertisement to correct. There is no eviction
marker and no wire change, so a peer cannot tell eviction from absence or throttling. A completed
Keep copy of the same chunk is still served through the composite store's fallback. The storage
pane's cached health report goes stale after eviction and is refreshed by the next explicit scan.

### 5. Default limit

**Proposed: on by default, 5 GiB per server.**

- It holds 20 files at the default 256 MiB server file limit, and is never smaller than the 1 GiB
  largest legal file, so a single file always fits.
- It stays modest on disk with several servers.
- The setting allows 1 GiB to 1 TiB, or **No limit** (today's behaviour, shown as usage without a
  percentage).
- **Upgrade migration**: a server with no saved setting gets
  `max(5 GiB, usage / 0.8 rounded up to a whole GiB)` and that value is saved. The upgrade itself
  therefore drops nothing and starts below the warning threshold; the user can lower it.

Without a holder probe (the older `design-retention-integration.md` contract), size eviction can
drop the last copy anyone has. The user-chosen limit and the leaving-soon warning are the
safeguards; this is listed as a question below.

### 6. "Leaving soon" and "nearly full"

- **Leaving soon** = the files the planner would drop if the next **10% of the limit** arrived as new
  data: walk the eviction order, skipping pins, until `usage + 10% of limit - freed <= limit`, and
  count the distinct listed files touched. Unclaimed blobs count toward the bytes but not the
  number.
- **Nearly full** = usage at or above **90%** of the limit. With no unclaimed blobs the two rules
  coincide: the count becomes non-zero exactly when the bar turns amber, before anything is
  dropped.
- **Over** = usage above the limit (only possible while pins hold it there, or between a write and
  the next pass).

### 7. The setting

- **Storage**: a new sealed record `servers/<id>.storage`, sealed under the vault `db_key` like
  `.net`, written with the existing atomic record helper. Version 1 holds the group id (a record
  naming another group is ignored), the limit in bytes (0 = no limit) and its origin (default,
  migrated or user). Per device, never replicated, unrelated to livery. `remove_server` deletes it
  with the other per-server records.
- **Unreadable record**: eviction stays off for that server and the settings page asks the user to
  choose a limit again. Losing data to a damaged setting is worse than a cache that grows a while.
- **UI**: a new Server settings page, **Storage on this device** (all servers, including DMs). It
  shows the same bar as the footer, a limit picker (1, 2, 5, 10, 20, 50, 100 GB, custom, No limit),
  what is pinned (Keep, Studio, Downloads, open now) and the leaving-soon list with names and sizes.
  Lowering the limit below current usage confirms first: "This removes 6 files (2.1 GB) from this
  device now."
- **Commands**: `get_cache_status(server)` and `set_cache_limit(server, bytes|null)`, registered in
  the command policy, capability and frontend security ledger like the other per-server commands.
  The status is cheap: the actor keeps the last plan (usage, limit, pinned bytes, leaving-soon count
  and bytes, state) and recomputes it at most once a second after an index change, store write or
  pass. Nothing is decrypted, and the expensive storage-health scan is not involved.

### 8. Footer

A segment in the existing `<footer class="statusbar">`, on the right after the spacer and before
the fingerprint, so it stays clear of the Transfers button and tab that PR #51 is changing:

- `CACHE` label, a slim token-coloured bar (`role="progressbar"` with values) and the percentage.
- At 90% the bar turns amber and a chip appears: "4 leaving soon". Over 100% it turns red with
  "over limit"; pinned-only overflow says "held by pinned files". Paused eviction says "check
  paused". No limit shows "CACHE 3.2 GB" without a bar.
- The title text reads, for example: "2.8 GB of 5 GB used on this device. The oldest files leave
  first. Kept files, Studio work and files in use stay."
- Clicking opens the Storage page above. Hidden when no server or DM is selected.
- Data: `get_cache_status` on server switch, plus a native `cache-status` event coalesced to at most
  once a second after writes and passes.
- No animation, so `[data-motion="off"]` needs nothing extra; compact density and terminal chrome
  reuse the footer's existing rules and tokens.

### 9. Crash-safe writes

- **`put`** on the primary cache store: seal, write to
  `incoming/<cid hex>.<process id>.<sequence>` with `create_new`, `sync_all`, rename onto
  `<cid hex>`, then a directory barrier on the cache directory. The naming follows the store's
  existing `atomic_write` staging (process id plus a process-wide counter, no randomness). The
  barrier reuses `kept::persistence::directory_barrier`, which works on Unix and Windows. The
  existing rule stays: an existing record is replaced only after a bounded read proved it absent or
  invalid.
- **Only the primary cache changes.** Keep reuses `SealingBlobStore` for each `kept/<cid>/` and
  `transfers-v2/<name>/` directory, and those loaders refuse any unexpected entry, so an `incoming/`
  folder or a leftover temporary file there would fault the whole Keep store. Keep already flushes
  each chunk and re-verifies its prefix after a crash, so its stores keep today's write path. The
  atomic mode is a constructor option used only by `ServerStore::blob_store` for the primary.
- **`put_staged`**: `sync_all` before close, so promotion's rename publishes durable bytes;
  promotion adds the directory barrier. `promote_staged_bounded` already flushes.
- **Barrier failure**: the rename is already visible, so `put` logs the failed directory flush and
  succeeds. Power loss can then lose the entry (it is fetched again) but cannot leave it torn.
- **Held means verified**: `put` derives the CID from the bytes it writes, and a CID name now
  appears only after a complete synced write. `has()` remains a cheap hint as documented; legacy
  torn files longer than a sealed header still fail their read check and are repaired by `put`, as
  today.
- **Mount recovery** (in the mount scan): delete `incoming/` files whose process id is not this
  process (a vault is locked to one process, so they are abandoned; this process's own files may
  belong to a concurrent handle), and, in the sealing store, delete CID-named files shorter than
  40 bytes (24-byte nonce plus 16-byte tag, the size of a sealed empty blob), which cannot be a
  valid record. The plain test store legitimately holds an empty blob as an empty file, so it skips
  this rule.

### 10. Leave cleanup

Order, after the existing steps (actor stopped and its blob handle dropped, persistence drained,
registry row removed and the registry persisted, so a crash during cleanup cannot bring the server
back with half its cache). The work runs on a blocking worker holding the store lock:

1. Write a `leaving` marker in `blobs/<group>/` and flush it, so an interrupted cleanup resumes.
2. Run a complete Studio reference scan (`creative_pinned_cids`, off the executor under the existing
   store custody). If it fails, skip step 3 entirely, keep the marker and report the cache as kept.
3. Delete every cache file through `ProtectedBlobs::delete`, which keeps Studio pins, then clear
   `staging/` and `incoming/`.
4. Open the Keep store (its lease is free once the actor stopped), bind it to the group and this
   device as Keep requires, and release every Keep and Download record through the existing
   durable release path. Close the store, then remove the remaining `format-v2` and `lease.lock`
   files and the empty `kept/` tree (Windows cannot delete the lease file while it is held).
   Exported files outside the vault are untouched; a leftover `.mewtual-part` stays for the user.
5. `remove_server` deletes `.bin`, `.net`, `.cache` and the new `.storage`.
6. Remove the marker, and the namespace directory if it is empty (Studio-pinned blobs keep it).
7. Return `{freed_bytes, kept_bytes, failed}`. The UI shows a toast such as "Left Lantern Room.
   Freed 1.2 GB on this device." and names any kept or failed amount.

A failed delete is counted and skipped; leaving still completes. At the next unlock, any namespace
with a `leaving` marker whose group is not in the registry is finished the same way, recomputing
Studio pins first. Rejoining the same group first removes the marker and keeps the cache. If
another registry row maps to the same group (aliases exist), the namespace is left alone. Caches of
servers left before this change carry no marker and are not touched automatically.

### 11. Saved files into the cache

Per chunk, during a Download's acquisition: when a chunk has passed its ciphertext CID check and
file-layer decryption and has been written to the private Download store
(`file_transfers.rs`, the network accept path and the local private-store hit), the same ciphertext
is also stored in the shared cache with the crash-safe `put`, unless the cache already holds a
healthy copy. Keep runs do not copy, because a completed Keep copy is already served. This is the
same per-chunk standard the display path uses; the whole-file check still gates the export.

Privacy note for the guide and threat model: after a save, this device answers members' fetches for
that file's chunks, as it does for any file it has displayed, so a member asking for them can learn
this device holds the file. Saved files are ordinary cache entries afterwards: an old saved file
may be among the first to leave once the save is no longer in progress.

Contract change: tests asserting that a Download never reaches the shared cache are updated
(`private_download_uses_real_acquisition_and_protected_store_without_publication` and the
six-client recovery check). The storage-level test that the private record stays out of public
reads still holds, because the copy happens in the app layer.

### 12. Docs to correct

- `USER_GUIDE.md` §6: the circulation date is displayed only; this device keeps cached files up to
  its limit and drops the oldest listings first; Keep holds a file. Remove "nothing is ever deleted
  from anyone's device", and say that wiki-embedded and "Keep forever" files never stop circulating
  but can still leave this device's cache.
- `catcoms-storage` crate docs (`lib.rs`, `retention.rs`): the retention engine is not used by the
  app; there is no SQLCipher persistence.
- `catcoms-app` `FileExpiry` docs: expiry is still not enforced; the size limit is the only
  eviction.
- `INTERFACES.md`, `ARCHITECTURE.md` files row, `THREAT-MODEL.md` (the "sealing disk store has no
  quota" note, the new privacy note, eviction without holder proof), `HANDOVER.md` limitations, and
  `design-file-reliability.md` remaining limits.

### Out of scope, honest limits

- No holder probe, remote possession acknowledgement or automatic replication.
- Expiry is not enforced.
- Caches of servers left before this change are not cleaned automatically. Unclaimed blobs written
  during the current mount wait for the next mount.
- A server reload between Studio's `publish_pix` and the save that references it makes that frame
  an unclaimed blob of the new mount; if the cache is over its limit it can be dropped, and the save
  then fails with the existing "publish the frame PIX before saving its reference" error.
- Usage measures sealed file lengths, not filesystem allocation.
- A hostile listing can name non-file blobs it knows as chunks of an old fake file. Known references
  (profile avatars and banners, Studio pins) are pinned; anything else it names can be dropped early
  and is fetched again on demand.

## Questions for the owner

1. Default limit on at 5 GiB per server, with the upgrade never dropping anything (section 5).
2. Leaving soon = files the next 10% of the limit would push out; nearly full at 90% (section 6).
3. Wiki embeds and "Keep forever" are not cache pins, because any member can set them for everyone;
   rename "Keep forever" to "Circulate forever" (section 2)?
4. No holder probe: the limit may drop the last copy anyone has; the warning is the safeguard.
5. Keep the cache duplicate of a kept file (literal), or drop it while a completed Keep copy exists?

## Verification

- Unit tests for eviction order, the pins, and the threshold and footer counts.
- Crash tests for a torn write, the rename and verify step, and recovery at mount.
- Leave-server cleanup tests that include Studio references.
- The save-fills-cache test.
- Footer and setting UI tests and visual-fixture screenshots at 900 x 640.
- The full suites, `test:startup` and `test:flows`.
- An adversarial review of the diff, as `AGENTS.md` requires.
