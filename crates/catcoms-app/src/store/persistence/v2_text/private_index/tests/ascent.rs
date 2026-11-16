//! Actual encrypted multi-level COW DATA, without a selected-source/acceptance claim.
//! Every sibling in this fixture has a real encrypted page; no child maximum or page
//! identity is fabricated to stand in for an unread authenticated boundary.
use super::*;
use catcoms_storage::private_chat_page::open_private_chat_page;

struct DeepFixture {
    context: PrivateIndexContextData,
    leaves: Vec<(PreparedPrivateChatPage, PrivateIndexChildRefData)>,
    branches: Vec<(PreparedPrivateChatPage, PrivateIndexChildRefData)>,
    branch_plain: Vec<Zeroizing<Vec<u8>>>,
    root: PreparedPrivateChatPage,
    root_ref: NodeRefData,
    root_plain: Zeroizing<Vec<u8>>,
}

fn branch_page(
    context: PrivateIndexContextData,
    refs: &[PrivateIndexChildRefData],
    placement: NodePlacementData,
    rng: &mut ChaCha20Rng,
) -> (
    PreparedPrivateChatPage,
    PrivateIndexChildRefData,
    Zeroizing<Vec<u8>>,
) {
    let plain = encode_private_branch_data(context, refs, placement).unwrap();
    let summary = PrivateIndexNodeFrame::decode(&plain, context, placement)
        .unwrap()
        .summary();
    let page = PreparedPrivateChatPage::prepare(&KEY, &plain, rng).unwrap();
    let reference = PrivateIndexChildRefData {
        context,
        node: NodeRefData {
            page: page.id(),
            summary,
        },
    };
    (page, reference, plain)
}

fn deep_fixture() -> DeepFixture {
    let context = context(PrivateIndexKindData::Publication);
    let mut rng = ChaCha20Rng::seed_from_u64(0xd33f);
    let leaves = (0..192u64)
        .map(|index| {
            let count = if index == 0 { 32 } else { 16 };
            let rows = (0..count)
                .map(|n| publication(index * 64 + n))
                .collect::<Vec<_>>();
            sealed(context, &rows, NodePlacementData::Child, &mut rng)
        })
        .collect::<Vec<_>>();
    let mut branches = Vec::with_capacity(2);
    let mut branch_plain = Vec::with_capacity(2);
    for offset in [0, 96] {
        let refs = leaves[offset..offset + 96]
            .iter()
            .map(|(_, reference)| *reference)
            .collect::<Vec<_>>();
        let (page, reference, plain) =
            branch_page(context, &refs, NodePlacementData::Child, &mut rng);
        branches.push((page, reference));
        branch_plain.push(plain);
    }
    let refs = branches
        .iter()
        .map(|(_, reference)| *reference)
        .collect::<Vec<_>>();
    let (root, reference, root_plain) =
        branch_page(context, &refs, NodePlacementData::Root, &mut rng);
    assert_eq!(reference.node.summary.height, 2);
    assert_eq!(reference.node.summary.keys, 3088);
    DeepFixture {
        context,
        leaves,
        branches,
        branch_plain,
        root,
        root_ref: reference.node,
        root_plain,
    }
}

fn seal_rewrite(
    context: PrivateIndexContextData,
    rewrite: &PrivateIndexRewriteData,
    rng: &mut ChaCha20Rng,
) -> Vec<(PreparedPrivateChatPage, PrivateIndexChildRefData)> {
    rewrite
        .nodes_data()
        .iter()
        .map(|node| {
            let page = PreparedPrivateChatPage::prepare(&KEY, node.plain_data(), rng).unwrap();
            let reference = PrivateIndexChildRefData {
                context,
                node: NodeRefData {
                    page: page.id(),
                    summary: node.summary_data(),
                },
            };
            (page, reference)
        })
        .collect()
}

#[test]
fn encrypted_two_level_leaf_split_ascends_and_replays_without_changing_sibling_rows() {
    let f = deep_fixture();
    let mut path = PrivateIndexPathData::new_data(f.context, f.root_ref, ordinal(32)).unwrap();
    path.accept_frame_data(&KEY, f.root.as_bytes()).unwrap();
    assert_eq!(path.next_page_data().unwrap(), Some(f.branches[0].0.id()));
    path.accept_frame_data(&KEY, f.branches[0].0.as_bytes())
        .unwrap();
    assert_eq!(path.next_page_data().unwrap(), Some(f.leaves[0].0.id()));
    path.accept_frame_data(&KEY, f.leaves[0].0.as_bytes())
        .unwrap();
    assert!(path.row_data().unwrap().is_none());
    let rewritten_leaf = path.rewrite_leaf_data(&publication(32)).unwrap();
    assert_eq!(rewritten_leaf.nodes_data().len(), 2);
    let left = PrivateIndexNodeFrame::decode(
        rewritten_leaf.nodes_data()[0].plain_data(),
        f.context,
        NodePlacementData::Child,
    )
    .unwrap();
    let right = PrivateIndexNodeFrame::decode(
        rewritten_leaf.nodes_data()[1].plain_data(),
        f.context,
        NodePlacementData::Child,
    )
    .unwrap();
    assert_eq!((left.row_count(), right.row_count()), (16, 17));
    assert_eq!(
        (left.summary().min, left.summary().max),
        (ordinal(0), ordinal(15))
    );
    assert_eq!(
        (right.summary().min, right.summary().max),
        (ordinal(16), ordinal(32))
    );
    let mut rng = ChaCha20Rng::seed_from_u64(0xd340);
    let leaf_pages = seal_rewrite(f.context, &rewritten_leaf, &mut rng);
    let leaf_refs = leaf_pages
        .iter()
        .map(|(_, reference)| *reference)
        .collect::<Vec<_>>();
    assert!(path.boundary_page_data(1, 2).unwrap().is_none());
    let rewritten_branch = path.rewrite_branch_data(1, &leaf_refs, None).unwrap();
    assert_eq!(rewritten_branch.nodes_data().len(), 1);
    let branch = PrivateIndexNodeFrame::decode(
        rewritten_branch.nodes_data()[0].plain_data(),
        f.context,
        NodePlacementData::Child,
    )
    .unwrap();
    assert_eq!(branch.row_count(), 97);
    assert_eq!(branch.summary().keys, 1553);
    let old_branch =
        PrivateIndexNodeFrame::decode(&f.branch_plain[0], f.context, NodePlacementData::Child)
            .unwrap();
    assert_eq!(&old_branch.packed[80..], &branch.packed[160..]);
    assert_eq!(branch.summary().max, old_branch.summary().max);
    let branch_pages = seal_rewrite(f.context, &rewritten_branch, &mut rng);
    let branch_refs = branch_pages
        .iter()
        .map(|(_, reference)| *reference)
        .collect::<Vec<_>>();
    assert!(path.boundary_page_data(0, 1).unwrap().is_none());
    let rewritten_root = path.rewrite_branch_data(0, &branch_refs, None).unwrap();
    assert_eq!(rewritten_root.nodes_data().len(), 1);
    let root = PrivateIndexNodeFrame::decode(
        rewritten_root.nodes_data()[0].plain_data(),
        f.context,
        NodePlacementData::Root,
    )
    .unwrap();
    assert_eq!(root.row_count(), 2);
    assert_eq!(root.summary().height, 2);
    assert_eq!(root.summary().keys, 3089);
    let old_root =
        PrivateIndexNodeFrame::decode(&f.root_plain, f.context, NodePlacementData::Root).unwrap();
    assert_eq!(&old_root.packed[80..], &root.packed[80..]);
    assert_eq!(root.summary().max, old_root.summary().max);
    let root_pages = seal_rewrite(f.context, &rewritten_root, &mut rng);
    let mut replay =
        PrivateIndexPathData::new_data(f.context, root_pages[0].1.node, ordinal(32)).unwrap();
    replay
        .accept_frame_data(&KEY, root_pages[0].0.as_bytes())
        .unwrap();
    replay
        .accept_frame_data(&KEY, branch_pages[0].0.as_bytes())
        .unwrap();
    assert_eq!(replay.next_page_data().unwrap(), Some(leaf_pages[1].0.id()));
    replay
        .accept_frame_data(&KEY, leaf_pages[1].0.as_bytes())
        .unwrap();
    let row = replay.row_data().unwrap().unwrap();
    assert_eq!(row.key, ordinal(32));
    assert_eq!(row.value, publication(32).value);
    assert!(replay.next_page_data().unwrap().is_none());
}

#[test]
fn encrypted_deep_path_rejects_wrong_context_and_actual_child_summary_without_losing_cursor() {
    let f = deep_fixture();
    let mut rng = ChaCha20Rng::seed_from_u64(0xd341);
    let foreign = PrivateIndexContextData {
        cid: TextContextIdData::from_bytes([77; 32]),
        ..f.context
    };
    let refs = f.leaves[..96]
        .iter()
        .map(|(_, reference)| PrivateIndexChildRefData {
            context: foreign,
            node: reference.node,
        })
        .collect::<Vec<_>>();
    let (foreign_page, foreign_ref, _) =
        branch_page(foreign, &refs, NodePlacementData::Child, &mut rng);
    let mut children = f
        .branches
        .iter()
        .map(|(_, reference)| *reference)
        .collect::<Vec<_>>();
    // A malformed DATA parent points to a real authenticated foreign-Cid child. This
    // does not claim such a parent was ever selected by the real Root/Current issuer.
    children[0].node.page = foreign_page.id();
    assert_eq!(children[0].node.summary, foreign_ref.node.summary);
    let (root, root_ref, _) = branch_page(f.context, &children, NodePlacementData::Root, &mut rng);
    let mut path = PrivateIndexPathData::new_data(f.context, root_ref.node, ordinal(0)).unwrap();
    path.accept_frame_data(&KEY, root.as_bytes()).unwrap();
    assert_eq!(path.next_page_data().unwrap(), Some(foreign_page.id()));
    assert_eq!(
        path.accept_frame_data(&KEY, foreign_page.as_bytes())
            .unwrap_err(),
        PrivateIndexError::Context
    );
    assert_eq!(path.next_page_data().unwrap(), Some(foreign_page.id()));
    assert!(path.row_data().is_err());
    assert_eq!(
        path.accept_frame_data(&KEY, f.branches[0].0.as_bytes())
            .unwrap_err(),
        PrivateIndexError::Authentication
    );
    assert_eq!(path.next_page_data().unwrap(), Some(foreign_page.id()));

    let mut wrong_summary = f
        .branches
        .iter()
        .map(|(_, reference)| *reference)
        .collect::<Vec<_>>();
    // Preserve a valid root count/range, but lie about the selected child's exact count.
    wrong_summary[0].node.summary.keys += 1;
    let (root, root_ref, _) =
        branch_page(f.context, &wrong_summary, NodePlacementData::Root, &mut rng);
    let mut path = PrivateIndexPathData::new_data(f.context, root_ref.node, ordinal(0)).unwrap();
    path.accept_frame_data(&KEY, root.as_bytes()).unwrap();
    assert!(path
        .accept_frame_data(&KEY, f.branches[0].0.as_bytes())
        .is_err());
    assert_eq!(path.next_page_data().unwrap(), Some(f.branches[0].0.id()));
    assert!(path.row_data().is_err());
    let plain =
        open_private_chat_page(&KEY, f.branches[0].0.id(), f.branches[0].0.as_bytes()).unwrap();
    assert_eq!(
        PrivateIndexNodeFrame::decode(&plain, f.context, NodePlacementData::Child)
            .unwrap()
            .summary()
            .keys,
        1552
    );
}
