//! Real encrypted page and signed-action boundaries where possible; manifest/link labels are
//! DATA and deliberately do not model mounted source custody or historical/current authority.
use super::super::acceptance_tree::{AcceptanceLeaf, Frontier, TreeContext};
use super::super::cold_actions::{
    assemble_action, ActionChunk, ActionPagesExpectation, PreparedActionPages,
};
use super::super::cold_nodes::encode_node_page;
use super::super::owner_bodies::{AppendBody, AppendSpec};
use super::super::{ActionSpec, BasisId, Body, GrantId, Realm, SignedAction, MAX_TEXT_BYTES};
use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_storage::private_chat_page::{open_private_chat_page, PreparedPrivateChatPage};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const GROUP: &[u8] = b"bounded-private-manifest";
const VAULT: [u8; 32] = [1; 32];
const KEY: [u8; 32] = [2; 32];
fn profile() -> OwnerFactId {
    OwnerFactId::from_bytes([3; 32])
}
fn body() -> OwnerBodyId {
    OwnerBodyId::from_bytes([4; 32])
}
fn old_header() -> OwnerHeaderId {
    OwnerHeaderId::from_bytes([5; 32])
}
fn new_header() -> OwnerHeaderId {
    OwnerHeaderId::from_bytes([6; 32])
}
fn source_head() -> ArchiveHeadId {
    ArchiveHeadId::from_bytes([7; 32])
}
fn previous_page() -> PrivateChatPageId {
    PrivateChatPageId::from_bytes([8; 32])
}
fn descriptor(role: PageRole, bytes: usize, entries: u16, seed: u8) -> PageDescriptor {
    PageDescriptor {
        role,
        page: PrivateChatPageId::from_bytes([seed; 32]),
        plaintext_bytes: bytes,
        entries,
    }
}
fn spec<'a>(op: ArchiveOperation, pages: &'a [PageDescriptor]) -> ManifestSpec<'a> {
    let genesis = matches!(op, ArchiveOperation::Genesis { .. });
    ManifestSpec {
        vault: VAULT,
        group: GROUP,
        profile: profile(),
        sequence: if genesis { 1 } else { 2 },
        previous: (!genesis).then(previous_page),
        source_head: (!genesis).then(source_head),
        before_header: (!genesis).then(old_header),
        after_header: if matches!(op, ArchiveOperation::RetainAction { .. }) {
            old_header()
        } else {
            new_header()
        },
        operation: op,
        pages,
    }
}
fn header_page(genesis: bool) -> [PageDescriptor; 1] {
    [descriptor(
        if genesis {
            PageRole::GenesisRecords
        } else {
            PageRole::HeaderBodyRecords
        },
        1000,
        if genesis { 3 } else { 2 },
        9,
    )]
}
fn append_pages(before: u64, after: u64, group_bytes: usize) -> [PageDescriptor; 3] {
    let (leaves, nodes) = append_geometry(before, after).unwrap();
    [
        descriptor(PageRole::AcceptanceRecords, 1000, leaves as u16, 10),
        descriptor(
            PageRole::CompleteNodes,
            MAX_NODE_PAGE_BYTES - MAX_GROUP_ID_BYTES - MAX_NODE_PAGE_HASHES * 32
                + group_bytes
                + nodes * 32,
            nodes as u16,
            11,
        ),
        descriptor(PageRole::HeaderBodyRecords, 1000, 2, 12),
    ]
}

#[test]
fn every_closed_operation_roundtrips_borrowed_context_and_fixed_descriptor_roles() {
    let genesis = header_page(true);
    let header = header_page(false);
    let append = append_pages(0, 64, GROUP.len());
    let operations = [
        (
            ArchiveOperation::Genesis { body: body() },
            genesis.as_slice(),
        ),
        (
            ArchiveOperation::Admission { body: body() },
            header.as_slice(),
        ),
        (
            ArchiveOperation::Append {
                body: body(),
                admission: profile(),
                before: 0,
                after: 64,
            },
            append.as_slice(),
        ),
        (
            ArchiveOperation::Removal { body: body() },
            header.as_slice(),
        ),
        (ArchiveOperation::Review { body: body() }, header.as_slice()),
    ];
    for (operation, pages) in operations {
        let expected = spec(operation, pages);
        let encoded = encode_manifest(&expected).unwrap();
        assert!(encoded.len() <= MAX_MANIFEST_PAGE_BYTES);
        let parsed = ManifestPage::decode(&encoded).unwrap();
        assert_eq!(parsed.operation(), operation);
        assert_eq!(parsed.vault_domain(), &VAULT);
        assert_eq!(parsed.group(), GROUP);
        assert_eq!(parsed.profile(), profile());
        assert_eq!(parsed.sequence(), expected.sequence);
        assert_eq!(parsed.previous_manifest(), expected.previous);
        assert_eq!(parsed.original_source_head(), expected.source_head);
        assert_eq!(parsed.before_header(), expected.before_header);
        assert_eq!(parsed.after_header(), expected.after_header);
        assert_eq!(parsed.page_count(), pages.len());
        for (index, page) in pages.iter().enumerate() {
            assert_eq!(parsed.page(index).unwrap(), *page);
        }
        assert!(parsed.page(pages.len()).is_err());
        assert!(parsed.page(usize::MAX).is_err());
        let group_offset = 4 + DOMAIN.len() + 2 + 36 + 4;
        assert_eq!(parsed.group.as_ptr(), encoded[group_offset..].as_ptr());
        assert_eq!(
            parsed.packed.as_ptr(),
            encoded[encoded.len() - pages.len() * DESCRIPTOR_BYTES..].as_ptr()
        );
    }
}

#[test]
fn real_signed_action_chunks_and_real_node_output_match_manifest_geometry() {
    let mut rng = ChaCha20Rng::from_seed([21; 32]);
    for text in ["original message".to_owned(), "x".repeat(MAX_TEXT_BYTES)] {
        let action = SignedAction::sign(
            &DeviceKeypair::from_seed(&[22; 32]),
            ActionSpec {
                realm: Realm::new(GROUP, 41).unwrap(),
                basis: BasisId::from_bytes([23; 32]),
                grant: GrantId::from_bytes([24; 32]),
                nonce: [25; 16],
                presented_ms: 1000,
                parents: vec![],
                body: Body::Create {
                    text,
                    reply_to: None,
                },
            },
        )
        .unwrap();
        let prepared = PreparedActionPages::prepare(VAULT, profile(), &action).unwrap();
        let plaintexts: Vec<_> = (0..prepared.chunk_count())
            .map(|i| prepared.encode_chunk(i).unwrap())
            .collect();
        let sealed: Vec<_> = plaintexts
            .iter()
            .map(|page| PreparedPrivateChatPage::prepare(&KEY, page, &mut rng).unwrap())
            .collect();
        let descriptors: Vec<_> = plaintexts
            .iter()
            .zip(&sealed)
            .map(|(raw, sealed)| PageDescriptor {
                role: PageRole::OriginalActionChunk,
                page: sealed.id(),
                plaintext_bytes: raw.len(),
                entries: 1,
            })
            .collect();
        let operation = ArchiveOperation::RetainAction {
            action: action.id(),
            original: prepared.original_digest(),
            total_bytes: action.encode().len() as u32,
        };
        let encoded = encode_manifest(&spec(operation, &descriptors)).unwrap();
        let manifest = ManifestPage::decode(&encoded).unwrap();
        let opened: Vec<_> = sealed
            .iter()
            .enumerate()
            .map(|(i, page)| {
                open_private_chat_page(&KEY, manifest.page(i).unwrap().page, page.as_bytes())
                    .unwrap()
            })
            .collect();
        let chunks: Vec<_> = opened
            .iter()
            .map(|raw| ActionChunk::decode(raw).unwrap())
            .collect();
        let restored = assemble_action(
            &ActionPagesExpectation {
                vault: &VAULT,
                group: GROUP,
                profile: profile(),
                action: action.id(),
                original: prepared.original_digest(),
            },
            &chunks,
        )
        .unwrap();
        assert_eq!(restored.original_bytes(), action.encode());
        assert_eq!(restored.action(), &action);
        let mut wrong = descriptors.clone();
        wrong[0].plaintext_bytes += 1;
        assert!(encode_manifest(&spec(operation, &wrong)).is_err());
    }
    let frontier = Frontier::empty(TreeContext {
        profile: profile(),
        admission: OwnerFactId::from_bytes([26; 32]),
    });
    let leaves: Vec<_> = (0..64)
        .map(|i| AcceptanceLeaf {
            channel: i,
            acceptance: OwnerFactId::from_bytes([i as u8; 32]),
        })
        .collect();
    let prepared = frontier.prepare_append(&leaves).unwrap();
    let append = AppendBody::new(AppendSpec {
        group: GROUP.to_vec(),
        previous_tip: old_header(),
        old_tip: prepared.old_tip(),
        new_tip: prepared.new_tip(),
        leaves,
    })
    .unwrap();
    let raw_nodes = encode_node_page(&VAULT, &append, &prepared).unwrap();
    let nodes = PreparedPrivateChatPage::prepare(&KEY, &raw_nodes, &mut rng).unwrap();
    let mut pages = append_pages(0, 64, GROUP.len());
    pages[1].page = nodes.id();
    assert_eq!(pages[1].plaintext_bytes, raw_nodes.len());
    assert_eq!(
        usize::from(pages[1].entries),
        prepared.created_nodes().len()
    );
    let encoded = encode_manifest(&spec(
        ArchiveOperation::Append {
            body: append.id(),
            admission: frontier.tip().context.admission,
            before: 0,
            after: 64,
        },
        &pages,
    ))
    .unwrap();
    let parsed = ManifestPage::decode(&encoded).unwrap();
    assert_eq!(
        open_private_chat_page(&KEY, parsed.page(1).unwrap().page, nodes.as_bytes())
            .unwrap()
            .as_slice(),
        raw_nodes.as_slice()
    );
}

#[test]
fn invalid_operation_shapes_counts_order_duplicate_ids_and_boundaries_refuse() {
    let pages = header_page(false);
    for operation in [
        ArchiveOperation::Admission { body: body() },
        ArchiveOperation::Removal { body: body() },
        ArchiveOperation::Review { body: body() },
    ] {
        let mut input = spec(operation, &pages);
        input.after_header = old_header();
        assert!(encode_manifest(&input).is_err());
        input.after_header = new_header();
        for sequence in [0, 1] {
            input.sequence = sequence;
            assert!(encode_manifest(&input).is_err());
        }
        input.sequence = 2;
        input.previous = None;
        assert!(encode_manifest(&input).is_err());
        input.previous = Some(previous_page());
        input.source_head = None;
        assert!(encode_manifest(&input).is_err());
        input.source_head = Some(source_head());
        input.before_header = None;
        assert!(encode_manifest(&input).is_err());
        input.before_header = Some(old_header());
        input.group = b"";
        assert!(encode_manifest(&input).is_err());
        let long_group = vec![1; MAX_GROUP_ID_BYTES + 1];
        input.group = &long_group;
        assert!(encode_manifest(&input).is_err());
    }
    let genesis = header_page(true);
    for which in 0..4 {
        let mut input = spec(ArchiveOperation::Genesis { body: body() }, &genesis);
        match which {
            0 => input.sequence = 2,
            1 => input.previous = Some(previous_page()),
            2 => input.source_head = Some(source_head()),
            _ => input.before_header = Some(old_header()),
        }
        assert!(encode_manifest(&input).is_err());
    }
    let operation = ArchiveOperation::Append {
        body: body(),
        admission: profile(),
        before: 0,
        after: 64,
    };
    let pages = append_pages(0, 64, GROUP.len());
    for invalid in [Vec::new(), pages[..2].to_vec(), vec![pages[0]; 4]] {
        assert!(encode_manifest(&spec(operation, &invalid)).is_err());
    }
    for which in 0..6 {
        let mut wrong = pages;
        match which {
            0 => wrong.swap(0, 1),
            1 => wrong[1].page = wrong[0].page,
            2 => wrong[1].entries -= 1,
            3 => wrong[0].entries += 1,
            4 => wrong[1].plaintext_bytes += 1,
            _ => wrong[2].role = PageRole::GenesisRecords,
        }
        assert!(encode_manifest(&spec(operation, &wrong)).is_err());
    }
    for bytes in [0, MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES + 1, usize::MAX] {
        let mut wrong = pages;
        wrong[0].plaintext_bytes = bytes;
        assert!(encode_manifest(&spec(operation, &wrong)).is_err());
    }
    for (before, after) in [(0, 0), (10, 9), (0, 65), (u64::MAX, 0)] {
        assert!(encode_manifest(&spec(
            ArchiveOperation::Append {
                body: body(),
                admission: profile(),
                before,
                after
            },
            &pages
        ))
        .is_err());
    }
}

#[test]
fn strict_borrowed_framing_rejects_unknown_tags_counts_lengths_and_every_truncation() {
    let pages = append_pages(0, 64, GROUP.len());
    let encoded = encode_manifest(&spec(
        ArchiveOperation::Append {
            body: body(),
            admission: profile(),
            before: 0,
            after: 64,
        },
        &pages,
    ))
    .unwrap();
    let payload_start = encoded.len() - pages.len() * DESCRIPTOR_BYTES;
    let option_start = 4 + DOMAIN.len() + 2 + 36 + 4 + GROUP.len() + 36 + 8;
    let operation_start = option_start + 3 * 37 + 36;
    for (offset, value) in [
        (4, 255),
        (4 + DOMAIN.len() + 1, 2),
        (option_start, 2),
        (operation_start, 255),
        (payload_start, 255),
        (payload_start - 5, 4),
    ] {
        let mut wrong = encoded.to_vec();
        wrong[offset] = value;
        assert!(ManifestPage::decode(&wrong).is_err(), "offset{offset}");
    }
    for offset in [0, payload_start - 4] {
        let mut wrong = encoded.to_vec();
        wrong[offset..offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(ManifestPage::decode(&wrong).is_err());
    }
    for end in 0..encoded.len() {
        assert!(ManifestPage::decode(&encoded[..end]).is_err());
    }
    let mut trailing = encoded.to_vec();
    trailing.push(0);
    assert!(ManifestPage::decode(&trailing).is_err());
    assert!(ManifestPage::decode(&vec![0; MAX_MANIFEST_PAGE_BYTES + 1]).is_err());
}

#[test]
fn exact_data_links_reject_other_context_original_head_page_and_public_header_labels() {
    let genesis_pages = header_page(true);
    let old_raw = encode_manifest(&spec(
        ArchiveOperation::Genesis { body: body() },
        &genesis_pages,
    ))
    .unwrap();
    let previous = ManifestPage::decode(&old_raw).unwrap();
    let pages = header_page(false);
    let mut input = spec(ArchiveOperation::Removal { body: body() }, &pages);
    input.before_header = Some(previous.after_header());
    input.after_header = OwnerHeaderId::from_bytes([31; 32]);
    let next_raw = encode_manifest(&input).unwrap();
    let next = ManifestPage::decode(&next_raw).unwrap();
    next.check_data_successor(previous_page(), source_head(), &previous)
        .unwrap();
    assert!(next
        .check_data_successor(
            PrivateChatPageId::from_bytes(*source_head().as_bytes()),
            source_head(),
            &previous
        )
        .is_err());
    assert!(next
        .check_data_successor(
            previous_page(),
            ArchiveHeadId::from_bytes(*previous_page().as_bytes()),
            &previous
        )
        .is_err());
    assert!(next
        .check_data_successor(
            PrivateChatPageId::from_bytes(*previous.after_header().as_bytes()),
            source_head(),
            &previous
        )
        .is_err());
    for which in 0..7 {
        let mut altered = spec(ArchiveOperation::Removal { body: body() }, &pages);
        altered.before_header = input.before_header;
        altered.after_header = input.after_header;
        match which {
            0 => altered.vault = [41; 32],
            1 => altered.group = b"same labels, another group",
            2 => altered.profile = OwnerFactId::from_bytes([42; 32]),
            3 => altered.previous = Some(PrivateChatPageId::from_bytes([43; 32])),
            4 => altered.source_head = Some(ArchiveHeadId::from_bytes([44; 32])),
            5 => altered.before_header = Some(old_header()),
            _ => altered.sequence = 3,
        }
        let raw = encode_manifest(&altered).unwrap();
        let parsed = ManifestPage::decode(&raw).unwrap();
        assert!(parsed
            .check_data_successor(previous_page(), source_head(), &previous)
            .is_err());
    }
    // Ordinary retention AFTER a control transition keeps that transition's exact header;
    // it cannot quietly return to an earlier global/public header or advance it on its own.
    let action_page = [descriptor(
        PageRole::OriginalActionChunk,
        MAX_ACTION_CHUNK_PAGE_BYTES - ACTION_CHUNK_BYTES - MAX_GROUP_ID_BYTES + GROUP.len() + 1,
        1,
        70,
    )];
    let mut retained = spec(
        ArchiveOperation::RetainAction {
            action: ActionId::from_bytes([71; 32]),
            original: OriginalActionDigest::from_bytes([72; 32]),
            total_bytes: 1,
        },
        &action_page,
    );
    let control_page = PrivateChatPageId::from_bytes([73; 32]);
    retained.sequence = 3;
    retained.previous = Some(control_page);
    retained.before_header = Some(next.after_header());
    retained.after_header = next.after_header();
    let raw = encode_manifest(&retained).unwrap();
    let parsed = ManifestPage::decode(&raw).unwrap();
    parsed
        .check_data_successor(control_page, source_head(), &next)
        .unwrap();
    retained.after_header = old_header();
    assert!(encode_manifest(&retained).is_err());
}

#[test]
fn complete_chunk_geometry_has_one_logical_action_and_at_most_three_content_pages() {
    // Synthetic envelope geometry, NOT evidence that an arbitrary byte envelope is signed.
    // The separate real-action test above crosses prepare/seal/open/assemble/signature boundaries.
    for total in [
        1,
        ACTION_CHUNK_BYTES,
        ACTION_CHUNK_BYTES + 1,
        2 * ACTION_CHUNK_BYTES,
        2 * ACTION_CHUNK_BYTES + 1,
        MAX_ACTION_BYTES,
    ] {
        let count = total.div_ceil(ACTION_CHUNK_BYTES);
        let pages: Vec<_> = (0..count)
            .map(|index| {
                descriptor(
                    PageRole::OriginalActionChunk,
                    MAX_ACTION_CHUNK_PAGE_BYTES - ACTION_CHUNK_BYTES - MAX_GROUP_ID_BYTES
                        + GROUP.len()
                        + (total - index * ACTION_CHUNK_BYTES).min(ACTION_CHUNK_BYTES),
                    1,
                    index as u8,
                )
            })
            .collect();
        let operation = ArchiveOperation::RetainAction {
            action: ActionId::from_bytes([80; 32]),
            original: OriginalActionDigest::from_bytes([81; 32]),
            total_bytes: total as u32,
        };
        let encoded = encode_manifest(&spec(operation, &pages)).unwrap();
        let parsed = ManifestPage::decode(&encoded).unwrap();
        assert_eq!(parsed.page_count(), count);
        assert!(count <= 3);
        let mut wrong = pages.clone();
        wrong[0].entries = 2;
        assert!(encode_manifest(&spec(operation, &wrong)).is_err());
    }
    let page = [descriptor(PageRole::OriginalActionChunk, 1000, 1, 82)];
    for total_bytes in [0, MAX_ACTION_BYTES as u32 + 1, u32::MAX] {
        assert!(encode_manifest(&spec(
            ArchiveOperation::RetainAction {
                action: ActionId::from_bytes([80; 32]),
                original: OriginalActionDigest::from_bytes([81; 32]),
                total_bytes,
            },
            &page
        ))
        .is_err());
    }
}

#[test]
fn max_current_geometry_and_long_finite_manifest_history_need_no_lifetime_page_cache() {
    let group = vec![1; MAX_GROUP_ID_BYTES];
    let before = (1u64 << 63) - 1;
    let after = before + 64;
    let pages = append_pages(before, after, group.len());
    assert_eq!(pages[1].entries, 184); //64 leaves+120 real carry positions at this count.
    let mut input = spec(
        ArchiveOperation::Append {
            body: body(),
            admission: profile(),
            before,
            after,
        },
        &pages,
    );
    input.group = &group;
    let raw = encode_manifest(&input).unwrap();
    assert!(raw.len() <= MAX_MANIFEST_PAGE_BYTES);
    assert_eq!(
        ManifestPage::decode(&raw).unwrap().page(1).unwrap(),
        pages[1]
    );
    let mut rng = ChaCha20Rng::from_seed([51; 32]);
    let genesis = header_page(true);
    let mut current_raw =
        encode_manifest(&spec(ArchiveOperation::Genesis { body: body() }, &genesis)).unwrap();
    let mut current = PreparedPrivateChatPage::prepare(&KEY, &current_raw, &mut rng).unwrap();
    let control = header_page(false);
    // This is bounded DATA linking/encryption, not5000 actual signed owner operations or a
    // production cold-lookup test. Only the last original segment is retained by this codec.
    for sequence in 2u64..=5001 {
        let previous = ManifestPage::decode(&current_raw).unwrap();
        let mut head = [0; 32];
        head[24..].copy_from_slice(&sequence.to_be_bytes());
        let mut input = spec(ArchiveOperation::Review { body: body() }, &control);
        input.sequence = sequence;
        input.previous = Some(current.id());
        input.source_head = Some(ArchiveHeadId::from_bytes(head));
        input.before_header = Some(previous.after_header());
        input.after_header = OwnerHeaderId::from_bytes(head);
        let next_raw = encode_manifest(&input).unwrap();
        let next = ManifestPage::decode(&next_raw).unwrap();
        next.check_data_successor(current.id(), ArchiveHeadId::from_bytes(head), &previous)
            .unwrap();
        assert_eq!(next.page_count(), 1);
        let sealed = PreparedPrivateChatPage::prepare(&KEY, &next_raw, &mut rng).unwrap();
        current_raw = next_raw;
        current = sealed;
    }
    let previous = ManifestPage::decode(&current_raw).unwrap();
    let mut last = spec(ArchiveOperation::Review { body: body() }, &control);
    last.sequence = u64::MAX;
    last.before_header = Some(previous.after_header());
    last.after_header = old_header();
    let last_raw = encode_manifest(&last).unwrap();
    let last = ManifestPage::decode(&last_raw).unwrap();
    let mut wrapped = spec(ArchiveOperation::Review { body: body() }, &control);
    wrapped.before_header = Some(last.after_header());
    let wrapped_raw = encode_manifest(&wrapped).unwrap();
    let wrapped = ManifestPage::decode(&wrapped_raw).unwrap();
    assert!(wrapped
        .check_data_successor(previous_page(), source_head(), &last)
        .is_err());
}
