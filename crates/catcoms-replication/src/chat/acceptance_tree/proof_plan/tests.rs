//! Reference plans recurse over mathematical ranges, not the production planner's traversal or
//! cold-record positions. These tests exercise only hash mathematics, not authenticated owners.
use super::*;
use crate::chat::acceptance_tree::{
    verify_inclusion, verify_prefix, AcceptanceLeaf, TreeContext, TreeTip,
};
use crate::chat::owner_facts::OwnerFactId;

fn context() -> TreeContext {
    TreeContext {
        profile: OwnerFactId::from_bytes([31; 32]),
        admission: OwnerFactId::from_bytes([32; 32]),
    }
}
fn leaf(index: u64) -> AcceptanceLeaf {
    let mut acceptance = [0; 32];
    acceptance[..8].copy_from_slice(&index.to_be_bytes());
    AcceptanceLeaf {
        channel: u128::from(index / 7),
        acceptance: OwnerFactId::from_bytes(acceptance),
    }
}
fn reference_split(count: u64) -> u64 {
    // Deliberately use a different iterative calculation from production leading_zeros.
    let mut power = 1;
    while power <= (count - 1) / 2 {
        power *= 2;
    }
    power
}
fn reference_inclusion(first: u64, count: u64, index: u64, out: &mut Vec<(u64, u64)>) {
    if count == 1 {
        return;
    }
    let left = reference_split(count);
    if index < left {
        reference_inclusion(first, left, index, out);
        out.push((first + left, count - left));
    } else {
        reference_inclusion(first + left, count - left, index - left, out);
        out.push((first, left));
    }
}
fn reference_prefix(first: u64, old: u64, count: u64, known: bool, out: &mut Vec<(u64, u64)>) {
    if old == count {
        if !known {
            out.push((first, count));
        }
        return;
    }
    let left = reference_split(count);
    if old <= left {
        reference_prefix(first, old, left, known, out);
        out.push((first + left, count - left));
    } else {
        reference_prefix(first + left, old - left, count - left, false, out);
        out.push((first, left));
    }
}
fn tuples(plan: &ProofPlan) -> Vec<(u64, u64)> {
    plan.ranges()
        .iter()
        .map(|r| (r.first(), r.count()))
        .collect()
}
fn reference_root(first: u64, count: u64) -> TreeHash {
    if count == 0 {
        return context().empty_hash();
    }
    if count == 1 {
        return context().leaf_hash(leaf(first));
    }
    let left = reference_split(count);
    parent(
        reference_root(first, left),
        reference_root(first + left, count - left),
    )
}
fn proof_hashes(plan: &ProofPlan) -> Vec<TreeHash> {
    plan.ranges()
        .iter()
        .map(|range| {
            let nodes: Vec<_> = range
                .complete_nodes_right_to_left()
                .map(|node| reference_root(node.first(), node.count()))
                .collect();
            let hash = range.hash_from_nodes(&nodes).unwrap();
            assert_eq!(hash, reference_root(range.first(), range.count()));
            hash
        })
        .collect()
}

#[test]
fn every_small_plan_has_exact_independent_recursive_range_order() {
    for count in 1..=128 {
        for index in 0..count {
            let plan = inclusion_ranges(count, index).unwrap();
            let mut expected = Vec::new();
            reference_inclusion(0, count, index, &mut expected);
            assert_eq!(tuples(&plan), expected, "inclusion {index}/{count}");
        }
        for old in 0..=count {
            let plan = prefix_ranges(old, count).unwrap();
            let mut expected = Vec::new();
            if old != 0 && old != count {
                reference_prefix(0, old, count, true, &mut expected);
            }
            assert_eq!(tuples(&plan), expected, "prefix {old}/{count}");
        }
    }
}

#[test]
fn reconstructed_complete_nodes_form_actual_inclusion_and_prefix_proofs() {
    for count in 1..=40 {
        let tip = TreeTip {
            context: context(),
            count,
            root: reference_root(0, count),
        };
        for index in 0..count {
            let hashes = proof_hashes(&inclusion_ranges(count, index).unwrap());
            verify_inclusion(tip, leaf(index), index, &hashes).unwrap();
            if !hashes.is_empty() {
                let mut wrong = hashes;
                wrong[0] = context().empty_hash();
                assert_eq!(
                    verify_inclusion(tip, leaf(index), index, &wrong),
                    Err(TreeError::Proof),
                    "a geometric plan never authenticates substituted node bytes"
                );
            }
        }
        for old_count in 0..=count {
            let hashes = proof_hashes(&prefix_ranges(old_count, count).unwrap());
            let old = TreeTip {
                context: context(),
                count: old_count,
                root: reference_root(0, old_count),
            };
            verify_prefix(old, tip, &hashes).unwrap();
        }
    }
}

fn inspect_plan(plan: &ProofPlan, total: u64) {
    let mut reads = 0;
    for range in plan.ranges() {
        assert!(range.count() > 0);
        let mut end = range.first().checked_add(range.count()).unwrap();
        assert!(end <= total);
        let mut nodes = range.complete_nodes_right_to_left();
        let mut remaining = range.node_reads();
        while let Some(node) = nodes.next() {
            remaining -= 1;
            assert_eq!(nodes.len(), remaining);
            assert!(node.count().is_power_of_two());
            assert_eq!(node.first() % node.count(), 0);
            assert_eq!(node.first().checked_add(node.count()), Some(end));
            let ordinal = node.creation_ordinal().unwrap();
            let last = creation_ordinal(total - 1, 1).unwrap();
            // Carries after the final leaf can follow that leaf's own ordinal.
            assert!(ordinal <= last + u128::from(total.trailing_zeros()));
            end = node.first();
            reads += 1;
        }
        assert_eq!(end, range.first());
        assert!(nodes.next().is_none());
        assert!(nodes.next().is_none());
        assert!(range.node_reads() <= 64);
    }
    assert_eq!(reads, plan.node_reads());
    assert!(reads <= MAX_PROOF_NODE_READS);
}

#[test]
fn actual_u64_boundary_plans_have_checked_ranges_decompositions_and_wide_ordinals() {
    for count in [
        1,
        2,
        (1 << 63) - 1,
        1 << 63,
        (1 << 63) + 1,
        u64::MAX - 1,
        u64::MAX,
    ] {
        for index in [0, count / 2, count - 1] {
            let plan = inclusion_ranges(count, index).unwrap();
            assert!(plan.ranges().len() <= super::super::MAX_INCLUSION_HASHES);
            inspect_plan(&plan, count);
            let mut reference = Vec::new();
            reference_inclusion(0, count, index, &mut reference);
            assert_eq!(tuples(&plan), reference);
        }
        for old in [0, 1, count / 2, count - 1, count] {
            let plan = prefix_ranges(old, count).unwrap();
            assert!(plan.ranges().len() <= super::super::MAX_CONSISTENCY_HASHES);
            inspect_plan(&plan, count);
            let mut reference = Vec::new();
            if old != 0 && old != count {
                reference_prefix(0, old, count, true, &mut reference);
            }
            assert_eq!(tuples(&plan), reference);
        }
    }
    let range = prefix_ranges(u64::MAX - 1, u64::MAX).unwrap();
    assert!(range
        .ranges()
        .iter()
        .flat_map(|range| range.complete_nodes_right_to_left())
        .any(|node| node.creation_ordinal().unwrap() > u128::from(u64::MAX)));
}

#[test]
fn distinct_inclusion64_and_consistency65_limits_have_real_plans() {
    let inclusion = inclusion_ranges(u64::MAX, 0).unwrap();
    assert_eq!(inclusion.ranges().len(), 64);
    inspect_plan(&inclusion, u64::MAX);
    let prefix = prefix_ranges((1 << 63) - 1, u64::MAX).unwrap();
    assert_eq!(prefix.ranges().len(), 65);
    inspect_plan(&prefix, u64::MAX);
}

#[test]
fn malformed_counts_and_arbitrary_unaligned_ranges_cannot_create_bounded_plans() {
    assert_eq!(inclusion_ranges(0, 0), Err(TreeError::Proof));
    assert_eq!(inclusion_ranges(4, 4), Err(TreeError::Proof));
    assert_eq!(inclusion_ranges(1, u64::MAX), Err(TreeError::Proof));
    assert_eq!(prefix_ranges(1, 0), Err(TreeError::Proof));
    assert_eq!(prefix_ranges(u64::MAX, u64::MAX - 1), Err(TreeError::Proof));
    for (first, count) in [(0, 0), (1, u64::MAX - 1), (1 << 63, 1 << 63), (3, 2)] {
        assert_eq!(
            SubtreeRange::new(first, count, u64::MAX),
            Err(TreeError::Node)
        );
    }
    assert_eq!(SubtreeRange::new(4, 3, 6), Err(TreeError::Node));
    let valid = SubtreeRange::new(1 << 63, (1 << 63) - 1, u64::MAX).unwrap();
    assert_eq!(valid.node_reads(), 63);
    inspect_plan(
        &ProofPlan {
            ranges: vec![valid],
        },
        u64::MAX,
    );
}

#[test]
fn range_folding_is_exact_right_association_and_refuses_short_or_extra_hashes() {
    let range = SubtreeRange::new(8, 7, 15).unwrap();
    let nodes: Vec<_> = range
        .complete_nodes_right_to_left()
        .map(|node| reference_root(node.first(), node.count()))
        .collect();
    assert_eq!(range.hash_from_nodes(&nodes).unwrap(), reference_root(8, 7));
    assert_eq!(range.hash_from_nodes(&nodes[..2]), Err(TreeError::Proof));
    let mut extra = nodes.clone();
    extra.push(context().empty_hash());
    assert_eq!(range.hash_from_nodes(&extra), Err(TreeError::Proof));
    let mut reversed = nodes;
    reversed.reverse();
    assert_ne!(
        range.hash_from_nodes(&reversed).unwrap(),
        reference_root(8, 7)
    );
}

#[test]
fn empty_equal_prefixes_do_not_claim_root_or_context_validation() {
    for count in [0, 1, u64::MAX] {
        let equal = prefix_ranges(count, count).unwrap();
        let empty = prefix_ranges(0, count).unwrap();
        assert!(equal.ranges().is_empty());
        assert!(empty.ranges().is_empty());
        assert_eq!(equal.node_reads(), 0);
        assert_eq!(empty.node_reads(), 0);
    }
    let bad = TreeTip {
        context: context(),
        count: 0,
        root: TreeHash::from_bytes([0; 32]),
    };
    assert_eq!(verify_prefix(bad, bad, &[]), Err(TreeError::Proof));
}
