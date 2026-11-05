//! Count-only plans for bounded proof export from the pending cold node log.
//!
//! Plans contain geometric labels, never hashes, authenticated tips, body availability or storage
//! capabilities. A future host must pay its I/O/scratch, load exact committed node records and
//! independently verify the completed proof against its authenticated signed tips. This module
//! performs no I/O and cannot turn a missing node into an empty or verified complete history.
use super::{creation_ordinal, parent, split, TreeError, TreeHash};

/// One proof has at most65 ranges, each decomposable into at most64 complete nodes. This is a
/// conservative logical read invoice, not an allocator/RSS fee or an efficient page-cache claim.
pub const MAX_PROOF_NODE_READS: usize = super::MAX_CONSISTENCY_HASHES * 64;

/// One canonical nonempty recursive subtree range. Construction stays PRIVATE: an arbitrary
/// interval can need126 aligned components, while these shapes have at most64. Exposing a raw
/// parser/constructor later must not silently inherit this bound for unrestricted intervals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubtreeRange {
    first: u64,
    count: u64,
}
impl SubtreeRange {
    fn new(first: u64, count: u64, total: u64) -> Result<Self, TreeError> {
        if count == 0 || first.checked_add(count).is_none_or(|end| end > total) {
            return Err(TreeError::Node);
        }
        let largest = 1u64 << (63 - count.leading_zeros());
        if first % largest != 0 || count.count_ones() > 64 {
            return Err(TreeError::Node);
        }
        Ok(Self { first, count })
    }

    pub fn first(self) -> u64 {
        self.first
    }
    pub fn count(self) -> u64 {
        self.count
    }

    /// Exact number of complete-node hash records needed to reconstruct this range. Every
    /// emitted component's range/ordinal must match its actual committed record at the host.
    pub fn node_reads(self) -> usize {
        self.count.count_ones() as usize
    }

    /// Rightmost-smallest to leftmost-largest. Fold with node(left, accumulated_right), not a
    /// left-associated accumulator; an incomplete tree's right edge is itself a subtree.
    pub fn complete_nodes_right_to_left(self) -> CompleteNodes {
        CompleteNodes {
            first: self.first,
            remaining: self.count,
        }
    }

    /// Fold exactly the hashes in complete_nodes_right_to_left order. Success is pure hash
    /// mathematics, NOT proof these hashes came from retained records or an authentic log. Only
    /// the final external inclusion/prefix verification can bind them to authenticated tips.
    pub fn hash_from_nodes(self, hashes: &[TreeHash]) -> Result<TreeHash, TreeError> {
        if hashes.len() != self.node_reads() {
            return Err(TreeError::Proof);
        }
        let mut hashes = hashes.iter().copied();
        let first = hashes.next().ok_or(TreeError::Proof)?;
        Ok(hashes.fold(first, |right, left| parent(left, right)))
    }
}

/// Complete aligned node geometry only. No hash, storage pathname or access capability is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompleteNode {
    first: u64,
    count: u64,
}
impl CompleteNode {
    pub fn first(self) -> u64 {
        self.first
    }
    pub fn count(self) -> u64 {
        self.count
    }
    /// Checked logical position in the exact leaf+carry creation sequence; still not a physical
    /// page address. Private page/manifest formats must independently authenticate actual bytes.
    pub fn creation_ordinal(self) -> Result<u128, TreeError> {
        creation_ordinal(self.first, self.count)
    }
}

/// Allocation-free decomposition, bounded by64 steps because only private canonical ranges can
/// construct it. The original checked end covers every subtraction/addition below.
#[derive(Clone, Debug)]
pub struct CompleteNodes {
    first: u64,
    remaining: u64,
}
impl Iterator for CompleteNodes {
    type Item = CompleteNode;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let count = 1u64 << self.remaining.trailing_zeros();
        self.remaining -= count;
        Some(CompleteNode {
            first: self.first + self.remaining,
            count,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.remaining.count_ones() as usize;
        (remaining, Some(remaining))
    }
}
impl ExactSizeIterator for CompleteNodes {}
impl std::iter::FusedIterator for CompleteNodes {}

/// Bounded count-only proof data. Range order is EXACTLY the verifier's sibling/SUBPROOF order.
/// Pay the fixed planner scratch plus MAX_PROOF_NODE_READS before admitting asynchronous export;
/// this returned smaller read invoice does not retroactively pay previous work or cold pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofPlan {
    ranges: Vec<SubtreeRange>,
}
impl ProofPlan {
    pub fn ranges(&self) -> &[SubtreeRange] {
        &self.ranges
    }
    pub fn node_reads(&self) -> usize {
        self.ranges.iter().map(|range| range.node_reads()).sum()
    }
}

/// Plan exact bottom-up inclusion siblings without reading history or hashing. Empty trees and
/// indices outside count refuse before allocating output. Returned labels are NOT a proof.
pub fn inclusion_ranges(count: u64, index: u64) -> Result<ProofPlan, TreeError> {
    if count == 0 || index >= count {
        return Err(TreeError::Proof);
    }
    let total = count;
    let (mut first, mut remaining, mut offset) = (0u64, count, index);
    let mut ranges = Vec::with_capacity(super::MAX_INCLUSION_HASHES);
    while remaining > 1 {
        let left = split(remaining);
        if offset < left {
            ranges.push(SubtreeRange::new(first + left, remaining - left, total)?);
            remaining = left;
        } else {
            ranges.push(SubtreeRange::new(first, left, total)?);
            first += left;
            remaining -= left;
            offset -= left;
        }
    }
    ranges.reverse();
    Ok(ProofPlan { ranges })
}

/// Plan canonical SUBPROOF ranges. Zero/equal prefixes have an empty proof, whose actual empty/
/// equal-root and context checks remain the verifier's responsibility. Reverse counts refuse
/// before output allocation. The explicit fixed stack bounds traversal even at u64::MAX.
pub fn prefix_ranges(old_count: u64, new_count: u64) -> Result<ProofPlan, TreeError> {
    if old_count > new_count {
        return Err(TreeError::Proof);
    }
    if old_count == 0 || old_count == new_count {
        return Ok(ProofPlan { ranges: Vec::new() });
    }
    let (mut old, mut remaining, mut first) = (old_count, new_count, 0u64);
    let mut known_old_root = true;
    let mut unwind = [None; super::MAX_INCLUSION_HASHES];
    let mut depth = 0;
    while old != remaining {
        let left = split(remaining);
        let sibling = if old <= left {
            let sibling = SubtreeRange::new(first + left, remaining - left, new_count)?;
            remaining = left;
            sibling
        } else {
            let sibling = SubtreeRange::new(first, left, new_count)?;
            old -= left;
            first += left;
            remaining -= left;
            known_old_root = false;
            sibling
        };
        let slot = unwind.get_mut(depth).ok_or(TreeError::Proof)?;
        *slot = Some(sibling);
        depth += 1;
    }
    let mut ranges = Vec::with_capacity(super::MAX_CONSISTENCY_HASHES);
    if !known_old_root {
        ranges.push(SubtreeRange::new(first, remaining, new_count)?);
    }
    for range in unwind[..depth].iter().rev() {
        ranges.push(range.ok_or(TreeError::Proof)?);
    }
    if ranges.len() > super::MAX_CONSISTENCY_HASHES
        || ranges.iter().map(|range| range.node_reads()).sum::<usize>() > MAX_PROOF_NODE_READS
    {
        return Err(TreeError::Proof);
    }
    Ok(ProofPlan { ranges })
}

#[cfg(test)]
mod tests;
