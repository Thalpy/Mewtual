# Disk persistence + encryption-at-rest; design

Status: **implemented (9a–9h ✅).** This scoped the work into reviewable, security-critical
slices; all are done (9c, 9e, 9h-b adversarially reviewed). The desktop app persists each
server sealed at rest under a launch passphrase, reloads on startup, re-dials peers, and
e2e-encrypts files. The original "entirely in-memory" state is closed.

## Goals / non-goals

**Goals.** A member's **servers, channels, member profiles, files, status, wiki** survive an
app restart; the persisted state is **encrypted at rest**; the app can **read history
offline** (no peer needed to see what you already have).

**Non-goals (first cut).** Cross-device sync of one identity (multi-device); cloud backup;
key escrow/recovery. A forgotten passphrase = data loss, by design.

## What must persist

Per server (one MLS group):

- **Device identity**; the `SignatureKeyPair` private key (`MlsDevice`, `device.rs:26`).
  Everything else in `MlsDevice` (credential, `device_id`) re-derives from it.
- **MLS group state**; the ratchet tree, epoch secrets, pending commits. *(The hard part;
  see below.)*
- **CRDT documents**; `EncryptedDoc` per `(DocType, doc_id)`: an automerge `AutoCommit` +
  the signed-op log (`doc.rs:24`). Channels, profile, file-index, status, wiki.
- **`ChannelSync` durable state** (`lib.rs:1257`): `commit_log`, `routing_label` +
  `routing_secrets` (`Zeroizing`), `peer_records`, `ledger`. (The rest; transport, topics,
  caches, rate-limit maps; is transient and rebuilt.)
- **Blob store**; avatars + downloaded files (`FsBlobStore`, already on disk by CID).

App-level:

- **Server registry**; the set of servers (their on-disk location, the founder's invite,
  the display name) + which to reload on startup.
- **Peer addresses**; to re-dial on reload (P2P has no DHT here yet).

## The pivotal constraint: openmls has no group snapshot

The `MlsGroup` (wrapped by `ServerGroup`, `group.rs:64`) is backed by `OpenMlsRustCrypto`'s
**in-memory `MemoryStorage`** (`device.rs:36`). openmls 0.8 has **no `MlsGroup::save()/load()`**
— the group state lives only in that provider's storage. Two ways to persist it:

- **A. Persistent `StorageProvider` (recommended).** openmls reads/writes all group state
  through the `StorageProvider` trait. Implement a **sealed, on-disk** provider (a key→value
  store where each value is `seal`ed under `mls_seal_key`). The group then "just persists";
  no replay, no snapshot API needed. Cost: a correct openmls storage provider is the single
  biggest, most delicate piece (it must satisfy openmls's read-your-writes + key lifecycle
  expectations); it needs its own tests against openmls.
- **B. Commit-log replay (fallback).** Persist the `commit_log` (+ the device, + a joiner's
  Welcome) and **rebuild** the group on startup by replaying commits. Simpler storage, but
  group reconstruction is fragile and slow, and openmls doesn't expose a clean "apply this
  historical commit to a fresh group" path. Use only if A proves intractable.

**Decision: pursue A.** The sealed on-disk `StorageProvider` is the clean long-term answer
and the at-rest sealing falls out of it for free (every stored value is sealed).

## At-rest sealing (already have the primitives)

`catcoms-crypto` provides the whole hierarchy (`keystore.rs`):

- **Root `Dek`** (32 bytes) → `KeyHierarchy` with `db_key()` / `mls_seal_key()` / `blob_key()`
  (HKDF-Expand subkeys, `keystore.rs:154`).
- **`seal`/`unseal`** = XChaCha20-Poly1305, 24-byte random nonce per seal (`keystore.rs:121`).
- **`SecureKeyStore`** seals the root `Dek` itself: `PassphraseKeyStore` (Argon2id, **implemented**),
  `InMemoryKeyStore` (test), and a `KeyTier` ladder (Hardware/OsSoftware/Passphrase/None) with
  **downgrade detection** (`requires_passphrase_confirmation`). OS-keychain/hardware tiers are
  enum-ready but not yet implemented; **passphrase is the at-rest protection for v1.**

So: on startup the user supplies a **passphrase** → unseal the `Dek` → derive `KeyHierarchy`.
MLS storage sealed under `mls_seal_key`, docs + sync-state under `db_key`, blobs under
`blob_key` (or per-file via the file-wrap-key, slice 9h).

## Transport re-establishment on reload (P2P)

Restoring state is necessary but not sufficient; connections are gone. On reload:

- **Founder**; re-bind + re-advertise exactly like founding; other members re-dial it. No
  reconnection logic needed; the invite's bootstrap still points here (if the address is
  stable); a changed LAN/public IP needs a fresh invite (already true today).
- **Joiner**; must reconnect. Persist the **dialable peer addresses** (the signed
  `peer_records` already carry member multiaddrs) and **re-dial on startup**; reconnect as
  peers come online. Rendezvous re-discovery (the deferred networking slice) is the better
  long-term answer. Until a peer is reachable, the joiner **reads its persisted history
  offline**; which is itself a real win.

## Slice breakdown

Each slice is **security-critical** (handles secret material at rest) → adversarial review
before commit, per project discipline. Suggested order:

| Slice | What | Risk |
|------|------|------|
| **9a ✅** | **Key vault** (`catcoms-storage::open_or_create_vault`); passphrase-sealed root `Dek` on disk (`PassphraseKeyStore` Argon2id + sealed file) → `KeyHierarchy`. *Done; wiring the startup passphrase prompt into the app is part of 9f.* | low–med (UX + key handling) |
| **9b ✅** | **Sealing blob store** (`catcoms-storage::SealingBlobStore`); every blob `seal`ed at rest under `blob_key`; content-addressed by **plaintext** CID so the mesh fetch is unchanged (seal at the disk boundary, not the wire). *Done; wiring it into `ChannelSync` per server is part of 9e/9f.* | med |
| **9c ✅** | **Snapshottable MLS state** (`catcoms-mls::snapshot_server`/`restore_server`); openmls 0.8 has no group snapshot, so we serialize the provider's `MemoryStorage` (public KV map) + signer pubkey + group id, and reload via `MlsGroup::load` (no 40-method `StorageProvider` impl). Adversarially reviewed (no blocking findings; verified the storage snapshot captures complete group state incl. pending commits). *Done; sealing it under `mls_seal_key` + the per-server snapshot cadence is 9e/9f.* | **high** |
| **9d ✅** | **Doc persistence** (`EncryptedDoc::snapshot`/`restore`); `AutoCommit::save()` + the signed-op log (rebuilds the `applied` dedup set), framed with the wire codec. Per-op signatures still verified on use, so a tampered snapshot can't inject forged history. *Done; sealing under `db_key` + restoring `ChannelSync`'s `docs` map is 9e.* | med |
| **9e ✅** | **Sync-state persistence** (`ChannelSync::snapshot`/`restore`); assembles the MLS state (9c) + every doc (9d) + `routing_label`/`routing_secrets` + `ledger` + `commit_log` + `peer_records` into one `Zeroizing` blob; reload reconstitutes `ChannelSync` on a **fresh** transport (`adopt_routing_state` recomputes identical topics). Adversarially reviewed (no blocking findings; durable set complete, invite-ledger round-trip closes the cross-restart double-redeem). *Done; sealing the blob under the vault key + writing it to disk is 9f.* | med–high (secrets) |
| **9f ✅** | **Registry + reload-on-startup.** `catcoms-app::store::ServerStore` (vault-sealed `servers/<id>.bin` + `registry.bin`, atomic writes, wrong-passphrase-safe) + `Server::snapshot`/`restore` + the actor `Snapshot` command. Bridge: a launch **passphrase gate** (`unlock` → open vault → reload each server onto a fresh transport → repopulate the rail) and **save-on-mutation** (seal after every found/join/send/profile/file/status/wiki, remove on leave). The desktop app now survives a restart: close it, reopen, enter the passphrase, your servers + full history are back (read offline). *Caveat:* a reloaded founder gets a new port, so new joiners need a fresh invite (existing limitation); peer re-dial is 9g. | med |
| **9g ✅** | **Transport re-establishment**; `peer_addrs_from_snapshot` extracts persisted public peer multiaddrs from a snapshot (no full restore; the bridge needs them before building the mesh). Post-join discovery later moved these through the bounded cache/scheduler. `ServerNet` v3 now also seals at most two direct-IP routes that a joiner actually completed and Noise-authenticated to the named inviter, together with explicit `Disabled` / `AuthorizedPeer` / `LegacyPending` provenance. Direct admission fetches the inviter's signed descriptor before the first post-join snapshot when bounded PEX succeeds; helper/reply/switchboard admissions are disabled, and legacy migration is limited to an unambiguous two-member overlap. Reload installs the route into `ChannelSync`, which rechecks canonical peer binding, a unique current roster claim, raw TCP/QUIC host shape and the shared dial scheduler. Thus an established same-LAN joiner can reconnect after close/reopen when the inviter keeps the same listener address without publishing the retained private route through PEX. This does not discover a new/changed LAN peer; mDNS or rendezvous remains necessary for that. | med |
| **9h ✅** | **Per-file encryption-at-rest**; two slices. **9h-a:** wired `SealingBlobStore` (over `FsBlobStore`) into each server under the vault `blob_key`, so files + avatars persist + are sealed at rest. **9h-b:** a **stable per-group file-wrap key** minted at founding, transferred at join **bundled into the routing transfer** (sealed under `routing_transfer_key`); `seal_file`/`open_file` so files are ciphertext keyed by the **ciphertext** CID with the wrapped key in the encrypted index; e2e, openable only by members holding the key. Adversarially reviewed (no blocking; joiner-key zeroing folded). | **high** (key mgmt + join handshake) |

**Progress: 9a–9f done**; "survive restart, encrypted at rest" is delivered end-to-end: the
vault, sealing blob store, MLS snapshot, doc snapshot, sync-state assembly, the vault-sealed
`ServerStore`/registry, and the desktop passphrase-gate + reload-on-startup (9c & 9e
adversarially reviewed; all Rust tested, the app verified via cargo check + svelte-check).
Close the app, reopen, enter your passphrase → your servers + history are back, read offline.
**Phase 9 is complete (9a–9h).** Disk persistence + encryption-at-rest is delivered end-to-end:
servers/channels/profiles/files/status/wiki survive a restart, sealed under a passphrase, read
offline; a reloaded joiner re-dials its peers; files are e2e-encrypted under a stable per-group
key (9c, 9e, 9h-b adversarially reviewed). Threat model below stands: at-rest + e2e, not
anti-malware. Future: rotating file keys for removed-member file forward-secrecy, rendezvous
re-discovery for moved peers, chunked large-file transfer.

9a–9f deliver "survive restart, encrypted at rest." 9g makes a reloaded joiner reconnect.
9h is the file-encryption follow-up that was correctly deferred until persistence exists.

## Threat model (be honest in the UI)

**At-rest encryption protects against:** a stolen disk / laptop, a leaked backup or
cloud-synced app-data folder, another OS user reading your files. The passphrase (Argon2id)
gates the `Dek`; without it the on-disk state is opaque.

**It does NOT protect against:** a live process compromise (RAM has the unsealed keys while
running), malware running as you, a keylogger capturing the passphrase, or a compromised OS.
This is the same envelope as Signal/desktop messengers; at-rest, not anti-malware.

## Open questions / risks

- **openmls `StorageProvider` correctness** (9c) is the principal risk; get it wrong and the
  group silently corrupts. Needs property tests + round-trip tests against openmls operations.
- **On-disk format versioning / migration**; tag every sealed blob with a version.
- **Atomic writes**; write-temp-then-rename to survive a crash mid-write; never leave a
  half-written group store.
- **Passphrase UX**; prompt on launch; "forgot" = data loss (no recovery in v1). Consider an
  OS-keychain tier (`KeyTier::OsSoftware`) later so the passphrase isn't needed every launch.
- **Concurrency**; multiple servers persisting concurrently; a per-server lock / single
  writer.

## Files this touches

`crates/catcoms-crypto/src/keystore.rs` (keystore, already built), `crates/catcoms-mls`
(device + group provider, 9c), `crates/catcoms-replication/src/doc.rs` (doc save/load, 9d),
`crates/catcoms-sync/src/lib.rs` (`ChannelSync` snapshot + `FsBlobStore` wiring, 9b/9e),
`crates/catcoms-storage/src/blob.rs` (sealing blob store, 9b), `crates/catcoms-app` +
`apps/desktop/src-tauri` (registry, reload, passphrase prompt, 9a/9f/9g).
