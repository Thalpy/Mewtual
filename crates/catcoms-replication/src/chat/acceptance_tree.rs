//! Bounded acceptance-tree mathematics for the pending owner-evidence/v2 log.
//!
//! Contexts, tips and successful hash checks are DATA, not authenticated owner facts, historical
//! membership, action approval, durable publication or Current capabilities. The real consumer
//! must independently verify the signed tips and retain/persist the exact leaves and nodes.
//! This module never scans a log, selects a root, changes the legacy format or performs I/O.
use super::owner_facts::OwnerFactId;

const EMPTY_DOMAIN: &[u8] = b"mewtual/chat/acceptance-empty/v2\0";
const LEAF_DOMAIN: &[u8] = b"mewtual/chat/acceptance-leaf/v2\0";
const NODE_DOMAIN: &[u8] = b"mewtual/chat/acceptance-node/v2\0";

pub const MAX_APPEND_LEAVES: usize = 64;
/// A conservative carry invoice; the reachable maximum at batch64 is120, not merely64.
pub const MAX_APPEND_INTERNAL_NODES: usize = 127;
pub const MAX_INCLUSION_HASHES: usize = 64;
/// Prefix consistency has a DIFFERENT bound: (2^63-1,u64::MAX) needs65 hashes.
pub const MAX_CONSISTENCY_HASHES: usize = 65;

/// An untrusted fixed hash label; construction verifies neither content nor authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TreeHash([u8; 32]);
impl TreeHash {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Exact admission incarnation namespace. These labels must come from independently verified
/// owner evidence at the consuming boundary; no constructor here authenticates their source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeContext {
    pub profile: OwnerFactId,
    pub admission: OwnerFactId,
}
impl TreeContext {
    pub fn empty_hash(self) -> TreeHash {
        let mut hash = blake3::Hasher::new();
        hash.update(EMPTY_DOMAIN);
        hash.update(self.profile.as_bytes());
        hash.update(self.admission.as_bytes());
        TreeHash(*hash.finalize().as_bytes())
    }

    pub fn leaf_hash(self, leaf: AcceptanceLeaf) -> TreeHash {
        let mut hash = blake3::Hasher::new();
        hash.update(LEAF_DOMAIN);
        hash.update(self.profile.as_bytes());
        hash.update(self.admission.as_bytes());
        hash.update(&leaf.channel.to_be_bytes());
        hash.update(leaf.acceptance.as_bytes());
        TreeHash(*hash.finalize().as_bytes())
    }
}

/// Fixed leaf semantics. The separate signed Acceptance fact binds the actual action/signer/
/// grant/basis/Create. Sorting this tuple is canonical batch encoding, not a policy decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AcceptanceLeaf {
    pub channel: u128,
    pub acceptance: OwnerFactId,
}

/// Claimed tree commitment, not a signature-verified log tip. Keeping context ON both tips
/// prevents accidental cross-admission prefix checks, especially where no leaf is hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeTip {
    pub context: TreeContext,
    pub count: u64,
    pub root: TreeHash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TreeError {
    #[error("acceptance tree counts are invalid or exhausted")]
    Count,
    #[error("acceptance tree contexts differ")]
    Context,
    #[error("acceptance frontier does not match its claimed tip")]
    Frontier,
    #[error("acceptance batch is empty, too large, duplicated or out of order")]
    Batch,
    #[error("acceptance proof does not match its exact path and tip")]
    Proof,
}

fn parent(left: TreeHash, right: TreeHash) -> TreeHash {
    let mut hash = blake3::Hasher::new();
    hash.update(NODE_DOMAIN);
    hash.update(left.as_bytes());
    hash.update(right.as_bytes());
    TreeHash(*hash.finalize().as_bytes())
}

/// Preflight the exact append-record count BEFORE cloning a frontier or creating output. The
/// host must separately pay these fixed records, scratch and its persistence/publication work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppendNodes {
    leaves: usize,
    internal: usize,
}
impl AppendNodes {
    pub fn leaves(self) -> usize {
        self.leaves
    }
    pub fn internal(self) -> usize {
        self.internal
    }
    pub fn total(self) -> usize {
        self.leaves + self.internal
    }
}
pub fn append_nodes(count: u64, leaves: usize) -> Result<AppendNodes, TreeError> {
    if leaves == 0 || leaves > MAX_APPEND_LEAVES {
        return Err(TreeError::Batch);
    }
    let after = count
        .checked_add(u64::try_from(leaves).map_err(|_| TreeError::Count)?)
        .ok_or(TreeError::Count)?;
    let before_bits = usize::try_from(count.count_ones()).map_err(|_| TreeError::Count)?;
    let after_bits = usize::try_from(after.count_ones()).map_err(|_| TreeError::Count)?;
    let internal = before_bits
        .checked_add(leaves)
        .and_then(|value| value.checked_sub(after_bits))
        .ok_or(TreeError::Count)?;
    if internal > MAX_APPEND_INTERNAL_NODES {
        return Err(TreeError::Count);
    }
    Ok(AppendNodes { leaves, internal })
}

/// One newly created complete subtree, in append creation order. Its range is [first,first+count).
/// The count is a power of two; the largest possible emitted subtree has2^63 leaves. Persist
/// these O(N) records, not a fresh full proof/path for every historical root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreatedNode {
    pub first: u64,
    pub count: u64,
    pub hash: TreeHash,
}

/// Bounded derived witness. Each occupied slot is the complete subtree corresponding to one
/// set bit of count. Its root is checked on import; that does NOT authenticate the supplied tip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frontier {
    context: TreeContext,
    count: u64,
    roots: [Option<TreeHash>; 64],
}
impl Frontier {
    pub fn empty(context: TreeContext) -> Self {
        Self {
            context,
            count: 0,
            roots: [None; 64],
        }
    }

    /// Import hashes in ascending occupied-bit order. The exact count/popcount and folded root
    /// must agree; extra zero/unused slots and short witnesses are refused without allocation.
    pub fn from_tip(tip: TreeTip, hashes: &[TreeHash]) -> Result<Self, TreeError> {
        if hashes.len() != tip.count.count_ones() as usize {
            return Err(TreeError::Frontier);
        }
        let mut result = Self::empty(tip.context);
        result.count = tip.count;
        let mut next = hashes.iter().copied();
        for bit in 0..64 {
            if (tip.count >> bit) & 1 != 0 {
                result.roots[bit] = next.next();
            }
        }
        if result.tip() != tip {
            return Err(TreeError::Frontier);
        }
        Ok(result)
    }

    pub fn hashes(&self) -> impl Iterator<Item = TreeHash> + '_ {
        self.roots.iter().filter_map(|root| *root)
    }

    pub fn tip(&self) -> TreeTip {
        // Low-to-high folding is deliberately RIGHT-associated at incomplete upper levels.
        // For7 leaves this is node(root4,node(root2,leaf)), never node(node(root4,root2),leaf).
        let root = self
            .hashes()
            .fold(None, |accumulated, higher| {
                Some(accumulated.map_or(higher, |right| parent(higher, right)))
            })
            .unwrap_or_else(|| self.context.empty_hash());
        TreeTip {
            context: self.context,
            count: self.count,
            root,
        }
    }

    /// Prepare only bounded math output; the original remains unchanged even on refusal.
    /// Cross-batch acceptance idempotency needs the owner's actual retained lookup, BEFORE
    /// calling this method. The primitive cannot decide whether a fact was accepted earlier.
    pub fn prepare_append(&self, leaves: &[AcceptanceLeaf]) -> Result<PreparedAppend, TreeError> {
        let invoice = append_nodes(self.count, leaves.len())?;
        if leaves.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(TreeError::Batch);
        }
        // Count/shape refusal precedes both the fixed frontier copy and the only output Vec.
        let old_tip = self.tip();
        let mut next = self.clone();
        let mut nodes = Vec::with_capacity(invoice.total());
        for leaf in leaves {
            let end = next.count.checked_add(1).ok_or(TreeError::Count)?;
            let mut hash = next.context.leaf_hash(*leaf);
            let mut width = 1u64;
            nodes.push(CreatedNode {
                first: next.count,
                count: width,
                hash,
            });
            let mut bit = 0;
            // The global checked count+k preflight makes a carry past bit63 impossible:
            // such a carry would require this pre-leaf count to be u64::MAX.
            while (next.count >> bit) & 1 != 0 {
                let left = next.roots[bit].take().ok_or(TreeError::Frontier)?;
                width = width.checked_mul(2).ok_or(TreeError::Count)?;
                hash = parent(left, hash);
                nodes.push(CreatedNode {
                    first: end.checked_sub(width).ok_or(TreeError::Frontier)?,
                    count: width,
                    hash,
                });
                bit += 1;
            }
            next.roots[bit] = Some(hash);
            next.count = end;
        }
        debug_assert_eq!(nodes.len(), invoice.total());
        Ok(PreparedAppend {
            old_tip,
            frontier: next,
            nodes,
        })
    }
}

/// Public hash data only. This is NOT a Saved result; retaining it pays neither cold pages nor
/// a signed owner transition. The real owner must persist those obligations before publication.
#[derive(Debug)]
pub struct PreparedAppend {
    old_tip: TreeTip,
    frontier: Frontier,
    nodes: Vec<CreatedNode>,
}
impl PreparedAppend {
    pub fn old_tip(&self) -> TreeTip {
        self.old_tip
    }
    pub fn new_tip(&self) -> TreeTip {
        self.frontier.tip()
    }
    pub fn created_nodes(&self) -> &[CreatedNode] {
        &self.nodes
    }
    pub fn frontier(&self) -> &Frontier {
        &self.frontier
    }
    pub fn into_frontier(self) -> Frontier {
        self.frontier
    }
}

fn split(count: u64) -> u64 {
    debug_assert!(count > 1);
    1u64 << (63 - (count - 1).leading_zeros())
}

/// Check exact bottom-up inclusion siblings. No signature, context provenance, actual body
/// availability or authority follows from success: independently verify the signed tip too.
pub fn verify_inclusion(
    tip: TreeTip,
    leaf: AcceptanceLeaf,
    index: u64,
    siblings: &[TreeHash],
) -> Result<(), TreeError> {
    if tip.count == 0 || index >= tip.count || siblings.len() > MAX_INCLUSION_HASHES {
        return Err(TreeError::Proof);
    }
    let (mut count, mut offset, mut depth) = (tip.count, index, 0);
    let mut right_child = [false; MAX_INCLUSION_HASHES];
    while count > 1 {
        let left = split(count);
        if offset < left {
            count = left;
        } else {
            offset -= left;
            count -= left;
            right_child[depth] = true;
        }
        depth += 1;
    }
    if siblings.len() != depth {
        return Err(TreeError::Proof);
    }
    let mut hash = tip.context.leaf_hash(leaf);
    for (sibling, right) in siblings.iter().zip(right_child[..depth].iter().rev()) {
        hash = if *right {
            parent(*sibling, hash)
        } else {
            parent(hash, *sibling)
        };
    }
    if hash != tip.root {
        return Err(TreeError::Proof);
    }
    Ok(())
}

/// Check the exact count-aware historical prefix relation (canonical SUBPROOF order).
/// Opaque subtree hashes cannot authenticate context labels. Even an empty/equal prefix must
/// compare BOTH labelled contexts, and the caller must authenticate both external signed tips.
pub fn verify_prefix(old: TreeTip, new: TreeTip, proof: &[TreeHash]) -> Result<(), TreeError> {
    if old.context != new.context {
        return Err(TreeError::Context);
    }
    if old.count > new.count || proof.len() > MAX_CONSISTENCY_HASHES {
        return Err(TreeError::Proof);
    }
    if old.count == 0 {
        return if old.root == old.context.empty_hash()
            && proof.is_empty()
            && (new.count != 0 || new.root == old.root)
        {
            Ok(())
        } else {
            Err(TreeError::Proof)
        };
    }
    if old.count == new.count {
        return if old.root == new.root && proof.is_empty() {
            Ok(())
        } else {
            Err(TreeError::Proof)
        };
    }
    let (mut first, mut second) = (old.count - 1, new.count - 1);
    while first & 1 != 0 {
        first >>= 1;
        second >>= 1;
    }
    let mut nodes = proof.iter().copied();
    let mut old_root = if first == 0 {
        old.root
    } else {
        nodes.next().ok_or(TreeError::Proof)?
    };
    let mut new_root = old_root;
    for node in nodes {
        if second == 0 {
            return Err(TreeError::Proof);
        }
        if first & 1 != 0 || first == second {
            old_root = parent(node, old_root);
            new_root = parent(node, new_root);
            while first != 0 && first & 1 == 0 {
                first >>= 1;
                second >>= 1;
            }
        } else {
            new_root = parent(new_root, node);
        }
        first >>= 1;
        second >>= 1;
    }
    if first != 0 || second != 0 || old_root != old.root || new_root != new.root {
        return Err(TreeError::Proof);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
