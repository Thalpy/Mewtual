//! Adversarial DATA tests; no selected root, complete miss or physical receipt is faked.
use super::super::request::{
    AcceptedLocatorData, PublicationPhaseData, RequestPhaseData, TextPublicationIdData,
    TextRequestDigestData, TextRetainIdData,
};
use super::*;
use catcoms_replication::chat::cold_actions::OriginalActionDigest;
use catcoms_replication::chat::cold_index::ReceiptDigestData;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_replication::chat::ActionId;
use catcoms_storage::private_chat_page::PreparedPrivateChatPage;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const KEY: [u8; 32] = [9; 32];
fn context(kind: PrivateIndexKindData) -> PrivateIndexContextData {
    PrivateIndexContextData {
        cid: TextContextIdData::from_bytes([1; 32]),
        kind,
    }
}
fn ordinal(n: u64) -> [u8; 32] {
    PublicationValueData::ordinal_key_data(n)
}
fn publication(n: u64) -> PrivateIndexLeafRowData {
    PrivateIndexLeafRowData {
        key: ordinal(n),
        value: PrivateIndexValueData::Publication(PublicationValueData {
            transaction: TextTransactionIdData::from_bytes([2; 32]),
            action: ActionId::from_bytes([3; 32]),
            receipt: ReceiptDigestData::from_bytes([4; 32]),
            header: OwnerHeaderId::from_bytes([5; 32]),
            publication: TextPublicationIdData::from_bytes([6; 32]),
            phase: PublicationPhaseData::Eligible,
        }),
    }
}
fn token(n: u8, phase: RequestPhaseData) -> PrivateIndexLeafRowData {
    let c = context(PrivateIndexKindData::Token);
    let token = [n; 32];
    let request =
        TextRequestDigestData::from_request_data(c.cid, token, 7, "exact original request")
            .unwrap();
    let value = TokenValueData {
        transaction: TextTransactionIdData::from_request_data(c.cid, token, request),
        token,
        request,
        retain: TextRetainIdData::from_bytes([4; 32]),
        action: ActionId::from_bytes([5; 32]),
        original: OriginalActionDigest::from_bytes([6; 32]),
        phase,
    };
    PrivateIndexLeafRowData {
        key: *value.key_data(c.cid).as_bytes(),
        value: PrivateIndexValueData::Token(value),
    }
}
fn sealed(
    context: PrivateIndexContextData,
    rows: &[PrivateIndexLeafRowData],
    placement: NodePlacementData,
    rng: &mut ChaCha20Rng,
) -> (PreparedPrivateChatPage, PrivateIndexChildRefData) {
    let plain = encode_private_leaf_data(context, rows, placement).unwrap();
    let summary = PrivateIndexNodeFrame::decode(&plain, context, placement)
        .unwrap()
        .summary();
    let frame = PreparedPrivateChatPage::prepare(&KEY, &plain, rng).unwrap();
    let reference = PrivateIndexChildRefData {
        context,
        node: NodeRefData {
            page: frame.id(),
            summary,
        },
    };
    (frame, reference)
}

#[test]
fn distinct_domains_cid_kind_and_token_transaction_are_checked_before_values() {
    let token_context = context(PrivateIndexKindData::Token);
    let row = token(
        2,
        RequestPhaseData::Accepted(AcceptedLocatorData {
            header: OwnerHeaderId::from_bytes([7; 32]),
            record_page: PrivateChatPageId::from_bytes([8; 32]),
            record_row: 63,
            ordinal: 99,
            receipt: ReceiptDigestData::from_bytes([10; 32]),
        }),
    );
    let raw = encode_private_leaf_data(token_context, &[row], NodePlacementData::Root).unwrap();
    assert_eq!(raw.len(), TOKEN_HEADER_BYTES + 34 + 336);
    assert_eq!(
        PrivateIndexNodeFrame::decode(&raw, token_context, NodePlacementData::Root)
            .unwrap()
            .leaf_row(0)
            .unwrap()
            .value,
        row.value
    );
    assert_eq!(
        PrivateIndexNodeFrame::decode(
            &raw,
            context(PrivateIndexKindData::Publication),
            NodePlacementData::Root
        )
        .unwrap_err(),
        PrivateIndexError::Context
    );
    let wrong_context = PrivateIndexContextData {
        cid: TextContextIdData::from_bytes([99; 32]),
        ..token_context
    };
    // Truncation after the bad Cid still reports Context rather than reading missing rows.
    assert_eq!(
        PrivateIndexNodeFrame::decode(&raw[..83], wrong_context, NodePlacementData::Root)
            .unwrap_err(),
        PrivateIndexError::Context
    );
    let mut wrong_kind = raw.to_vec();
    wrong_kind[83] = 2;
    assert!(
        PrivateIndexNodeFrame::decode(&wrong_kind, token_context, NodePlacementData::Root).is_err()
    );
    let mut bad_key = row;
    bad_key.key[0] ^= 1;
    assert!(encode_private_leaf_data(token_context, &[bad_key], NodePlacementData::Root).is_err());
    let mut bad_transaction = row;
    let PrivateIndexValueData::Token(ref mut value) = bad_transaction.value else {
        unreachable!()
    };
    value.transaction = TextTransactionIdData::from_bytes([100; 32]);
    assert!(
        encode_private_leaf_data(token_context, &[bad_transaction], NodePlacementData::Root)
            .is_err()
    );
    assert!(
        encode_private_leaf_data(token_context, &[publication(1)], NodePlacementData::Root)
            .is_err()
    );
    let mut pub_bad_key = publication(1);
    pub_bad_key.key[0] = 1;
    assert!(encode_private_leaf_data(
        context(PrivateIndexKindData::Publication),
        &[pub_bad_key],
        NodePlacementData::Root
    )
    .is_err());
    let mut tail = raw.to_vec();
    tail.push(0);
    assert!(PrivateIndexNodeFrame::decode(&tail, token_context, NodePlacementData::Root).is_err());
    for end in 0..raw.len() {
        assert!(
            PrivateIndexNodeFrame::decode(&raw[..end], token_context, NodePlacementData::Root)
                .is_err(),
            "prefix {end}"
        );
    }
}

#[test]
fn root_and_child_occupancy_leaf_split_and_packed_replacement_are_exact() {
    let c = context(PrivateIndexKindData::Publication);
    let singleton =
        encode_private_leaf_data(c, &[publication(0)], NodePlacementData::Root).unwrap();
    assert_eq!(singleton.len(), PUBLICATION_HEADER_BYTES + 34 + 181);
    assert!(PrivateIndexNodeFrame::decode(&singleton, c, NodePlacementData::Child).is_err());
    assert!(encode_private_leaf_data(
        c,
        &(0..15).map(publication).collect::<Vec<_>>(),
        NodePlacementData::Child
    )
    .is_err());
    let rows = (0..32).map(|i| publication(i * 2)).collect::<Vec<_>>();
    let plain = encode_private_leaf_data(c, &rows, NodePlacementData::Root).unwrap();
    let old = PrivateIndexNodeFrame::decode(&plain, c, NodePlacementData::Root).unwrap();
    let split = rewrite_private_leaf_data(&old, &publication(31), NodePlacementData::Root).unwrap();
    assert_eq!(split.nodes_data().len(), 2);
    let left = PrivateIndexNodeFrame::decode(
        split.nodes_data()[0].plain_data(),
        c,
        NodePlacementData::Child,
    )
    .unwrap();
    let right = PrivateIndexNodeFrame::decode(
        split.nodes_data()[1].plain_data(),
        c,
        NodePlacementData::Child,
    )
    .unwrap();
    assert_eq!((left.row_count(), right.row_count()), (16, 17));
    for i in 0..16 {
        assert_eq!(
            old.packed_leaf_row(i).unwrap(),
            left.packed_leaf_row(i).unwrap()
        );
    }
    for i in 16..32 {
        assert_eq!(
            old.packed_leaf_row(i).unwrap(),
            right.packed_leaf_row(i - 15).unwrap()
        );
    }
    let mut replacement = publication(10);
    let PrivateIndexValueData::Publication(ref mut value) = replacement.value else {
        unreachable!()
    };
    value.phase = PublicationPhaseData::Acknowledged;
    let replaced = rewrite_private_leaf_data(&old, &replacement, NodePlacementData::Root).unwrap();
    assert_eq!(replaced.nodes_data().len(), 1);
    let changed = PrivateIndexNodeFrame::decode(
        replaced.nodes_data()[0].plain_data(),
        c,
        NodePlacementData::Root,
    )
    .unwrap();
    assert_eq!(changed.summary(), old.summary());
    for i in 0..32 {
        if i != 5 {
            assert_eq!(
                changed.packed_leaf_row(i).unwrap(),
                old.packed_leaf_row(i).unwrap()
            );
        }
    }
    let mut unordered = rows.clone();
    unordered.swap(7, 8);
    assert!(encode_private_leaf_data(c, &unordered, NodePlacementData::Root).is_err());
    let mut duplicate = rows;
    duplicate[8] = duplicate[7];
    assert!(encode_private_leaf_data(c, &duplicate, NodePlacementData::Root).is_err());
}

struct BranchFixture {
    context: PrivateIndexContextData,
    children: Vec<(PreparedPrivateChatPage, PrivateIndexChildRefData)>,
    plain: Zeroizing<Vec<u8>>,
    frame: PreparedPrivateChatPage,
    root: NodeRefData,
}
fn branch_fixture() -> BranchFixture {
    let context = context(PrivateIndexKindData::Publication);
    let mut rng = ChaCha20Rng::seed_from_u64(0x991);
    let mut children = Vec::with_capacity(MAX_BRANCH_ROWS);
    for i in 0..MAX_BRANCH_ROWS {
        let rows = if i == 0 {
            (0..32).map(|n| publication(n * 2)).collect::<Vec<_>>()
        } else {
            (0..16)
                .map(|n| publication(i as u64 * 128 + n))
                .collect::<Vec<_>>()
        };
        children.push(sealed(context, &rows, NodePlacementData::Child, &mut rng));
    }
    let refs = children
        .iter()
        .map(|(_, reference)| *reference)
        .collect::<Vec<_>>();
    let plain = encode_private_branch_data(context, &refs, NodePlacementData::Root).unwrap();
    let summary = PrivateIndexNodeFrame::decode(&plain, context, NodePlacementData::Root)
        .unwrap()
        .summary();
    let frame = PreparedPrivateChatPage::prepare(&KEY, &plain, &mut rng).unwrap();
    let root = NodeRefData {
        page: frame.id(),
        summary,
    };
    BranchFixture {
        context,
        children,
        plain,
        frame,
        root,
    }
}
fn opened_path(f: &BranchFixture) -> PrivateIndexPathData {
    let mut path = PrivateIndexPathData::new_data(f.context, f.root, ordinal(31)).unwrap();
    path.accept_frame_data(&KEY, f.frame.as_bytes()).unwrap();
    assert_eq!(path.next_page_data().unwrap(), Some(f.children[0].0.id()));
    path.accept_frame_data(&KEY, f.children[0].0.as_bytes())
        .unwrap();
    path
}

#[test]
fn splitting_193_authenticates_the_actual_boundary_and_preserves_all_unchanged_rows() {
    let f = branch_fixture();
    let path = opened_path(&f);
    assert!(path.row_data().unwrap().is_none());
    let leaves = path.rewrite_leaf_data(&publication(31)).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x992);
    let replacements = leaves
        .nodes_data()
        .iter()
        .map(|node| {
            let page = PreparedPrivateChatPage::prepare(&KEY, node.plain_data(), &mut rng).unwrap();
            PrivateIndexChildRefData {
                context: f.context,
                node: NodeRefData {
                    page: page.id(),
                    summary: node.summary_data(),
                },
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        path.boundary_page_data(0, 2).unwrap(),
        Some(f.children[94].0.id())
    );
    assert!(path.rewrite_branch_data(0, &replacements, None).is_err());
    assert_eq!(
        path.rewrite_branch_data(0, &replacements, Some((&KEY, f.children[95].0.as_bytes())))
            .unwrap_err(),
        PrivateIndexError::Authentication
    );
    let rewritten = path
        .rewrite_branch_data(0, &replacements, Some((&KEY, f.children[94].0.as_bytes())))
        .unwrap();
    let left = PrivateIndexNodeFrame::decode(
        rewritten.nodes_data()[0].plain_data(),
        f.context,
        NodePlacementData::Child,
    )
    .unwrap();
    let right = PrivateIndexNodeFrame::decode(
        rewritten.nodes_data()[1].plain_data(),
        f.context,
        NodePlacementData::Child,
    )
    .unwrap();
    assert_eq!((left.row_count(), right.row_count()), (96, 97));
    assert_eq!(left.summary().max, ordinal(94 * 128 + 15));
    assert_ne!(left.summary().max, ordinal(95 * 128 - 1));
    let parent =
        PrivateIndexNodeFrame::decode(&f.plain, f.context, NodePlacementData::Root).unwrap();
    for old in 1..192 {
        let new = old + 1;
        let (node, index) = if new < 96 {
            (&left, new)
        } else {
            (&right, new - 96)
        };
        assert_eq!(
            &parent.packed[old * 80..(old + 1) * 80],
            &node.packed[index * 80..(index + 1) * 80]
        );
    }
    // The duplicate spans FUTURE split outputs, so per-output decoder checks alone miss it.
    let mut duplicate = replacements.clone();
    duplicate[0].node.page = f.children[190].0.id();
    assert!(path
        .rewrite_branch_data(0, &duplicate, Some((&KEY, f.children[94].0.as_bytes())))
        .is_err());
}

#[test]
fn real_child_descriptors_refuse_height_count_range_context_and_child_underoccupancy() {
    let f = branch_fixture();
    let parent =
        PrivateIndexNodeFrame::decode(&f.plain, f.context, NodePlacementData::Root).unwrap();
    let descriptor = parent.branch_child_data(94).unwrap();
    let plain = encode_private_leaf_data(
        f.context,
        &(0..16)
            .map(|n| publication(94 * 128 + n))
            .collect::<Vec<_>>(),
        NodePlacementData::Child,
    )
    .unwrap();
    let actual =
        PrivateIndexNodeFrame::decode(&plain, f.context, NodePlacementData::Child).unwrap();
    assert_eq!(
        descriptor.check_child(&actual).unwrap().summary,
        actual.summary()
    );
    for (base, count) in [(94 * 128 + 1, 16), (94 * 128, 17), (94 * 128, 15)] {
        let raw = encode_private_leaf_data(
            f.context,
            &(0..count)
                .map(|n| publication(base + n))
                .collect::<Vec<_>>(),
            NodePlacementData::Root,
        )
        .unwrap();
        let wrong =
            PrivateIndexNodeFrame::decode(&raw, f.context, NodePlacementData::Root).unwrap();
        assert!(descriptor.check_child(&wrong).is_err());
    }
    let wrong_context = PrivateIndexContextData {
        cid: TextContextIdData::from_bytes([55; 32]),
        ..f.context
    };
    let raw = encode_private_leaf_data(
        wrong_context,
        &(0..16)
            .map(|n| publication(94 * 128 + n))
            .collect::<Vec<_>>(),
        NodePlacementData::Child,
    )
    .unwrap();
    assert!(descriptor
        .check_child(
            &PrivateIndexNodeFrame::decode(&raw, wrong_context, NodePlacementData::Child).unwrap()
        )
        .is_err());
    let last = parent.branch_child_data(191).unwrap();
    let raw = encode_private_leaf_data(
        f.context,
        &(0..16)
            .map(|n| publication(191 * 128 + n * 2))
            .collect::<Vec<_>>(),
        NodePlacementData::Child,
    )
    .unwrap();
    assert!(last
        .check_child(
            &PrivateIndexNodeFrame::decode(&raw, f.context, NodePlacementData::Child).unwrap()
        )
        .is_err());
    let refs = [f.children[94].1, f.children[95].1];
    let root_branch =
        encode_private_branch_data(f.context, &refs, NodePlacementData::Root).unwrap();
    assert!(
        PrivateIndexNodeFrame::decode(&root_branch, f.context, NodePlacementData::Child).is_err()
    );
    assert!(descriptor
        .check_child(
            &PrivateIndexNodeFrame::decode(&root_branch, f.context, NodePlacementData::Root)
                .unwrap()
        )
        .is_err());
}

#[test]
fn full_branch_parser_refuses_reordering_duplicate_ids_counts_height_and_wrong_leaf_kind() {
    let f = branch_fixture();
    let mut raw = f.plain.to_vec();
    raw[PUBLICATION_HEADER_BYTES + 80 + 32..PUBLICATION_HEADER_BYTES + 80 + 64]
        .copy_from_slice(&f.plain[PUBLICATION_HEADER_BYTES + 32..PUBLICATION_HEADER_BYTES + 64]);
    assert!(PrivateIndexNodeFrame::decode(&raw, f.context, NodePlacementData::Root).is_err());
    let mut raw = f.plain.to_vec();
    let first = raw[PUBLICATION_HEADER_BYTES + 80..PUBLICATION_HEADER_BYTES + 160].to_vec();
    raw.copy_within(
        PUBLICATION_HEADER_BYTES + 160..PUBLICATION_HEADER_BYTES + 240,
        PUBLICATION_HEADER_BYTES + 80,
    );
    raw[PUBLICATION_HEADER_BYTES + 160..PUBLICATION_HEADER_BYTES + 240].copy_from_slice(&first);
    assert!(PrivateIndexNodeFrame::decode(&raw, f.context, NodePlacementData::Root).is_err());
    let mut raw = f.plain.to_vec();
    raw[PUBLICATION_HEADER_BYTES + 64..PUBLICATION_HEADER_BYTES + 80].fill(255);
    assert!(PrivateIndexNodeFrame::decode(&raw, f.context, NodePlacementData::Root).is_err());
    let mut raw = f.plain.to_vec();
    raw[91] = 2;
    assert!(PrivateIndexNodeFrame::decode(&raw, f.context, NodePlacementData::Root).is_err());
    let mut raw = f.plain.to_vec();
    raw[90] = 1;
    assert!(PrivateIndexNodeFrame::decode(&raw, f.context, NodePlacementData::Root).is_err());
    let mut refs = f.children.iter().map(|(_, r)| *r).collect::<Vec<_>>();
    refs[191].node.page = refs[0].node.page;
    assert!(encode_private_branch_data(f.context, &refs, NodePlacementData::Root).is_err());
}

#[test]
fn hash_key_context_root_summary_and_post_leaf_refusals_leave_path_unadvanced() {
    let c = context(PrivateIndexKindData::Token);
    let row = token(2, RequestPhaseData::Retained);
    let mut rng = ChaCha20Rng::seed_from_u64(0x993);
    let (frame, reference) = sealed(c, &[row], NodePlacementData::Root, &mut rng);
    let mut path = PrivateIndexPathData::new_data(c, reference.node, row.key).unwrap();
    let mut bad = frame.as_bytes().to_vec();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert_eq!(
        path.accept_frame_data(&KEY, &bad).unwrap_err(),
        PrivateIndexError::Authentication
    );
    assert_eq!(path.next_page_data().unwrap(), Some(frame.id()));
    assert!(path.accept_frame_data(&[8; 32], frame.as_bytes()).is_err());
    assert!(path.row_data().is_err());
    path.accept_frame_data(&KEY, frame.as_bytes()).unwrap();
    assert_eq!(path.row_data().unwrap().unwrap().value, row.value);
    assert_eq!(path.next_page_data().unwrap(), None);
    assert!(path.accept_frame_data(&KEY, frame.as_bytes()).is_err());
    let mut wrong = reference.node;
    // Preserve min<=max and the generic count bound while contradicting the actual node.
    wrong.summary.max = [255; 32];
    let mut path = PrivateIndexPathData::new_data(c, wrong, row.key).unwrap();
    assert_eq!(
        path.accept_frame_data(&KEY, frame.as_bytes()).unwrap_err(),
        PrivateIndexError::Context
    );
    let mismatch = PrivateIndexContextData {
        cid: TextContextIdData::from_bytes([77; 32]),
        ..c
    };
    let mut path = PrivateIndexPathData::new_data(mismatch, reference.node, row.key).unwrap();
    assert!(path.accept_frame_data(&KEY, frame.as_bytes()).is_err());
    assert_eq!(path.next_page_data().unwrap(), Some(frame.id()));
}

#[test]
fn maximum_height_and_u64_counts_are_checked_without_wraparound_or_growing_a_tree() {
    // Structural labels only: this test does not pretend to own or authenticate the
    // enormous children. It pins exact integer/height geometry BEFORE any page allocation.
    let c = context(PrivateIndexKindData::Publication);
    let minimum = u64::try_from(min_child_keys(8).unwrap()).unwrap();
    let left_max = ordinal(u64::MAX - minimum - 1);
    let right_min = ordinal(u64::MAX - minimum);
    let children = [
        PrivateIndexChildRefData {
            context: c,
            node: NodeRefData {
                page: PrivateChatPageId::from_bytes([1; 32]),
                summary: NodeSummaryData {
                    height: 8,
                    keys: u64::MAX - minimum,
                    min: [0; 32],
                    max: left_max,
                },
            },
        },
        PrivateIndexChildRefData {
            context: c,
            node: NodeRefData {
                page: PrivateChatPageId::from_bytes([2; 32]),
                summary: NodeSummaryData {
                    height: 8,
                    keys: minimum,
                    min: right_min,
                    max: ordinal(u64::MAX),
                },
            },
        },
    ];
    let raw = encode_private_branch_data(c, &children, NodePlacementData::Root).unwrap();
    let root = PrivateIndexNodeFrame::decode(&raw, c, NodePlacementData::Root).unwrap();
    assert_eq!(root.summary().height, 9);
    assert_eq!(root.summary().keys, u64::MAX);
    let mut overflow = children;
    overflow[0].node.summary.keys += 1;
    assert!(encode_private_branch_data(c, &overflow, NodePlacementData::Root).is_err());
    let mut too_high = children;
    for child in &mut too_high {
        child.node.summary.height = 9;
    }
    assert!(encode_private_branch_data(c, &too_high, NodePlacementData::Root).is_err());
    let mut encoded_overflow = raw.to_vec();
    encoded_overflow[94..110].fill(255);
    assert_eq!(
        PrivateIndexNodeFrame::decode(&encoded_overflow, c, NodePlacementData::Root).unwrap_err(),
        PrivateIndexError::Bound
    );
}

// Real encrypted multi-level COW ascent and malformed-child cursor preservation.
mod ascent;

mod observed;
