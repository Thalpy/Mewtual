//! Bounded private original-action chunk DATA. No file/namespace or authority is created here.
//! A private committed manifest must select the expected original before assembly; every page
//! remains untrusted even when its framing/digest is consistent. Actual historical delegation,
//! acceptance, current effects and privileged native read authorization remain separate checks.
use super::owner_facts::OwnerFactId;
use super::{ActionId, ChatError, SignedAction, MAX_ACTION_BYTES, MAX_GROUP_ID_BYTES};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

const DOMAIN: &[u8] = b"catcoms/private-chat-action-chunk/v1\0";
const ORIGINAL_DOMAIN: &[u8] = b"catcoms/private-chat-original-action/v1\0";
pub const ACTION_CHUNK_BYTES: usize = 60 * 1024;
/// Conservative encoded-action ceiling allows3 chunks; the actual bounded grammar can be smaller.
pub const MAX_ACTION_CHUNKS: usize = MAX_ACTION_BYTES.div_ceil(ACTION_CHUNK_BYTES);
/// Requested per-page output/input ceiling, not a whole operation/SDK/native memory invoice.
pub const MAX_ACTION_CHUNK_PAGE_BYTES: usize = 4
    + DOMAIN.len()
    + 2
    + 36
    + 4
    + MAX_GROUP_ID_BYTES
    + 3 * 36
    + 4
    + 2
    + 2
    + 4
    + ACTION_CHUNK_BYTES;

/// Exact original envelope representation, including its valid signature. This is deliberately
/// distinct from semantic ActionId: genuine alternate signatures have one ActionId but different
/// original byte digests and cannot be mixed across chunks. Construction authenticates nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OriginalActionDigest([u8; 32]);
impl OriginalActionDigest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Copy)]
struct Geometry {
    total: usize,
    index: usize,
    count: usize,
    offset: usize,
    payload: usize,
}
impl Geometry {
    fn new(total: usize, index: usize, count: usize) -> Result<Self, ChatError> {
        if total == 0
            || total > MAX_ACTION_BYTES
            || count == 0
            || count > MAX_ACTION_CHUNKS
            || count != total.div_ceil(ACTION_CHUNK_BYTES)
            || index >= count
        {
            return Err(ChatError::Bound);
        }
        let offset = index
            .checked_mul(ACTION_CHUNK_BYTES)
            .ok_or(ChatError::Bound)?;
        let payload = total
            .checked_sub(offset)
            .ok_or(ChatError::Bound)?
            .min(ACTION_CHUNK_BYTES);
        Ok(Self {
            total,
            index,
            count,
            offset,
            payload,
        })
    }
}

/// Owns immutable context and the SAME verified canonical envelope. A later caller cannot
/// retarget a page by supplying another action/group after preparation. This is not a saved
/// source; actual host capacity must cover this original plus all simultaneous page buffers.
pub struct PreparedActionPages {
    original: Zeroizing<Vec<u8>>,
    group: Vec<u8>,
    vault: [u8; 32],
    profile: OwnerFactId,
    action: ActionId,
    digest: OriginalActionDigest,
}
impl std::fmt::Debug for PreparedActionPages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedActionPages")
            .field("chunks", &self.chunk_count())
            .finish_non_exhaustive()
    }
}
impl PreparedActionPages {
    pub fn prepare(
        vault: [u8; 32],
        profile: OwnerFactId,
        action: &SignedAction,
    ) -> Result<Self, ChatError> {
        if !action.verify() {
            return Err(ChatError::Signature);
        }
        let original = Zeroizing::new(action.encode());
        if original.is_empty() || original.len() > MAX_ACTION_BYTES {
            return Err(ChatError::Bound);
        }
        let mut hash = original_hasher();
        hash.update(&original);
        Ok(Self {
            digest: OriginalActionDigest(*hash.finalize().as_bytes()),
            original,
            group: action.spec().realm.group().to_vec(),
            vault,
            profile,
            action: action.id(),
        })
    }
    pub fn action_id(&self) -> ActionId {
        self.action
    }
    pub fn original_digest(&self) -> OriginalActionDigest {
        self.digest
    }
    pub fn chunk_count(&self) -> usize {
        self.original.len().div_ceil(ACTION_CHUNK_BYTES)
    }
    /// Allocate only the requested valid page. Invalid indices refuse before allocation.
    pub fn encode_chunk(&self, index: usize) -> Result<Zeroizing<Vec<u8>>, ChatError> {
        let g = Geometry::new(self.original.len(), index, self.chunk_count())?;
        let mut e = Encoder::with_capacity(MAX_ACTION_CHUNK_PAGE_BYTES);
        e.put_bytes(DOMAIN)?;
        e.put_u16(1);
        e.put_bytes(&self.vault)?;
        e.put_bytes(&self.group)?;
        e.put_bytes(self.profile.as_bytes())?;
        e.put_bytes(self.action.as_bytes())?;
        e.put_bytes(self.digest.as_bytes())?;
        e.put_u32(g.total as u32)
            .put_u16(g.index as u16)
            .put_u16(g.count as u16);
        e.put_bytes(&self.original[g.offset..g.offset + g.payload])?;
        Ok(Zeroizing::new(e.finish()))
    }
}

/// Complete borrowed chunk syntax, never a committed page or a verified action/authority.
pub struct ActionChunk<'a> {
    vault: [u8; 32],
    group: &'a [u8],
    profile: OwnerFactId,
    action: ActionId,
    digest: OriginalActionDigest,
    geometry: Geometry,
    payload: &'a [u8],
}
impl std::fmt::Debug for ActionChunk<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionChunk")
            .field("index", &self.geometry.index)
            .finish_non_exhaustive()
    }
}
impl<'a> ActionChunk<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_ACTION_CHUNK_PAGE_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes()? != DOMAIN || d.get_u16()? != 1 {
            return Err(ChatError::Malformed);
        }
        let vault = digest(&mut d)?;
        let group = d.get_bytes()?;
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        let profile = OwnerFactId::from_bytes(digest(&mut d)?);
        let action = ActionId::from_bytes(digest(&mut d)?);
        let original = OriginalActionDigest(digest(&mut d)?);
        let total = d.get_u32()? as usize;
        let index = usize::from(d.get_u16()?);
        let count = usize::from(d.get_u16()?);
        let geometry = Geometry::new(total, index, count)?;
        let payload = d.get_bytes()?;
        if payload.len() != geometry.payload {
            return Err(ChatError::Malformed);
        }
        d.finish()?;
        Ok(Self {
            vault,
            group,
            profile,
            action,
            digest: original,
            geometry,
            payload,
        })
    }
    pub fn action_id(&self) -> ActionId {
        self.action
    }
    pub fn original_digest(&self) -> OriginalActionDigest {
        self.digest
    }
    pub fn chunk_index(&self) -> usize {
        self.geometry.index
    }
    pub fn chunk_count(&self) -> usize {
        self.geometry.count
    }
}

/// Exact lookup metadata from the expected ORIGINAL private manifest, not selected from the
/// parsed pages themselves. Raw construction is DATA and cannot authenticate a manifest/source.
pub struct ActionPagesExpectation<'a> {
    pub vault: &'a [u8; 32],
    pub group: &'a [u8],
    pub profile: OwnerFactId,
    pub action: ActionId,
    pub original: OriginalActionDigest,
}

/// Signature-checked assembled original DATA only. This is NOT historical authority, a durable
/// source, Current or permission to expose text. Raw and typed owned representations overlap;
/// their actual memory plus decode/re-encode/signature scratch needs separate caller capacity.
pub struct AssembledActionOriginal {
    original: Zeroizing<Vec<u8>>,
    action: SignedAction,
}
impl std::fmt::Debug for AssembledActionOriginal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssembledActionOriginal")
            .finish_non_exhaustive()
    }
}
impl AssembledActionOriginal {
    pub fn original_bytes(&self) -> &[u8] {
        &self.original
    }
    pub fn action(&self) -> &SignedAction {
        &self.action
    }
}

/// Verify ALL borrowed framing/context/order/digest before allocating an assembled buffer.
/// Missing pages, wrong requests and mixed representations never yield a partial action.
/// The original digest is computed incrementally over borrowed payloads, not after copying.
pub fn assemble_action(
    expected: &ActionPagesExpectation<'_>,
    chunks: &[ActionChunk<'_>],
) -> Result<AssembledActionOriginal, ChatError> {
    let first = chunks.first().ok_or(ChatError::Bound)?;
    if chunks.len() != first.geometry.count || chunks.len() > MAX_ACTION_CHUNKS {
        return Err(ChatError::Bound);
    }
    let mut hash = original_hasher();
    for (index, chunk) in chunks.iter().enumerate() {
        if &chunk.vault != expected.vault
            || chunk.group != expected.group
            || chunk.profile != expected.profile
            || chunk.action != expected.action
            || chunk.digest != expected.original
            || chunk.geometry.total != first.geometry.total
            || chunk.geometry.count != chunks.len()
            || chunk.geometry.index != index
        {
            return Err(ChatError::Scope);
        }
        hash.update(chunk.payload);
    }
    if hash.finalize().as_bytes() != expected.original.as_bytes() {
        return Err(ChatError::Dependency);
    }
    let mut original = Zeroizing::new(Vec::with_capacity(first.geometry.total));
    for chunk in chunks {
        original.extend_from_slice(chunk.payload);
    }
    let action = SignedAction::decode(&original)?;
    if !action.verify() {
        return Err(ChatError::Signature);
    }
    if action.id() != expected.action || action.spec().realm.group() != expected.group {
        return Err(ChatError::Scope);
    }
    Ok(AssembledActionOriginal { original, action })
}
fn original_hasher() -> blake3::Hasher {
    let mut h = blake3::Hasher::new();
    h.update(ORIGINAL_DOMAIN);
    h
}
fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}

#[cfg(test)]
mod tests;
