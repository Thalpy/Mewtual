//! Actual signatures and strict DATA codecs, not real MLS/Add/root/native acceptance.
use super::*;
use crate::chat::owner_header::{
    HeaderKind, HeaderSpec, OwnerHeaderId, OwnerStateCommitment, SignedOwnerHeader,
};
use crate::chat::owner_profile::{
    AdmissionBody, AdmissionOrigin, AdmissionSpec, ProfileSpec, SignedOwnerProfile,
};
use catcoms_crypto::DeviceKeypair;
use std::cell::Cell;

fn key(seed: u8) -> DeviceKeypair {
    DeviceKeypair::from_seed(&[seed; 32])
}
fn public(seed: u8) -> [u8; 32] {
    key(seed).verifying_key().to_bytes()
}
fn commitment(kind: JoinPayloadKindData, bytes: &[u8]) -> [u8; 32] {
    kind.commitment_data(bytes).unwrap()
}
fn invite(spec: InviteBindingSpecData) -> SignedInviteBindingData {
    SignedInviteBindingData::sign_using(spec, |bytes| Ok(key(2).sign(bytes))).unwrap()
}
fn request(spec: JoinRequestBindingSpecData) -> SignedJoinRequestBindingData {
    SignedJoinRequestBindingData::sign_using(spec, |bytes| Ok(key(3).sign(bytes))).unwrap()
}
fn welcome(spec: WelcomeBindingSpecData) -> SignedWelcomeBindingData {
    SignedWelcomeBindingData::sign_using(spec, |bytes| Ok(key(1).sign(bytes))).unwrap()
}

struct Fixture {
    invite: SignedInviteBindingData,
    request: SignedJoinRequestBindingData,
    welcome: SignedWelcomeBindingData,
    profile: SignedOwnerProfile,
    admission: AdmissionBody,
    header: SignedOwnerHeader,
}
impl Fixture {
    fn new() -> Self {
        let profile = SignedOwnerProfile::sign_using(
            ProfileSpec {
                group: vec![7; MAX_GROUP_ID_BYTES],
                founder: public(1),
                creation: [8; 32],
            },
            |bytes| Ok(key(1).sign(bytes)),
        )
        .unwrap();
        let inv = invite(InviteBindingSpecData {
            anchor: GenesisAnchorBindingData {
                group: profile.spec().group.clone(),
                founder: public(1),
                profile: *profile.id().as_bytes(),
                admission: [9; 32],
                tenure: *profile.tenure().as_bytes(),
                header: [10; 32],
                state: [11; 32],
            },
            inviter: public(2),
            nonce: [12; 16],
            expires_at_ms: 100,
            policy: commitment(JoinPayloadKindData::Policy, b"policy"),
            public_routing_plan: commitment(JoinPayloadKindData::RoutingPlan, b"public plan"),
        });
        let req = request(JoinRequestBindingSpecData {
            invite: inv.id_data(),
            anchor: inv.spec_data().anchor.commitment_data().unwrap(),
            joiner: public(3),
            request_token: [13; 32],
            key_package_wire: commitment(
                JoinPayloadKindData::KeyPackageWire,
                b"exact package wire",
            ),
            key_package_reference: [14; 32],
            credential: [15; 32],
        });
        let admission = AdmissionBody::new(AdmissionSpec {
            group: profile.spec().group.clone(),
            profile: profile.id(),
            subject: public(3),
            origin: AdmissionOrigin::Add {
                commit: [16; 32],
                before_epoch: 0,
                after_epoch: 1,
                key_package: [14; 32],
                credential: [15; 32],
            },
        })
        .unwrap();
        let header = SignedOwnerHeader::sign_using(
            HeaderSpec {
                group: profile.spec().group.clone(),
                profile: profile.id(),
                issuer: public(1),
                tenure: profile.tenure(),
                parent: OwnerHeaderId::from_bytes([10; 32]),
                sequence: 2,
                kind: HeaderKind::Admission,
                body: admission.id(),
                state: OwnerStateCommitment::from_bytes([17; 32]),
            },
            |bytes| Ok(key(1).sign(bytes)),
        )
        .unwrap();
        let wel = welcome(WelcomeBindingSpecData {
            invite: inv.id_data(),
            anchor: req.spec_data().anchor,
            request_binding: req.id_data(),
            key_package_wire: req.spec_data().key_package_wire,
            key_package_reference: req.spec_data().key_package_reference,
            admitter: public(1),
            add_header: *header.id().as_bytes(),
            admission: *admission.admission_id().as_bytes(),
            welcome: commitment(JoinPayloadKindData::Welcome, b"exact Welcome"),
            policy: inv.spec_data().policy,
            public_routing_plan: inv.spec_data().public_routing_plan,
            sealed_routing_transfer: commitment(
                JoinPayloadKindData::RoutingTransfer,
                b"exact sealed routing",
            ),
        });
        Self {
            invite: inv,
            request: req,
            welcome: wel,
            profile,
            admission,
            header,
        }
    }
    fn links(
        &self,
        request: &SignedJoinRequestBindingData,
        welcome: &SignedWelcomeBindingData,
    ) -> Result<(), ChatError> {
        welcome.check_links_data(
            &self.invite,
            request,
            &self.profile,
            &self.admission,
            &self.header,
        )
    }
}
fn wrap(semantic: &[u8], signature: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new();
    field(&mut e, ENVELOPE);
    field(&mut e, semantic);
    field(&mut e, signature);
    e.finish()
}

#[test]
fn real_signatures_roundtrip_maximum_group_and_distinct_inviter_admitter() {
    let f = Fixture::new();
    let inv = SignedInviteBindingData::decode(&f.invite.encode()).unwrap();
    let req = SignedJoinRequestBindingData::decode(&f.request.encode()).unwrap();
    let wel = SignedWelcomeBindingData::decode(&f.welcome.encode()).unwrap();
    assert!(inv.verify() && req.verify() && wel.verify());
    assert_ne!(inv.spec_data().inviter, wel.spec_data().admitter);
    assert_eq!(f.links(&req, &wel), Ok(()));
    for bytes in [inv.encode(), req.encode(), wel.encode()] {
        assert!(bytes.len() <= MAX_JOIN_BINDING_BYTES);
    }
    inv.check_public_bodies_data(b"policy", b"public plan")
        .unwrap();
    req.check_key_package_wire_data(b"exact package wire")
        .unwrap();
    wel.check_payloads_data(b"exact Welcome", b"exact sealed routing")
        .unwrap();
    assert!(!inv.expired_at_data(99));
    assert!(inv.expired_at_data(100));
    assert!(inv.expired_at_data(u64::MAX));
    // Expired arrival does not invalidate an exact already-successful binding.
    assert_eq!(
        wel.check_links_data(&inv, &req, &f.profile, &f.admission, &f.header),
        Ok(())
    );
}

macro_rules! exhaustive_frame_checks {
    ($test:ident,$field:ident,$ty:ty) => {
        #[test]
        fn $test() {
            let f = Fixture::new();
            let value = &f.$field;
            let bytes = value.encode();
            for end in 0..bytes.len() {
                assert!(<$ty>::decode(&bytes[..end]).is_err(), "truncation {end}");
            }
            let mut tail = bytes.clone();
            tail.push(0);
            assert!(<$ty>::decode(&tail).is_err());
            assert!(matches!(
                <$ty>::decode(&vec![0; MAX_JOIN_BINDING_BYTES + 1]),
                Err(ChatError::Bound)
            ));
            for index in 0..bytes.len() {
                let mut changed = bytes.clone();
                changed[index] ^= 1;
                if let Ok(parsed) = <$ty>::decode(&changed) {
                    assert!(!parsed.verify(), "accepted substituted byte {index}");
                }
            }
            let mut semantic = value.spec.semantic_bytes();
            // Version starts immediately after the semantic domain field.
            let width = u32::from_be_bytes(semantic[..4].try_into().unwrap()) as usize;
            semantic[4 + width + 1] = 3;
            assert!(matches!(
                <$ty>::decode(&wrap(&semantic, &value.signature)),
                Err(ChatError::Scope)
            ));
        }
    };
}
exhaustive_frame_checks!(
    invite_rejects_every_truncation_substitution_trailer_future_and_oversize,
    invite,
    SignedInviteBindingData
);
exhaustive_frame_checks!(
    request_rejects_every_truncation_substitution_trailer_future_and_oversize,
    request,
    SignedJoinRequestBindingData
);
exhaustive_frame_checks!(
    welcome_rejects_every_truncation_substitution_trailer_future_and_oversize,
    welcome,
    SignedWelcomeBindingData
);

#[test]
fn invalid_signature_with_the_same_semantic_id_never_bypasses_links() {
    let f = Fixture::new();
    let bad =
        SignedInviteBindingData::decode(&wrap(&f.invite.spec.semantic_bytes(), &[0; 64])).unwrap();
    assert_eq!(bad.id_data(), f.invite.id_data());
    assert!(!bad.verify());
    assert_eq!(
        f.request.check_invite_links_data(&bad),
        Err(ChatError::Signature)
    );
    assert_eq!(f.request.check_invite_links_data(&f.invite), Ok(()));
    let bad =
        SignedJoinRequestBindingData::decode(&wrap(&f.request.spec.semantic_bytes(), &[0; 64]))
            .unwrap();
    assert_eq!(bad.id_data(), f.request.id_data());
    assert_eq!(f.links(&bad, &f.welcome), Err(ChatError::Signature));
    let bad = SignedWelcomeBindingData::decode(&wrap(&f.welcome.spec.semantic_bytes(), &[0; 64]))
        .unwrap();
    assert_eq!(bad.id_data(), f.welcome.id_data());
    assert_eq!(f.links(&f.request, &bad), Err(ChatError::Signature));
}

#[test]
fn all_request_fields_are_bound_and_sdk_reference_is_not_wire_hash() {
    let f = Fixture::new();
    let changes: [fn(&mut JoinRequestBindingSpecData); 7] = [
        |s| s.invite[0] ^= 1,
        |s| s.anchor[0] ^= 1,
        |s| s.joiner = public(4),
        |s| s.request_token[0] ^= 1,
        |s| s.key_package_wire[0] ^= 1,
        |s| s.key_package_reference[0] ^= 1,
        |s| s.credential[0] ^= 1,
    ];
    for change in changes {
        let mut spec = f.request.spec.clone();
        change(&mut spec);
        let changed = SignedJoinRequestBindingData::sign_using(spec.clone(), |bytes| {
            Ok(key(if spec.joiner == public(4) { 4 } else { 3 }).sign(bytes))
        })
        .unwrap();
        assert!(f.links(&changed, &f.welcome).is_err());
    }
    assert_ne!(
        f.request.spec.key_package_wire,
        f.request.spec.key_package_reference
    );
    assert_eq!(
        f.request
            .check_key_package_wire_data(b"substituted package"),
        Err(ChatError::Scope)
    );
    // Even a freshly signed Welcome for another subject/reference/credential
    // must match the genuine governing Admission, not just the request tuple.
    for change in [changes[2], changes[5], changes[6]] {
        let mut spec = f.request.spec.clone();
        change(&mut spec);
        let req = SignedJoinRequestBindingData::sign_using(spec.clone(), |bytes| {
            Ok(key(if spec.joiner == public(4) { 4 } else { 3 }).sign(bytes))
        })
        .unwrap();
        let mut spec = f.welcome.spec.clone();
        spec.request_binding = req.id_data();
        spec.key_package_reference = req.spec.key_package_reference;
        assert_eq!(f.links(&req, &welcome(spec)), Err(ChatError::Scope));
    }
}

#[test]
fn every_welcome_field_is_signed_and_foreign_admitter_is_not_governing_authority() {
    let f = Fixture::new();
    let changes: [fn(&mut WelcomeBindingSpecData); 12] = [
        |s| s.invite[0] ^= 1,
        |s| s.anchor[0] ^= 1,
        |s| s.request_binding[0] ^= 1,
        |s| s.key_package_wire[0] ^= 1,
        |s| s.key_package_reference[0] ^= 1,
        |s| s.admitter = public(2),
        |s| s.add_header[0] ^= 1,
        |s| s.admission[0] ^= 1,
        |s| s.welcome[0] ^= 1,
        |s| s.policy[0] ^= 1,
        |s| s.public_routing_plan[0] ^= 1,
        |s| s.sealed_routing_transfer[0] ^= 1,
    ];
    for change in changes {
        let mut spec = f.welcome.spec.clone();
        change(&mut spec);
        let parsed = SignedWelcomeBindingData {
            spec,
            signature: f.welcome.signature,
        };
        assert!(!parsed.verify());
    }
    let mut spec = f.welcome.spec.clone();
    spec.admitter = public(2);
    let forged =
        SignedWelcomeBindingData::sign_using(spec, |bytes| Ok(key(2).sign(bytes))).unwrap();
    assert_eq!(f.links(&f.request, &forged), Err(ChatError::Scope));
    assert_eq!(
        f.welcome
            .check_payloads_data(b"changed Welcome", b"exact sealed routing"),
        Err(ChatError::Scope)
    );
    assert_eq!(
        f.welcome
            .check_payloads_data(b"exact Welcome", b"changed sealed routing"),
        Err(ChatError::Scope)
    );
    assert_eq!(
        f.invite
            .check_public_bodies_data(b"policy", b"different public plan"),
        Err(ChatError::Scope)
    );
}

#[test]
fn founder_admission_and_self_signed_foreign_profile_cannot_replace_the_actual_add() {
    let f = Fixture::new();
    let founder = AdmissionBody::new(AdmissionSpec {
        group: f.profile.spec().group.clone(),
        profile: f.profile.id(),
        subject: public(1),
        origin: AdmissionOrigin::Founder {
            creation: f.profile.spec().creation,
        },
    })
    .unwrap();
    let mut spec = f.header.spec().clone();
    spec.sequence = 1;
    spec.parent = f.profile.genesis_parent();
    spec.body = founder.id();
    let header = SignedOwnerHeader::sign_using(spec, |bytes| Ok(key(1).sign(bytes))).unwrap();
    founder.verify_profile_binding(&f.profile, &header).unwrap();
    // Make the request's subject agree too: rejection must specifically require
    // an Add origin, rather than pass accidentally on a subject mismatch.
    let mut request_spec = f.request.spec.clone();
    request_spec.joiner = public(1);
    let founder_request =
        SignedJoinRequestBindingData::sign_using(request_spec, |bytes| Ok(key(1).sign(bytes)))
            .unwrap();
    let mut spec = f.welcome.spec.clone();
    spec.request_binding = founder_request.id_data();
    spec.add_header = *header.id().as_bytes();
    spec.admission = *founder.admission_id().as_bytes();
    assert_eq!(
        welcome(spec).check_links_data(&f.invite, &founder_request, &f.profile, &founder, &header),
        Err(ChatError::Scope)
    );
    let foreign = SignedOwnerProfile::sign_using(
        ProfileSpec {
            group: f.profile.spec().group.clone(),
            founder: public(4),
            creation: [8; 32],
        },
        |bytes| Ok(key(4).sign(bytes)),
    )
    .unwrap();
    assert!(f
        .welcome
        .check_links_data(&f.invite, &f.request, &foreign, &f.admission, &f.header)
        .is_err());
}

#[test]
fn coherently_resigned_foreign_anchors_still_cannot_replace_the_add_profile() {
    let f = Fixture::new();
    assert_eq!(f.links(&f.request, &f.welcome), Ok(()));
    let changes: [fn(&mut GenesisAnchorBindingData); 4] = [
        |anchor| anchor.group[0] ^= 1,
        |anchor| anchor.founder = public(4),
        |anchor| anchor.profile[0] ^= 1,
        |anchor| anchor.tenure[0] ^= 1,
    ];
    for change in changes {
        let mut spec = f.invite.spec.clone();
        change(&mut spec.anchor);
        let inv = invite(spec);
        let mut spec = f.request.spec.clone();
        spec.invite = inv.id_data();
        spec.anchor = inv.spec.anchor.commitment_data().unwrap();
        let req = request(spec);
        let mut spec = f.welcome.spec.clone();
        spec.invite = inv.id_data();
        spec.anchor = req.spec.anchor;
        spec.request_binding = req.id_data();
        let wel = welcome(spec);
        // Every new signature and request/anchor hash is valid. Only the
        // new anchor-to-original Profile checks can reject this substitution.
        assert!(inv.verify() && req.verify() && wel.verify());
        req.check_invite_links_data(&inv).unwrap();
        f.admission
            .verify_profile_binding(&f.profile, &f.header)
            .unwrap();
        assert_eq!(
            wel.check_links_data(&inv, &req, &f.profile, &f.admission, &f.header),
            Err(ChatError::Scope)
        );
    }
}

#[test]
fn invalid_grammar_refuses_before_signing_and_debug_redacts_all_bearer_data() {
    let f = Fixture::new();
    let calls = Cell::new(0);
    for group in [vec![], vec![1; MAX_GROUP_ID_BYTES + 1]] {
        let mut spec = f.invite.spec.clone();
        spec.anchor.group = group;
        assert!(SignedInviteBindingData::sign_using(spec, |_| {
            calls.set(calls.get() + 1);
            Ok([0; 64])
        })
        .is_err());
    }
    let mut spec = f.invite.spec.clone();
    spec.nonce = [0; 16];
    assert!(SignedInviteBindingData::sign_using(spec, |_| {
        calls.set(calls.get() + 1);
        Ok([0; 64])
    })
    .is_err());
    let mut spec = f.request.spec.clone();
    spec.request_token = [0; 32];
    assert!(SignedJoinRequestBindingData::sign_using(spec, |_| {
        calls.set(calls.get() + 1);
        Ok([0; 64])
    })
    .is_err());
    assert_eq!(calls.get(), 0);
    assert_eq!(format!("{:?}", f.invite), "SignedInviteBindingData { .. }");
    assert_eq!(
        format!("{:?}", f.invite.spec),
        "InviteBindingSpecData { .. }"
    );
    assert_eq!(
        format!("{:?}", f.request),
        "SignedJoinRequestBindingData { .. }"
    );
    assert_eq!(
        format!("{:?}", f.welcome),
        "SignedWelcomeBindingData { .. }"
    );
}

#[test]
fn body_hashes_have_separate_exact_domains_and_refuse_before_unbounded_work() {
    let kinds = [
        JoinPayloadKindData::KeyPackageWire,
        JoinPayloadKindData::Policy,
        JoinPayloadKindData::RoutingPlan,
        JoinPayloadKindData::RoutingTransfer,
        JoinPayloadKindData::Welcome,
    ];
    let ids: std::collections::BTreeSet<_> = kinds
        .map(|kind| commitment(kind, b"same bytes"))
        .into_iter()
        .collect();
    assert_eq!(ids.len(), kinds.len());
    for kind in kinds {
        assert_eq!(kind.commitment_data(&[]), Err(ChatError::Bound));
        assert!(kind
            .commitment_data(&vec![1; MAX_JOIN_PAYLOAD_BYTES])
            .is_ok());
        assert_eq!(
            kind.commitment_data(&vec![1; MAX_JOIN_PAYLOAD_BYTES + 1]),
            Err(ChatError::Bound)
        );
    }
}

#[test]
fn legacy_invite_decoder_and_other_binding_decoders_refuse_new_frames() {
    let f = Fixture::new();
    let mut legacy = catcoms_mls::InviteToken {
        group_id: vec![7; 16],
        inviter_device_id: catcoms_crypto::DeviceId::from_public_key_bytes(&public(2)),
        inviter_public_key: public(2).to_vec(),
        invite_nonce: [12; 16],
        expires_at_ms: 100,
        bootstrap: vec![],
        rendezvous: vec![],
        policy: None,
        signature: [0; 64],
    };
    let encoded = legacy.encode();
    legacy.signature = key(2).sign(&encoded[..encoded.len() - 68]);
    assert!(legacy.verify(&public(2)));
    assert_eq!(
        catcoms_mls::InviteToken::decode(&legacy.encode()).unwrap(),
        legacy
    );
    assert!(catcoms_mls::InviteToken::decode(&f.invite.encode()).is_err());
    assert!(catcoms_mls::InviteToken::decode(&f.invite.spec.semantic_bytes()).is_err());
    assert!(SignedJoinRequestBindingData::decode(&f.invite.encode()).is_err());
    assert!(SignedWelcomeBindingData::decode(&f.request.encode()).is_err());
    assert!(SignedInviteBindingData::decode(&f.welcome.encode()).is_err());
}
