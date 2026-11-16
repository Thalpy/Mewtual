//! Real encoded/AEAD-backed synthetic DATA. No selected Root, acceptance or I/O permission.
use super::*;

struct Child {
    plain: Zeroizing<Vec<u8>>,
    reference: ChildRefData<'static>,
}
fn child(start: u64, n: usize, seed: u64) -> Child {
    let ctx = context(IndexScopeData::Directory);
    let rows: Vec<_> = (0..n)
        .map(|i| LeafRowData {
            key: key(start + i as u64 * 5),
            value: ValueData::Directory(unknown()),
        })
        .collect();
    let plain = encode_leaf_data(ctx, &rows, NodePlacementData::Child).unwrap();
    let sealed = seal(&plain, seed);
    Child {
        plain,
        reference: ChildRefData {
            context: ctx,
            node: sealed.node,
        },
    }
}
fn sparse_parent(selected: usize) -> (Vec<Child>, Zeroizing<Vec<u8>>) {
    let children: Vec<_> = (0..MAX_BRANCH_ROWS)
        .map(|i| {
            child(
                100 + i as u64 * 1000,
                if i == selected { 32 } else { 16 },
                i as u64 + 1,
            )
        })
        .collect();
    let refs: Vec<_> = children.iter().map(|c| c.reference).collect();
    let plain = encode_branch_data(
        context(IndexScopeData::Directory),
        &refs,
        NodePlacementData::Root,
    )
    .unwrap();
    (children, plain)
}

#[test]
fn actual_sparse_split_boundary_on_each_side_and_in_replacement_never_invents_max() {
    for selected in [10, 94, 95, 160] {
        let (children, plain) = sparse_parent(selected);
        let parent = NodeFrame::decode(&plain, NodePlacementData::Root).unwrap();
        let start = 100 + selected as u64 * 1000;
        let replacement = [child(start, 16, 1000), child(start + 80, 17, 1001)];
        let replacements = [replacement[0].reference, replacement[1].reference];
        let required = parent.branch_split_boundary_data(selected, 2).unwrap();
        let boundary = required.as_ref().map(|d| {
            let c = children
                .iter()
                .find(|c| c.reference.node.page == d.expected_page_data())
                .unwrap();
            let actual = NodeFrame::decode(&c.plain, NodePlacementData::Child).unwrap();
            assert_eq!(d.check_child(&actual).unwrap(), c.reference.node);
            c.reference
        });
        if boundary.is_some() {
            assert!(rewrite_branch_data(
                &parent,
                selected,
                &children[selected].reference,
                &replacements,
                None,
                NodePlacementData::Root
            )
            .is_err());
        }
        let result = rewrite_branch_data(
            &parent,
            selected,
            &children[selected].reference,
            &replacements,
            boundary.as_ref(),
            NodePlacementData::Root,
        )
        .unwrap();
        assert_eq!(result.nodes_data().len(), 2);
        let left = NodeFrame::decode(
            result.nodes_data()[0].plain_data(),
            NodePlacementData::Child,
        )
        .unwrap();
        let right = NodeFrame::decode(
            result.nodes_data()[1].plain_data(),
            NodePlacementData::Child,
        )
        .unwrap();
        assert_eq!((left.row_count(), right.row_count()), (96, 97));
        assert_eq!(
            left.summary().keys + right.summary().keys,
            parent.summary().keys + 1
        );
        let expected_max = boundary.map_or(
            replacements[if selected == 94 { 1 } else { 0 }]
                .node
                .summary
                .max,
            |b| b.node.summary.max,
        );
        assert_eq!(left.summary().max, expected_max);
        assert!(left.summary().max < right.summary().min);
        assert!(
            left.summary().max < key(100 + 96 * 1000 - 1),
            "a sparse child max is not next lower minus one"
        );
        assert_eq!(right.summary().max, parent.summary().max);
        // Actual old packed rows at either end retain the exact original page/count/min.
        assert_eq!(left.branch_row(0).unwrap(), parent.branch_row(0).unwrap());
        assert_eq!(
            right.branch_row(96).unwrap(),
            parent.branch_row(191).unwrap()
        );
    }
}

#[test]
fn edge_descent_uses_real_child_and_rewrite_preserves_untouched_rows_and_parent_max() {
    let ctx = context(IndexScopeData::Directory);
    let children = [child(100, 32, 2000), child(1000, 16, 2001)];
    let refs = [children[0].reference, children[1].reference];
    let plain = encode_branch_data(ctx, &refs, NodePlacementData::Root).unwrap();
    let parent = NodeFrame::decode(&plain, NodePlacementData::Root).unwrap();
    // Key95 is outside the parent range. A writer must open the actual first child,
    // rather than interpreting a normal read-query range miss as a leaf insertion site.
    let edge = parent.branch_child_data(0).unwrap();
    assert_eq!(edge.lower_data(), key(100));
    let actual = NodeFrame::decode(&children[0].plain, NodePlacementData::Child).unwrap();
    edge.check_child(&actual).unwrap();
    let replacement = [child(95, 16, 2002), child(175, 17, 2003)];
    let replacements = [replacement[0].reference, replacement[1].reference];
    let result = rewrite_branch_data(
        &parent,
        0,
        &refs[0],
        &replacements,
        None,
        NodePlacementData::Root,
    )
    .unwrap();
    let result =
        NodeFrame::decode(result.nodes_data()[0].plain_data(), NodePlacementData::Root).unwrap();
    assert_eq!(result.summary().min, key(95));
    assert_eq!(result.summary().max, parent.summary().max);
    assert_eq!(result.branch_row(2).unwrap(), parent.branch_row(1).unwrap());
    assert_eq!(
        &result.packed[2 * BRANCH_ROW_BYTES..],
        &parent.packed[BRANCH_ROW_BYTES..]
    );
    assert!(parent.branch_child_data(2).is_err());
}

#[test]
fn foreign_original_or_boundary_and_duplicate_page_across_split_refuse() {
    let selected = 10;
    let (children, plain) = sparse_parent(selected);
    let parent = NodeFrame::decode(&plain, NodePlacementData::Root).unwrap();
    let replacement = [child(10100, 16, 3000), child(10180, 17, 3001)];
    let mut replacements = [replacement[0].reference, replacement[1].reference];
    let boundary = children[94].reference;
    for changed in 0..5 {
        let mut wrong = boundary;
        match changed {
            0 => wrong.node.page = page(254),
            1 => wrong.context.vault = [0; 32],
            2 => wrong.node.summary.keys += 1,
            3 => wrong.node.summary.height += 1,
            _ => wrong.node.summary.min = key(0),
        }
        assert!(rewrite_branch_data(
            &parent,
            selected,
            &children[selected].reference,
            &replacements,
            Some(&wrong),
            NodePlacementData::Root
        )
        .is_err());
    }
    assert!(rewrite_branch_data(
        &parent,
        selected,
        &children[11].reference,
        &replacements,
        Some(&boundary),
        NodePlacementData::Root
    )
    .is_err());
    // Each future output individually has unique pages; only the complete193-row check
    // catches this substituted reference to a page on the OTHER side of the split.
    replacements[0].node.page = children[150].reference.node.page;
    assert!(matches!(
        rewrite_branch_data(
            &parent,
            selected,
            &children[selected].reference,
            &replacements,
            Some(&boundary),
            NodePlacementData::Root
        ),
        Err(ChatError::Malformed)
    ));
}

#[test]
fn root_only_occupancy_cannot_be_relabelled_child_and_shared_claim_bound_stays_strict() {
    let ctx = context(IndexScopeData::Directory);
    let tiny = encode_leaf_data(
        ctx,
        &[LeafRowData {
            key: key(1),
            value: ValueData::Directory(unknown()),
        }],
        NodePlacementData::Root,
    )
    .unwrap();
    let tiny = NodeFrame::decode(&tiny, NodePlacementData::Root).unwrap();
    let (_, parent_bytes) = sparse_parent(10);
    let parent = NodeFrame::decode(&parent_bytes, NodePlacementData::Root).unwrap();
    assert!(parent
        .branch_child_data(0)
        .unwrap()
        .check_child(&tiny)
        .is_err());
    assert!(tiny.check_claim_bounds_data(None).is_ok());
    assert!(tiny.check_claim_bounds_data(Some(1)).is_err());
    let ctx = actions();
    let k = key(1);
    let raw = encode_leaf_data(
        ctx,
        &[LeafRowData {
            key: k,
            value: ValueData::Action(claim(ctx, k, 800, ClaimHint::Canonical)),
        }],
        NodePlacementData::Root,
    )
    .unwrap();
    let action = NodeFrame::decode(&raw, NodePlacementData::Root).unwrap();
    assert!(action.check_claim_bounds_data(None).is_err());
    assert!(action.check_claim_bounds_data(Some(800)).is_err());
    assert!(action.check_claim_bounds_data(Some(801)).is_ok());
}
