use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn context() -> TreeContext {
    TreeContext {
        profile: OwnerFactId::from_bytes([1; 32]),
        admission: OwnerFactId::from_bytes([2; 32]),
    }
}
fn leaf(index: u64) -> AcceptanceLeaf {
    let mut id = [0; 32];
    id[24..].copy_from_slice(&index.to_be_bytes());
    AcceptanceLeaf {
        channel: 42,
        acceptance: OwnerFactId::from_bytes(id),
    }
}
fn hash_parts(parts: &[&[u8]]) -> TreeHash {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(part);
    }
    TreeHash::from_bytes(*hasher.finalize().as_bytes())
}
fn reference_leaf(context: TreeContext, value: AcceptanceLeaf) -> TreeHash {
    hash_parts(&[
        b"mewtual/chat/acceptance-leaf/v2\0",
        context.profile.as_bytes(),
        context.admission.as_bytes(),
        &value.channel.to_be_bytes(),
        value.acceptance.as_bytes(),
    ])
}
fn reference_parent(left: TreeHash, right: TreeHash) -> TreeHash {
    hash_parts(&[
        b"mewtual/chat/acceptance-node/v2\0",
        left.as_bytes(),
        right.as_bytes(),
    ])
}

// Independent recursive reference, with memoized subtree roots so exhaustive prefix/path
// coverage tests do not repeatedly rehash the same small history. It never uses Frontier,
// production split(), proof verification, carry arithmetic or created-node output.
struct Reference {
    leaves: Vec<TreeHash>,
    roots: BTreeMap<(usize, usize), TreeHash>,
}
impl Reference {
    fn new(count: usize) -> Self {
        Self {
            leaves: (0..count)
                .map(|index| reference_leaf(context(), leaf(index as u64)))
                .collect(),
            roots: BTreeMap::new(),
        }
    }
    fn cut(count: usize) -> usize {
        let mut cut = 1;
        while cut <= (count - 1) / 2 {
            cut *= 2;
        }
        cut
    }
    fn root(&mut self, start: usize, count: usize) -> TreeHash {
        if count == 0 {
            return hash_parts(&[b"mewtual/chat/acceptance-empty/v2\0", &[1; 32], &[2; 32]]);
        }
        if let Some(root) = self.roots.get(&(start, count)) {
            return *root;
        }
        let result = if count == 1 {
            self.leaves[start]
        } else {
            let cut = Self::cut(count);
            reference_parent(self.root(start, cut), self.root(start + cut, count - cut))
        };
        self.roots.insert((start, count), result);
        result
    }
    fn tip(&mut self, count: usize) -> TreeTip {
        TreeTip {
            context: context(),
            count: count as u64,
            root: self.root(0, count),
        }
    }
    fn inclusion(&mut self, start: usize, count: usize, index: usize) -> Vec<TreeHash> {
        if count == 1 {
            return Vec::new();
        }
        let cut = Self::cut(count);
        if index < cut {
            let mut proof = self.inclusion(start, cut, index);
            proof.push(self.root(start + cut, count - cut));
            proof
        } else {
            let mut proof = self.inclusion(start + cut, count - cut, index - cut);
            proof.push(self.root(start, cut));
            proof
        }
    }
    fn subproof(&mut self, start: usize, old: usize, new: usize, complete: bool) -> Vec<TreeHash> {
        if old == new {
            return if complete {
                Vec::new()
            } else {
                vec![self.root(start, new)]
            };
        }
        let cut = Self::cut(new);
        if old <= cut {
            let mut proof = self.subproof(start, old, cut, complete);
            proof.push(self.root(start + cut, new - cut));
            proof
        } else {
            let mut proof = self.subproof(start + cut, old - cut, new - cut, false);
            proof.push(self.root(start, cut));
            proof
        }
    }
    fn prefix(&mut self, old: usize, new: usize) -> Vec<TreeHash> {
        if old == 0 {
            Vec::new()
        } else {
            self.subproof(0, old, new, true)
        }
    }
}

#[test]
fn fixed_hash_domains_and_leaf_integer_bytes_are_explicit() {
    let context = TreeContext {
        profile: OwnerFactId::from_bytes([0x11; 32]),
        admission: OwnerFactId::from_bytes([0x22; 32]),
    };
    let value = AcceptanceLeaf {
        channel: 0x0102030405060708090a0b0c0d0e0f10,
        acceptance: OwnerFactId::from_bytes([0x33; 32]),
    };
    let mut frozen_frame = b"mewtual/chat/acceptance-leaf/v2\0".to_vec();
    frozen_frame.extend_from_slice(&[0x11; 32]);
    frozen_frame.extend_from_slice(&[0x22; 32]);
    frozen_frame.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    frozen_frame.extend_from_slice(&[0x33; 32]);
    assert_eq!(context.leaf_hash(value), hash_parts(&[&frozen_frame]));
    assert_ne!(context.empty_hash(), context.leaf_hash(value));
    assert_ne!(
        context.leaf_hash(value),
        reference_parent(context.leaf_hash(value), context.leaf_hash(value))
    );
}

#[test]
fn every_small_inclusion_and_prefix_matches_the_recursive_reference() {
    let mut reference = Reference::new(257);
    for new_count in 0..=257 {
        let new = reference.tip(new_count);
        for index in 0..new_count {
            let proof = reference.inclusion(0, new_count, index);
            assert!(verify_inclusion(new, leaf(index as u64), index as u64, &proof).is_ok());
        }
        for old_count in 0..=new_count {
            let old = reference.tip(old_count);
            let proof = reference.prefix(old_count, new_count);
            assert!(
                verify_prefix(old, new, &proof).is_ok(),
                "prefix {old_count}/{new_count}"
            );
        }
    }
}

#[test]
fn inclusion_rejects_wrong_leaf_context_index_count_and_path_shape() {
    let mut reference = Reference::new(37);
    let tip = reference.tip(37);
    for index in 0..37 {
        let proof = reference.inclusion(0, 37, index);
        let mut different = leaf(index as u64);
        different.channel += 1;
        assert!(verify_inclusion(tip, different, index as u64, &proof).is_err());
        different = leaf(index as u64 + 1000);
        assert!(verify_inclusion(tip, different, index as u64, &proof).is_err());
        assert!(
            verify_inclusion(tip, leaf(index as u64), (index as u64 + 1) % 37, &proof).is_err()
        );
        let mut other_context = tip;
        other_context.context.admission = OwnerFactId::from_bytes([3; 32]);
        assert!(verify_inclusion(other_context, leaf(index as u64), index as u64, &proof).is_err());
        let mut other_count = tip;
        other_count.count -= 1;
        assert!(verify_inclusion(other_count, leaf(index as u64), index as u64, &proof).is_err());
        for at in 0..proof.len() {
            let mut mutated = proof.clone();
            mutated[at] = TreeHash::from_bytes([0xff; 32]);
            assert!(verify_inclusion(tip, leaf(index as u64), index as u64, &mutated).is_err());
        }
        let mut extra = proof.clone();
        extra.push(TreeHash::from_bytes([0; 32]));
        assert!(verify_inclusion(tip, leaf(index as u64), index as u64, &extra).is_err());
        assert!(verify_inclusion(tip, leaf(index as u64), index as u64, &proof[1..]).is_err());
    }
    assert!(verify_inclusion(reference.tip(0), leaf(0), 0, &[]).is_err());
    assert!(verify_inclusion(tip, leaf(0), 37, &[]).is_err());
    assert!(verify_inclusion(tip, leaf(0), 0, &[tip.root; 65]).is_err());
}

#[test]
fn prefix_rejects_mutations_swaps_short_extra_and_cross_context_even_at_empty_or_equal() {
    let mut reference = Reference::new(65);
    for (old_count, new_count) in [(1, 2), (3, 4), (7, 15), (16, 65), (63, 65)] {
        let old = reference.tip(old_count);
        let new = reference.tip(new_count);
        let proof = reference.prefix(old_count, new_count);
        for at in 0..proof.len() {
            let mut changed = proof.clone();
            changed[at] = TreeHash::from_bytes([0xff; 32]);
            assert!(verify_prefix(old, new, &changed).is_err());
        }
        assert!(verify_prefix(old, new, &proof[1..]).is_err());
        let mut extra = proof.clone();
        extra.push(new.root);
        assert!(verify_prefix(old, new, &extra).is_err());
        if proof.len() > 1 {
            let mut swapped = proof.clone();
            swapped.swap(0, 1);
            assert!(verify_prefix(old, new, &swapped).is_err());
        }
        assert!(verify_prefix(new, old, &proof).is_err());
        let mut other = new;
        other.context.profile = OwnerFactId::from_bytes([9; 32]);
        assert_eq!(verify_prefix(old, other, &proof), Err(TreeError::Context));
    }
    let empty = reference.tip(0);
    let one = reference.tip(1);
    assert!(verify_prefix(empty, one, &[]).is_ok());
    assert!(verify_prefix(empty, one, &[one.root]).is_err());
    let mut wrong_empty = empty;
    wrong_empty.root = one.root;
    assert!(verify_prefix(wrong_empty, one, &[]).is_err());
    assert!(verify_prefix(one, one, &[]).is_ok());
    assert!(verify_prefix(one, one, &[one.root]).is_err());
    let mut other = one;
    other.context.admission = OwnerFactId::from_bytes([8; 32]);
    assert_eq!(verify_prefix(empty, other, &[]), Err(TreeError::Context));
    assert_eq!(verify_prefix(one, other, &[]), Err(TreeError::Context));
    assert!(verify_prefix(empty, one, &[one.root; 66]).is_err());
}

#[test]
fn append_partitions_match_recursive_roots_and_emit_each_exact_complete_node_once() {
    let leaves: Vec<_> = (0..257).map(leaf).collect();
    let mut reference = Reference::new(leaves.len());
    for batch_size in [1, 2, 3, 7, 16, 64] {
        let mut frontier = Frontier::empty(context());
        let mut ranges = BTreeSet::new();
        for batch in leaves.chunks(batch_size) {
            let original = frontier.clone();
            let invoice = append_nodes(frontier.tip().count, batch.len()).unwrap();
            let prepared = frontier.prepare_append(batch).unwrap();
            assert_eq!(frontier, original);
            assert_eq!(prepared.old_tip(), original.tip());
            assert_eq!(prepared.created_nodes().len(), invoice.total());
            let after = prepared.new_tip();
            assert_eq!(after, reference.tip(after.count as usize));
            for node in prepared.created_nodes() {
                assert!(node.count.is_power_of_two());
                assert_eq!(node.first % node.count, 0);
                assert!(node.first.checked_add(node.count).unwrap() <= after.count);
                assert!(ranges.insert((node.first, node.count)));
                assert_eq!(
                    node.hash,
                    reference.root(node.first as usize, node.count as usize)
                );
            }
            let exported: Vec<_> = prepared.frontier().hashes().collect();
            assert_eq!(
                Frontier::from_tip(after, &exported).unwrap(),
                *prepared.frontier()
            );
            frontier = prepared.into_frontier();
        }
        assert_eq!(frontier.tip(), reference.tip(257));
        assert_eq!(ranges.len(), 2 * 257 - 257usize.count_ones() as usize);
    }
}

#[test]
fn invalid_batch_or_frontier_never_mutates_the_original_and_cross_batch_lookup_is_external() {
    let frontier = Frontier::empty(context())
        .prepare_append(&[leaf(1), leaf(2), leaf(3)])
        .unwrap()
        .into_frontier();
    let before = frontier.clone();
    for batch in [
        vec![],
        vec![leaf(3), leaf(3)],
        vec![leaf(4), leaf(2)],
        vec![leaf(9); 65],
    ] {
        assert_eq!(
            frontier.prepare_append(&batch).unwrap_err(),
            TreeError::Batch
        );
        assert_eq!(frontier, before);
    }
    let tip = frontier.tip();
    let hashes: Vec<_> = frontier.hashes().collect();
    assert!(Frontier::from_tip(tip, &hashes[..1]).is_err());
    let mut wrong_root = tip;
    wrong_root.root = context().empty_hash();
    assert!(Frontier::from_tip(wrong_root, &hashes).is_err());
    let mut reordered = hashes.clone();
    reordered.swap(0, 1);
    assert!(Frontier::from_tip(tip, &reordered).is_err());
    let mut wrong_empty = Frontier::empty(context()).tip();
    wrong_empty.root = tip.root;
    assert!(Frontier::from_tip(wrong_empty, &[]).is_err());
    // No false idempotency claim: only the real retained owner lookup can reject an already
    // accepted fact in a DIFFERENT batch. Here it is deliberately another mathematical leaf.
    assert_eq!(
        frontier.prepare_append(&[leaf(3)]).unwrap().new_tip().count,
        4
    );
}

// Large-count fixtures use independent SUBPROOF ranges. Opaque hashes stand in for complete
// committed subtrees, never imagined allocations of2^64 leaves or action authority. Both roots
// are independently reconstructed from the same disjoint proof segments.
fn wide_cut(count: u64) -> u64 {
    let mut cut = 1;
    while cut <= (count - 1) / 2 {
        cut *= 2;
    }
    cut
}
fn prefix_ranges(start: u64, old: u64, new: u64, complete: bool, out: &mut Vec<(u64, u64)>) {
    if old == new {
        if !complete {
            out.push((start, new));
        }
    } else {
        let cut = wide_cut(new);
        if old <= cut {
            prefix_ranges(start, old, cut, complete, out);
            out.push((start + cut, new - cut));
        } else {
            prefix_ranges(start + cut, old - cut, new - cut, false, out);
            out.push((start, cut));
        }
    }
}
fn opaque(range: (u64, u64)) -> TreeHash {
    hash_parts(&[
        b"test opaque subtree",
        &range.0.to_be_bytes(),
        &range.1.to_be_bytes(),
    ])
}
fn root_from_parts(start: u64, count: u64, parts: &BTreeMap<(u64, u64), TreeHash>) -> TreeHash {
    if let Some(root) = parts.get(&(start, count)) {
        return *root;
    }
    assert!(count > 1, "missing reference subtree {start}/{count}");
    let cut = wide_cut(count);
    reference_parent(
        root_from_parts(start, cut, parts),
        root_from_parts(start + cut, count - cut, parts),
    )
}

#[test]
fn historical_prefix_really_needs_65_hashes_at_the_u64_boundary() {
    for (old_count, new_count, expected_len) in [(7, 15, 5), ((1u64 << 63) - 1, u64::MAX, 65)] {
        let mut ranges = Vec::new();
        prefix_ranges(0, old_count, new_count, true, &mut ranges);
        assert_eq!(ranges.len(), expected_len);
        let mut parts: BTreeMap<_, _> = ranges
            .iter()
            .map(|range| (*range, opaque(*range)))
            .collect();
        if old_count.is_power_of_two() {
            parts.insert((0, old_count), opaque((0, old_count)));
        }
        let old = TreeTip {
            context: context(),
            count: old_count,
            root: root_from_parts(0, old_count, &parts),
        };
        let new = TreeTip {
            context: context(),
            count: new_count,
            root: root_from_parts(0, new_count, &parts),
        };
        let proof: Vec<_> = ranges.iter().map(|range| parts[range]).collect();
        assert!(verify_prefix(old, new, &proof).is_ok());
        assert!(verify_prefix(old, new, &proof[..proof.len() - 1]).is_err());
    }
}

#[test]
fn checked_carry_invoice_and_frontier_handle_u64_limits_without_unbounded_history() {
    assert_eq!(append_nodes(u64::MAX, 1), Err(TreeError::Count));
    assert_eq!(append_nodes(u64::MAX - 63, 64), Err(TreeError::Count));
    assert_eq!(append_nodes(0, 0), Err(TreeError::Batch));
    assert_eq!(append_nodes(0, 65), Err(TreeError::Batch));
    let mut observed_max = 0;
    for batch in 1..=64 {
        for bit in 0..64 {
            let power = 1u64 << bit;
            if let Some(count) = power.checked_sub(batch as u64) {
                let invoice = append_nodes(count, batch).unwrap();
                assert_eq!(
                    invoice.internal(),
                    count.count_ones() as usize + batch - power.count_ones() as usize
                );
                assert!(invoice.internal() <= 120);
                observed_max = observed_max.max(invoice.internal());
            }
        }
    }
    assert_eq!(observed_max, 120);
    let count = (1u64 << 63) - 64;
    let mut parts = BTreeMap::new();
    let mut hashes = Vec::new();
    for bit in 0..64 {
        if count >> bit & 1 != 0 {
            let width = 1u64 << bit;
            let first = count & !((width - 1) | width);
            let range = (first, width);
            parts.insert(range, opaque(range));
            hashes.push(opaque(range));
        }
    }
    let tip = TreeTip {
        context: context(),
        count,
        root: root_from_parts(0, count, &parts),
    };
    let frontier = Frontier::from_tip(tip, &hashes).unwrap();
    let prepared = frontier
        .prepare_append(&(0..64).map(leaf).collect::<Vec<_>>())
        .unwrap();
    assert_eq!(prepared.new_tip().count, 1u64 << 63);
    assert_eq!(prepared.created_nodes().len(), 64 + 120);
    assert_eq!(prepared.created_nodes().last().unwrap().count, 1u64 << 63);
    assert_eq!(frontier.tip(), tip);
}

#[test]
fn inclusion_at_u64_max_uses_only_64_siblings_and_a_fixed_path_stack() {
    let count = u64::MAX;
    let mut ranges = Vec::new();
    let mut remaining = count;
    while remaining > 1 {
        let cut = wide_cut(remaining);
        ranges.push((cut, remaining - cut));
        remaining = cut;
        // This independent fixture follows index0; its range start always stays zero.
    }
    ranges.reverse();
    assert_eq!(ranges.len(), 64);
    let mut parts: BTreeMap<_, _> = ranges
        .iter()
        .map(|range| (*range, opaque(*range)))
        .collect();
    parts.insert((0, 1), reference_leaf(context(), leaf(0)));
    let tip = TreeTip {
        context: context(),
        count,
        root: root_from_parts(0, count, &parts),
    };
    let proof: Vec<_> = ranges.iter().map(|range| parts[range]).collect();
    assert!(verify_inclusion(tip, leaf(0), 0, &proof).is_ok());
    assert!(verify_inclusion(tip, leaf(0), 0, &proof[..63]).is_err());
}
