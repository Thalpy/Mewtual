//! Negative-only proofs of ACTUALLY selected governing history. The private frontier grows
//! by one leaf at successful selection, not receipt or derivation. Root must durably retain
//! the original records/nodes before evicting hot parents; this module performs no IO and
//! supplies no archive, restored ownership, Admission, frozen cut or save capability.
//!
//! These are separate hash domains from the admission/action acceptance tree. Only its
//! count/range geometry may be reused by a paid cold reader. A 64-hash proof can require
//! MORE than 64 node/page reads; Root must fund and bound the actual read plan independently.
use super::*;

const CONTEXT: &[u8] = b"mewtual/chat/selected-history-context/v1\0";
const EMPTY: &[u8] = b"mewtual/chat/selected-history-empty/v1\0";
const LEAF: &[u8] = b"mewtual/chat/selected-history-leaf/v1\0";
const NODE: &[u8] = b"mewtual/chat/selected-history-node/v1\0";
pub const MAX_SELECTED_HISTORY_PROOF_HASHES: usize = 64;
const MAX_CREATED_NODES: usize = 64;

/// Untrusted digest DATA, not an accepted tree node or storage witness.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct V2SelectedHistoryHashData([u8; 32]);
impl V2SelectedHistoryHashData {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Public snapshot labels only. The observation API compares these with its PRIVATE tip;
/// a decoded/imported tuple cannot choose the commitment against which it verifies a proof.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct V2SelectedHistoryTipData {
    pub context: [u8; 32],
    pub count: u64,
    pub root: V2SelectedHistoryHashData,
}
impl V2SelectedHistoryTipData {
    /// Pure hash arithmetic for folding committed complete-node ranges during cold export.
    /// This authenticates neither the tip nor the inputs; the final proof must still verify.
    pub fn node_hash_data(
        self,
        left: V2SelectedHistoryHashData,
        right: V2SelectedHistoryHashData,
    ) -> V2SelectedHistoryHashData {
        node(&self.context, left, right)
    }
}

/// One leaf/carry node in append-creation order. Actual physical page mapping, authenticated
/// node retrieval and selector barriers remain the Root writer/reader's responsibility.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct V2SelectedHistoryNodeData {
    first: u64,
    count: u64,
    hash: V2SelectedHistoryHashData,
}
impl V2SelectedHistoryNodeData {
    pub fn first_data(&self) -> u64 {
        self.first
    }
    pub fn count_data(&self) -> u64 {
        self.count
    }
    pub fn hash_data(&self) -> V2SelectedHistoryHashData {
        self.hash
    }
}

/// Borrowed untrusted proof against an original requested tip. Records include their own
/// signature representations: semantic-ID equality never bypasses those signature checks.
pub struct V2SelectedParentProof<'a> {
    pub tip: V2SelectedHistoryTipData,
    pub index: u64,
    pub parent: &'a SignedOwnerHeader,
    pub selected_child: &'a SignedOwnerHeader,
    pub siblings: &'a [V2SelectedHistoryHashData],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum V2ArchivedHeaderRefusal {
    #[error("selected history changed; retry the original bounded read")]
    StaleTip,
    #[error(transparent)]
    Rejected(#[from] ChatError),
}

/// Precomputed DATA attached to the original opaque candidate. No selection, write or
/// publication follows from these getters. Root must cover the exact record/node outputs
/// and selected-history update before dropping the original hot parent.
pub struct V2PreparedSelectedHistoryData {
    pub(super) next: Box<HistoryFrontier>,
    pub(super) nodes: Vec<V2SelectedHistoryNodeData>,
    old: V2SelectedHistoryTipData,
    parent: OwnerHeaderId,
    child: OwnerHeaderId,
}
impl V2PreparedSelectedHistoryData {
    pub fn old_tip_data(&self) -> V2SelectedHistoryTipData {
        self.old
    }
    pub fn new_tip_data(&self) -> V2SelectedHistoryTipData {
        self.next.tip()
    }
    pub fn parent_id_data(&self) -> OwnerHeaderId {
        self.parent
    }
    pub fn selected_child_id_data(&self) -> OwnerHeaderId {
        self.child
    }
    pub fn created_nodes_data(&self) -> &[V2SelectedHistoryNodeData] {
        &self.nodes
    }
}

redacted_debug!(
    V2SelectedHistoryHashData,
    V2SelectedHistoryTipData,
    V2SelectedHistoryNodeData,
    V2SelectedParentProof<'_>,
    V2PreparedSelectedHistoryData
);

/// No decoder/import constructor. The immutable context starts at independently pinned,
/// completely verified genesis; only an actual selected candidate can move the frontier.
#[derive(Clone)]
pub(super) struct HistoryFrontier {
    context: [u8; 32],
    count: u64,
    roots: [Option<V2SelectedHistoryHashData>; 64],
    // Cached outside writers: final selection performs no hashing, allocation or IO.
    root: V2SelectedHistoryHashData,
}
impl HistoryFrontier {
    pub(super) fn new(
        profile: &crate::chat::owner_profile::SignedOwnerProfile,
        genesis: &SignedOwnerHeader,
    ) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(CONTEXT);
        h.update(&(profile.spec().group.len() as u64).to_be_bytes());
        h.update(&profile.spec().group);
        h.update(profile.id().as_bytes());
        h.update(profile.tenure().as_bytes());
        h.update(genesis.id().as_bytes());
        h.update(genesis.spec().state.as_bytes());
        let context = *h.finalize().as_bytes();
        let mut empty = blake3::Hasher::new();
        empty.update(EMPTY);
        empty.update(&context);
        Self {
            context,
            count: 0,
            roots: [None; 64],
            root: V2SelectedHistoryHashData(*empty.finalize().as_bytes()),
        }
    }
    pub(super) fn tip(&self) -> V2SelectedHistoryTipData {
        V2SelectedHistoryTipData {
            context: self.context,
            count: self.count,
            root: self.root,
        }
    }
    /// All fallible arithmetic and finite output allocation precede candidate selection.
    pub(super) fn prepare(
        &self,
        parent: &SignedOwnerHeader,
        child: &SignedOwnerHeader,
    ) -> Result<V2PreparedSelectedHistoryData, ChatError> {
        let end = self.count.checked_add(1).ok_or(ChatError::Bound)?;
        if parent.spec().sequence != end
            || child.spec().parent != parent.id()
            || parent.spec().sequence.checked_add(1) != Some(child.spec().sequence)
        {
            return Err(ChatError::Dependency);
        }
        let mut next = self.clone();
        let capacity = 1 + self.count.trailing_ones() as usize;
        if capacity > MAX_CREATED_NODES {
            return Err(ChatError::Bound);
        }
        let mut nodes = Vec::with_capacity(capacity);
        let mut hash = leaf(&self.context, parent.id(), child.id());
        let mut width = 1u64;
        nodes.push(V2SelectedHistoryNodeData {
            first: self.count,
            count: width,
            hash,
        });
        let mut bit = 0;
        while (self.count >> bit) & 1 != 0 {
            let left = next.roots[bit].take().ok_or(ChatError::Dependency)?;
            width = width.checked_mul(2).ok_or(ChatError::Bound)?;
            hash = node(&self.context, left, hash);
            nodes.push(V2SelectedHistoryNodeData {
                first: end.checked_sub(width).ok_or(ChatError::Dependency)?,
                count: width,
                hash,
            });
            bit += 1;
        }
        next.roots[bit] = Some(hash);
        next.count = end;
        next.root = next
            .roots
            .iter()
            .filter_map(|hash| *hash)
            .fold(None, |right, left| {
                Some(right.map_or(left, |right| node(&self.context, left, right)))
            })
            .ok_or(ChatError::Dependency)?;
        Ok(V2PreparedSelectedHistoryData {
            old: self.tip(),
            next: Box::new(next),
            nodes,
            parent: parent.id(),
            child: child.id(),
        })
    }
}

fn leaf(
    context: &[u8; 32],
    parent: OwnerHeaderId,
    child: OwnerHeaderId,
) -> V2SelectedHistoryHashData {
    let mut h = blake3::Hasher::new();
    h.update(LEAF);
    h.update(context);
    h.update(parent.as_bytes());
    h.update(child.as_bytes());
    V2SelectedHistoryHashData(*h.finalize().as_bytes())
}
fn node(
    context: &[u8; 32],
    left: V2SelectedHistoryHashData,
    right: V2SelectedHistoryHashData,
) -> V2SelectedHistoryHashData {
    let mut h = blake3::Hasher::new();
    h.update(NODE);
    h.update(context);
    h.update(left.as_bytes());
    h.update(right.as_bytes());
    V2SelectedHistoryHashData(*h.finalize().as_bytes())
}
fn verify(
    tip: V2SelectedHistoryTipData,
    parent: OwnerHeaderId,
    child: OwnerHeaderId,
    index: u64,
    siblings: &[V2SelectedHistoryHashData],
) -> Result<(), ChatError> {
    if tip.count == 0 || index >= tip.count || siblings.len() > MAX_SELECTED_HISTORY_PROOF_HASHES {
        return Err(ChatError::Dependency);
    }
    let (mut width, mut offset, mut depth) = (tip.count, index, 0);
    let mut right_child = [false; MAX_SELECTED_HISTORY_PROOF_HASHES];
    while width > 1 {
        let left = 1u64 << (63 - (width - 1).leading_zeros());
        if offset < left {
            width = left;
        } else {
            offset -= left;
            width -= left;
            right_child[depth] = true;
        }
        depth += 1;
    }
    if siblings.len() != depth {
        return Err(ChatError::Dependency);
    }
    let mut hash = leaf(&tip.context, parent, child);
    for (sibling, right) in siblings.iter().zip(right_child[..depth].iter().rev()) {
        hash = if *right {
            node(&tip.context, *sibling, hash)
        } else {
            node(&tip.context, hash, *sibling)
        };
    }
    if hash != tip.root {
        return Err(ChatError::Dependency);
    }
    Ok(())
}

/// Extra retained/candidate/disposal overlap, ADDITIONAL to the inline structs in the
/// parent invoice. This covers frontier boxes, created-node backing and one cold conflict.
/// It is a logical allocation inventory, not allocator/RSS or physical archive capacity.
pub(super) const fn retained_layout_bytes() -> usize {
    3 * std::mem::size_of::<HistoryFrontier>()
        + 2 * MAX_CREATED_NODES * std::mem::size_of::<V2SelectedHistoryNodeData>()
        + std::mem::size_of::<V2KnownGoverningParent>()
        + 3 * crate::chat::owner_header::MAX_HEADER_BYTES
        + 16 * std::mem::size_of::<usize>()
}

impl V2GeneralPrefixVerifier {
    pub fn selected_history_tip_data(&self) -> V2SelectedHistoryTipData {
        self.history.tip()
    }

    /// Independent caller payment before proof handling, beyond retained metadata and
    /// caller-owned record/proof buffers. Root's page/read/cancellation invoice is separate.
    pub const fn selected_history_observation_temporary_layout_bytes() -> usize {
        8 * crate::chat::owner_header::MAX_HEADER_BYTES
            + 8 * crate::chat::owner_header::MAX_HEADER_SEMANTIC_BYTES
            + 4 * std::mem::size_of::<blake3::Hasher>()
            + MAX_SELECTED_HISTORY_PROOF_HASHES
                * (std::mem::size_of::<V2SelectedHistoryHashData>() + 1)
            + std::mem::size_of::<V2SelectedParentProof<'static>>()
    }

    /// Validate untrusted cold evidence ONLY against this verifier's private selected tip.
    /// This may close the original view on a confirmed signed fork. It never returns a
    /// reconstructed parent, Admission, frozen cut or other positive authority.
    pub fn observe_proven_archived_header(
        &mut self,
        proof: V2SelectedParentProof<'_>,
        incoming: &SignedOwnerHeader,
    ) -> Result<(), V2ArchivedHeaderRefusal> {
        self.live()?;
        let tip = self.history.tip();
        if proof.tip.context != tip.context {
            return Err(ChatError::Scope.into());
        }
        if proof.tip.count != tip.count {
            return Err(V2ArchivedHeaderRefusal::StaleTip);
        }
        if proof.tip.root != tip.root {
            return Err(ChatError::Dependency.into());
        }
        if proof.index.checked_add(1) != Some(proof.parent.spec().sequence) {
            return Err(ChatError::Dependency.into());
        }
        verify(
            tip,
            proof.parent.id(),
            proof.selected_child.id(),
            proof.index,
            proof.siblings,
        )?;
        // A proof authenticates semantic identities, NOT newly supplied signature bytes.
        proof.selected_child.verify_successor_of(proof.parent)?;
        incoming.verify_successor_of(proof.parent)?;
        self.live()?;
        if incoming.id() == proof.selected_child.id() {
            return Ok(());
        }
        let parent = Box::new(V2KnownGoverningParent {
            identity: self.identity.clone(),
            parent: proof.parent.clone(),
            first: Some(proof.selected_child.clone()),
            second: Some(incoming.clone()),
        });
        self.live()?;
        // Cache BOTH valid siblings BEFORE irreversible closure. Losing the caller's reply
        // cannot lose the evidence. Root must still persist it and restore the fence cold.
        self.archived_conflict = Some(parent);
        self.fence_fork();
        Err(ChatError::AuthorityConflict.into())
    }

    /// Actual cached contradiction, still readable after closure. This is not a durable
    /// receipt. Root must retain/persist it before destroying the original verifier.
    pub fn archived_contradiction_data(
        &self,
    ) -> Option<(&SignedOwnerHeader, &SignedOwnerHeader, &SignedOwnerHeader)> {
        let parent = self.archived_conflict.as_ref()?;
        Some((
            &parent.parent,
            parent.first.as_ref()?,
            parent.second.as_ref()?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::owner_facts::OwnerFactId;
    use crate::chat::owner_header::{
        HeaderKind, HeaderSpec, OwnerBodyId, OwnerStateCommitment, OwnerTenureId,
    };
    use catcoms_crypto::DeviceKeypair;

    #[test]
    fn selected_history_arithmetic_exhaustion_does_not_mutate_frontier() {
        // Arithmetic-only negative fixture, never installed into a verifier or used to
        // assert membership/selection. Even an inconsistent full frontier must refuse
        // checked count overflow before deriving output, touching peaks or allocating it.
        let key = DeviceKeypair::from_seed(&[71; 32]);
        let parent = SignedOwnerHeader::sign_using(
            HeaderSpec {
                group: b"overflow-arithmetic-only".to_vec(),
                profile: OwnerFactId::from_bytes([1; 32]),
                issuer: key.verifying_key().to_bytes(),
                tenure: OwnerTenureId::from_bytes([2; 32]),
                parent: OwnerHeaderId::from_bytes([3; 32]),
                sequence: 1,
                kind: HeaderKind::AcceptanceAppend,
                body: OwnerBodyId::from_bytes([4; 32]),
                state: OwnerStateCommitment::from_bytes([5; 32]),
            },
            |bytes| Ok(key.sign(bytes)),
        )
        .unwrap();
        let frontier = HistoryFrontier {
            context: [9; 32],
            count: u64::MAX,
            roots: [None; 64],
            root: V2SelectedHistoryHashData([7; 32]),
        };
        let before = frontier.tip();
        assert_eq!(
            frontier.prepare(&parent, &parent).unwrap_err(),
            ChatError::Bound
        );
        assert_eq!(frontier.tip(), before);
        assert!(frontier.roots.iter().all(Option::is_none));
    }

    #[test]
    fn selected_history_geometry_is_bounded_at_u64_maximum() {
        // Pure inclusion geometry; these synthetic hashes supply no signed provenance.
        let context = [17; 32];
        let parent = OwnerHeaderId::from_bytes([18; 32]);
        let child = OwnerHeaderId::from_bytes([19; 32]);
        let siblings = [V2SelectedHistoryHashData([20; 32]); 64];
        let root = siblings
            .iter()
            .fold(leaf(&context, parent, child), |left, right| {
                node(&context, left, *right)
            });
        let tip = V2SelectedHistoryTipData {
            context,
            count: u64::MAX,
            root,
        };
        assert_eq!(verify(tip, parent, child, 0, &siblings), Ok(()));
        assert!(verify(tip, parent, child, 0, &siblings[..63]).is_err());
        assert!(verify(tip, parent, child, u64::MAX, &siblings).is_err());
    }
}
