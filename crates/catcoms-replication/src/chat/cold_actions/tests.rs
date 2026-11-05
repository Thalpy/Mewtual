//! Actual signatures and exact original bytes. No private file/current authority is modeled.
use super::super::{ActionSpec, BasisId, Body, GrantId, Realm, MAX_PARENTS, MAX_TEXT_BYTES};
use super::*;
use catcoms_crypto::DeviceKeypair;

const VAULT: [u8; 32] = [1; 32];
fn profile() -> OwnerFactId {
    OwnerFactId::from_bytes([2; 32])
}
fn action(large: bool, nonce: u8) -> SignedAction {
    let (body, parents) = if large {
        (
            Body::Edit {
                create: ActionId::from_bytes([99; 32]),
                text: "x".repeat(MAX_TEXT_BYTES),
            },
            (0..MAX_PARENTS)
                .map(|value| ActionId::from_bytes([value as u8; 32]))
                .collect(),
        )
    } else {
        (
            Body::Create {
                text: "original".into(),
                reply_to: None,
            },
            vec![],
        )
    };
    SignedAction::sign(
        &DeviceKeypair::from_seed(&[3; 32]),
        ActionSpec {
            realm: Realm::new(b"exact-original-action-pages", 41).unwrap(),
            basis: BasisId::from_bytes([4; 32]),
            grant: GrantId::from_bytes([5; 32]),
            nonce: [nonce; 16],
            presented_ms: 1000,
            parents,
            body,
        },
    )
    .unwrap()
}
fn expected<'a>(group: &'a [u8], prepared: &PreparedActionPages) -> ActionPagesExpectation<'a> {
    ActionPagesExpectation {
        vault: &VAULT,
        group,
        profile: profile(),
        action: prepared.action_id(),
        original: prepared.original_digest(),
    }
}
fn pages(prepared: &PreparedActionPages) -> Vec<Zeroizing<Vec<u8>>> {
    (0..prepared.chunk_count())
        .map(|index| prepared.encode_chunk(index).unwrap())
        .collect()
}
fn parse(pages: &[Zeroizing<Vec<u8>>]) -> Vec<ActionChunk<'_>> {
    pages
        .iter()
        .map(|page| ActionChunk::decode(page).unwrap())
        .collect()
}

#[test]
fn private_action_pages_roundtrip_exact_small_and_max_text_envelopes_with_actual_signatures() {
    for large in [false, true] {
        let action = action(large, 1);
        let prepared = PreparedActionPages::prepare(VAULT, profile(), &action).unwrap();
        if large {
            assert!(prepared.chunk_count() > 1);
        }
        let pages = pages(&prepared);
        let chunks = parse(&pages);
        for (index, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index(), index);
            assert_eq!(chunk.chunk_count(), pages.len());
            assert_eq!(chunk.action_id(), action.id());
            assert_eq!(chunk.original_digest(), prepared.original_digest());
            assert!(pages[index].len() <= MAX_ACTION_CHUNK_PAGE_BYTES);
            let payload_start = pages[index].len() - chunk.payload.len();
            assert_eq!(
                chunk.payload.as_ptr(),
                pages[index][payload_start..].as_ptr()
            );
        }
        let original =
            assemble_action(&expected(action.spec().realm.group(), &prepared), &chunks).unwrap();
        assert_eq!(original.original_bytes(), action.encode());
        assert_eq!(original.action(), &action);
        assert!(original.action().verify());
        assert!(prepared.encode_chunk(prepared.chunk_count()).is_err());
        assert!(prepared.encode_chunk(usize::MAX).is_err());
    }
}

#[test]
fn private_action_assembly_refuses_missing_reordered_duplicate_and_mixed_original_chunks() {
    let a = action(true, 2);
    let b = action(true, 3);
    let pa = PreparedActionPages::prepare(VAULT, profile(), &a).unwrap();
    let pb = PreparedActionPages::prepare(VAULT, profile(), &b).unwrap();
    let aa = pages(&pa);
    let bb = pages(&pb);
    let chunks = parse(&aa);
    let e = expected(a.spec().realm.group(), &pa);
    assert!(assemble_action(&e, &[]).is_err());
    assert!(assemble_action(&e, &chunks[..chunks.len() - 1]).is_err());
    let mut reversed = parse(&aa);
    reversed.reverse();
    assert!(assemble_action(&e, &reversed).is_err());
    let duplicate = vec![
        ActionChunk::decode(&aa[0]).unwrap(),
        ActionChunk::decode(&aa[0]).unwrap(),
    ];
    assert!(assemble_action(&e, &duplicate).is_err());
    let mixed = vec![
        ActionChunk::decode(&aa[0]).unwrap(),
        ActionChunk::decode(&bb[1]).unwrap(),
    ];
    assert!(assemble_action(&e, &mixed).is_err());
    let mut corrupt: Vec<_> = aa
        .iter()
        .map(|page| Zeroizing::new(page.to_vec()))
        .collect();
    *corrupt[0].last_mut().unwrap() ^= 1;
    assert!(matches!(
        assemble_action(&e, &parse(&corrupt)),
        Err(ChatError::Dependency)
    ));
}

#[test]
fn private_action_assembly_uses_external_vault_profile_group_action_and_original_expectations() {
    let action = action(false, 4);
    let prepared = PreparedActionPages::prepare(VAULT, profile(), &action).unwrap();
    let pages = pages(&prepared);
    let chunks = parse(&pages);
    let group = action.spec().realm.group();
    let mut e = expected(group, &prepared);
    let other_vault = [8; 32];
    e.vault = &other_vault;
    assert!(matches!(
        assemble_action(&e, &chunks),
        Err(ChatError::Scope)
    ));
    let mut e = expected(group, &prepared);
    e.profile = OwnerFactId::from_bytes([9; 32]);
    assert!(matches!(
        assemble_action(&e, &chunks),
        Err(ChatError::Scope)
    ));
    let mut e = expected(group, &prepared);
    e.group = b"another exact group";
    assert!(matches!(
        assemble_action(&e, &chunks),
        Err(ChatError::Scope)
    ));
    let mut e = expected(group, &prepared);
    e.action = ActionId::from_bytes([10; 32]);
    assert!(matches!(
        assemble_action(&e, &chunks),
        Err(ChatError::Scope)
    ));
    let mut e = expected(group, &prepared);
    e.original = OriginalActionDigest::from_bytes([11; 32]);
    assert!(matches!(
        assemble_action(&e, &chunks),
        Err(ChatError::Scope)
    ));
}

fn raw_page(
    group: &[u8],
    action: ActionId,
    original: OriginalActionDigest,
    total: usize,
    index: usize,
    count: usize,
    payload: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(DOMAIN).unwrap();
    e.put_u16(1);
    e.put_bytes(&VAULT).unwrap();
    e.put_bytes(group).unwrap();
    e.put_bytes(profile().as_bytes()).unwrap();
    e.put_bytes(action.as_bytes()).unwrap();
    e.put_bytes(original.as_bytes()).unwrap();
    e.put_u32(total as u32)
        .put_u16(index as u16)
        .put_u16(count as u16);
    e.put_bytes(payload).unwrap();
    e.finish()
}

#[test]
fn private_action_envelope_digest_cannot_replace_actual_signature_or_semantic_identity_checks() {
    let action = action(false, 5);
    let group = action.spec().realm.group();
    let mut bad = action.encode();
    *bad.last_mut().unwrap() ^= 1;
    let decoded_bad = SignedAction::decode(&bad).unwrap();
    assert!(!decoded_bad.verify());
    assert!(matches!(
        PreparedActionPages::prepare(VAULT, profile(), &decoded_bad),
        Err(ChatError::Signature)
    ));
    let mut hash = original_hasher();
    hash.update(&bad);
    let original = OriginalActionDigest(*hash.finalize().as_bytes());
    let raw = raw_page(group, action.id(), original, bad.len(), 0, 1, &bad);
    let e = ActionPagesExpectation {
        vault: &VAULT,
        group,
        profile: profile(),
        action: action.id(),
        original,
    };
    assert!(matches!(
        assemble_action(&e, &[ActionChunk::decode(&raw).unwrap()]),
        Err(ChatError::Signature)
    ));
    let good = action.encode();
    let mut hash = original_hasher();
    hash.update(&good);
    let original = OriginalActionDigest(*hash.finalize().as_bytes());
    let wrong = ActionId::from_bytes([12; 32]);
    let raw = raw_page(group, wrong, original, good.len(), 0, 1, &good);
    let e = ActionPagesExpectation {
        vault: &VAULT,
        group,
        profile: profile(),
        action: wrong,
        original,
    };
    assert!(matches!(
        assemble_action(&e, &[ActionChunk::decode(&raw).unwrap()]),
        Err(ChatError::Scope)
    ));
}

#[test]
fn private_action_signature_variants_share_action_identity_but_never_original_representation() {
    fn unhex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks_exact(2)
            .map(|bytes| u8::from_str_radix(std::str::from_utf8(bytes).unwrap(), 16).unwrap())
            .collect()
    }
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("../tests/signature-variants.json")).unwrap();
    let actions: Vec<_> = vectors["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| SignedAction::decode(&unhex(frame.as_str().unwrap())).unwrap())
        .collect();
    assert_eq!(actions.len(), 2);
    assert!(actions.iter().all(SignedAction::verify));
    assert_eq!(actions[0].id(), actions[1].id());
    let first = PreparedActionPages::prepare(VAULT, profile(), &actions[0]).unwrap();
    let second = PreparedActionPages::prepare(VAULT, profile(), &actions[1]).unwrap();
    assert_ne!(first.original_digest(), second.original_digest());
    let second_pages = pages(&second);
    let second_chunks = parse(&second_pages);
    assert!(matches!(
        assemble_action(
            &expected(actions[0].spec().realm.group(), &first),
            &second_chunks
        ),
        Err(ChatError::Scope)
    ));
    let assembled = assemble_action(
        &expected(actions[1].spec().realm.group(), &second),
        &second_chunks,
    )
    .unwrap();
    assert_eq!(assembled.original_bytes(), actions[1].encode());
}

#[test]
fn private_action_chunk_complete_borrowed_framing_and_geometry_refuse_before_assembly() {
    let action = action(false, 6);
    let prepared = PreparedActionPages::prepare(VAULT, profile(), &action).unwrap();
    let bytes = prepared.encode_chunk(0).unwrap();
    for end in 0..bytes.len() {
        assert!(ActionChunk::decode(&bytes[..end]).is_err());
    }
    let mut extra = bytes.to_vec();
    extra.push(0);
    assert!(ActionChunk::decode(&extra).is_err());
    let oversized = vec![0; MAX_ACTION_CHUNK_PAGE_BYTES + 1];
    assert!(ActionChunk::decode(&oversized).is_err());
    let mut version = bytes.to_vec();
    version[4 + DOMAIN.len() + 1] = 2;
    assert!(ActionChunk::decode(&version).is_err());
    let id = action.id();
    let original = prepared.original_digest();
    let group = action.spec().realm.group();
    for raw in [
        raw_page(group, id, original, 0, 0, 0, &[]),
        raw_page(group, id, original, MAX_ACTION_BYTES + 1, 0, 3, &[]),
        raw_page(group, id, original, 1, 1, 1, &[0]),
        raw_page(group, id, original, 1, 0, 2, &[0]),
        raw_page(group, id, original, 2, 0, 1, &[0]),
        raw_page(group, id, original, 1, 0, 1, &[0, 0]),
        raw_page(&[], id, original, 1, 0, 1, &[0]),
    ] {
        assert!(ActionChunk::decode(&raw).is_err());
    }
}
