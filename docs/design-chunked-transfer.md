# Design; chunked large-file transfer

Status: **done, including streaming** (see § Streaming). Removes the ~16 MiB file ceiling by
splitting a file into chunks, each transferred as its own content-addressed blob, described by a
manifest; and moves those chunks across the desktop bridge one at a time so a transfer never
occupies the webview or the server actor for longer than a chunk.

See also: [`HANDOVER.md`](HANDOVER.md) fileshare; the 8l blob layer.

## Why 16 MiB today

`MAX_FILE_BYTES` (catcoms-app/lib.rs) is exactly `MAX_BLOB_RESPONSE = 16 MiB` (catcoms-sync/lib.rs:84),
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
  (like a single `FileRef` is today); it's small (N × ~150 B; a 256 MiB file at 8 MiB chunks = 32
  refs ≈ 5 KB). `plaintext_cid` is `Cid::of(whole plaintext)`; the file's stable identity.
- **File identity = `plaintext_cid`.** `add_file` returns it; `files()` reports it as `UiFile.cid`;
  `download_file(cid)` matches `manifest.plaintext_cid`; embeds (`cid:HEX`) reference it. (Was the
  single ciphertext CID; a clean change; old ciphertext-cid embeds in pre-release vaults may need
  re-insert. A legacy single-`FileRef` entry is read via a fallback: decode as `FileManifest`, else
  decode as one `FileRef` and treat it as a 1-chunk manifest, so existing files still download.)
- **`add_file`:** chunk → `seal_file` each → `put_blob` each → build + encode the manifest into
  `F_REF`. `MAX_FILE_BYTES` becomes a (larger) **total** cap (256 MiB); the blob cap then bounds only
  per-chunk size.
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
  a 23 MB JS string, serialized whole on the main thread; 256 MiB (the declared cap) is 341 MB.
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
  BLAKE3) cannot be rewound. Dedup therefore lands *after* sealing, so `publish_upload` collects
  the redundant chunks it just wrote (`discard_upload_chunks`, dedup-safe like `delete_file`'s GC).
- **Download.** Already one chunk per actor command (`file_download_plan` + `fetch_file_chunk`).
  What remained was the saved-file path: `download_file` returned the whole file as base64 and the
  webview handed it straight back to `save_download`, crossing the bridge twice whole.
  `save_group_file` replaces both: the bridge reserves the Downloads name, streams chunk to file,
  verifies the whole-file address, and reveals it. The plaintext never enters the webview.
  `download_file` remains for the small in-page cases (embeds, emoji, previews).

## Scope / deferred

- **No transport-protocol change:** the per-blob RR codec, signing, member gate, and CID re-verify
  are unchanged; chunking is additive above them.
- **Still one holder at a time:** multi-holder fan-out, a holder index, GB-scale files and
  resumable-across-restart transfers are follow-ups. 256 MiB covers typical photos/video/docs.
- **Sealing is still on the async runtime:** a chunk's AEAD + blob write runs on the actor's
  thread rather than `spawn_blocking`. At 8 MiB that is tens of milliseconds per chunk, which the
  loop absorbs; it only matters if `CHUNK_BYTES` grows.

## Security

Per chunk, the design inherits the reviewed 8l/9h properties: member-only serve+fetch, the response
signature bound to `(group_id, requester_pubkey, ts, nonce, epoch)`, `Cid::of(blob)==cid` re-hash
before store, and AEAD on unseal. A tampered/substituted chunk fails the ciphertext CID re-hash (at
fetch) or the AEAD tag (at open); a reordered/short manifest fails the final `plaintext_cid` check.
The manifest is in the encrypted channel CRDT (members-only). The bytes-budget keeps the DoS bound;
the adversarial review focuses on the rate-limit change + the reassembly/verify path.
