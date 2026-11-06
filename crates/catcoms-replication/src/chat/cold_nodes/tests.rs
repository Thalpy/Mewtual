//! Real bounded hash mathematics, not actual retained files or selected governing authority.
use super::super::acceptance_tree::{AcceptanceLeaf, TreeTip};
use super::super::owner_bodies::AppendSpec;
use super::super::owner_header::{
    HeaderKind, HeaderSpec, OwnerHeaderId, OwnerStateCommitment, OwnerTenureId, SignedOwnerHeader,
};
use super::*;

const GROUP: &[u8] = b"bounded-private-node-page";
const VAULT: [u8; 32] = [3; 32];
fn context() -> TreeContext {
    TreeContext {
        profile: OwnerFactId::from_bytes([1; 32]),
        admission: OwnerFactId::from_bytes([2; 32]),
    }
}
fn leaves(first: u64, count: usize) -> Vec<AcceptanceLeaf> {
    (0..count)
        .map(|offset| {
            let value = first + offset as u64;
            let mut label = [0; 32];
            label[24..].copy_from_slice(&value.to_be_bytes());
            AcceptanceLeaf {
                channel: u128::from(value),
                acceptance: OwnerFactId::from_bytes(label),
            }
        })
        .collect()
}
fn append(frontier: &Frontier, count: usize) -> (AppendBody, PreparedAppend) {
    let leaves = leaves(frontier.tip().count, count);
    let prepared = frontier.prepare_append(&leaves).unwrap();
    let body = AppendBody::new(AppendSpec {
        group: GROUP.to_vec(),
        previous_tip: OwnerHeaderId::from_bytes([4; 32]),
        old_tip: prepared.old_tip(),
        new_tip: prepared.new_tip(),
        leaves,
    })
    .unwrap();
    (body, prepared)
}
fn parent(left: TreeHash, right: TreeHash) -> TreeHash {
    // Independent spelling of the published tree grammar for synthetic high-count frontiers;
    // this is hash data only, not evidence that2^63 real actions were accepted.
    let mut h = blake3::Hasher::new();
    h.update(b"mewtual/chat/acceptance-node/v2\0");
    h.update(left.as_bytes());
    h.update(right.as_bytes());
    TreeHash::from_bytes(*h.finalize().as_bytes())
}
fn synthetic_frontier(count: u64) -> Frontier {
    let roots: Vec<_> = (0..64)
        .filter(|bit| count >> bit & 1 != 0)
        .map(|bit| TreeHash::from_bytes([bit as u8 + 1; 32]))
        .collect();
    let root = roots
        .iter()
        .copied()
        .fold(None, |right, left| {
            Some(right.map_or(left, |right| parent(left, right)))
        })
        .unwrap_or_else(|| context().empty_hash());
    Frontier::from_tip(
        TreeTip {
            context: context(),
            count,
            root,
        },
        &roots,
    )
    .unwrap()
}

#[test]
fn private_node_pages_preserve_every_actual_leaf_and_carry_ordinal_across_batches() {
    let mut frontier = Frontier::empty(context());
    for count in [1, 1, 1, 5, 64, 3] {
        let (body, prepared) = append(&frontier, count);
        let encoded = encode_node_page(&VAULT, &body, &prepared).unwrap();
        let page = NodePage::decode(&encoded).unwrap();
        let group_start = 4 + DOMAIN.len() + 2 + 36 + 4;
        assert_eq!(page.group.as_ptr(), encoded[group_start..].as_ptr());
        assert_eq!(
            page.packed.as_ptr(),
            encoded[encoded.len() - page.node_count() * 32..].as_ptr()
        );
        assert_eq!(page.vault_domain(), &VAULT);
        assert_eq!(page.group(), GROUP);
        assert_eq!(page.context(), context());
        assert_eq!(page.append_body(), body.id());
        assert_eq!(page.before_count(), prepared.old_tip().count);
        assert_eq!(page.after_count(), prepared.new_tip().count);
        page.verify_exact_append(&VAULT, &body, &frontier).unwrap();
        for (offset, node) in prepared.created_nodes().iter().enumerate() {
            assert_eq!(
                node.creation_ordinal().unwrap(),
                page.first_ordinal() + offset as u128
            );
            assert_eq!(
                page.hash_at_ordinal(node.creation_ordinal().unwrap()),
                Some(node.hash)
            );
        }
        assert_eq!(
            page.hash_at_ordinal(page.first_ordinal() + page.node_count() as u128),
            None
        );
        if page.first_ordinal() != 0 {
            assert_eq!(page.hash_at_ordinal(page.first_ordinal() - 1), None);
        }
        assert_eq!(page.hash_at_ordinal(u128::MAX), None);
        assert!(encoded.len() <= MAX_NODE_PAGE_BYTES && MAX_NODE_PAGE_BYTES <= 65536);
        frontier = prepared.into_frontier();
    }
}

#[test]
fn private_node_page_handles_120_carries_and_ordinals_above_u64_without_narrowing() {
    for before in [(1u64 << 63) - 64, 1u64 << 63, u64::MAX - 64] {
        let frontier = synthetic_frontier(before);
        let (body, prepared) = append(&frontier, 64);
        let encoded = encode_node_page(&VAULT, &body, &prepared).unwrap();
        let page = NodePage::decode(&encoded).unwrap();
        page.verify_exact_append(&VAULT, &body, &frontier).unwrap();
        if before == (1u64 << 63) - 64 {
            assert_eq!(page.node_count(), 64 + 120);
        }
        if before >= 1u64 << 63 {
            assert!(prepared
                .created_nodes()
                .iter()
                .any(|node| node.creation_ordinal().unwrap() > u128::from(u64::MAX)));
        }
        assert_eq!(page.hash_at_ordinal(u128::MAX), None);
        for node in prepared.created_nodes() {
            assert_eq!(
                page.hash_at_ordinal(node.creation_ordinal().unwrap()),
                Some(node.hash)
            );
        }
    }
}

#[test]
fn private_node_page_hash_substitution_fails_even_with_genuine_unchanged_signed_append() {
    let frontier = Frontier::empty(context());
    let (body, prepared) = append(&frontier, 8);
    let signer = catcoms_crypto::DeviceKeypair::from_seed(&[9; 32]);
    let header = SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: GROUP.to_vec(),
            profile: context().profile,
            issuer: signer.verifying_key().to_bytes(),
            tenure: OwnerTenureId::from_bytes([10; 32]),
            parent: OwnerHeaderId::from_bytes([11; 32]),
            sequence: 1,
            kind: HeaderKind::AcceptanceAppend,
            body: body.id(),
            state: OwnerStateCommitment::from_bytes([12; 32]),
        },
        |bytes| Ok(signer.sign(bytes)),
    )
    .unwrap();
    body.verify_header_binding(&header).unwrap(); // data binding, NOT a genuine genesis/Current
    let encoded = encode_node_page(&VAULT, &body, &prepared).unwrap();
    let node_count = prepared.created_nodes().len();
    let packed_start = encoded.len() - node_count * 32;
    let mut forged = encoded.to_vec();
    forged[packed_start] ^= 1; // an interior leaf; keep the last created root unchanged
    let page = NodePage::decode(&forged).unwrap();
    let last = prepared.created_nodes().last().unwrap();
    assert_eq!(
        page.hash_at_ordinal(last.creation_ordinal().unwrap()),
        Some(last.hash)
    );
    assert_eq!(
        page.verify_exact_append(&VAULT, &body, &frontier),
        Err(ChatError::Dependency)
    );
    body.verify_header_binding(&header).unwrap();
}

#[test]
fn private_node_page_refuses_vault_group_context_body_and_prepared_append_substitution() {
    let frontier = Frontier::empty(context());
    let (body, prepared) = append(&frontier, 2);
    let encoded = encode_node_page(&VAULT, &body, &prepared).unwrap();
    let page = NodePage::decode(&encoded).unwrap();
    assert_eq!(
        page.verify_exact_append(&[8; 32], &body, &frontier),
        Err(ChatError::Scope)
    );
    let wrong_frontier = Frontier::empty(TreeContext {
        admission: OwnerFactId::from_bytes([7; 32]),
        ..context()
    });
    assert!(page
        .verify_exact_append(&VAULT, &body, &wrong_frontier)
        .is_err());
    let (_, wrong_prepared) = append(&frontier, 3);
    assert!(encode_node_page(&VAULT, &body, &wrong_prepared).is_err());
    let mut other_spec = body.spec().clone();
    other_spec.previous_tip = OwnerHeaderId::from_bytes([6; 32]);
    let other = AppendBody::new(other_spec).unwrap();
    assert_eq!(
        page.verify_exact_append(&VAULT, &other, &frontier),
        Err(ChatError::Scope)
    );
    let vault_start = 4 + DOMAIN.len() + 2 + 4;
    let group_start = vault_start + 32 + 4;
    let profile_start = group_start + GROUP.len() + 4;
    for position in [
        vault_start,
        group_start,
        profile_start,
        profile_start + 36,
        profile_start + 72,
    ] {
        let mut swapped = encoded.to_vec();
        swapped[position] ^= 1;
        let swapped = NodePage::decode(&swapped).unwrap();
        assert_eq!(
            swapped.verify_exact_append(&VAULT, &body, &frontier),
            Err(ChatError::Scope)
        );
    }
}

#[test]
fn private_node_encoder_refuses_changed_leaf_list_with_identical_declared_counts_and_roots() {
    let frontier = Frontier::empty(context());
    let (body, prepared) = append(&frontier, 8);
    let mut spec = body.spec().clone();
    spec.leaves[0].acceptance = OwnerFactId::from_bytes([42; 32]);
    let substituted = AppendBody::new(spec).unwrap();
    assert_eq!(substituted.spec().old_tip, prepared.old_tip());
    assert_eq!(substituted.spec().new_tip, prepared.new_tip());
    assert_ne!(substituted.id(), body.id());
    assert!(substituted.prepare_on_frontier(&frontier).is_err());
    assert!(matches!(
        encode_node_page(&VAULT, &substituted, &prepared),
        Err(ChatError::Dependency)
    ));
    let original = encode_node_page(&VAULT, &body, &prepared).unwrap();
    NodePage::decode(&original)
        .unwrap()
        .verify_exact_append(&VAULT, &body, &frontier)
        .unwrap();
}

fn raw_counts(before: u64, after: u64, count: u16, packed: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(DOMAIN).unwrap();
    e.put_u16(VERSION);
    e.put_bytes(&VAULT).unwrap();
    e.put_bytes(GROUP).unwrap();
    e.put_bytes(context().profile.as_bytes()).unwrap();
    e.put_bytes(context().admission.as_bytes()).unwrap();
    e.put_bytes(&[0; 32]).unwrap();
    e.put_u64(before).put_u64(after).put_u16(count);
    e.put_bytes(packed).unwrap();
    e.finish()
}

#[test]
fn private_node_page_checks_full_borrowed_framing_and_exact_geometry_without_owned_vectors() {
    let frontier = Frontier::empty(context());
    let (body, prepared) = append(&frontier, 2);
    let encoded = encode_node_page(&VAULT, &body, &prepared).unwrap();
    for end in 0..encoded.len() {
        assert!(NodePage::decode(&encoded[..end]).is_err());
    }
    let mut extra = encoded.to_vec();
    extra.push(0);
    assert!(NodePage::decode(&extra).is_err());
    let oversized = vec![0; MAX_NODE_PAGE_BYTES + 1];
    assert!(NodePage::decode(&oversized).is_err());
    let mut version = encoded.to_vec();
    version[4 + DOMAIN.len() + 1] = 2;
    assert!(NodePage::decode(&version).is_err());
    let mut empty_group = encoded.to_vec();
    let group_length = 4 + DOMAIN.len() + 2 + 36;
    empty_group[group_length..group_length + 4].copy_from_slice(&0u32.to_be_bytes());
    assert!(NodePage::decode(&empty_group).is_err());
    for bytes in [
        raw_counts(0, 0, 0, &[]),
        raw_counts(2, 1, 1, &[0; 32]),
        raw_counts(u64::MAX, 0, 1, &[0; 32]),
        raw_counts(0, 65, 65, &[0; 32]),
        raw_counts(0, 2, 2, &[0; 64]), // two leaves MUST also create their parent: count3
        raw_counts(0, 2, 3, &[0; 64]),
        raw_counts(0, 2, 3, &[0; 128]),
    ] {
        assert!(NodePage::decode(&bytes).is_err());
    }
    assert!(
        NodePage::decode(&raw_counts(0, 2, 3, &[0; 96])).is_ok(),
        "syntax cannot certify a claimed body or hashes"
    );
}
