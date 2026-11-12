//! Bounded encrypted private chat page DATA, deliberately separate from [`crate::BlobStore`].
//!
//! There is no path, network service, filesystem writer or namespace activation here. A caller
//! must obtain its expected page identity from the actual selected private manifest/root, pay
//! for all overlapping buffers, and validate the decrypted inner vault/group/Profile/kind.
//! Successful decryption alone grants neither historical/current authority nor admin access.
use crate::StorageError;
use catcoms_crypto::{seal, unseal, SealedBlob};
use catcoms_rt::CryptoRngCore;
use zeroize::Zeroizing;

const FRAME_DOMAIN: &[u8] = b"catcoms/private-chat-sealed-page/v1\0";
const ID_DOMAIN: &[u8] = b"catcoms/private-chat-page-id/v1\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-page-key/v1";
const VERSION: u16 = 1;
const NONCE_BYTES: usize = 24;
const AEAD_TAG_BYTES: usize = 16;
/// Fixed u32 domain length, domain, u16 version, raw nonce and u32 ciphertext length.
const FRAME_OVERHEAD: usize = 4 + FRAME_DOMAIN.len() + 2 + NONCE_BYTES + 4 + AEAD_TAG_BYTES;

/// Limit for one inner encoded action-chunk/node/manifest/batch page, not a whole host invoice.
pub const MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES: usize = 64 * 1024;
/// Exact maximum complete sealed frame; the complete original nonce participates in its ID.
pub const MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES: usize =
    FRAME_OVERHEAD + MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES;

/// Private ciphertext identity, intentionally not a public file Cid or semantic chat record ID.
/// Wrapping bytes authenticates nothing: the selected original manifest must supply the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrivateChatPageId([u8; 32]);
impl PrivateChatPageId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    fn of(frame: &[u8]) -> Self {
        let mut hash = blake3::Hasher::new();
        hash.update(ID_DOMAIN);
        hash.update(frame);
        Self(*hash.finalize().as_bytes())
    }
}

/// One immutable original sealed candidate. A retry borrows these SAME bytes; resealing the
/// same inner page creates a different candidate and cannot replace an uncertain journal entry.
/// This object is DATA, not proof of file durability, resource ownership or a mounted writer.
pub struct PreparedPrivateChatPage {
    frame: Zeroizing<Vec<u8>>,
    id: PrivateChatPageId,
}
impl std::fmt::Debug for PreparedPrivateChatPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedPrivateChatPage")
            .field("frame_bytes", &self.frame.len())
            .finish_non_exhaustive()
    }
}
impl PreparedPrivateChatPage {
    /// Reject the plaintext bound BEFORE key/RNG/AEAD work. Derive a protocol-specific key
    /// internally: ordinary db-key ciphertext must never be relabelled as a private chat page.
    /// Both the temporary ciphertext (p+16) and final frame (p+overhead) overlap. The caller also
    /// retains its input and any journal copies; it must prepay that complete inventory.
    pub fn prepare(
        vault_db_key: &[u8; 32],
        plaintext: &[u8],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, StorageError> {
        if plaintext.is_empty() || plaintext.len() > MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES {
            return Err(StorageError::BlobSizeLimit);
        }
        let key = page_key(vault_db_key);
        let sealed = seal(&key, plaintext, rng)?;
        let frame = encode_frame(&sealed);
        Ok(Self {
            id: PrivateChatPageId::of(&frame),
            frame,
        })
    }
    pub const fn id(&self) -> PrivateChatPageId {
        self.id
    }
    /// Borrowing neither consumes entropy nor produces a replacement envelope.
    pub fn as_bytes(&self) -> &[u8] {
        &self.frame
    }
}

/// Open one bounded frame selected by EXTERNAL trusted source metadata. Strict borrowed syntax
/// and the whole expected ciphertext identity are checked BEFORE ciphertext copying/decryption.
/// The returned zeroizing inner bytes remain untrusted until their own exact context/parser and
/// public cryptographic obligations pass; no saved/current/read capability is minted here.
///
/// During AEAD opening, the copied ciphertext and SDK output can EACH reserve p+16 bytes, even
/// though the successful plaintext length is p. Add the caller's original raw/prepared frame,
/// other retained journal copies, inner decode/typed records and fixed crypto state separately.
pub fn open_private_chat_page(
    vault_db_key: &[u8; 32],
    expected: PrivateChatPageId,
    frame: &[u8],
) -> Result<Zeroizing<Vec<u8>>, StorageError> {
    let parsed = parse_frame(frame)?;
    if PrivateChatPageId::of(frame) != expected {
        return Err(StorageError::CidMismatch);
    }
    let key = page_key(vault_db_key);
    let sealed = SealedBlob {
        nonce: parsed.nonce,
        ciphertext: parsed.ciphertext.to_vec(),
    };
    let plaintext = Zeroizing::new(unseal(&key, &sealed)?);
    // The strict ciphertext bound already implies this. Keep the check next to the SDK result
    // so a future AEAD replacement cannot silently widen the private-page contract.
    if plaintext.is_empty() || plaintext.len() > MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES {
        return Err(StorageError::BlobSizeLimit);
    }
    Ok(plaintext)
}

fn page_key(vault_db_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(blake3::derive_key(KEY_DOMAIN, vault_db_key))
}

fn encode_frame(sealed: &SealedBlob) -> Zeroizing<Vec<u8>> {
    // Only actual seal output reaches this encoder. Public callers cannot provide a ciphertext
    // or nonce for a new Prepared owner, and no base-db-key compatibility fallback exists.
    let mut out = Zeroizing::new(Vec::with_capacity(
        FRAME_OVERHEAD - AEAD_TAG_BYTES + sealed.ciphertext.len(),
    ));
    out.extend_from_slice(&(FRAME_DOMAIN.len() as u32).to_be_bytes());
    out.extend_from_slice(FRAME_DOMAIN);
    out.extend_from_slice(&VERSION.to_be_bytes());
    out.extend_from_slice(&sealed.nonce);
    out.extend_from_slice(&(sealed.ciphertext.len() as u32).to_be_bytes());
    out.extend_from_slice(&sealed.ciphertext);
    out
}

struct ParsedFrame<'a> {
    nonce: [u8; NONCE_BYTES],
    ciphertext: &'a [u8],
}

/// Fixed offsets make every possible copy constant-size and every variable field borrowed. In
/// particular an untrusted length cannot select a Vec capacity, and no trailing bytes survive.
fn parse_frame(frame: &[u8]) -> Result<ParsedFrame<'_>, StorageError> {
    if frame.len() < FRAME_OVERHEAD + 1 || frame.len() > MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES {
        return Err(StorageError::BlobSizeLimit);
    }
    let domain_end = 4 + FRAME_DOMAIN.len();
    let version_end = domain_end + 2;
    let nonce_end = version_end + NONCE_BYTES;
    let ciphertext_start = nonce_end + 4;
    if frame[..4] != (FRAME_DOMAIN.len() as u32).to_be_bytes()
        || &frame[4..domain_end] != FRAME_DOMAIN
        || frame[domain_end..version_end] != VERSION.to_be_bytes()
    {
        return Err(StorageError::Malformed);
    }
    let declared = u32::from_be_bytes(
        frame[nonce_end..ciphertext_start]
            .try_into()
            .map_err(|_| StorageError::Malformed)?,
    ) as usize;
    let ciphertext = &frame[ciphertext_start..];
    if declared != ciphertext.len()
        || declared <= AEAD_TAG_BYTES
        || declared > MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES + AEAD_TAG_BYTES
    {
        return Err(StorageError::Malformed);
    }
    let mut nonce = [0; NONCE_BYTES];
    nonce.copy_from_slice(&frame[version_end..nonce_end]);
    Ok(ParsedFrame { nonce, ciphertext })
}

#[cfg(test)]
mod tests;
