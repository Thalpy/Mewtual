# Private sealed page DATA (5 November; authored, unexecuted)

`catcoms-storage::private_chat_page` is an encryption/data prerequisite for the private Chat V2
archive. It implements no paths, filesystem writes, BlobStore/file service, critical namespace
handler, canonical root, resource account or privileged read command. ARCHIVE ID2 remains rejected.
The surrounding archive/journal/CAS consumer is still required before any file is produced.

## Exact format and cryptographic separation

The complete frame is:

| Field | Encoding |
| --- | --- |
| Domain | u32 length36, `catcoms/private-chat-sealed-page/v1\0` |
| Version | u16 big-endian1 |
| Original nonce |24 raw bytes |
| Ciphertext | u32 length, exact XChaCha20-Poly1305 ciphertext including16-byte tag |

Plaintext length is1..65536; complete frame length is plaintext+86, at most65622 bytes. No
trailing bytes, different domain/version, alternative length framing or empty plaintext is
accepted. The borrowed parser performs no variable-size copy/allocation.

Both prepare and open internally derive the32-byte encryption key as
`BLAKE3 derive_key("catcoms/private-chat-page-key/v1", original_vault_db_key)`. The ephemeral
derived key is zeroizing. There is no base-db-key fallback or caller-supplied already-derived-key
mode. This is a new private-page storage format; it changes neither the existing DEK hierarchy
nor any old snapshot/file format. Existing `seal` uses no associated data, so strict outer domain
framing alone would not stop a db-key snapshot ciphertext being rewrapped into a page. The
dedicated key prevents that substitution in both directions.

`PrivateChatPageId` is BLAKE3(raw `catcoms/private-chat-page-id/v1\0` || COMPLETE frame). It is a
distinct private ciphertext type, not a public BlobStore Cid, semantic ActionId, header ID or
acceptance proof. Its digest includes the original nonce. `PreparedPrivateChatPage` owns one
immutable sealed frame; retries borrow exactly those bytes and ID. Resealing identical plaintext
creates a new candidate and cannot replace an uncertain pending journal entry.

Open requires an EXTERNAL expected page ID and checks maximum size, complete framing and whole
expected ID before copying ciphertext or invoking AEAD. A successful result is zeroizing inner
DATA only. The actual selected authenticated manifest/root must supply that expected ID; taking
an ID from the same untrusted input authenticates no source. The consumer must then validate the
inner page's exact vault/group/Profile/kind, full original-record geometry/signature and applicable
historical/current authority. Native admin authorization is a separate prerequisite.

## Buffer invoice and missing integration

For plaintext length p, preparation overlaps caller input, AEAD ciphertext p+16 and the final
frame p+86. Opening borrows caller raw bytes, copies ciphertext p+16 and can allocate AEAD output
with capacity p+16 even though its successful length is p. The original prepared/journal buffers,
read capacity, inner parsers/typed output, fixed hash/crypto state and any subsequent encoding
overlap must also be charged. Zeroizing wrapper does not release a fee or bound an SDK heap.

This module has no account/lease and claims no whole-host physical memory bound. The actual Store
must reserve before read/seal/copy/AEAD, retain original fees through lower copies and file barriers,
and mount the supported critical archive namespace before any dependent I/O. There is no read,
write, saved/current capability or raw-file recovery constructor here.

Five regressions are authored for0/1/65536/65537 plaintext boundaries and no RNG on refusal,
original nonce/frame retry, expected-ID-before-crypto, nonce/tag tampering, wrong keys,
cross-protocol ciphertext substitution, strict lengths/domain/version/EOF/truncation and distinct
private/public IDs. They are **UNRUN** under the user compilation pause. Direct rustfmt parsing,
static checks and read-only adversarial review are not compiled or runtime evidence.
