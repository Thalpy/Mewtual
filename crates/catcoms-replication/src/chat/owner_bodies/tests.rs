use super::*;
use crate::chat::acceptance_tree::{verify_inclusion, TreeHash};
use crate::chat::owner_header::{HeaderSpec, OwnerStateCommitment, OwnerTenureId};
use crate::chat::{ActionSpec, Body, Realm};
use catcoms_crypto::DeviceKeypair;

fn context() -> TreeContext {
    TreeContext {
        profile: OwnerFactId::from_bytes([1; 32]),
        admission: OwnerFactId::from_bytes([2; 32]),
    }
}
fn action(body: Body) -> SignedAction {
    let key = DeviceKeypair::from_seed(&[41; 32]);
    SignedAction::sign(
        &key,
        ActionSpec {
            realm: Realm::new(&[7; MAX_GROUP_ID_BYTES], 5).unwrap(),
            basis: BasisId::from_bytes([3; 32]),
            grant: GrantId::from_bytes([4; 32]),
            nonce: [5; 16],
            presented_ms: 100,
            parents: vec![],
            body,
        },
    )
    .unwrap()
}
fn record(action: &SignedAction) -> AcceptanceRecord {
    AcceptanceRecord::new(AcceptanceSpec {
        group: action.spec().realm.group().to_vec(),
        context: context(),
        channel: action.spec().realm.channel(),
        action: action.id(),
        subject: *action.signer(),
        grant: action.spec().grant,
        basis: action.spec().basis,
        create: action.spec().body.create(),
    })
    .unwrap()
}
fn maximum_record() -> AcceptanceRecord {
    record(&action(Body::Edit {
        create: ActionId::from_bytes([9; 32]),
        text: "edit".into(),
    }))
}
fn append(count: usize) -> (Frontier, AppendBody) {
    let frontier = Frontier::empty(context());
    let leaves: Vec<_> = (0..count)
        .map(|n| AcceptanceLeaf {
            channel: n as u128,
            acceptance: OwnerFactId::from_bytes([6; 32]),
        })
        .collect();
    let prepared = frontier.prepare_append(&leaves).unwrap();
    let body = AppendBody::new(AppendSpec {
        group: vec![7; MAX_GROUP_ID_BYTES],
        previous_tip: OwnerHeaderId::from_bytes([8; 32]),
        old_tip: frontier.tip(),
        new_tip: prepared.new_tip(),
        leaves,
    })
    .unwrap();
    (frontier, body)
}
fn removal() -> RemovalBody {
    RemovalBody::new(RemovalSpec {
        group: vec![7; MAX_GROUP_ID_BYTES],
        subject: [9; 32],
        commit: [10; 32],
        before_epoch: 6,
        after_epoch: 7,
        tip_header: OwnerHeaderId::from_bytes([11; 32]),
        frozen_tip: append(1).1.spec().new_tip,
    })
    .unwrap()
}
fn header(kind: HeaderKind, body: OwnerBodyId) -> SignedOwnerHeader {
    let key = DeviceKeypair::from_seed(&[42; 32]);
    SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: vec![7; MAX_GROUP_ID_BYTES],
            profile: context().profile,
            issuer: key.verifying_key().to_bytes(),
            tenure: OwnerTenureId::from_bytes([12; 32]),
            parent: OwnerHeaderId::from_bytes([13; 32]),
            sequence: 1,
            kind,
            body,
            state: OwnerStateCommitment::from_bytes([14; 32]),
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap()
}
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

#[test]
fn exact_caps_round_trip_without_history_dependent_removal_growth() {
    let record = maximum_record();
    let (_, append) = append(64);
    let removal = removal();
    assert_eq!(MAX_ACCEPTANCE_RECORD_BYTES, 567);
    assert_eq!(MAX_APPEND_BODY_BYTES, 3572);
    assert_eq!(MAX_REMOVAL_BODY_BYTES, 528);
    assert_eq!(record.encode().len(), 567);
    assert_eq!(append.encode().len(), 3572);
    assert_eq!(removal.encode().len(), 528);
    assert_eq!(AcceptanceRecord::decode(&record.encode()).unwrap(), record);
    assert_eq!(AppendBody::decode(&append.encode()).unwrap(), append);
    assert_eq!(RemovalBody::decode(&removal.encode()).unwrap(), removal);
    let mut many = removal.spec().clone();
    // Claimed count only, not a million real accepted actions or a verified frozen cut.
    many.frozen_tip.count = 1_000_000;
    let many = RemovalBody::new(many).unwrap();
    assert_eq!(many.encode().len(), removal.encode().len());
    assert_ne!(many.id(), removal.id());
}

#[test]
fn record_bytes_and_ids_use_exact_domains_and_fixed_big_endian_fields() {
    let record = maximum_record();
    let spec = record.spec();
    let mut expected = Vec::new();
    field(&mut expected, b"mewtual/chat/acceptance-record/v2\0");
    field(&mut expected, &spec.group);
    field(&mut expected, spec.context.profile.as_bytes());
    field(&mut expected, spec.context.admission.as_bytes());
    expected.extend_from_slice(&spec.channel.to_be_bytes());
    field(&mut expected, spec.action.as_bytes());
    field(&mut expected, &spec.subject);
    field(&mut expected, spec.grant.as_bytes());
    field(&mut expected, spec.basis.as_bytes());
    expected.push(1);
    field(&mut expected, spec.create.unwrap().as_bytes());
    assert_eq!(record.encode(), expected);
    let mut id_input = b"mewtual/chat/acceptance-record-id/v2\0".to_vec();
    id_input.extend_from_slice(&expected);
    assert_eq!(record.id().as_bytes(), blake3::hash(&id_input).as_bytes());
    let (_, append) = append(1);
    let mut body_input = b"mewtual/chat/owner-body-id/v2\0".to_vec();
    body_input.extend_from_slice(&append.encode());
    assert_eq!(append.id().as_bytes(), blake3::hash(&body_input).as_bytes());
}

#[test]
fn all_action_kinds_bind_exact_real_signature_and_action_fields() {
    for body in [
        Body::Create {
            text: "post".into(),
            reply_to: None,
        },
        Body::Edit {
            create: ActionId::from_bytes([9; 32]),
            text: "edit".into(),
        },
        Body::Delete {
            create: ActionId::from_bytes([9; 32]),
        },
        Body::Join {
            create: ActionId::from_bytes([9; 32]),
        },
    ] {
        let action = action(body);
        let original = record(&action);
        assert_eq!(
            AcceptanceRecord::decode(&original.encode()).unwrap(),
            original
        );
        assert_eq!(original.matches_action(&action), Ok(()));
        for changed_field in 0..7 {
            let mut changed = original.spec().clone();
            match changed_field {
                0 => changed.group[0] ^= 1,
                1 => changed.channel += 1,
                2 => changed.action = ActionId::from_bytes([51; 32]),
                3 => changed.subject = [52; 32],
                4 => changed.grant = GrantId::from_bytes([53; 32]),
                5 => changed.basis = BasisId::from_bytes([54; 32]),
                _ => changed.create = Some(ActionId::from_bytes([55; 32])),
            }
            let changed = AcceptanceRecord::new(changed).unwrap();
            assert_eq!(changed.matches_action(&action), Err(ChatError::Scope));
            assert_ne!(changed.id(), original.id());
        }
        let mut encoded = action.encode();
        *encoded.last_mut().unwrap() ^= 1;
        let invalid = SignedAction::decode(&encoded).unwrap();
        assert_eq!(original.matches_action(&invalid), Err(ChatError::Signature));
    }
}

#[test]
fn a1_a2_separate_record_and_tree_identity_but_action_matching_is_not_incarnation_proof() {
    let action = action(Body::Create {
        text: "same full signer".into(),
        reply_to: None,
    });
    let a1 = record(&action);
    let mut second = a1.spec().clone();
    second.context.admission = OwnerFactId::from_bytes([22; 32]);
    let a2 = AcceptanceRecord::new(second).unwrap();
    assert_ne!(a1.id(), a2.id());
    assert_ne!(a1.leaf(), a2.leaf());
    assert_ne!(
        a1.spec().context.leaf_hash(a1.leaf()),
        a2.spec().context.leaf_hash(a2.leaf())
    );
    assert_eq!(a1.matches_action(&action), Ok(()));
    assert_eq!(a2.matches_action(&action), Ok(()), "unchanged action/v1 does not directly prove AdmissionID; delegated Grant/Basis verification is still required");
    let tip = Frontier::empty(a1.spec().context)
        .prepare_append(&[a1.leaf()])
        .unwrap()
        .new_tip();
    assert_eq!(verify_inclusion(tip, a1.leaf(), 0, &[]), Ok(()));
    assert!(verify_inclusion(tip, a2.leaf(), 0, &[]).is_err());
}

#[test]
fn append_preparation_matches_actual_frontier_without_mutating_it_or_authenticating_context() {
    let (frontier, body) = append(64);
    let original = frontier.clone();
    let prepared = body.prepare_on_frontier(&frontier).unwrap();
    assert_eq!(prepared.old_tip(), body.spec().old_tip);
    assert_eq!(prepared.new_tip(), body.spec().new_tip);
    assert_eq!(frontier, original);
    let mut wrong = body.spec().clone();
    wrong.new_tip.root = TreeHash::from_bytes([66; 32]);
    assert_eq!(
        AppendBody::new(wrong)
            .unwrap()
            .prepare_on_frontier(&frontier)
            .err(),
        Some(ChatError::Dependency)
    );
    let foreign = Frontier::empty(TreeContext {
        profile: context().profile,
        admission: OwnerFactId::from_bytes([67; 32]),
    });
    assert_eq!(
        body.prepare_on_frontier(&foreign).err(),
        Some(ChatError::Scope)
    );
    let mut mixed = body.spec().clone();
    mixed.new_tip.context = foreign.tip().context;
    assert_eq!(AppendBody::new(mixed), Err(ChatError::Scope));
}

#[test]
fn actual_signed_header_record_and_tree_links_do_not_create_an_owner_witness() {
    let action = action(Body::Create {
        text: "bound record".into(),
        reply_to: None,
    });
    let record = record(&action);
    let frontier = Frontier::empty(record.spec().context);
    let prepared = frontier.prepare_append(&[record.leaf()]).unwrap();
    let append = AppendBody::new(AppendSpec {
        group: record.spec().group.clone(),
        previous_tip: OwnerHeaderId::from_bytes([8; 32]),
        old_tip: frontier.tip(),
        new_tip: prepared.new_tip(),
        leaves: vec![record.leaf()],
    })
    .unwrap();
    let header = header(HeaderKind::AcceptanceAppend, append.id());
    assert_eq!(append.verify_header_binding(&header), Ok(()));
    assert_eq!(record.matches_action(&action), Ok(()));
    append.prepare_on_frontier(&frontier).unwrap();
    assert_eq!(
        verify_inclusion(append.spec().new_tip, record.leaf(), 0, &[]),
        Ok(())
    );
    // These are real cryptographic links under a test key, not mounted root/tenure provenance,
    // delegated ordinary rights, a governing-chain selection or a durable Saved result.
}

#[test]
fn append_overflow_bad_count_order_duplicates_and_empty_tip_refuse() {
    let (_, body) = append(64);
    for bad in [0usize, 65] {
        let mut spec = body.spec().clone();
        spec.leaves.resize(bad, body.spec().leaves[0]);
        assert_eq!(AppendBody::new(spec), Err(ChatError::Bound));
    }
    let mut repeated = body.spec().clone();
    repeated.leaves[1] = repeated.leaves[0];
    assert_eq!(AppendBody::new(repeated), Err(ChatError::Bound));
    let mut unordered = body.spec().clone();
    unordered.leaves.swap(0, 1);
    assert_eq!(AppendBody::new(unordered), Err(ChatError::Bound));
    let mut overflow = body.spec().clone();
    overflow.old_tip.count = u64::MAX;
    overflow.new_tip.count = 63;
    assert_eq!(AppendBody::new(overflow), Err(ChatError::Dependency));
    let mut wrong = body.spec().clone();
    wrong.new_tip.count += 1;
    assert_eq!(AppendBody::new(wrong), Err(ChatError::Dependency));
    let mut wrong = body.spec().clone();
    wrong.old_tip.root = TreeHash::from_bytes([77; 32]);
    assert_eq!(AppendBody::new(wrong), Err(ChatError::Dependency));
}

#[test]
fn removal_epoch_wrap_skip_and_noncanonical_empty_tip_refuse() {
    let body = removal();
    for (before, after) in [(u64::MAX, 0), (6, 6), (6, 8)] {
        let mut spec = body.spec().clone();
        spec.before_epoch = before;
        spec.after_epoch = after;
        assert_eq!(RemovalBody::new(spec.clone()), Err(ChatError::Dependency));
        let raw = RemovalBody { spec }.encode();
        assert_eq!(RemovalBody::decode(&raw), Err(ChatError::Dependency));
    }
    let mut empty = body.spec().clone();
    empty.frozen_tip = Frontier::empty(context()).tip();
    RemovalBody::new(empty.clone()).unwrap();
    empty.frozen_tip.root = TreeHash::from_bytes([78; 32]);
    assert_eq!(RemovalBody::new(empty), Err(ChatError::Dependency));
}

#[test]
fn signed_header_binding_checks_exact_fields_and_signature_but_not_owner_provenance() {
    let (_, body) = append(1);
    let valid = header(HeaderKind::AcceptanceAppend, body.id());
    assert_eq!(body.verify_header_binding(&valid), Ok(()));
    let key = DeviceKeypair::from_seed(&[42; 32]);
    for field in 0..4 {
        let mut changed = valid.spec().clone();
        match field {
            0 => changed.kind = HeaderKind::Removal,
            1 => changed.group[0] ^= 1,
            2 => changed.profile = OwnerFactId::from_bytes([80; 32]),
            _ => changed.body = OwnerBodyId::from_bytes([81; 32]),
        }
        let changed = SignedOwnerHeader::sign_using(changed, |bytes| Ok(key.sign(bytes))).unwrap();
        assert_eq!(body.verify_header_binding(&changed), Err(ChatError::Scope));
    }
    let mut bad = valid.encode();
    *bad.last_mut().unwrap() ^= 1;
    assert_eq!(
        body.verify_header_binding(&SignedOwnerHeader::decode(&bad).unwrap()),
        Err(ChatError::Signature)
    );
    let other = DeviceKeypair::from_seed(&[43; 32]);
    let mut unpinned = valid.spec().clone();
    unpinned.issuer = other.verifying_key().to_bytes();
    unpinned.state = OwnerStateCommitment::from_bytes([82; 32]);
    let unpinned = SignedOwnerHeader::sign_using(unpinned, |bytes| Ok(other.sign(bytes))).unwrap();
    assert_eq!(
        body.verify_header_binding(&unpinned),
        Ok(()),
        "owner/tenure/state provenance must be checked by the governing consumer"
    );
    let removal = removal();
    let removal_header = header(HeaderKind::Removal, removal.id());
    assert_eq!(removal.verify_header_binding(&removal_header), Ok(()));
    assert_eq!(removal.verify_header_binding(&valid), Err(ChatError::Scope));
}

#[test]
fn every_truncated_body_and_unknown_options_counts_packed_length_order_refuse() {
    let record = maximum_record().encode();
    let (_, body) = append(64);
    let append = body.encode();
    let removal = removal().encode();
    for end in 0..record.len() {
        assert!(AcceptanceRecord::decode(&record[..end]).is_err());
    }
    for end in 0..append.len() {
        assert!(AppendBody::decode(&append[..end]).is_err());
    }
    for end in 0..removal.len() {
        assert!(RemovalBody::decode(&removal[..end]).is_err());
    }
    let option = record.len() - 37;
    let mut bad = record.clone();
    bad[option] = 2;
    assert_eq!(AcceptanceRecord::decode(&bad), Err(ChatError::Malformed));
    let count_at = append.len() - 4 - 64 * PACKED_LEAF_BYTES - 2;
    for count in [0u16, 65, u16::MAX] {
        let mut bad = append.clone();
        bad[count_at..count_at + 2].copy_from_slice(&count.to_be_bytes());
        assert_eq!(AppendBody::decode(&bad), Err(ChatError::Bound));
    }
    for length in [0u32, 48, u32::MAX] {
        let mut bad = append.clone();
        bad[count_at + 2..count_at + 6].copy_from_slice(&length.to_be_bytes());
        assert!(AppendBody::decode(&bad).is_err());
    }
    let packed = count_at + 6;
    let mut repeated = append.clone();
    repeated.copy_within(
        packed..packed + PACKED_LEAF_BYTES,
        packed + PACKED_LEAF_BYTES,
    );
    assert_eq!(AppendBody::decode(&repeated), Err(ChatError::Bound));
    let mut swapped = append.clone();
    for i in 0..PACKED_LEAF_BYTES {
        swapped.swap(packed + i, packed + PACKED_LEAF_BYTES + i);
    }
    assert_eq!(AppendBody::decode(&swapped), Err(ChatError::Bound));
    for mut frame in [record, append, removal] {
        frame.push(0);
        assert!(AcceptanceRecord::decode(&frame).is_err());
        assert!(AppendBody::decode(&frame).is_err());
        assert!(RemovalBody::decode(&frame).is_err());
    }
}

#[test]
fn v1_signed_owner_facts_and_v2_body_kinds_never_reinterpret_each_other() {
    use crate::chat::owner_facts::{OwnerFactBody, OwnerFactSpec, SignedOwnerFact};
    let key = DeviceKeypair::from_seed(&[42; 32]);
    let legacy = SignedOwnerFact::sign_using(
        OwnerFactSpec {
            group: vec![7],
            profile: None,
            body: OwnerFactBody::Profile {
                creation: [1; 32],
                founder: key.verifying_key().to_bytes(),
            },
        },
        key.verifying_key().to_bytes(),
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap()
    .encode();
    assert!(AcceptanceRecord::decode(&legacy).is_err());
    assert!(AppendBody::decode(&legacy).is_err());
    assert!(RemovalBody::decode(&legacy).is_err());
    for bytes in [
        maximum_record().encode(),
        append(1).1.encode(),
        removal().encode(),
    ] {
        assert!(SignedOwnerFact::decode(&bytes).is_err());
    }
    assert!(RemovalBody::decode(&append(1).1.encode()).is_err());
    assert!(AppendBody::decode(&removal().encode()).is_err());
}

#[test]
fn fixed_digest_lengths_empty_or_oversized_groups_and_trailing_create_refuse() {
    let record = maximum_record();
    let (_, append) = append(1);
    let removal = removal();
    let record_base = 4 + RECORD_DOMAIN.len() + 4 + MAX_GROUP_ID_BYTES;
    for offset in [0, 36, 88, 124, 160, 196, 233] {
        for length in [0u32, 31, 33, u32::MAX] {
            let mut bytes = record.encode();
            bytes[record_base + offset..record_base + offset + 4]
                .copy_from_slice(&length.to_be_bytes());
            assert!(AcceptanceRecord::decode(&bytes).is_err());
        }
    }
    let append_base = 4 + APPEND_DOMAIN.len() + 4 + MAX_GROUP_ID_BYTES;
    for offset in [0, 36, 72, 116, 160] {
        for length in [0u32, 31, 33, u32::MAX] {
            let mut bytes = append.encode();
            bytes[append_base + offset..append_base + offset + 4]
                .copy_from_slice(&length.to_be_bytes());
            assert!(AppendBody::decode(&bytes).is_err());
        }
    }
    let removal_base = 4 + REMOVAL_DOMAIN.len() + 4 + MAX_GROUP_ID_BYTES;
    for offset in [0, 36, 72, 108, 160, 204] {
        for length in [0u32, 31, 33, u32::MAX] {
            let mut bytes = removal.encode();
            bytes[removal_base + offset..removal_base + offset + 4]
                .copy_from_slice(&length.to_be_bytes());
            assert!(RemovalBody::decode(&bytes).is_err());
        }
    }
    for group in [Vec::new(), vec![7; MAX_GROUP_ID_BYTES + 1]] {
        let mut record_spec = record.spec().clone();
        record_spec.group = group.clone();
        assert!(AcceptanceRecord::new(record_spec.clone()).is_err());
        assert!(
            AcceptanceRecord::decode(&AcceptanceRecord { spec: record_spec }.encode()).is_err()
        );
        let mut append_spec = append.spec().clone();
        append_spec.group = group.clone();
        assert!(AppendBody::new(append_spec.clone()).is_err());
        assert!(AppendBody::decode(&AppendBody { spec: append_spec }.encode()).is_err());
        let mut removal_spec = removal.spec().clone();
        removal_spec.group = group;
        assert!(RemovalBody::new(removal_spec.clone()).is_err());
        assert!(RemovalBody::decode(&RemovalBody { spec: removal_spec }.encode()).is_err());
    }
    let mut unexpected_create = record.encode();
    unexpected_create[record_base + 232] = 0;
    assert!(AcceptanceRecord::decode(&unexpected_create).is_err());
    for mut bytes in [record.encode(), append.encode(), removal.encode()] {
        bytes[4] ^= 1;
        assert!(AcceptanceRecord::decode(&bytes).is_err());
        assert!(AppendBody::decode(&bytes).is_err());
        assert!(RemovalBody::decode(&bytes).is_err());
    }
}

#[test]
fn genuine_action_signature_variants_share_exact_record_and_tree_identity() {
    fn unhex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../tests/signature-variants.json")).unwrap();
    let actions: Vec<_> = fixture["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| SignedAction::decode(&unhex(value.as_str().unwrap())).unwrap())
        .collect();
    assert_eq!(actions.len(), 2);
    assert!(actions.iter().all(SignedAction::verify));
    assert_ne!(actions[0].signature(), actions[1].signature());
    let first = record(&actions[0]);
    let second = record(&actions[1]);
    assert_eq!(first.matches_action(&actions[0]), Ok(()));
    assert_eq!(first.matches_action(&actions[1]), Ok(()));
    assert_eq!(first, second);
    assert_eq!(first.id(), second.id());
    assert_eq!(first.leaf(), second.leaf());
}
