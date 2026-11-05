//! Borrowed private node-page DATA for the pending scalable archive. No I/O or namespace
//! activation occurs here; a parsed page is neither a retained source nor an authenticated tip.
//! The real Store must pay/read/unseal its original committed page, check the expected context
//! and use the completed public proof verifier before any acceptance/effect can follow.
use super::acceptance_tree::{append_nodes, Frontier, PreparedAppend, TreeContext, TreeHash};
use super::owner_bodies::AppendBody;
use super::owner_facts::OwnerFactId;
use super::owner_header::OwnerBodyId;
use super::{ChatError, MAX_GROUP_ID_BYTES};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

const DOMAIN: &[u8] = b"catcoms/private-chat-node-page/v1\0";
const VERSION: u16 = 1;
/// Conservative leaf+carry capacity, not a claim that all191 rows can occur in one real batch.
pub const MAX_NODE_PAGE_HASHES: usize =
    super::acceptance_tree::MAX_APPEND_LEAVES + super::acceptance_tree::MAX_APPEND_INTERNAL_NODES;
/// Per-plaintext-buffer ceiling. Parsing borrows ALL variable data and allocates no containers.
/// Encoding requests this capacity once and appends packed hashes directly, without a second
/// packed Vec. The caller separately pays the original/verification PreparedAppend node Vecs,
/// fixed frontiers, and body.id scratch (up to3572 encoded +3072 packed leaf bytes). AEAD/raw
/// file buffers, actual allocations/host fees and the complete compound Store invoice are NOT
/// supplied by this codec. It must not be used to claim bounded SDK/provider memory.
pub const MAX_NODE_PAGE_BYTES: usize = 4
    + DOMAIN.len()
    + 2
    + 36
    + 4
    + MAX_GROUP_ID_BYTES
    + 3 * 36
    + 8
    + 8
    + 2
    + 4
    + MAX_NODE_PAGE_HASHES * 32;

#[derive(Clone, Copy)]
struct Geometry {
    before: u64,
    after: u64,
    first: u128,
    count: usize,
}
impl Geometry {
    fn new(before: u64, after: u64) -> Result<Self, ChatError> {
        let leaves = after.checked_sub(before).ok_or(ChatError::Dependency)?;
        if leaves == 0 || leaves > super::acceptance_tree::MAX_APPEND_LEAVES as u64 {
            return Err(ChatError::Bound);
        }
        let count = append_nodes(before, leaves as usize)
            .map_err(|_| ChatError::Dependency)?
            .total();
        // There are2*n-popcount(n) complete leaf/carry records BEFORE the next append. Widen
        // before multiplication: this position exceeds u64 at valid high-count frontiers.
        let first = 2 * u128::from(before) - u128::from(before.count_ones());
        Ok(Self {
            before,
            after,
            first,
            count,
        })
    }
}

/// Strict borrowed private page. Domain/group/context/body labels and hashes are UNTRUSTED
/// data. The immutable original byte owner must outlive it; it owns no provider, authority,
/// decoded group/string/hash Vec or ability to read a path from disk.
pub struct NodePage<'a> {
    vault: [u8; 32],
    group: &'a [u8],
    context: TreeContext,
    body: OwnerBodyId,
    geometry: Geometry,
    packed: &'a [u8],
}
impl std::fmt::Debug for NodePage<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodePage")
            .field("nodes", &self.geometry.count)
            .finish_non_exhaustive()
    }
}
impl<'a> NodePage<'a> {
    /// Complete borrowed framing/count validation. This does not authenticate hashes or their
    /// claimed context/body; invalid hash bytes intentionally remain parsed untrusted data.
    pub fn decode(bytes: &'a [u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_NODE_PAGE_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes()? != DOMAIN || d.get_u16()? != VERSION {
            return Err(ChatError::Malformed);
        }
        let vault = digest(&mut d)?;
        let group = d.get_bytes()?;
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        let context = TreeContext {
            profile: OwnerFactId::from_bytes(digest(&mut d)?),
            admission: OwnerFactId::from_bytes(digest(&mut d)?),
        };
        let body = OwnerBodyId::from_bytes(digest(&mut d)?);
        let geometry = Geometry::new(d.get_u64()?, d.get_u64()?)?;
        let count = usize::from(d.get_u16()?);
        if count != geometry.count || count > MAX_NODE_PAGE_HASHES {
            return Err(ChatError::Bound);
        }
        let packed = d.get_bytes()?;
        if packed.len() != count * 32 {
            return Err(ChatError::Malformed);
        }
        d.finish()?;
        Ok(Self {
            vault,
            group,
            context,
            body,
            geometry,
            packed,
        })
    }
    pub fn vault_domain(&self) -> &[u8; 32] {
        &self.vault
    }
    pub fn group(&self) -> &[u8] {
        self.group
    }
    pub fn context(&self) -> TreeContext {
        self.context
    }
    pub fn append_body(&self) -> OwnerBodyId {
        self.body
    }
    pub fn before_count(&self) -> u64 {
        self.geometry.before
    }
    pub fn after_count(&self) -> u64 {
        self.geometry.after
    }
    pub fn first_ordinal(&self) -> u128 {
        self.geometry.first
    }
    pub fn node_count(&self) -> usize {
        self.geometry.count
    }

    /// Geometric lookup ONLY. A valid proof-planner ordinal is not permission to trust this
    /// page/hash. Compare actual committed page/context and verify the final signed-tip proof.
    /// Wide subtraction and range check precede conversion, so outside ordinals never wrap.
    pub fn hash_at_ordinal(&self, ordinal: u128) -> Option<TreeHash> {
        let offset = ordinal.checked_sub(self.geometry.first)?;
        if offset >= self.geometry.count as u128 {
            return None;
        }
        let offset = usize::try_from(offset).ok()?.checked_mul(32)?;
        Some(TreeHash::from_bytes(
            self.packed.get(offset..offset + 32)?.try_into().ok()?,
        ))
    }

    /// Verify exact bounded math against the expected original vault/body/frontier. This checks
    /// EVERY newly created hash, not only the last root or count. It still proves no on-disk
    /// durability, selected owner/header, Current membership or acceptance of an actual action.
    pub fn verify_exact_append(
        &self,
        vault: &[u8; 32],
        body: &AppendBody,
        frontier: &Frontier,
    ) -> Result<(), ChatError> {
        let spec = body.spec();
        if &self.vault != vault
            || self.group != spec.group.as_slice()
            || self.context != spec.old_tip.context
            || self.body != body.id()
            || self.geometry.before != spec.old_tip.count
            || self.geometry.after != spec.new_tip.count
        {
            return Err(ChatError::Scope);
        }
        let prepared = body.prepare_on_frontier(frontier)?;
        if prepared.created_nodes().len() != self.geometry.count {
            return Err(ChatError::Dependency);
        }
        for (offset, node) in prepared.created_nodes().iter().enumerate() {
            let ordinal = node.creation_ordinal().map_err(|_| ChatError::Dependency)?;
            if ordinal != self.geometry.first + offset as u128
                || self.hash_at_ordinal(ordinal) != Some(node.hash)
            {
                return Err(ChatError::Dependency);
            }
        }
        Ok(())
    }
}

/// Encode one already computed bounded append. Refusal precedes output allocation; the
/// PreparedAppend is private-construction math data, not proof of a real selected owner/save.
/// Its actual range/ordinal sequence and exact old/new tips are checked independently here.
pub fn encode_node_page(
    vault: &[u8; 32],
    body: &AppendBody,
    prepared: &PreparedAppend,
) -> Result<Zeroizing<Vec<u8>>, ChatError> {
    let spec = body.spec();
    if prepared.old_tip() != spec.old_tip || prepared.new_tip() != spec.new_tip {
        return Err(ChatError::Scope);
    }
    let geometry = Geometry::new(spec.old_tip.count, spec.new_tip.count)?;
    if prepared.created_nodes().len() != geometry.count {
        return Err(ChatError::Dependency);
    }
    let mut leaf_index = 0usize;
    for (offset, node) in prepared.created_nodes().iter().enumerate() {
        if node.creation_ordinal().map_err(|_| ChatError::Dependency)?
            != geometry.first + offset as u128
        {
            return Err(ChatError::Dependency);
        }
        // A body can declare another sorted leaf list while retaining the same CLAIMED
        // old/new tip. Tip equality alone does not bind a prepared append to that list.
        // PreparedAppend's private constructor computes its internal nodes; independently
        // bind every actual leaf here before emitting this body's CID over those hashes.
        if node.count == 1 {
            let leaf = spec.leaves.get(leaf_index).ok_or(ChatError::Dependency)?;
            if node.first != geometry.before + leaf_index as u64
                || node.hash != spec.old_tip.context.leaf_hash(*leaf)
            {
                return Err(ChatError::Dependency);
            }
            leaf_index += 1;
        }
    }
    if leaf_index != spec.leaves.len() {
        return Err(ChatError::Dependency);
    }
    let body_id = body.id(); // bounded existing body/packed-leaf scratch is released before output
    let mut e = Encoder::with_capacity(MAX_NODE_PAGE_BYTES);
    e.put_bytes(DOMAIN)?;
    e.put_u16(VERSION);
    e.put_bytes(vault)?;
    e.put_bytes(&spec.group)?;
    e.put_bytes(spec.old_tip.context.profile.as_bytes())?;
    e.put_bytes(spec.old_tip.context.admission.as_bytes())?;
    e.put_bytes(body_id.as_bytes())?;
    e.put_u64(geometry.before).put_u64(geometry.after);
    e.put_u16(geometry.count as u16)
        .put_u32((geometry.count * 32) as u32);
    let mut encoded = Zeroizing::new(e.finish());
    for node in prepared.created_nodes() {
        encoded.extend_from_slice(node.hash.as_bytes());
    }
    debug_assert!(encoded.len() <= MAX_NODE_PAGE_BYTES);
    Ok(encoded)
}

fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}

#[cfg(test)]
mod tests;
