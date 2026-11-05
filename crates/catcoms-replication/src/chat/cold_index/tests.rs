//! Real private AEAD/frame links over synthetic index DATA. These fixtures establish no actual
//! accepted history, selected source, grant, balanced writer, namespace, Current or native path.
use super::*;
use crate::chat::acceptance_tree::TreeContext;
use crate::chat::cold_actions::OriginalActionDigest;
use crate::chat::cold_manifest::ArchiveHeadId;
use crate::chat::owner_header::{OwnerBodyId, OwnerHeaderId};
use crate::chat::ActionId;
use catcoms_storage::private_chat_page::PreparedPrivateChatPage;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const KEY: [u8; 32] = [90; 32];
fn context(scope: IndexScopeData) -> IndexContextData<'static> {
    IndexContextData {
        vault: [1; 32],
        group: b"private-index-test-group-do-not-log",
        profile: OwnerFactId::from_bytes([2; 32]),
        scope,
    }
}
fn actions() -> IndexContextData<'static> {
    context(IndexScopeData::Actions(OwnerFactId::from_bytes([3; 32])))
}
fn key(n: u64) -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    bytes
}
fn page(n: u8) -> PrivateChatPageId {
    PrivateChatPageId::from_bytes([n; 32])
}
fn claim(ctx: IndexContextData<'_>, k: [u8; 32], ordinal: u64, hint: ClaimHint) -> ActionClaimData {
    let IndexScopeData::Actions(a) = ctx.scope else {
        panic!("Actions fixture")
    };
    let record = [4; 32];
    let accepting_header = OwnerHeaderId::from_bytes([5; 32]);
    ActionClaimData {
        record,
        ordinal,
        channel: 9,
        accepting_header,
        header_sequence: 2,
        append_body: OwnerBodyId::from_bytes([6; 32]),
        acceptance_manifest: page(7),
        manifest_sequence: 3,
        record_page: page(8),
        record_row: 0,
        header_body_page: page(9),
        retention: RetentionData::Known {
            manifest: page(10),
            original: OriginalActionDigest::from_bytes([11; 32]),
            bytes: 128,
        },
        receipt: ReceiptDigestData::from_tuple(
            ctx.profile,
            a,
            ActionId::from_bytes(k),
            record,
            ordinal,
            accepting_header,
        ),
        occurrences: 1,
        hint,
    }
}
fn unknown() -> DirectoryData {
    DirectoryData {
        tree: DirectoryTreeData::Unknown,
        accepted_leaf_count: 0,
        unique_action_count: 0,
        accepted_root: crate::chat::acceptance_tree::TreeHash::from_bytes([0; 32]),
        own_tip_header: OwnerHeaderId::from_bytes([12; 32]),
        tip_body: OwnerBodyId::from_bytes([13; 32]),
        prefix_manifest: page(14),
        prefix_sequence: 1,
        prefix_head: ArchiveHeadId::from_bytes([15; 32]),
    }
}
fn empty(ctx: IndexContextData<'_>, a: [u8; 32]) -> DirectoryData {
    DirectoryData {
        tree: DirectoryTreeData::Empty,
        accepted_root: TreeContext {
            profile: ctx.profile,
            admission: OwnerFactId::from_bytes(a),
        }
        .empty_hash(),
        tip_body: OwnerBodyId::from_bytes(a),
        ..unknown()
    }
}
struct Sealed {
    frame: PreparedPrivateChatPage,
    node: NodeRefData,
}
fn seal(plain: &[u8], seed: u64) -> Sealed {
    let summary = NodeFrame::decode(plain, NodePlacementData::Root)
        .unwrap()
        .summary();
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    let frame = PreparedPrivateChatPage::prepare(&KEY, plain, &mut rng).unwrap();
    let node = NodeRefData {
        page: frame.id(),
        summary,
    };
    Sealed { frame, node }
}
fn leaf(ctx: IndexContextData<'_>, start: u64, n: usize, seed: u64) -> Sealed {
    let rows: Vec<_> = (0..n)
        .map(|i| {
            let k = key(start + i as u64);
            LeafRowData {
                key: k,
                value: match ctx.scope {
                    IndexScopeData::Directory => ValueData::Directory(unknown()),
                    IndexScopeData::Actions(_) => {
                        ValueData::Action(claim(ctx, k, i as u64, ClaimHint::Canonical))
                    }
                },
            }
        })
        .collect();
    seal(
        &encode_leaf_data(ctx, &rows, NodePlacementData::Root).unwrap(),
        seed,
    )
}
fn root(ctx: IndexContextData<'_>, nodes: &[Sealed], seed: u64) -> Sealed {
    let refs: Vec<_> = nodes
        .iter()
        .map(|node| ChildRefData {
            context: ctx,
            node: node.node,
        })
        .collect();
    seal(
        &encode_branch_data(ctx, &refs, NodePlacementData::Root).unwrap(),
        seed,
    )
}
fn spec<'a>(
    ctx: IndexContextData<'a>,
    node: NodeRefData,
    k: [u8; 32],
    leaves: Option<u64>,
) -> QueryRootData<'a> {
    QueryRootData {
        context: ctx,
        root: node,
        key: k,
        accepted_leaf_count: leaves,
    }
}
fn open(ctx: IndexContextData<'_>, node: &Sealed, k: [u8; 32]) -> IndexQueryData {
    open_query_data(
        &KEY,
        spec(
            ctx,
            node.node,
            k,
            match ctx.scope {
                IndexScopeData::Directory => None,
                IndexScopeData::Actions(_) => Some(64),
            },
        ),
        node.frame.as_bytes(),
    )
    .unwrap()
}
fn raw_branch(
    ctx: IndexContextData<'_>,
    summary: NodeSummaryData,
    rows: &[([u8; 32], PrivateChatPageId, u128)],
) -> Vec<u8> {
    let mut e = header(
        ctx,
        NodeKindData::Branch,
        summary,
        rows.len(),
        rows.len() * 80,
    )
    .unwrap();
    for (lower, id, count) in rows {
        for b in lower {
            e.put_u8(*b);
        }
        for b in id.as_bytes() {
            e.put_u8(*b);
        }
        e.put_u128(*count);
    }
    e.finish()
}
fn offsets(ctx: IndexContextData<'_>) -> (usize, usize) {
    let kind = 4
        + DOMAIN.len()
        + 2
        + 36
        + 4
        + ctx.group.len()
        + 36
        + 1
        + if matches!(ctx.scope, IndexScopeData::Actions(_)) {
            36
        } else {
            0
        };
    (kind, kind + 1 + 1 + 2 + 16 + 36 + 36 + 4)
}

#[test]
fn real_sealed_paths_preserve_original_query_and_parent_transition_ownership() {
    let ctx = actions();
    let children = vec![leaf(ctx, 0, 16, 1), leaf(ctx, 100, 16, 2)];
    let parent = root(ctx, &children, 3);
    let query = open(ctx, &parent, key(105));
    let SearchData::Child(step) = query.search_data().unwrap() else {
        panic!("branch")
    };
    assert_eq!(step.expected_page_data(), children[1].node.page);
    assert!(step.open_data(&KEY, children[0].frame.as_bytes()).is_err());
    let child = step.open_data(&KEY, children[1].frame.as_bytes()).unwrap();
    assert_eq!(
        query.node().row_count(),
        2,
        "parent plaintext is necessarily still retained during transition"
    );
    assert_eq!(child.root_page_data(), parent.node.page);
    assert_eq!(child.key_data(), key(105));
    drop(query); // Child has its own plaintext/context, not a retained parent owner.
    assert!(matches!(
        child.search_data().unwrap(),
        SearchData::Action(_)
    ));
    let gap = open(ctx, &parent, key(50));
    let SearchData::Child(step) = gap.search_data().unwrap() else {
        panic!("gap needs actual child")
    };
    let child = step.open_data(&KEY, children[0].frame.as_bytes()).unwrap();
    let SearchData::NoActionKey(miss) = child.search_data().unwrap() else {
        panic!("structural gap")
    };
    assert_eq!(miss.root_page_data(), parent.node.page);
    assert_eq!(miss.key_data(), key(50));
}

#[test]
fn directory_unknown_empty_present_and_missing_remain_distinct() {
    let ctx = context(IndexScopeData::Directory);
    let a = key(1);
    for (state, is_empty) in [(unknown(), false), (empty(ctx, a), true)] {
        let node = seal(
            &encode_leaf_data(
                ctx,
                &[LeafRowData {
                    key: a,
                    value: ValueData::Directory(state),
                }],
                NodePlacementData::Root,
            )
            .unwrap(),
            4,
        );
        let q = open(ctx, &node, a);
        let SearchData::Directory(hit) = q.search_data().unwrap() else {
            panic!("entry")
        };
        assert_eq!(
            matches!(hit.action_route_data(key(55)), DirectoryRouteData::Empty(_)),
            is_empty
        );
        assert_eq!(
            matches!(
                hit.action_route_data(key(55)),
                DirectoryRouteData::Unknown(_)
            ),
            !is_empty
        );
        assert!(matches!(
            open(ctx, &node, key(2)).search_data().unwrap(),
            SearchData::NoDirectoryEntry(_)
        ));
    }
    let inner_ctx = context(IndexScopeData::Actions(OwnerFactId::from_bytes(a)));
    let inner = leaf(inner_ctx, 0, 16, 5);
    let value = DirectoryData {
        tree: DirectoryTreeData::Present(inner.node),
        accepted_leaf_count: 32,
        unique_action_count: 16,
        ..unknown()
    };
    let dir = seal(
        &encode_leaf_data(
            ctx,
            &[LeafRowData {
                key: a,
                value: ValueData::Directory(value),
            }],
            NodePlacementData::Root,
        )
        .unwrap(),
        6,
    );
    let q = open(ctx, &dir, a);
    let SearchData::Directory(hit) = q.search_data().unwrap() else {
        panic!("entry")
    };
    let DirectoryRouteData::Root(next) = hit.action_route_data(key(3)) else {
        panic!("root")
    };
    assert_eq!(next.context, inner_ctx);
    assert_eq!(next.accepted_leaf_count, Some(32));
    let innerq = open_query_data(&KEY, next, inner.frame.as_bytes()).unwrap();
    assert!(matches!(
        innerq.search_data().unwrap(),
        SearchData::Action(_)
    ));
    let mut wrong = value;
    wrong.unique_action_count = 15;
    assert!(encode_leaf_data(
        ctx,
        &[LeafRowData {
            key: a,
            value: ValueData::Directory(wrong)
        }],
        NodePlacementData::Root
    )
    .is_err());
}

#[test]
fn incomplete_and_conflict_are_present_and_every_reached_claim_is_leaf_bounded() {
    let ctx = actions();
    for hint in [ClaimHint::Incomplete, ClaimHint::Conflict] {
        let mut v = claim(ctx, key(1), 0, hint);
        v.retention = RetentionData::Unknown;
        let bytes = encode_leaf_data(
            ctx,
            &[LeafRowData {
                key: key(1),
                value: ValueData::Action(v),
            }],
            NodePlacementData::Root,
        )
        .unwrap();
        let node = seal(&bytes, 7);
        let q = open(ctx, &node, key(1));
        let SearchData::Action(actual) = q.search_data().unwrap() else {
            panic!("a known claim cannot be absent")
        };
        assert_eq!(actual.hint, hint);
    }
    // The unselected second row exceeds the carried leaf bound. Reject the NODE, not just its hit.
    let rows = [
        LeafRowData {
            key: key(1),
            value: ValueData::Action(claim(ctx, key(1), 0, ClaimHint::Canonical)),
        },
        LeafRowData {
            key: key(2),
            value: ValueData::Action(claim(ctx, key(2), 3, ClaimHint::Canonical)),
        },
    ];
    let node = seal(
        &encode_leaf_data(ctx, &rows, NodePlacementData::Root).unwrap(),
        8,
    );
    assert!(open_query_data(
        &KEY,
        spec(ctx, node.node, key(1), Some(2)),
        node.frame.as_bytes()
    )
    .is_err());
    assert!(open_query_data(
        &KEY,
        spec(ctx, node.node, key(99), Some(2)),
        node.frame.as_bytes()
    )
    .is_err());
    assert!(open_query_data(
        &KEY,
        spec(ctx, node.node, key(1), None),
        node.frame.as_bytes()
    )
    .is_err());
    assert!(open_query_data(
        &KEY,
        spec(ctx, node.node, key(1), Some(1)),
        node.frame.as_bytes()
    )
    .is_err());
}

#[test]
fn maximum_layouts_root_exceptions_and_nonroot_minima_are_bounded() {
    let ctx = context(IndexScopeData::Directory);
    let rows: Vec<_> = (0..32)
        .map(|i| LeafRowData {
            key: key(i),
            value: ValueData::Directory(unknown()),
        })
        .collect();
    let bytes = encode_leaf_data(ctx, &rows, NodePlacementData::Child).unwrap();
    assert!(bytes.len() <= MAX_INDEX_NODE_BYTES);
    assert!(encode_leaf_data(ctx, &rows[..15], NodePlacementData::Child).is_err());
    assert!(encode_leaf_data(ctx, &rows[..1], NodePlacementData::Root).is_ok());
    let over: Vec<_> = (0..33)
        .map(|i| LeafRowData {
            key: key(i),
            value: ValueData::Directory(unknown()),
        })
        .collect();
    assert!(encode_leaf_data(ctx, &over, NodePlacementData::Root).is_err());
    let children: Vec<_> = (0..192)
        .map(|i| ChildRefData {
            context: ctx,
            node: NodeRefData {
                page: PrivateChatPageId::from_bytes(key(i + 1)),
                summary: NodeSummaryData {
                    height: 0,
                    keys: 16,
                    min: key(i * 32),
                    max: key(i * 32 + 15),
                },
            },
        })
        .collect();
    let bytes = encode_branch_data(ctx, &children, NodePlacementData::Root).unwrap();
    let node = NodeFrame::decode(&bytes, NodePlacementData::Root).unwrap();
    assert_eq!(node.row_count(), 192);
    assert_eq!(node.summary().keys, 3072);
    assert!(bytes.len() <= MAX_INDEX_NODE_BYTES);
    assert!(encode_branch_data(ctx, &children[..95], NodePlacementData::Child).is_err());
    assert!(encode_branch_data(ctx, &children[..96], NodePlacementData::Child).is_ok());
    assert!(encode_branch_data(ctx, &children[..2], NodePlacementData::Root).is_ok());
    assert!(encode_branch_data(ctx, &children[..1], NodePlacementData::Root).is_err());
    let mut too_many = children.clone();
    too_many.push(ChildRefData {
        context: ctx,
        node: NodeRefData {
            page: page(250),
            summary: NodeSummaryData {
                height: 0,
                keys: 16,
                min: key(192 * 32),
                max: key(192 * 32 + 15),
            },
        },
    });
    assert!(encode_branch_data(ctx, &too_many, NodePlacementData::Root).is_err());
    let ctx = actions();
    let values: Vec<_> = (0..32)
        .map(|i| LeafRowData {
            key: key(i),
            value: ValueData::Action(claim(ctx, key(i), i, ClaimHint::Canonical)),
        })
        .collect();
    let bytes = encode_leaf_data(ctx, &values, NodePlacementData::Child).unwrap();
    assert!(bytes.len() <= MAX_INDEX_NODE_BYTES);
    assert_eq!(
        ValueData::Action(claim(ctx, key(0), 0, ClaimHint::Canonical))
            .encode(ctx, key(0))
            .unwrap()
            .len(),
        343
    );
}

#[test]
fn wrong_expected_roots_scope_vault_profile_group_and_summary_refuse_real_frames() {
    let ctx = actions();
    let node = leaf(ctx, 0, 16, 9);
    let base = spec(ctx, node.node, key(1), Some(32));
    let mut wrong = base;
    wrong.root.page = page(99);
    assert!(open_query_data(&KEY, wrong, node.frame.as_bytes()).is_err());
    let mut wrong = base;
    wrong.root.summary.keys += 1;
    assert!(open_query_data(&KEY, wrong, node.frame.as_bytes()).is_err());
    let mut wrong = base;
    wrong.root.summary.max = key(30);
    assert!(open_query_data(&KEY, wrong, node.frame.as_bytes()).is_err());
    let mut wrong = base;
    wrong.root.summary.height = 1;
    assert!(open_query_data(&KEY, wrong, node.frame.as_bytes()).is_err());
    for context in [
        IndexContextData {
            vault: [99; 32],
            ..ctx
        },
        IndexContextData {
            profile: OwnerFactId::from_bytes([99; 32]),
            ..ctx
        },
        IndexContextData {
            group: b"different private group",
            ..ctx
        },
        IndexContextData {
            scope: IndexScopeData::Actions(OwnerFactId::from_bytes([99; 32])),
            ..ctx
        },
    ] {
        assert!(open_query_data(
            &KEY,
            QueryRootData { context, ..base },
            node.frame.as_bytes()
        )
        .is_err());
    }
    assert!(open_query_data(&[91; 32], base, node.frame.as_bytes()).is_err());
    assert!(open_query_data(&KEY, base, &vec![0; MAX_INDEX_FRAME_BYTES + 1]).is_err());
}

#[test]
fn child_edge_last_max_count_height_and_context_are_checked_after_real_decryption() {
    let ctx = context(IndexScopeData::Directory);
    let children = vec![leaf(ctx, 0, 16, 10), leaf(ctx, 100, 16, 11)];
    let parent = root(ctx, &children, 12);
    let summary = parent.node.summary;
    let rows = [
        (children[0].node.summary.min, children[0].node.page, 16),
        (children[1].node.summary.min, children[1].node.page, 16),
    ];
    let bad = seal(
        &raw_branch(
            ctx,
            NodeSummaryData {
                max: key(120),
                ..summary
            },
            &rows,
        ),
        13,
    );
    let q = open(ctx, &bad, key(105));
    let SearchData::Child(step) = q.search_data().unwrap() else {
        panic!("step")
    };
    assert!(
        step.open_data(&KEY, children[1].frame.as_bytes()).is_err(),
        "last actual max115 cannot satisfy parent max120"
    );
    let mut wrong_rows = rows;
    wrong_rows[0].2 = 17;
    let bad = seal(
        &raw_branch(
            ctx,
            NodeSummaryData {
                keys: 33,
                ..summary
            },
            &wrong_rows,
        ),
        14,
    );
    let q = open(ctx, &bad, key(2));
    let SearchData::Child(step) = q.search_data().unwrap() else {
        panic!("step")
    };
    assert!(step.open_data(&KEY, children[0].frame.as_bytes()).is_err());
    let foreign = leaf(
        IndexContextData {
            vault: [99; 32],
            ..ctx
        },
        0,
        16,
        15,
    );
    let bad = seal(
        &raw_branch(ctx, summary, &[(key(0), foreign.node.page, 16), rows[1]]),
        16,
    );
    let q = open(ctx, &bad, key(2));
    let SearchData::Child(step) = q.search_data().unwrap() else {
        panic!("step")
    };
    assert!(step.open_data(&KEY, foreign.frame.as_bytes()).is_err());
    // Nonlast boundary: child max110 overlaps the next lower100, despite a matching minimum.
    let crossing_rows: Vec<_> = (0..16)
        .map(|i| LeafRowData {
            key: if i == 15 { key(110) } else { key(i) },
            value: ValueData::Directory(unknown()),
        })
        .collect();
    let crossing = seal(
        &encode_leaf_data(ctx, &crossing_rows, NodePlacementData::Root).unwrap(),
        17,
    );
    let bad = seal(
        &raw_branch(ctx, summary, &[(key(0), crossing.node.page, 16), rows[1]]),
        18,
    );
    let q = open(ctx, &bad, key(2));
    let SearchData::Child(step) = q.search_data().unwrap() else {
        panic!("step")
    };
    assert!(step.open_data(&KEY, crossing.frame.as_bytes()).is_err());
}

#[test]
fn strict_node_framing_counts_order_and_tags_refuse_without_normalization() {
    let ctx = context(IndexScopeData::Directory);
    let rows = [
        LeafRowData {
            key: key(1),
            value: ValueData::Directory(unknown()),
        },
        LeafRowData {
            key: key(2),
            value: ValueData::Directory(unknown()),
        },
    ];
    let bytes = encode_leaf_data(ctx, &rows, NodePlacementData::Root).unwrap();
    for end in 0..bytes.len() {
        assert!(NodeFrame::decode(&bytes[..end], NodePlacementData::Root).is_err());
    }
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(NodeFrame::decode(&trailing, NodePlacementData::Root).is_err());
    let (kind, packed) = offsets(ctx);
    for offset in [4, 4 + DOMAIN.len() + 1, kind - 1, kind, kind + 1] {
        let mut bad = bytes.to_vec();
        bad[offset] = 255;
        assert!(NodeFrame::decode(&bad, NodePlacementData::Root).is_err());
    }
    for count in [0u16, 3, 33, u16::MAX] {
        let mut bad = bytes.to_vec();
        bad[kind + 2..kind + 4].copy_from_slice(&count.to_be_bytes());
        assert!(NodeFrame::decode(&bad, NodePlacementData::Root).is_err());
    }
    let mut bad = bytes.to_vec();
    bad[kind + 4..kind + 20].copy_from_slice(&u128::MAX.to_be_bytes());
    assert!(NodeFrame::decode(&bad, NodePlacementData::Root).is_err());
    let mut bad = bytes.to_vec();
    bad[packed + 32..packed + 34].copy_from_slice(&385u16.to_be_bytes());
    assert!(NodeFrame::decode(&bad, NodePlacementData::Root).is_err());
    let mut reversed = rows;
    reversed.swap(0, 1);
    assert!(encode_leaf_data(ctx, &reversed, NodePlacementData::Root).is_err());
    let duplicate = [rows[0], rows[0]];
    assert!(encode_leaf_data(ctx, &duplicate, NodePlacementData::Root).is_err());
    assert!(
        NodeFrame::decode(&vec![0; MAX_INDEX_NODE_BYTES + 1], NodePlacementData::Root).is_err()
    );
}

#[test]
fn branch_sum_duplicate_ids_separators_underflow_and_overflow_refuse() {
    let ctx = context(IndexScopeData::Directory);
    let summary = NodeSummaryData {
        height: 1,
        keys: 32,
        min: key(0),
        max: key(115),
    };
    let good = [(key(0), page(1), 16), (key(100), page(2), 16)];
    for rows in [
        [good[0], (key(100), page(1), 16)],
        [(key(100), page(1), 16), good[0]],
        [(key(0), page(1), 15), good[1]],
        [(key(0), page(1), 33), good[1]],
        [(key(0), page(1), u128::MAX), good[1]],
    ] {
        assert!(
            NodeFrame::decode(&raw_branch(ctx, summary, &rows), NodePlacementData::Root).is_err()
        );
    }
    assert!(NodeFrame::decode(
        &raw_branch(
            ctx,
            NodeSummaryData {
                keys: 33,
                ..summary
            },
            &good
        ),
        NodePlacementData::Root
    )
    .is_err());
    assert!(NodeFrame::decode(
        &raw_branch(
            ctx,
            NodeSummaryData {
                height: 0,
                ..summary
            },
            &good
        ),
        NodePlacementData::Root
    )
    .is_err());
    assert!(NodeFrame::decode(
        &raw_branch(
            ctx,
            NodeSummaryData {
                height: 10,
                ..summary
            },
            &good
        ),
        NodePlacementData::Root
    )
    .is_err());
    assert!(
        NodeFrame::decode(
            &raw_branch(
                ctx,
                NodeSummaryData {
                    max: key(105),
                    ..summary
                },
                &good
            ),
            NodePlacementData::Root
        )
        .is_err(),
        "last interval cannot fit16 distinct keys"
    );
    assert!(!distance_at_least(key(3), key(2), 0));
    assert!(distance_at_least(key(2), key(18), 16));
    assert!(!distance_at_least(key(2), key(17), 16));
    assert_eq!(32u128 * 96u128.pow(9), 22_161_087_866_383_368_192);
    assert!(32u128 * 96u128.pow(9) > u64::MAX as u128);
    let mut max = [255; 32];
    max[0] = 254;
    let wide = NodeSummaryData {
        height: 9,
        keys: u64::MAX,
        min: [0; 32],
        max,
    };
    assert!(wide.validate_data(NodePlacementData::Root).is_ok());
    assert!(NodeSummaryData { height: 10, ..wide }
        .validate_data(NodePlacementData::Root)
        .is_err());
}

#[test]
fn value_scope_receipt_lengths_alias_bounds_and_unknown_states_are_strict() {
    let ctx = actions();
    let k = key(1);
    let value = claim(ctx, k, 0, ClaimHint::Canonical);
    let bytes = ValueData::Action(value).encode(ctx, k).unwrap();
    assert_eq!(bytes.len(), 343);
    for end in 0..bytes.len() {
        assert!(ValueData::decode(ctx, k, &bytes[..end]).is_err());
    }
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(ValueData::decode(ctx, k, &trailing).is_err());
    assert!(ValueData::decode(ctx, key(2), &bytes).is_err());
    assert!(ValueData::decode(
        IndexContextData {
            profile: OwnerFactId::from_bytes([99; 32]),
            ..ctx
        },
        k,
        &bytes
    )
    .is_err());
    assert!(ValueData::decode(
        IndexContextData {
            scope: IndexScopeData::Actions(OwnerFactId::from_bytes([99; 32])),
            ..ctx
        },
        k,
        &bytes
    )
    .is_err());
    assert!(ValueData::Directory(unknown()).encode(ctx, k).is_err());
    for offset in [233, 342] {
        let mut wrong = bytes.to_vec();
        wrong[offset] = 255;
        assert!(ValueData::decode(ctx, k, &wrong).is_err());
    }
    for bad in [
        ActionClaimData {
            occurrences: 0,
            ..value
        },
        ActionClaimData {
            ordinal: u64::MAX,
            ..value
        },
        ActionClaimData {
            record_row: 64,
            ..value
        },
        ActionClaimData {
            retention: RetentionData::Unknown,
            ..value
        },
    ] {
        assert!(ValueData::Action(bad).encode(ctx, k).is_err());
    }
    let incomplete = ActionClaimData {
        retention: RetentionData::Unknown,
        hint: ClaimHint::Incomplete,
        ..value
    };
    assert_eq!(
        ValueData::Action(incomplete).encode(ctx, k).unwrap().len(),
        275
    );
    let dctx = context(IndexScopeData::Directory);
    let a = key(3);
    let zero = empty(dctx, a);
    assert_eq!(
        ValueData::Directory(zero).encode(dctx, a).unwrap().len(),
        185
    );
    assert!(ValueData::Directory(DirectoryData {
        accepted_leaf_count: 1,
        ..zero
    })
    .encode(dctx, a)
    .is_err());
    assert!(ValueData::Directory(DirectoryData {
        tip_body: OwnerBodyId::from_bytes(key(4)),
        ..zero
    })
    .encode(dctx, a)
    .is_err());
    let mut bad = ValueData::Directory(zero).encode(dctx, a).unwrap().to_vec();
    bad[0] = 255;
    assert!(ValueData::decode(dctx, a, &bad).is_err());
}

#[test]
fn diagnostics_omit_private_context_payload_and_claim_identifiers() {
    let ctx = actions();
    let node = leaf(ctx, 0, 16, 19);
    let query = open(ctx, &node, key(1));
    for debug in [
        format!("{ctx:?}"),
        format!("{:?}", ctx.scope),
        format!("{query:?}"),
        format!("{:?}", query.node()),
        format!("{:?}", query.search_data().unwrap()),
    ] {
        assert!(!debug.contains("private-index-test-group"));
        assert!(!debug.contains("record_page"));
        assert!(!debug.contains("accepting_header"));
        assert!(!debug.contains("profile"));
    }
}
