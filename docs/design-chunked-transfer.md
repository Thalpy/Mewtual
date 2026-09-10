# Design; chunked large-file transfer

Status: **done, including streaming** (see § Streaming). Removes the ~16 MiB file ceiling by
splitting a file into chunks, each transferred as its own content-addressed blob, described by a
manifest; and moves those chunks across the desktop bridge one at a time so a transfer never
occupies the webview or the server actor for longer than a chunk.

See also: [`HANDOVER.md`](HANDOVER.md) fileshare; the 8l blob layer.

## Why 16 MiB was the ceiling (the problem this solved)

Before chunking, `MAX_FILE_BYTES` (catcoms-app/lib.rs) was exactly
`MAX_BLOB_RESPONSE = 16 MiB` (catcoms-sync/lib.rs:493),
the cap on a **single** blob-fetch request/response round-trip (enforced fetch-side at ~3624 and
serve-side at ~3961/3997). It exists because a fetch is one buffered frame and a 32-byte CID is a
strong request amplifier. The **storage** layer has no size limit at all; `Cid::of`, `BlobStore`,
`seal_file`/`open_file` take whole `&[u8]`. So the ceiling is purely the one-shot transport.

## The shape

- **Reuse `seal_file` per chunk.** Split the plaintext into `CHUNK_BYTES`-sized chunks (well under
  the cap so each *sealed* chunk fits one response). Seal each chunk with the existing
  `ChannelSync::seal_file` (fresh per-chunk content key, wrapped under the group file-wrap key; the
  already-reviewed 9h primitive) → a `FileRef` + ciphertext blob; `put_blob` each. No new crypto.
- **`FileManifest`** (new, in `catcoms-storage::filecrypto`) wraps the file: `{ mime, total_size,
  plaintext_cid, chunks: Vec<FileRef> }`, with `encode`/`decode`. It is stored **inline in `F_REF`**
  (like a single `FileRef` is today); it's small (N × ~150 B; a 1 GiB file at 8 MiB chunks = 128
  refs ≈ 19 KB). `plaintext_cid` is `Cid::of(whole plaintext)`; the file's stable identity.
- **File identity = `plaintext_cid`.** `add_file` returns it; `files()` reports it as `UiFile.cid`;
  `download_file(cid)` matches `manifest.plaintext_cid`; embeds (`cid:HEX`) reference it. (Was the
  single ciphertext CID; a clean change; old ciphertext-cid embeds in pre-release vaults may need
  re-insert. A legacy single-`FileRef` entry is read via a fallback: decode as `FileManifest`, else
  decode as one `FileRef` and treat it as a 1-chunk manifest, so existing files still download.)
- **`add_file`:** chunk → `seal_file` each → `put_blob` each → build + encode the manifest into
  `F_REF`. `MAX_FILE_BYTES` becomes a (larger) **total** cap; the blob cap then bounds only
  per-chunk size. Two numbers, not one: `MAX_FILE_BYTES` is the protocol ceiling, **1 GiB**
  (`crates/catcoms-app/src/lib.rs:2725`, raised from 256 MiB on 2026-09-04), past which a manifest
  does not parse; `DEFAULT_FILE_SIZE_LIMIT` (`:2733`) is the **256 MiB** policy limit a server
  starts with and its owner may set lower (never higher). Wherever this doc says 256 MiB below,
  read it as "what a default server accepts", not "what the format allows".
- **`download_file`:** decode the manifest; for each chunk `FileRef` in order; if `!has_blob`,
  `request_blob_best`; `get_blob`; `open_file` → chunk plaintext; append. Verify
  `Cid::of(reassembled) == manifest.plaintext_cid` end-to-end. Keep the five precise per-chunk
  errors; add a manifest/reassembly-mismatch variant. Each fetched chunk makes this node a holder,
  so availability spreads.

## The rate-limit change (required, security-sensitive)

The current per-requester `MIN_BLOB_INTERVAL_MS = 200ms` serves ~5 blobs/sec and returns **empty**
when throttled; indistinguishable from "not held". A rapid multi-chunk fetch from one holder would
mis-read throttle as absence and fail. Replace the single last-serve timestamp (`blob_served_at`)
with a **fixed-window bytes budget** per requester: allow up to `BLOB_BUDGET_BYTES` (96 MiB) per
`BLOB_BUDGET_WINDOW_MS` (1000 ms); a serve that would exceed it replies empty. This:
- lets a legitimate download pull many chunks back-to-back (≈96 MiB/s/holder; comparable to the
  old worst case of one 16 MiB blob/200ms ≈ 80 MiB/s), so chunked fetch works without artificial
  spacing;
- still bounds a flooder to `BLOB_BUDGET_BYTES`/window **per requester per holder** (the DoS bound,
  now in bytes not blob-count, which is the meaningful quantity);
- keeps the existing member-only + fresh-signature gate, the 64 KiB inbound request bound, the
  CID re-hash, and the response-size cap unchanged. Only the throttle accounting changes.
The per-requester map stays bounded (`max_known_peers`), now holding `(window_start, bytes)`.

## Streaming (the two deferred items above, now done)

The two "deferred" notes below turned out to be the same bug, and it was not a slow transfer: it
was the app hanging. Sharing a ~17 MiB file stuck the progress bar at 10% and froze everything
around it, because a transfer occupied **both** single-threaded surfaces end to end.

- **The webview.** `add_file` took the whole file as one base64 `invoke` argument. A 17 MiB file is
  a 23 MB JS string, serialized whole on the main thread; 256 MiB (a default server's limit) is
  341 MB, and the 1 GiB protocol ceiling is 1.37 GB.
- **The server actor.** `catcoms-app`'s actor is one `select!` loop over `(commands, sync_once)`,
  biased to commands, and each command runs to completion inline. Sealing every chunk inside one
  `AddFile` meant that for the whole upload the server drained no inbound sync and answered no
  other command, so every other UI call for that server queued behind the transfer.

Both are now bounded by a chunk instead of by the file:

- **Upload.** `begin_file_upload` → N × `push_file_chunk` → `finish_file_upload` (+
  `cancel_file_upload`). The IPC unit is a **slice** (`UPLOAD_SLICE_BYTES`, 1 MiB, mirrored as
  `TRANSFER_SLICE_BYTES` in the frontend) and the seal unit stays the **chunk** (`CHUNK_BYTES`,
  8 MiB); the bridge buffers slices until it has a chunk, which is why a slice must divide a chunk
  exactly (uniform chunks are what the media reader's `offset / CHUNK_BYTES` depends on). Each
  chunk is sealed by its own actor command (`Server::seal_upload_chunk`), so the actor returns to
  its loop between chunks. `Server::publish_upload` writes the manifest at the end. Slices are
  offset-addressed and must arrive in order, exactly once, full-size until the file ends; a
  violation fails the upload, because the running whole-file address (`CidHasher`, streaming
  BLAKE3) cannot be rewound. Dedup therefore lands *after* sealing. `publish_upload` collects
  the staged chunks only after verifying a complete locally held twin. A metadata-only twin enters
  repair: promote and verify the fresh ciphertext, then publish a compatible attested variant.
  Only a fully verified local-device row at the same name/path/CID is replaceable; other signed
  listings survive. The old chunks are collected only when no live listing references them.
- **Download.** Already one chunk per actor command (`file_download_plan` + `fetch_file_chunk`).
  What remained was the saved-file path: `download_file` returned the whole file as base64 and the
  webview handed it straight back to a `save_download` command (since removed), crossing the
  bridge twice whole.
  `save_group_file` replaces both: the bridge reserves the Downloads name, streams chunk to file,
  verifies the whole-file address, and reveals it. The plaintext never enters the webview.
  `download_file` remains for the small in-page cases (embeds, emoji, previews).

## The manifest layout invariant (security-critical)

`total_size` and `chunks` are two member-authored fields describing one thing, and originally
nothing tied them together: the declared size bounded the file and drove the UI, while the chunk
count decided how much a reader fetched, decrypted and wrote. A member could therefore declare one
byte and attach a full chunk list; the reader did the whole cap's worth of work for a file its own UI called one
byte, and the end-to-end address check did not catch it because the author simply computed that
address over the expansion. (`MAX_CHUNKS` was 4096, so the ceiling was ~32 GiB.)

So the layout is an **equality**, not a range: for a file of `total_size` there is exactly one legal
chunk list. `FileManifest::validate_layout` requires `chunks.len() == max(1, ceil(total_size /
CHUNK_BYTES))`, every non-final chunk to declare exactly `CHUNK_BYTES` and the last the remainder,
and it runs inside `FileManifest::decode` so no reader can forget it. `MAX_CHUNKS` is now the
product's true maximum, **128** (`crates/catcoms-storage/src/filecrypto.rs:212`; raised from 32
alongside the 1 GiB `MAX_FILE_BYTES`), static-asserted against `MAX_FILE_BYTES` in `catcoms-app`.
`publish_upload` validates before it posts, so this node never authors a listing its own reader
would reject. Underneath, `open_file` holds a decrypted chunk to the length its `FileRef` declared,
because `size` is the field the layout is made of. And both readers (`save_group_file`, the
all-in-one `download_file`) still bound actual bytes against the declared total independently.

The legacy un-chunked entry (one `FileRef`, no tag) predates chunking and is the one shape allowed
a chunk larger than `CHUNK_BYTES`; it is bounded at one blob-fetch response, so it cannot amplify.

## Upload identity and lifecycle

- **A generation, not the caller's id.** Sealing releases the bridge's upload-map lock across an
  actor round-trip, and the completion has to find its upload again afterwards. Keyed by the public
  upload id, a caller that restarted that id meanwhile would have the earlier generation's chunk
  attached to the new one: silent, and it produces a listing whose chunks are not the file its
  address names. `begin_file_upload` mints a fresh token and returns it in an `UploadTicket`; the
  map is keyed by it, and a completion is attached only if that generation is still waiting for
  exactly that chunk index. Anything else and the sealed blob is collected.
- **One contract, stated once.** The ticket also carries `chunk_total` and `slice_bytes`, so the
  frontend never recomputes them from its own copy of the protocol's constants. Two languages
  holding the same numbers is a drift neither language's tests can see.
- **Lock is re-checked after every await** that could have run across it: before each write of
  decrypted bytes, before the final rename/reveal, and before the irreversible index post.
- **Bounded by bytes, not just entries.** `MAX_PENDING_UPLOADS` bounds map growth;
  `MAX_STAGED_UPLOAD_BYTES` bounds the sealed-but-unpublished data itself, and an idle timeout
  collects uploads whose caller vanished (a webview reload loses the ids while the native side
  keeps running). Discard is an acknowledged actor command: cancel and lock report cleanup, so
  they must not return while deletion is merely queued.

## Staging: an upload's chunks are not held content

An upload's chunks used to be written straight into the blob store, where the only record that they
were unpublished was a map in memory. A hard exit between sealing and publishing therefore left
blobs indistinguishable from real ones: no manifest named them, and the storage-health pass walks
only what the file index references, so nothing could ever find them again.

The blob store now has a second namespace. `BlobStore::put_staged` writes into a `staging/`
subdirectory that `get`, `has` and `cids` do not see, so a staged chunk is on disk without being
*held*. `seal_upload_chunk` stages; `publish_upload` calls `promote_staged` (a rename, so even a
1 GiB file costs a directory entry rather than a rewrite); everything else drops them. Cancelling is then
safe by construction rather than by check: a staged blob is referenced by nothing, so there is no
dedup question to get wrong.

`clear_staging` runs once per server at startup, from `attach_blob_store`, before anything can
stage something new. Whatever is in staging at that moment belonged to an upload that did not
survive the last process, and the only thing that knew what it was for died with that process, so
it is unambiguously garbage. That is what makes this a sweep the store can perform on its own,
where a whole-store mark-and-sweep would have had to enumerate every blob owner and could delete
live data by missing one.

**Ordering:** promote, then post. The remaining crash window is between those two, and this
direction makes it strand orphans (blobs nothing names, collected by nothing but costing only
space) rather than a published listing whose chunks are still in staging, which the next startup
sweep would delete out from under the only device that holds them.

## Reading a file inline is bounded

`download_file` returns a whole file as one base64 string, which is the same shape that froze the
app on upload: cost scales with the file, and the webview serializes it whole on its main thread.
It was reachable without any deliberate action, because embeds, custom emoji, event posters, card
thumbnails and file previews all went through it, so scrolling past a message containing a large
file was enough.

Everything visual now renders from the `catcoms-media:` protocol instead (`sharedMediaUrl`), which
answers Range requests off the vault and fetches missing chunks from peers exactly as before, so
the element streams and the bytes never become a JS string. `download_file` keeps exactly three
callers, all of which genuinely need bytes in JS: the text reader, the take deck and the patch
loader (`apps/desktop/src/inline-transfer.test.ts:102` asserts the count, so a fourth has to be
argued for rather than slipped past). Each is bounded by a soft cap in the UI (2 MiB for the text
reader) and by `MAX_INLINE_DOWNLOAD_BYTES` (16 MiB) natively, so no "read it anyway" button can
pull a gigabyte listing into the window. The native bound is meaningful only because the manifest layout check
above makes a listing's declared size trustworthy.

## Saving is staged

`save_group_file` reserves the final Downloads name, writes into a sibling `.part`, verifies size
and whole-file address, and only then renames. The peer-chosen filename never exists holding bytes
this device has not authenticated, so a crash or kill mid-transfer leaves a `.part` rather than
something that looks like the finished file.

## Scope / deferred

- **No transport-protocol change:** the per-blob RR codec, signing, member gate, and CID re-verify
  are unchanged; chunking is additive above them.
- **Still no parallel fan-out:** a chunk is fetched from one holder at a time. Failover across
  holders *has* landed: a read job builds a candidate list of `(FileRef, PeerId)` pairs across
  every blob-fetch peer and walks it, dropping a reference a provider could not authenticate
  (`crates/catcoms-app/src/actor/file_transfers.rs:109,533,542,679`). What does not exist is
  fetching several chunks from several holders *at once*, or a holder index. Resumable-across-
  restart transfers are also still a follow-up, so a failed 1 GiB transfer restarts from zero.
- **Actor latency on the write path remains:** chunk sealing, local I/O and network waits for an
  *upload* still execute on the actor. Publication verifies complete local possession and the
  whole-file hash before reuse or success, holding only one plaintext chunk at a time but
  potentially occupying the actor for a whole file. Queue/provider/decrypt timing instrumentation
  is still a follow-up.
- **Bounded off-actor transfer tasks: done.** Reads no longer suspend on the network while
  borrowing the `Server`. `crates/catcoms-app/src/actor/file_transfers.rs:17-25` runs each fetch
  attempt as a worker holding an opaque signed request, admitted by two semaphores (`4` attempts
  per server, `8` per process) with an `8s` per-attempt deadline and a `60s` read deadline; the
  actor only prepares and commits each step.
- **The promote/post window (small, open).** A crash between promoting an upload's chunks and
  posting its index entry leaves those chunks held but unnamed. Unlike the pre-staging behaviour
  this window includes complete local verification after promotion; it costs space rather than
  correctness. Closing it entirely needs the index post and the promotion to be one atomic step,
  which they cannot be while one is a CRDT operation and the other is a filesystem rename.

## Security

Per chunk, the design inherits the reviewed 8l/9h properties: member-only serve+fetch, the response
signature bound to `(group_id, requester_pubkey, ts, nonce, epoch)`, `Cid::of(blob)==cid` re-hash
before store, and AEAD on unseal. A tampered/substituted chunk fails the ciphertext CID re-hash (at
fetch) or the AEAD tag (at open); a reordered/short manifest fails the final `plaintext_cid` check.
The manifest is in the encrypted channel CRDT (members-only). The bytes-budget keeps the DoS bound;
the adversarial review focuses on the rate-limit change + the reassembly/verify path.
