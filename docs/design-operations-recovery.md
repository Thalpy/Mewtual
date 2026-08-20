# Operations, recovery, and durable local history

Status: implementation contract for phases 12b-12d.

## Storage health and repair

The server storage view reports only facts the node can verify locally:

- listed files and referenced chunks;
- chunks whose sealed-at-rest record opens, matches its CID, and opens under its file reference;
- missing chunks, unreadable/corrupt chunks, and invalid manifests;
- whether any authenticated member connection currently exists from which repair can be attempted.

“Repair” re-fetches missing or unreadable chunks through the existing members-only, signed,
request-bound blob response. The response is content-address verified before it replaces an
unreadable local record. It never deletes a merely unreferenced blob: avatars, banners, livery media,
deduplicated files, and future surfaces share the same store, and last-copy-safe retention is not yet
implemented. A failed repair leaves the item explicitly missing/unreadable.

The Transfers surface repeats the concise health summary because transfer failures and storage
failures are otherwise too easy to confuse. The dedicated sidebar control opens the full report.

The desktop bridge performs at most one ordinary verification scan per server per process session
and caches the report across frontend remounts. File events and repeated navigation do not trigger
another scan. Explicit repair is the exception: its security contract requires a post-recovery
verification pass, and that result replaces the cache. The report also deduplicates whole-file CIDs,
lists the ten largest files, and groups logical/local-estimated bytes by content type and wiki pin.
Only `verified_bytes` is an exact authenticated ciphertext count; category/local totals are labelled
estimates based on held-chunk ratios and must not be described as filesystem allocation.

## Connectivity assistant

The assistant reuses the bridge's existing three-state diagnostic. It may say an action succeeded,
failed, or was merely started; it may not equate a listen address, UPnP mapping, rendezvous
registration, or issued dial with proven inbound reachability. Current authenticated live-member
connections are reported separately from the last onboarding attempt.

The assistant offers copyable evidence and concrete next steps: check that another member is
online, configure rendezvous/relay, inspect the operator's join log, or enable the privacy-labelled
debug log. AutoNAT remains a tracked hardening item.

## Vault-sealed durable UI state

Composer drafts and per-channel read marks move from plaintext web storage/in-memory state to one
bounded `ui-state.bin` record sealed with the vault database key. The frontend sends a versioned JSON
object; the bridge accepts at most 1 MiB and the store authenticates it at rest. It is local to this
device and is never replicated to peers.

On first use, a valid legacy read-mark map may be migrated from local storage. The plaintext copy is
removed only after the sealed save succeeds. Locking clears the visible copy; unlocking reloads it.

## Backup and recovery centre

A backup is a coherent persisted cut of the encrypted vault directory after every running server
and the registry have been snapshotted. It includes the passphrase-wrapped vault, sealed server/network
records, blob stores, pairing ledger, and sealed UI state. It is written to a fresh Downloads folder
without replacing an earlier backup and is never decrypted for export.

The current centre reports the completed copy's file/byte count and location. Source records are
freshly persisted through their normal authenticated writers, but this slice does **not** claim a
post-copy cryptographic verification or expose an import action. A backup cannot prove every remote
member still holds missing blobs and cannot recover a forgotten passphrase; it requires the vault
secret that protected it.

Export does not reduce the AEAD/KDF strength of any copied record, but it increases exposure. The
new directory is another offline Argon2 guessing target; its directory names, filenames, sizes,
timestamps and blob layout remain filesystem-visible; it freezes historical state/key material that
may later be deleted from the live vault; and later changing the live secret cannot revoke the old
copy. Per-record authentication detects tampering when records are opened, but this slice still has
no separately verified manifest covering the backup as one complete artifact.

In-app destructive restore is deliberately not part of this slice: it must work from the locked
screen when the active vault is corrupt, stage into a separate directory, verify with the supplied
secret, and retain a rollback copy before an atomic swap. Until that reviewed flow lands, the centre
opens the backup folder and tells the operator to preserve the source copy; it does not publish a
manual live-overwrite recipe.

## Vault-secret rotation

`change_vault_passphrase` authenticates the current secret, generates a fresh salt and nonce, and
atomically replaces only `vault.bin` with the same random root DEK rewrapped by the new Argon2-derived
key. Database/blob/MLS subkeys are derived from the DEK, not from the human secret, so all existing
sealed records remain openable and a bulk half-rekeyed tree is impossible. The generated wrapper is
opened once before replacement. Empty, oversized and unchanged secrets are rejected; the desktop
holds the store mutex so rotation cannot race a snapshot or backup.

Passphrase, sigil and melody entry all produce the same bounded secret string. The frontend keeps
current/new values only for the three-step ceremony and clears them on success, failure, cancel or
session lock. This is best-effort in JavaScript memory, not a live-process compromise defence.

## Antagonist review checklist

1. Corrupt a sealed blob, snapshot, network record, registry, and UI state independently.
2. Ensure health never labels a physical-but-unreadable blob “ready”.
3. Ensure repair accepts only request-bound, member-signed bytes matching the requested CID.
4. Lose connectivity midway through repair; no valid local copy may be destroyed.
5. Inject traversal names/symlinks into a backup source; export must not follow them.
6. Race a mutation with backup; atomic source files and the store lock must yield valid old or new
   records, never partial files.
7. Feed oversized/malformed durable state; reject it without replacing the last valid record.
8. Verify that drafts/read marks are absent from plaintext web storage after migration.
9. Before import ships, verify a backup from another vault is rejected without exposing its contents.
10. Confirm every recovery screen states that backups do not recover a forgotten secret.
11. Change the secret, prove all derived data keys are unchanged, and prove the old secret no longer
    opens the live vault.
12. Fail rotation with a wrong current secret and verify `vault.bin` is byte-for-byte unchanged.
13. Keep an old export after rotation and confirm the UI says it still uses the old secret.
