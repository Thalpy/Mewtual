//! Real page encryption over inert geometry. Synthetic Directory rows here are not the
//! sole-founder selected Directory, actual history, accepted absence or a query lease.
use super::*;
use catcoms_replication::chat::acceptance_tree::TreeContext;
use catcoms_replication::chat::cold_index::{
    encode_branch_data, DirectoryData, DirectoryTreeData, NodeSummaryData, ValueData,
};
use catcoms_replication::chat::cold_manifest::ArchiveHeadId;
use catcoms_replication::chat::owner_header::{OwnerBodyId, OwnerHeaderId};
use catcoms_storage::private_chat_page::PreparedPrivateChatPage;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
const KEY: [u8; 32] = [64; 32];
fn context() -> IndexContextData<'static> {
    IndexContextData {
        vault: [1; 32],
        group: b"inert geometry only",
        profile: OwnerFactId::from_bytes([2; 32]),
        scope: IndexScopeData::Directory,
    }
}
fn key(n: u64) -> [u8; 32] {
    let mut out = [0; 32];
    out[24..].copy_from_slice(&n.to_be_bytes());
    out
}
fn row(n: u64) -> LeafRowData {
    let k = key(n);
    LeafRowData {
        key: k,
        value: ValueData::Directory(DirectoryData {
            tree: DirectoryTreeData::Empty,
            accepted_leaf_count: 0,
            unique_action_count: 0,
            accepted_root: TreeContext {
                profile: context().profile,
                admission: OwnerFactId::from_bytes(k),
            }
            .empty_hash(),
            own_tip_header: OwnerHeaderId::from_bytes([3; 32]),
            tip_body: OwnerBodyId::from_bytes(k),
            prefix_manifest: PrivateChatPageId::from_bytes([4; 32]),
            prefix_sequence: 1,
            prefix_head: ArchiveHeadId::from_bytes([5; 32]),
        }),
    }
}
fn leaf(start: u64, n: usize, placement: NodePlacementData) -> Zeroizing<Vec<u8>> {
    encode_leaf_data(
        context(),
        &(0..n).map(|i| row(start + i as u64)).collect::<Vec<_>>(),
        placement,
    )
    .unwrap()
}
fn seal(
    raw: &[u8],
    placement: NodePlacementData,
    rng: &mut ChaCha20Rng,
) -> (PreparedPrivateChatPage, NodeRefData) {
    let page = PreparedPrivateChatPage::prepare(&KEY, raw, rng).unwrap();
    let reference = NodeRefData {
        page: page.id(),
        summary: NodeFrame::decode(raw, placement).unwrap().summary(),
    };
    (page, reference)
}
#[test]
fn actual_edge_path_and_leaf_split_preserve_rows_and_original_on_refusal() {
    let mut rng = ChaCha20Rng::seed_from_u64(43);
    let raw = leaf(10, 32, NodePlacementData::Root);
    let (page, root) = seal(&raw, NodePlacementData::Root, &mut rng);
    let mut path = AcceptedIndexPathData::new_data(context(), root, key(42), None).unwrap();
    assert!(path.row_data().is_err());
    assert_eq!(path.next_page_data().unwrap(), Some(root.page));
    let (foreign, _) = seal(&raw, NodePlacementData::Root, &mut rng);
    assert!(path.accept_frame_data(&KEY, foreign.as_bytes()).is_err());
    assert_eq!(path.next_page_data().unwrap(), Some(root.page));
    path.accept_frame_data(&KEY, page.as_bytes()).unwrap();
    assert!(path.next_page_data().unwrap().is_none());
    assert!(path.row_data().unwrap().is_none());
    assert!(path.accept_frame_data(&KEY, page.as_bytes()).is_err());
    let split = path.rewrite_leaf_data(row(42)).unwrap();
    assert_eq!(split.len(), 2);
    assert_eq!(
        NodeFrame::decode(&split[0], NodePlacementData::Child)
            .unwrap()
            .row_count(),
        16
    );
    assert_eq!(
        NodeFrame::decode(&split[1], NodePlacementData::Child)
            .unwrap()
            .row_count(),
        17
    );
    let keys: Vec<_> = split
        .iter()
        .flat_map(|raw| {
            let node = NodeFrame::decode(raw, NodePlacementData::Child).unwrap();
            (0..node.row_count())
                .map(|i| node.leaf_row(i).unwrap().key)
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(keys, (10..=42).map(key).collect::<Vec<_>>());
    assert!(path.rewrite_leaf_data(row(43)).is_err()); // cannot silently retarget the request
    assert_eq!(format!("{path:?}"), "AcceptedIndexPathData { .. }");
}
#[test]
fn branch_split_requires_actual_boundary_child_not_next_lower_minus_one() {
    let mut rng = ChaCha20Rng::seed_from_u64(44);
    let selected_plain = leaf(0, 32, NodePlacementData::Child);
    let (selected_page, selected) = seal(&selected_plain, NodePlacementData::Child, &mut rng);
    let boundary_plain = leaf(9400, 16, NodePlacementData::Child);
    let (boundary_page, boundary) = seal(&boundary_plain, NodePlacementData::Child, &mut rng);
    let mut children = Vec::with_capacity(192);
    for i in 0..192u64 {
        let reference = if i == 0 {
            selected
        } else if i == 94 {
            boundary
        } else {
            NodeRefData {
                page: PrivateChatPageId::from_bytes(key(i + 1000)),
                summary: NodeSummaryData {
                    height: 0,
                    keys: 16,
                    min: key(i * 100),
                    max: key(i * 100 + 15),
                },
            }
        };
        children.push(ChildRefData {
            context: context(),
            node: reference,
        });
    }
    let root_plain = encode_branch_data(context(), &children, NodePlacementData::Root).unwrap();
    let (root_page, root) = seal(&root_plain, NodePlacementData::Root, &mut rng);
    let mut path = AcceptedIndexPathData::new_data(context(), root, key(32), None).unwrap();
    path.accept_frame_data(&KEY, root_page.as_bytes()).unwrap();
    assert_eq!(path.next_page_data().unwrap(), Some(selected.page));
    path.accept_frame_data(&KEY, selected_page.as_bytes())
        .unwrap();
    let leaf_outputs = path.rewrite_leaf_data(row(32)).unwrap();
    let outputs: Vec<_> = leaf_outputs
        .iter()
        .map(|plain| seal(plain, NodePlacementData::Child, &mut rng))
        .collect();
    let refs: Vec<_> = outputs
        .iter()
        .map(|(_, node)| ChildRefData {
            context: context(),
            node: *node,
        })
        .collect();
    assert_eq!(path.boundary_page_data(0, 2).unwrap(), Some(boundary.page));
    assert!(path.rewrite_branch_data(0, &refs, None).is_err());
    assert!(path
        .rewrite_branch_data(0, &refs, Some((&KEY, selected_page.as_bytes())))
        .is_err());
    let branches = path
        .rewrite_branch_data(0, &refs, Some((&KEY, boundary_page.as_bytes())))
        .unwrap();
    let nodes = branches.nodes_data();
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].summary_data().max, key(9415));
    assert_ne!(nodes[0].summary_data().max, key(9499));
    assert_eq!(
        nodes[0].summary_data().keys + nodes[1].summary_data().keys,
        root.summary.keys + 1
    );
    assert_eq!(
        NodeFrame::decode(nodes[0].plain_data(), NodePlacementData::Child)
            .unwrap()
            .row_count(),
        96
    );
    assert_eq!(
        NodeFrame::decode(nodes[1].plain_data(), NodePlacementData::Child)
            .unwrap()
            .row_count(),
        97
    );
}

mod observed;
