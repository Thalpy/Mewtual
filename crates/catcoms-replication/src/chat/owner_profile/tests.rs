//! Codec and exact binding tests, not actual MLS/fresh-pin/current-governance evidence.
use super::*;
use crate::chat::owner_header::{HeaderSpec, OwnerBodyId, OwnerStateCommitment};
use crate::chat::{ActionSpec, Body, Realm, Rights, SignedAction};
use catcoms_crypto::DeviceKeypair;
use std::cell::Cell;
use std::collections::BTreeSet;

fn key(seed: u8) -> DeviceKeypair {
    DeviceKeypair::from_seed(&[seed; 32])
}
fn profile() -> SignedOwnerProfile {
    SignedOwnerProfile::sign_using(
        ProfileSpec {
            group: vec![7; MAX_GROUP_ID_BYTES],
            founder: key(42).verifying_key().to_bytes(),
            creation: [1; 32],
        },
        |bytes| Ok(key(42).sign(bytes)),
    )
    .unwrap()
}
fn founder(profile: &SignedOwnerProfile) -> AdmissionBody {
    AdmissionBody::new(AdmissionSpec {
        group: profile.spec().group.clone(),
        profile: profile.id(),
        subject: profile.spec().founder,
        origin: AdmissionOrigin::Founder {
            creation: profile.spec().creation,
        },
    })
    .unwrap()
}
fn add(profile: &SignedOwnerProfile, marker: u8) -> AdmissionBody {
    AdmissionBody::new(AdmissionSpec {
        group: profile.spec().group.clone(),
        profile: profile.id(),
        subject: key(41).verifying_key().to_bytes(),
        origin: AdmissionOrigin::Add {
            commit: [2; 32],
            before_epoch: 4,
            after_epoch: 5,
            key_package: [marker; 32],
            credential: [3; 32],
        },
    })
    .unwrap()
}
fn header(
    profile: &SignedOwnerProfile,
    body: &AdmissionBody,
    sequence: u64,
    parent: OwnerHeaderId,
) -> SignedOwnerHeader {
    SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: body.spec().group.clone(),
            profile: body.spec().profile,
            issuer: key(42).verifying_key().to_bytes(),
            tenure: profile.tenure(),
            parent,
            sequence,
            kind: HeaderKind::Admission,
            body: body.id(),
            state: OwnerStateCommitment::from_bytes([8; 32]),
        },
        |bytes| Ok(key(42).sign(bytes)),
    )
    .unwrap()
}
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn wrap(semantic: &[u8], signature: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, b"mewtual/chat/owner-profile-envelope/v2\0");
    field(&mut out, semantic);
    field(&mut out, signature);
    out
}
fn unhex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn raw_admission(spec: &AdmissionSpec) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, b"mewtual/chat/admission/v2\0");
    field(&mut out, &spec.group);
    field(&mut out, spec.profile.as_bytes());
    field(&mut out, &spec.subject);
    match spec.origin {
        AdmissionOrigin::Founder { creation } => {
            out.push(0);
            field(&mut out, &creation);
        }
        AdmissionOrigin::Add {
            commit,
            before_epoch,
            after_epoch,
            key_package,
            credential,
        } => {
            out.push(1);
            field(&mut out, &commit);
            out.extend_from_slice(&before_epoch.to_be_bytes());
            out.extend_from_slice(&after_epoch.to_be_bytes());
            field(&mut out, &key_package);
            field(&mut out, &credential);
        }
    }
    out.extend_from_slice(&23u16.to_be_bytes());
    out.push(1);
    out
}

#[test]
fn exact_profile_and_admission_caps_round_trip_actual_signatures() {
    let profile = profile();
    assert_eq!(MAX_PROFILE_SEMANTIC_BYTES, 379);
    assert_eq!(MAX_PROFILE_BYTES, 494);
    assert_eq!(profile.semantic_bytes().len(), 379);
    assert_eq!(profile.encode().len(), 494);
    assert!(profile.verify());
    assert_eq!(
        SignedOwnerProfile::decode(&profile.encode()).unwrap(),
        profile
    );
    assert_eq!(MAX_CURRENT_MEMBERS, 64);
    for body in [founder(&profile), add(&profile, 4)] {
        let encoded = body.encode();
        assert!(encoded.len() <= MAX_ADMISSION_BYTES);
        assert_eq!(encoded, raw_admission(body.spec()));
        assert_eq!(AdmissionBody::decode(&encoded).unwrap(), body);
    }
    assert_eq!(MAX_ADMISSION_BYTES, 490);
    assert_eq!(add(&profile, 4).encode().len(), 490);
}

#[test]
fn profile_byte_policy_tenure_and_genesis_domains_match_independent_frames() {
    let profile = profile();
    let mut expected = Vec::new();
    field(&mut expected, b"mewtual/chat/owner-profile/v2\0");
    expected.extend_from_slice(&2u16.to_be_bytes());
    field(&mut expected, &profile.spec().group);
    field(&mut expected, &profile.spec().founder);
    field(&mut expected, &profile.spec().creation);
    expected.extend_from_slice(&[1, 0, 0, 23, 1, 1, 2, 2, 2, 0, 64]);
    assert_eq!(profile.semantic_bytes(), expected);
    let mut id_input = b"mewtual/chat/owner-profile-id/v2\0".to_vec();
    id_input.extend_from_slice(&expected);
    assert_eq!(profile.id().as_bytes(), blake3::hash(&id_input).as_bytes());
    let mut tenure = b"mewtual/chat/owner-tenure/v2\0".to_vec();
    tenure.extend_from_slice(profile.id().as_bytes());
    tenure.extend_from_slice(&profile.spec().founder);
    tenure.extend_from_slice(&profile.spec().creation);
    assert_eq!(
        profile.tenure().as_bytes(),
        blake3::hash(&tenure).as_bytes()
    );
    let mut genesis = b"mewtual/chat/owner-genesis/v2\0".to_vec();
    genesis.extend_from_slice(profile.id().as_bytes());
    genesis.extend_from_slice(profile.tenure().as_bytes());
    genesis.extend_from_slice(&profile.spec().founder);
    genesis.extend_from_slice(&profile.spec().creation);
    assert_eq!(
        profile.genesis_parent().as_bytes(),
        blake3::hash(&genesis).as_bytes()
    );
}

#[test]
fn genuine_profile_signature_variants_share_root_label_but_invalid_first_is_not_valid() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("signature-variants.json")).unwrap();
    let profiles: Vec<_> = fixture["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| SignedOwnerProfile::decode(&unhex(frame.as_str().unwrap())).unwrap())
        .collect();
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0], profile());
    assert!(profiles.iter().all(SignedOwnerProfile::verify));
    assert_ne!(profiles[0].signature(), profiles[1].signature());
    assert_eq!(profiles[0].id(), profiles[1].id());
    assert_eq!(profiles[0].tenure(), profiles[1].tenure());
    assert_eq!(profiles[0].genesis_parent(), profiles[1].genesis_parent());
    let mut signature = *profiles[0].signature();
    signature[0] ^= 1;
    let bad = SignedOwnerProfile::decode(&wrap(&profiles[0].semantic_bytes(), &signature)).unwrap();
    assert_eq!(bad.id(), profiles[1].id());
    assert!(!bad.verify());
    assert!(profiles[1].verify());
}

#[test]
fn every_unknown_profile_policy_and_framing_variant_refuses_before_trust() {
    let profile = profile();
    let semantic = profile.semantic_bytes();
    let encoded = profile.encode();
    for end in 0..encoded.len() {
        assert!(SignedOwnerProfile::decode(&encoded[..end]).is_err());
    }
    for index in semantic.len() - 11..semantic.len() {
        let mut changed = semantic.clone();
        changed[index] ^= 1;
        assert_eq!(
            SignedOwnerProfile::decode(&wrap(&changed, profile.signature())),
            Err(ChatError::Authority)
        );
    }
    let first_digest = 4 + DOMAIN.len() + 2 + 4 + MAX_GROUP_ID_BYTES;
    for position in [first_digest, first_digest + 36] {
        for length in [0u32, 31, 33, u32::MAX] {
            let mut changed = semantic.clone();
            changed[position..position + 4].copy_from_slice(&length.to_be_bytes());
            assert!(SignedOwnerProfile::decode(&wrap(&changed, profile.signature())).is_err());
        }
    }
    for length in [0, 63, 65] {
        assert!(SignedOwnerProfile::decode(&wrap(&semantic, &vec![0; length])).is_err());
    }
    let mut trailing = encoded;
    trailing.push(0);
    assert!(SignedOwnerProfile::decode(&trailing).is_err());
    let mut wrong_version = semantic.clone();
    wrong_version[4 + DOMAIN.len() + 1] = 1;
    assert_eq!(
        SignedOwnerProfile::decode(&wrap(&wrong_version, profile.signature())),
        Err(ChatError::Scope)
    );
    let mut wrong_domain = semantic;
    wrong_domain[4] ^= 1;
    assert_eq!(
        SignedOwnerProfile::decode(&wrap(&wrong_domain, profile.signature())),
        Err(ChatError::Malformed)
    );
}

#[test]
fn signer_callback_uses_exact_founder_and_invalid_group_never_enters_custody() {
    let original = profile();
    let calls = Cell::new(0);
    let signed = SignedOwnerProfile::sign_using(original.spec().clone(), |bytes| {
        calls.set(calls.get() + 1);
        assert_eq!(bytes, original.semantic_bytes());
        Ok(key(42).sign(bytes))
    })
    .unwrap();
    assert_eq!(signed, original);
    assert_eq!(calls.get(), 1);
    assert_eq!(
        SignedOwnerProfile::sign_using(original.spec().clone(), |bytes| Ok(key(43).sign(bytes))),
        Err(ChatError::Signature)
    );
    assert_eq!(
        SignedOwnerProfile::sign_using(original.spec().clone(), |_| Err(ChatError::Permission)),
        Err(ChatError::Permission)
    );
    calls.set(0);
    for group in [Vec::new(), vec![7; MAX_GROUP_ID_BYTES + 1]] {
        let mut spec = original.spec().clone();
        spec.group = group;
        assert!(SignedOwnerProfile::sign_using(spec, |_| {
            calls.set(calls.get() + 1);
            Ok([0; 64])
        })
        .is_err());
    }
    assert_eq!(calls.get(), 0);
}

#[test]
fn first_header_requires_exact_founder_body_not_just_a_self_signed_profile() {
    let profile = profile();
    let body = founder(&profile);
    let first = header(&profile, &body, 1, profile.genesis_parent());
    assert_eq!(profile.verify_genesis_binding(&body, &first), Ok(()));
    assert_eq!(body.verify_profile_binding(&profile, &first), Ok(()));
    for field in 0..8 {
        let mut changed = first.spec().clone();
        let mut seed = 42;
        match field {
            0 => changed.group[0] ^= 1,
            1 => changed.profile = OwnerFactId::from_bytes([20; 32]),
            2 => {
                seed = 43;
                changed.issuer = key(seed).verifying_key().to_bytes();
            }
            3 => changed.tenure = OwnerTenureId::from_bytes([21; 32]),
            4 => changed.parent = OwnerHeaderId::from_bytes([22; 32]),
            5 => changed.sequence = 2,
            6 => changed.kind = HeaderKind::Removal,
            _ => changed.body = OwnerBodyId::from_bytes([23; 32]),
        }
        let changed =
            SignedOwnerHeader::sign_using(changed, |bytes| Ok(key(seed).sign(bytes))).unwrap();
        assert!(profile.verify_genesis_binding(&body, &changed).is_err());
    }
    let mut wrong = body.spec().clone();
    wrong.origin = AdmissionOrigin::Founder { creation: [24; 32] };
    let wrong = AdmissionBody::new(wrong).unwrap();
    let wrong_header = header(&profile, &wrong, 1, profile.genesis_parent());
    assert_eq!(
        profile.verify_genesis_binding(&wrong, &wrong_header),
        Err(ChatError::Dependency)
    );
    let mut wrong = body.spec().clone();
    wrong.subject = key(41).verifying_key().to_bytes();
    let wrong = AdmissionBody::new(wrong).unwrap();
    let wrong_header = header(&profile, &wrong, 1, profile.genesis_parent());
    assert_eq!(
        profile.verify_genesis_binding(&wrong, &wrong_header),
        Err(ChatError::Scope)
    );
    let added = add(&profile, 4);
    let wrong_header = header(&profile, &added, 1, profile.genesis_parent());
    assert!(profile
        .verify_genesis_binding(&added, &wrong_header)
        .is_err());
}

#[test]
fn add_header_profile_binding_is_crypto_only_not_known_parent_or_current_state() {
    let profile = profile();
    let founder = founder(&profile);
    let first = header(&profile, &founder, 1, profile.genesis_parent());
    let added = add(&profile, 4);
    let second = header(&profile, &added, 2, first.id());
    assert_eq!(added.verify_profile_binding(&profile, &second), Ok(()));
    assert_eq!(second.verify_successor_of(&first), Ok(()));
    let unknown_parent = header(&profile, &added, 2, OwnerHeaderId::from_bytes([99; 32]));
    assert_eq!(
        added.verify_profile_binding(&profile, &unknown_parent),
        Ok(()),
        "known common-prefix parent selection remains a separate consumer gate"
    );
    assert!(unknown_parent.verify_successor_of(&first).is_err());
    let sequence_one = header(&profile, &added, 1, profile.genesis_parent());
    assert_eq!(
        added.verify_profile_binding(&profile, &sequence_one),
        Err(ChatError::Dependency)
    );
    let mut bad = second.encode();
    *bad.last_mut().unwrap() ^= 1;
    assert_eq!(
        added.verify_profile_binding(&profile, &SignedOwnerHeader::decode(&bad).unwrap()),
        Err(ChatError::Signature)
    );
}

#[test]
fn ordinary_ids_match_independent_length_prefixed_member_only_frames() {
    let profile = profile();
    let body = add(&profile, 4);
    let realm = Realm::new(&profile.spec().group, 16).unwrap();
    let labels = body.ordinary_labels(&realm).unwrap();
    let admission_id = body.admission_id();
    let realm_id = realm.id();
    let mut grant = Vec::new();
    field(&mut grant, b"mewtual/chat/ordinary-grant/v2\0");
    for bytes in [
        &body.spec().group[..],
        body.spec().profile.as_bytes(),
        admission_id.as_bytes(),
        &body.spec().subject,
        realm_id.as_bytes(),
    ] {
        field(&mut grant, bytes);
    }
    grant.extend_from_slice(&[0, 23, 1]);
    assert_eq!(labels.grant.as_bytes(), blake3::hash(&grant).as_bytes());
    let mut basis = Vec::new();
    field(&mut basis, b"mewtual/chat/ordinary-basis/v2\0");
    for bytes in [
        &body.spec().group[..],
        body.spec().profile.as_bytes(),
        admission_id.as_bytes(),
        &body.spec().subject,
        realm_id.as_bytes(),
        labels.grant.as_bytes(),
    ] {
        field(&mut basis, bytes);
    }
    basis.extend_from_slice(&[0, 23, 1]);
    assert_eq!(labels.basis.as_bytes(), blake3::hash(&basis).as_bytes());
    assert!(!Rights::MEMBER.contains(Rights::DELETE_ANY));
}

fn member_action(
    body: &AdmissionBody,
    channel: u128,
    reply: Option<crate::chat::ActionId>,
) -> SignedAction {
    let realm = Realm::new(&body.spec().group, channel).unwrap();
    let labels = body.ordinary_labels(&realm).unwrap();
    SignedAction::sign(
        &key(41),
        ActionSpec {
            realm,
            basis: labels.basis,
            grant: labels.grant,
            nonce: [1; 16],
            presented_ms: 1,
            parents: reply.as_slice().to_vec(),
            body: Body::Create {
                text: "owner-offline post".into(),
                reply_to: reply,
            },
        },
    )
    .unwrap()
}
#[test]
fn same_key_rejoin_new_realms_and_replies_need_distinct_stable_labels_not_owner_refresh() {
    let profile = profile();
    let a1 = add(&profile, 4);
    let a2 = add(&profile, 5);
    assert_eq!(a1.spec().subject, a2.spec().subject);
    assert_ne!(a1.admission_id(), a2.admission_id());
    let first = member_action(&a1, 1, None);
    let later = member_action(&a1, 2, Some(first.id()));
    assert_eq!(a1.matches_action_delegation(&first), Ok(()));
    assert_eq!(a1.matches_action_delegation(&later), Ok(()));
    assert_ne!(first.spec().grant, later.spec().grant);
    assert_ne!(first.spec().basis, later.spec().basis);
    assert_eq!(
        a2.matches_action_delegation(&first),
        Err(ChatError::Authority)
    );
    let rejoined = member_action(&a2, 1, None);
    assert_ne!(first.id(), rejoined.id());
    assert_eq!(a2.matches_action_delegation(&rejoined), Ok(()));
    let foreign = Realm::new(b"foreign group", 1).unwrap();
    assert_eq!(a1.ordinary_labels(&foreign), Err(ChatError::Scope));
    // Only public label/action math is exercised. This does not issue a semantic witness or
    // establish real current membership/projection while the owner is offline.
}

#[test]
fn delegation_rejects_wrong_subject_labels_signature_and_profile_binding() {
    let profile = profile();
    let body = add(&profile, 4);
    let action = member_action(&body, 1, None);
    for field in 0..2 {
        let mut changed = action.spec().clone();
        if field == 0 {
            changed.grant = crate::chat::GrantId::from_bytes([31; 32]);
        } else {
            changed.basis = crate::chat::BasisId::from_bytes([32; 32]);
        }
        let changed = SignedAction::sign(&key(41), changed).unwrap();
        assert_eq!(
            body.matches_action_delegation(&changed),
            Err(ChatError::Authority)
        );
    }
    let foreign_signer = SignedAction::sign(&key(43), action.spec().clone()).unwrap();
    assert_eq!(
        body.matches_action_delegation(&foreign_signer),
        Err(ChatError::Authority)
    );
    let mut raw = action.encode();
    *raw.last_mut().unwrap() ^= 1;
    assert_eq!(
        body.matches_action_delegation(&SignedAction::decode(&raw).unwrap()),
        Err(ChatError::Signature)
    );
    let mut changed = body.spec().clone();
    changed.profile = OwnerFactId::from_bytes([33; 32]);
    assert_eq!(
        AdmissionBody::new(changed)
            .unwrap()
            .matches_action_delegation(&action),
        Err(ChatError::Authority)
    );
}

#[test]
fn strict_admission_epochs_policy_fixed_fields_and_all_truncations_refuse() {
    let profile = profile();
    let body = add(&profile, 4);
    let encoded = body.encode();
    for end in 0..encoded.len() {
        assert!(AdmissionBody::decode(&encoded[..end]).is_err());
    }
    let base = 4 + b"mewtual/chat/admission/v2\0".len() + 4 + MAX_GROUP_ID_BYTES;
    for offset in [0, 36, 73, 125, 161] {
        for length in [0u32, 31, 33, u32::MAX] {
            let mut bad = encoded.clone();
            bad[base + offset..base + offset + 4].copy_from_slice(&length.to_be_bytes());
            assert!(AdmissionBody::decode(&bad).is_err());
        }
    }
    let mut unknown = encoded.clone();
    unknown[base + 72] = 2;
    assert_eq!(AdmissionBody::decode(&unknown), Err(ChatError::Malformed));
    for index in encoded.len() - 3..encoded.len() {
        let mut bad = encoded.clone();
        bad[index] ^= 1;
        assert_eq!(AdmissionBody::decode(&bad), Err(ChatError::Authority));
    }
    for (before, after) in [(u64::MAX, 0), (4, 4), (4, 6)] {
        let mut bad = body.spec().clone();
        bad.origin = AdmissionOrigin::Add {
            commit: [2; 32],
            before_epoch: before,
            after_epoch: after,
            key_package: [4; 32],
            credential: [3; 32],
        };
        assert_eq!(AdmissionBody::new(bad.clone()), Err(ChatError::Dependency));
        assert_eq!(
            AdmissionBody::decode(&raw_admission(&bad)),
            Err(ChatError::Dependency)
        );
    }
    let mut trailing = encoded;
    trailing.push(0);
    assert!(AdmissionBody::decode(&trailing).is_err());
    for group in [Vec::new(), vec![7; MAX_GROUP_ID_BYTES + 1]] {
        let mut bad = body.spec().clone();
        bad.group = group;
        assert!(AdmissionBody::decode(&raw_admission(&bad)).is_err());
    }
}

#[test]
fn data_codec_has_no_lifetime64_cliff_and_never_accepts_legacy_owner_facts() {
    let profile = profile();
    let mut ids = BTreeSet::new();
    for epoch in 0..130u64 {
        let mut spec = add(&profile, 4).spec().clone();
        spec.origin = AdmissionOrigin::Add {
            commit: [2; 32],
            before_epoch: epoch,
            after_epoch: epoch + 1,
            key_package: [4; 32],
            credential: [3; 32],
        };
        let body = AdmissionBody::new(spec).unwrap();
        ids.insert(body.admission_id());
    }
    assert_eq!(
        ids.len(),
        130,
        "synthetic origin labels only, not 130 actual accepted MLS joins"
    );
    use crate::chat::owner_facts::{OwnerFactBody, OwnerFactSpec, SignedOwnerFact};
    let legacy = SignedOwnerFact::sign_using(
        OwnerFactSpec {
            group: vec![7],
            profile: None,
            body: OwnerFactBody::Profile {
                creation: [1; 32],
                founder: key(42).verifying_key().to_bytes(),
            },
        },
        key(42).verifying_key().to_bytes(),
        |bytes| Ok(key(42).sign(bytes)),
    )
    .unwrap()
    .encode();
    assert!(SignedOwnerProfile::decode(&legacy).is_err());
    assert!(AdmissionBody::decode(&legacy).is_err());
    assert!(SignedOwnerFact::decode(&profile.encode()).is_err());
    assert!(SignedOwnerFact::decode(&add(&profile, 4).encode()).is_err());
    assert!(SignedOwnerProfile::decode(&add(&profile, 4).encode()).is_err());
    assert!(AdmissionBody::decode(&profile.encode()).is_err());
}
