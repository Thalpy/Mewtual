//! Public image/hash consistency, NOT actual MLS transitions, trusted root acquisition or a
//! governing/current-effect issuer. Real signatures exercise the exact binding APIs only.
use super::super::owner_header::HeaderSpec;
use super::super::owner_profile::{AdmissionOrigin, AdmissionSpec, ProfileSpec};
use super::*;
use catcoms_crypto::DeviceKeypair;

const GROUP: &[u8] = b"public-state-real-signature-fixture";

#[test]
fn metadata_debug_omits_identity_and_history_fields() {
    assert_eq!(format!("{:?}", meta(GROUP)), "StateMeta { .. }");
}
fn empty_slot(profile: OwnerFactId, id: u8, subject: [u8; 32]) -> CurrentAdmissionSlot {
    let admission = OwnerBodyId::from_bytes([id; 32]);
    CurrentAdmissionSlot {
        admission,
        subject,
        tip_body: admission,
        accepted_count: 0,
        accepted_root: TreeContext {
            profile,
            admission: OwnerFactId::from_bytes(*admission.as_bytes()),
        }
        .empty_hash(),
    }
}
fn meta<'a>(group: &'a [u8]) -> StateMeta<'a> {
    StateMeta {
        group,
        profile: OwnerFactId::from_bytes([1; 32]),
        issuer: [5; 32],
        tenure: OwnerTenureId::from_bytes([6; 32]),
        parent: OwnerHeaderId::from_bytes([7; 32]),
        previous: OwnerStateCommitment::from_bytes([8; 32]),
        sequence: 2,
        kind: HeaderKind::AcceptanceAppend,
        body: OwnerBodyId::from_bytes([9; 32]),
        mls_epoch: 1,
    }
}
fn signed(key: &DeviceKeypair, bytes: &[u8]) -> SignedOwnerHeader {
    let frame = StateFrame::decode(bytes).unwrap();
    let m = frame.meta();
    SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: m.group.to_vec(),
            profile: m.profile,
            issuer: m.issuer,
            tenure: m.tenure,
            parent: m.parent,
            sequence: m.sequence,
            kind: m.kind,
            body: m.body,
            state: frame.commitment(),
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap()
}
fn founder() -> (
    DeviceKeypair,
    SignedOwnerProfile,
    AdmissionBody,
    Vec<u8>,
    SignedOwnerHeader,
) {
    let key = DeviceKeypair::from_seed(&[2; 32]);
    let profile = SignedOwnerProfile::sign_using(
        ProfileSpec {
            group: GROUP.to_vec(),
            founder: key.verifying_key().to_bytes(),
            creation: [3; 32],
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap();
    let body = AdmissionBody::new(AdmissionSpec {
        group: GROUP.to_vec(),
        profile: profile.id(),
        subject: profile.spec().founder,
        origin: AdmissionOrigin::Founder {
            creation: profile.spec().creation,
        },
    })
    .unwrap();
    let slot = CurrentAdmissionSlot {
        admission: body.id(),
        subject: profile.spec().founder,
        tip_body: body.id(),
        accepted_count: 0,
        accepted_root: TreeContext {
            profile: profile.id(),
            admission: body.admission_id(),
        }
        .empty_hash(),
    };
    let fields = StateMeta {
        group: GROUP,
        profile: profile.id(),
        issuer: profile.spec().founder,
        tenure: profile.tenure(),
        parent: profile.genesis_parent(),
        previous: virtual_genesis_state(&profile).unwrap(),
        sequence: 1,
        kind: HeaderKind::Admission,
        body: body.id(),
        mls_epoch: 0,
    };
    let bytes = encode_state(fields, &[slot]).unwrap();
    let header = signed(&key, &bytes);
    (key, profile, body, bytes, header)
}

#[test]
fn owner_public_state_max_current_image_is_bounded_borrowed_and_canonically_packed() {
    let group = [99; MAX_GROUP_ID_BYTES];
    let mut m = meta(&group);
    m.issuer = [128; 32];
    let slots: Vec<_> = (0..MAX_CURRENT_MEMBERS)
        .map(|index| empty_slot(m.profile, index as u8, [128 + index as u8; 32]))
        .collect();
    let bytes = encode_state(m, &slots).unwrap();
    assert_eq!(bytes.len(), MAX_STATE_BYTES);
    let frame = StateFrame::decode(&bytes).unwrap();
    assert_eq!(frame.slots().collect::<Vec<_>>(), slots);
    assert_eq!(frame.raw.as_ptr(), bytes.as_ptr());
    assert_eq!(
        frame.meta().group.as_ptr(),
        bytes[4 + DOMAIN.len() + 2 + 4..].as_ptr()
    );
    assert_eq!(
        frame.packed.as_ptr(),
        bytes[bytes.len() - slots.len() * SLOT_BYTES..].as_ptr()
    );
    assert_eq!(
        encode_state(frame.meta(), &frame.slots().collect::<Vec<_>>()).unwrap(),
        bytes
    );
}

#[test]
fn owner_public_state_binds_real_profile_founder_empty_tree_and_acyclic_first_header() {
    let (_, profile, body, bytes, header) = founder();
    let frame = StateFrame::decode(&bytes).unwrap();
    frame
        .verify_genesis_binding(&profile, &body, &header)
        .unwrap();
    assert_eq!(frame.commitment(), header.spec().state);
    assert_eq!(frame.slots().next().unwrap().tip_body, body.id());
    assert_eq!(
        frame.meta().previous,
        virtual_genesis_state(&profile).unwrap()
    );
    let mut bad = profile.encode();
    *bad.last_mut().unwrap() ^= 1;
    let bad = SignedOwnerProfile::decode(&bad).unwrap();
    assert_eq!(
        bad.id(),
        profile.id(),
        "semantic identity never caches validity of a new signature representation"
    );
    assert_eq!(virtual_genesis_state(&bad), Err(ChatError::Signature));
}

#[test]
fn owner_public_state_checks_signed_parent_and_previous_root_not_only_self_signature() {
    let (key, _, _, genesis, parent) = founder();
    let old = StateFrame::decode(&genesis).unwrap();
    let slots: Vec<_> = old.slots().collect();
    let mut next = old.meta();
    next.sequence = 2;
    next.parent = parent.id();
    next.previous = parent.spec().state;
    next.kind = HeaderKind::ReviewDecision;
    next.body = OwnerBodyId::from_bytes([19; 32]);
    let bytes = encode_state(next, &slots).unwrap();
    let header = signed(&key, &bytes);
    let frame = StateFrame::decode(&bytes).unwrap();
    frame.verify_successor_binding(&header, &parent).unwrap();
    // This DATA binding does not authenticate a case or apply that Review. The actual future
    // governor must independently prove current case/transition/complete selected prefix.
    let mut wrong = next;
    wrong.previous = OwnerStateCommitment::from_bytes([20; 32]);
    let bytes = encode_state(wrong, &slots).unwrap();
    let header = signed(&key, &bytes);
    assert!(StateFrame::decode(&bytes)
        .unwrap()
        .verify_header_binding(&header)
        .is_ok());
    assert_eq!(
        StateFrame::decode(&bytes)
            .unwrap()
            .verify_successor_binding(&header, &parent),
        Err(ChatError::Dependency)
    );
    let mut wrong = next;
    wrong.parent = OwnerHeaderId::from_bytes([21; 32]);
    let bytes = encode_state(wrong, &slots).unwrap();
    let header = signed(&key, &bytes);
    assert!(StateFrame::decode(&bytes)
        .unwrap()
        .verify_successor_binding(&header, &parent)
        .is_err());
}

#[test]
fn owner_public_state_never_erases_history_by_returning_to_the_same_current_slots() {
    let m = meta(GROUP);
    let slots = [empty_slot(m.profile, 10, m.issuer)];
    let first = encode_state(m, &slots).unwrap();
    let first_root = StateFrame::decode(&first).unwrap().commitment();
    let mut after = m;
    after.previous = first_root;
    after.sequence += 1;
    after.body = OwnerBodyId::from_bytes([22; 32]);
    let bytes = encode_state(after, &slots).unwrap();
    let image = StateFrame::decode(&bytes).unwrap();
    assert_eq!(image.slots().collect::<Vec<_>>(), slots);
    assert_ne!(image.commitment(), first_root);
    // Not a historical membership/acceptance proof: the complete selected prior chain still
    // needs its exact retained source, even when two images have identical current slots.
}

#[test]
fn owner_public_state_refuses_order_duplicate_subject_removed_issuer_and_invalid_empty_tip() {
    let m = meta(GROUP);
    let owner = empty_slot(m.profile, 10, m.issuer);
    let member = empty_slot(m.profile, 11, [12; 32]);
    assert!(encode_state(m, &[]).is_err());
    assert!(encode_state(m, &[member, owner]).is_err());
    assert!(encode_state(m, &[owner, owner]).is_err());
    let duplicate_subject = CurrentAdmissionSlot {
        subject: owner.subject,
        ..member
    };
    assert_eq!(
        encode_state(m, &[owner, duplicate_subject]),
        Err(ChatError::AuthorityConflict)
    );
    assert_eq!(encode_state(m, &[member]), Err(ChatError::Authority));
    let mut wrong = owner;
    wrong.tip_body = member.admission;
    assert!(encode_state(m, &[wrong]).is_err());
    let mut wrong = owner;
    wrong.accepted_root = TreeHash::from_bytes([23; 32]);
    assert!(encode_state(m, &[wrong]).is_err());
    let mut wrong = owner;
    wrong.accepted_count = 1;
    assert!(encode_state(m, &[wrong]).is_err());
    let mut zero = m;
    zero.sequence = 0;
    assert!(encode_state(zero, &[owner]).is_err());
}

#[test]
fn owner_public_state_decode_and_header_binding_reject_truncation_counts_extra_and_substitution() {
    let (_, _, _, bytes, header) = founder();
    for end in 0..bytes.len() {
        assert!(StateFrame::decode(&bytes[..end]).is_err());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(StateFrame::decode(&extra).is_err());
    let oversized = vec![0; MAX_STATE_BYTES + 1];
    assert!(StateFrame::decode(&oversized).is_err());
    let original = StateFrame::decode(&bytes).unwrap();
    let slots: Vec<_> = original.slots().collect();
    let mut m = original.meta();
    m.mls_epoch += 1;
    let changed = encode_state(m, &slots).unwrap();
    assert_eq!(
        StateFrame::decode(&changed)
            .unwrap()
            .verify_header_binding(&header),
        Err(ChatError::Dependency)
    );
    let mut wrong_header = header.encode();
    *wrong_header.last_mut().unwrap() ^= 1;
    let wrong_header = SignedOwnerHeader::decode(&wrong_header).unwrap();
    assert_eq!(
        original.verify_header_binding(&wrong_header),
        Err(ChatError::Signature)
    );
    let packed_length = bytes.len() - slots.len() * SLOT_BYTES - 4;
    let count_offset = packed_length - 2;
    for count in [0u16, 2, MAX_CURRENT_MEMBERS + 1] {
        let mut bad = bytes.clone();
        bad[count_offset..count_offset + 2].copy_from_slice(&count.to_be_bytes());
        assert!(StateFrame::decode(&bad).is_err());
    }
    let mut bad = bytes.clone();
    bad[4 + DOMAIN.len() + 1] = 3;
    assert!(StateFrame::decode(&bad).is_err());
}
