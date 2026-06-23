# Design — chunked large-file transfer

Status: **design approved, implementing.** Removes the ~16 MiB file ceiling by splitting a file
into chunks, each transferred as its own content-addressed blob, described by a manifest.

See also: [`HANDOVER.md`](HANDOVER.md) fileshare; the 8l blob layer.

## Why 16 MiB today

`MAX_FILE_BYTES` (catcoms-app/lib.rs) is exactly `MAX_BLOB_RESPONSE = 16 MiB` (catcoms-sync/lib.rs:84),
the cap on a **single** blob-fetch request/response round-trip (enforced fetch-side at ~3624 and
serve-side at ~3961/3997). It exists because a fetch is one buffered frame and a 32-byte CID is a
strong request amplifier. The **storage** layer has no size limit at all — `Cid::of`, `BlobStore`,
`seal_file`/`open_file` take whole `&[u8]`. So the ceiling is purely the one-shot transport.

## The shape

- **Reuse `seal_file` per chunk.** Split the plaintext into `CHUNK_BYTES`-sized chunks (well under
  the cap so each *sealed* chunk fits one response). Seal each chunk with the existing
  `ChannelSync::seal_file` (fresh per-chunk content key, wrapped under the group file-wrap key — the
  already-reviewed 9h primitive) → a `FileRef` + ciphertext blob; `put_blob` each. No new crypto.
- **`FileManifest`** (new, in `catcoms-storage::filecrypto`) wraps the file: `{ mime, total_size,
  plaintext_cid, chunks: Vec<FileRef> }`, with `encode`/`decode`. It is stored **inline in `F_REF`**
  (like a single `FileRef` is today) — it's small (N × ~150 B; a 256 MiB file at 8 MiB chunks = 32
  refs ≈ 5 KB). `plaintext_cid` is `Cid::of(whole plaintext)` — the file's stable identity.
- **File identity = `plaintext_cid`.** `add_file` returns it; `files()` reports it as `UiFile.cid`;
  `download_file(cid)` matches `manifest.plaintext_cid`; embeds (`cid:HEX`) reference it. (Was the
  single ciphertext CID — a clean change; old ciphertext-cid embeds in pre-release vaults may need
  re-insert. A legacy single-`FileRef` entry is read via a fallback: decode as `FileManifest`, else
  decode as one `FileRef` and treat it as a 1-chunk manifest, so existing files still download.)
- **`add_file`:** chunk → `seal_file` each → `put_blob` each → build + encode the manifest into
  `F_REF`. `MAX_FILE_BYTES` becomes a (larger) **total** cap (256 MiB); the blob cap then bounds only
  per-chunk size.
- **`download_file`:** decode the manifest; for each chunk `FileRef` in order — if `!has_blob`,
  `request_blob_best`; `get_blob`; `open_file` → chunk plaintext; append. Verify
  `Cid::of(reassembled) == manifest.plaintext_cid` end-to-end. Keep the five precise per-chunk
  errors; add a manifest/reassembly-mismatch variant. Each fetched chunk makes this node a holder,
  so availability spreads.

## The rate-limit change (required, security-sensitive)

The current per-requester `MIN_BLOB_INTERVAL_MS = 200ms` serves ~5 blobs/sec and returns **empty**
when throttled — indistinguishable from "not held". A rapid multi-chunk fetch from one holder would
mis-read throttle as absence and fail. Replace the single last-serve timestamp (`blob_served_at`)
with a **fixed-window bytes budget** per requester: allow up to `BLOB_BUDGET_BYTES` (96 MiB) per
`BLOB_BUDGET_WINDOW_MS` (1000 ms); a serve that would exceed it replies empty. This:
- lets a legitimate download pull many chunks back-to-back (≈96 MiB/s/holder — comparable to the
  old worst case of one 16 MiB blob/200ms ≈ 80 MiB/s), so chunked fetch works without artificial
  spacing;
- still bounds a flooder to `BLOB_BUDGET_BYTES`/window **per requester per holder** (the DoS bound,
  now in bytes not blob-count, which is the meaningful quantity);
- keeps the existing member-only + fresh-signature gate, the 64 KiB inbound request bound, the
  CID re-hash, and the response-size cap unchanged. Only the throttle accounting changes.
The per-requester map stays bounded (`max_known_peers`), now holding `(window_start, bytes)`.

## Scope / deferred

- **Actor-blocking:** `download_file` stays a synchronous command; a 256 MiB download (~a few
  seconds at the new budget) blocks that server's actor for its duration. A background/streaming
  download with progress events, GB-scale files, multi-holder fan-out, and a holder index are
  follow-ups. 256 MiB covers typical photos/video/docs.
- **No transport-protocol change:** the per-blob RR codec, signing, member gate, and CID re-verify
  are unchanged — chunking is additive above them.
- **No bridge/frontend change:** `add_file`/`download_file` keep their signatures (the cid is passed
  opaquely), so chunking is transparent to the desktop app. The Tauri IPC still moves the whole file
  as one base64 buffer, so practical desktop limits are bounded by IPC/RAM, not the protocol —
  streaming the chunks across IPC (and a progress bar) is the same background-download follow-up.

## Security

Per chunk, the design inherits the reviewed 8l/9h properties: member-only serve+fetch, the response
signature bound to `(group_id, requester_pubkey, ts, nonce, epoch)`, `Cid::of(blob)==cid` re-hash
before store, and AEAD on unseal. A tampered/substituted chunk fails the ciphertext CID re-hash (at
fetch) or the AEAD tag (at open); a reordered/short manifest fails the final `plaintext_cid` check.
The manifest is in the encrypted channel CRDT (members-only). The bytes-budget keeps the DoS bound;
the adversarial review focuses on the rate-limit change + the reassembly/verify path.
