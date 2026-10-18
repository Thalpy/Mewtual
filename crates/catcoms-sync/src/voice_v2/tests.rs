use super::*;

fn device(seed: u8) -> DeviceKeypair {
    DeviceKeypair::from_seed(&[seed; 32])
}
fn scope() -> Scope {
    Scope::for_group(b"exact MLS group", Some([8; 32]), 123)
}
fn key(join: u8) -> JoinKey {
    // SEC2 P-256 generator, a valid uncompressed point. Point validation in the endpoint uses
    // WebCrypto, independently of this native structural/signature codec.
    let bytes = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap();
    JoinKey::new([join; 32], bytes.try_into().unwrap()).unwrap()
}
fn announcement(signer: &DeviceKeypair, join: u8) -> SignedObject {
    SignedObject::issue(
        scope(),
        signer,
        [join; 32],
        1000,
        31_000,
        Body::Announcement {
            key: key(join),
            epoch: 7,
            renewal: 1,
        },
    )
    .unwrap()
}
fn encrypted(signer: &DeviceKeypair, target: DeviceId) -> SignedObject {
    let id = [33; 32];
    let header = EncryptedHeader {
        source_join: [1; 32],
        target,
        target_join: [2; 32],
        source_key_ref: key(1).key_ref,
        target_key_ref: key(2).key_ref,
        handshake: [3; 32],
        low_to_high: signer.device_id() < target,
        kind: EncryptedKind::Offer,
        sequence: 1,
        offer: Some(OfferId {
            issuer: signer.device_id(),
            message: id,
        }),
    };
    SignedObject::issue(
        scope(),
        signer,
        id,
        1000,
        16_000,
        Body::Encrypted {
            header,
            ciphertext: vec![77; 48],
        },
    )
    .unwrap()
}
fn context(local: DeviceId) -> Context {
    Context {
        scope: scope(),
        local,
        now: 2000,
        epoch: 7,
        indirect: true,
    }
}
fn roundtrip(value: &SignedObject) {
    assert!(SignedObject::decode(&value.encode()).unwrap() == *value);
    let encoded = Envelope::Direct(value.clone()).encode();
    assert!(Envelope::decode(&encoded).unwrap().object() == value);
}

fn controls(a: &DeviceKeypair, c: &DeviceKeypair) -> [SignedObject; 2] {
    [
        SignedObject::issue(
            scope(),
            a,
            [2; 32],
            1000,
            16_000,
            Body::WatchChallenge {
                watcher: [3; 32],
                target: c.device_id(),
                join: [4; 32],
                key_ref: [5; 32],
                challenge: [6; 32],
            },
        )
        .unwrap(),
        SignedObject::issue(
            scope(),
            c,
            [3; 32],
            1000,
            16_000,
            Body::JoinProof {
                join: [4; 32],
                key_ref: [5; 32],
                watcher: a.device_id(),
                watcher_incarnation: [3; 32],
                challenge: [6; 32],
                challenge_id: [2; 32],
            },
        )
        .unwrap(),
    ]
}

#[test]
fn closed_object_kinds_roundtrip_without_exposing_plaintext() {
    let a = device(1);
    let c = device(3);
    let objects = [
        announcement(&a, 1),
        SignedObject::issue(
            scope(),
            &a,
            [2; 32],
            1000,
            16_000,
            Body::WatchChallenge {
                watcher: [3; 32],
                target: c.device_id(),
                join: [4; 32],
                key_ref: [5; 32],
                challenge: [6; 32],
            },
        )
        .unwrap(),
        SignedObject::issue(
            scope(),
            &a,
            [3; 32],
            1000,
            16_000,
            Body::JoinProof {
                join: [4; 32],
                key_ref: [5; 32],
                watcher: c.device_id(),
                watcher_incarnation: [6; 32],
                challenge: [7; 32],
                challenge_id: [2; 32],
            },
        )
        .unwrap(),
        encrypted(&a, c.device_id()),
    ];
    for value in objects {
        roundtrip(&value);
    }
}

#[test]
fn every_signed_byte_is_integrity_protected_and_trailing_bytes_refuse() {
    let a = device(1);
    let c = device(3);
    for object in [announcement(&a, 1), encrypted(&a, c.device_id())]
        .into_iter()
        .chain(controls(&a, &c))
    {
        let encoded = object.encode();
        for offset in 0..encoded.len() {
            let mut altered = encoded.clone();
            altered[offset] ^= 1;
            assert!(
                SignedObject::decode(&altered).is_err(),
                "accepted mutation at {offset}"
            );
        }
        let mut trailing = encoded;
        trailing.push(0);
        assert!(SignedObject::decode(&trailing).is_err());
    }
}

#[test]
fn confirmation_and_negotiation_roles_reject_impossible_senders() {
    let a = device(1);
    let c = device(3);
    let (low, high) = if a.device_id() < c.device_id() {
        (&a, &c)
    } else {
        (&c, &a)
    };
    for (sender, target, kind, valid) in [
        (low, high, EncryptedKind::ConfirmHello, true),
        (high, low, EncryptedKind::ConfirmHello, false),
        (low, high, EncryptedKind::ConfirmFinish, true),
        (high, low, EncryptedKind::ConfirmFinish, false),
        (high, low, EncryptedKind::ConfirmProof, true),
        (low, high, EncryptedKind::ConfirmProof, false),
    ] {
        let mut packet = encrypted(sender, target.device_id());
        let Body::Encrypted { header, .. } = &mut packet.body else {
            panic!()
        };
        header.kind = kind;
        header.offer = None;
        let result = SignedObject::issue(scope(), sender, packet.id, 1000, 16_000, packet.body);
        assert_eq!(result.is_ok(), valid);
        if let Ok(value) = result {
            roundtrip(&value);
        }
    }
    for kind in [EncryptedKind::Answer, EncryptedKind::Ice] {
        for offer_issuer in [low.device_id(), high.device_id(), device(9).device_id()] {
            let mut packet = encrypted(low, high.device_id());
            let Body::Encrypted { header, .. } = &mut packet.body else {
                panic!()
            };
            header.kind = kind;
            header.offer.as_mut().unwrap().issuer = offer_issuer;
            let result = SignedObject::issue(scope(), low, packet.id, 1000, 16_000, packet.body);
            let valid = offer_issuer == high.device_id()
                || (kind == EncryptedKind::Ice && offer_issuer == low.device_id());
            assert_eq!(result.is_ok(), valid);
            if let Ok(value) = result {
                roundtrip(&value);
            }
        }
    }
}

#[test]
fn targeted_public_controls_cannot_use_room_permits_or_another_recipient() {
    let a = device(1);
    let b = device(2);
    let c = device(3);
    for object in controls(&a, &c) {
        let target = object.body.target().unwrap();
        let signer = if object.issuer == a.device_id() {
            &a
        } else {
            &c
        };
        let direct = Envelope::Direct(object.clone());
        assert!(
            direct.authorize(signer.device_id(), &context(target), |_| true)
                == Ok(Disposition::Deliver)
        );
        assert!(direct
            .authorize(signer.device_id(), &context(b.device_id()), |_| true)
            .is_err());
        let mut permit = RoutePermit::issue(signer, &object, b.device_id(), 1000, 16_000).unwrap();
        assert!(permit.final_scope == FinalScope::Endpoint(target));
        let delivery = Envelope::HelperDelivery {
            object: object.clone(),
            permit: permit.clone(),
        };
        assert!(
            delivery.authorize(b.device_id(), &context(target), |_| true)
                == Ok(Disposition::Deliver)
        );
        permit.final_scope = FinalScope::Room;
        permit.signature = signer.sign(&permit.unsigned());
        assert!(
            RoutePermit::decode(&permit.encode()).is_ok(),
            "malicious issuer can sign a structurally valid room permit"
        );
        let confused = Envelope::ToHelper { object, permit };
        assert!(
            confused
                .authorize(signer.device_id(), &context(b.device_id()), |_| true)
                .is_err(),
            "only Announcement may match room scope"
        );
    }
}

#[test]
fn an_internal_signer_failure_never_constructs_a_verified_object() {
    struct WrongSigner(DeviceKeypair);
    impl VoiceSigner for WrongSigner {
        fn voice_public_key(&self) -> Result<[u8; 32], VoiceError> {
            Ok(self.0.verifying_key().to_bytes())
        }
        fn sign_voice(&self, _: &[u8]) -> Result<[u8; 64], VoiceError> {
            Ok([0; 64])
        }
    }
    let signer = WrongSigner(device(1));
    assert!(SignedObject::issue(
        scope(),
        &signer,
        [1; 32],
        1000,
        31_000,
        Body::Announcement {
            key: key(1),
            epoch: 7,
            renewal: 1
        }
    )
    .is_err());
}

#[test]
fn native_context_requires_active_supported_policy_and_current_local_membership() {
    use catcoms_rt::{Hub, ManualClock, PeerId};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    let device = catcoms_mls::MlsDevice::generate().unwrap();
    let group = catcoms_mls::ServerGroup::create(&device).unwrap();
    let mut sync = crate::ChannelSync::new(
        Hub::new().join(PeerId::from_u64(1)),
        group,
        device,
        ChaCha20Rng::seed_from_u64(77),
        Box::new(ManualClock::new(1000)),
    );
    let legacy = sync.voice_v2_context(123).unwrap();
    assert!(legacy.scope.policy.is_none());
    assert!(!legacy.indirect);
    sync.initialize_group_policy(crate::GroupMode::PeerToPeer)
        .unwrap();
    assert!(
        sync.voice_v2_context(123).is_err(),
        "pending persistence is not absent legacy policy"
    );
    sync.snapshot().unwrap();
    sync.publish_group_policy().unwrap();
    let active = sync.voice_v2_context(123).unwrap();
    assert!(active.scope.policy.is_some());
    assert!(active.indirect);
    assert!(active.scope != legacy.scope);
    sync.group_policy = Some(
        catcoms_mls::GroupPolicy::issue(&sync.group, &sync.device, crate::GroupMode::Dedicated)
            .unwrap(),
    );
    assert!(
        sync.voice_v2_context(123).is_err(),
        "unsupported dedicated mode cannot become direct legacy eligibility"
    );
    sync.group_policy_active = false;
    assert!(sync.voice_v2_context(123).is_err());
    sync.group_policy = None;
    sync.device = catcoms_mls::MlsDevice::generate().unwrap();
    assert!(
        sync.voice_v2_context(123).is_err(),
        "an active group object does not admit a local nonmember"
    );
}

#[test]
fn header_binds_aad_but_ciphertext_has_its_own_original_signature() {
    let a = device(1);
    let c = device(3);
    let packet = encrypted(&a, c.device_id());
    let mut altered = packet.clone();
    let Body::Encrypted { ciphertext, .. } = &mut altered.body else {
        panic!()
    };
    ciphertext[0] ^= 1;
    assert_eq!(packet.aad().unwrap(), altered.aad().unwrap());
    assert!(SignedObject::decode(&altered.encode()).is_err());
    let mut altered = packet.clone();
    let Body::Encrypted { header, .. } = &mut altered.body else {
        panic!()
    };
    header.target_join[0] ^= 1;
    assert_ne!(packet.aad().unwrap(), altered.aad().unwrap());
    assert!(SignedObject::decode(&altered.encode()).is_err());
    assert!(announcement(&a, 1).aad().is_err());
}

#[test]
fn expiry_is_original_signed_time_and_cannot_be_renewed_by_arrival() {
    let a = device(1);
    let object = announcement(&a, 1);
    assert!(object.check_freshness(36_000).is_ok());
    assert_eq!(object.check_freshness(36_001), Err(VoiceError::Freshness));
    assert!(object.check_freshness(0).is_ok()); // <=5s permitted wall-clock skew, not liveness.
    let future =
        SignedObject::issue(scope(), &a, [1; 32], 10_000, 40_000, object.body.clone()).unwrap();
    assert_eq!(future.check_freshness(4999), Err(VoiceError::Freshness));
    assert!(SignedObject::issue(scope(), &a, [1; 32], 1000, 31_001, object.body.clone()).is_err());
    assert!(SignedObject::issue(scope(), &a, [1; 32], 1000, 1000, object.body.clone()).is_err());
}

#[test]
fn pair_context_is_canonical_and_stable_across_renewals_but_not_joins_or_scope() {
    let a = device(1);
    let c = device(3);
    let first = announcement(&a, 1);
    let peer = announcement(&c, 2);
    let transcript = pair_transcript(&first, &peer).unwrap();
    assert_eq!(transcript, pair_transcript(&peer, &first).unwrap());
    let renewal = SignedObject::issue(
        scope(),
        &a,
        [91; 32],
        5000,
        35_000,
        Body::Announcement {
            key: key(1),
            epoch: 8,
            renewal: 2,
        },
    )
    .unwrap();
    assert_eq!(transcript, pair_transcript(&renewal, &peer).unwrap());
    assert_ne!(
        transcript,
        pair_transcript(&announcement(&a, 3), &peer).unwrap()
    );
    assert!(pair_transcript(&first, &first).is_err());
    let mut wrong = peer.clone();
    wrong.scope.room += 1;
    assert!(pair_transcript(&first, &wrong).is_err());
    assert!(pair_transcript(&first, &encrypted(&c, a.device_id())).is_err());
}

#[test]
fn direct_and_two_edge_roles_bind_the_original_issuer_and_exact_final_member() {
    let a = device(1);
    let b = device(2);
    let c = device(3);
    let d = device(4);
    let members = [a.device_id(), b.device_id(), c.device_id(), d.device_id()];
    let member = |id| members.contains(&id);
    let object = encrypted(&a, c.device_id());
    let direct = Envelope::Direct(object.clone());
    assert!(
        direct.authorize(a.device_id(), &context(c.device_id()), member)
            == Ok(Disposition::Deliver)
    );
    assert!(direct
        .authorize(b.device_id(), &context(c.device_id()), member)
        .is_err());
    assert!(direct
        .authorize(a.device_id(), &context(b.device_id()), member)
        .is_err());
    let permit = RoutePermit::issue(&a, &object, b.device_id(), 1000, 16_000).unwrap();
    let to = Envelope::ToHelper {
        object: object.clone(),
        permit: permit.clone(),
    };
    let delivery = Envelope::HelperDelivery {
        object: object.clone(),
        permit: permit.clone(),
    };
    assert!(
        to.authorize(a.device_id(), &context(b.device_id()), member)
            == Ok(Disposition::Forward(FinalScope::Endpoint(c.device_id())))
    );
    assert!(
        delivery.authorize(b.device_id(), &context(c.device_id()), member)
            == Ok(Disposition::Deliver)
    );
    for requester in [b.device_id(), c.device_id(), d.device_id()] {
        assert!(to
            .authorize(requester, &context(b.device_id()), member)
            .is_err());
    }
    for requester in [a.device_id(), c.device_id(), d.device_id()] {
        assert!(delivery
            .authorize(requester, &context(c.device_id()), member)
            .is_err());
    }
    assert!(to
        .authorize(a.device_id(), &context(c.device_id()), member)
        .is_err());
    assert!(delivery
        .authorize(b.device_id(), &context(b.device_id()), member)
        .is_err());
    assert!(delivery
        .authorize(b.device_id(), &context(d.device_id()), member)
        .is_err());
    for envelope in [to, delivery] {
        let encoded = envelope.encode();
        let decoded = Envelope::decode(&encoded).unwrap();
        assert!(decoded.object() == &object);
        let mut trailing = encoded;
        trailing.push(1);
        assert!(Envelope::decode(&trailing).is_err());
    }
}

#[test]
fn helper_replacement_changes_only_the_permit_not_endpoint_identity_or_ciphertext() {
    let a = device(1);
    let b = device(2);
    let c = device(3);
    let d = device(4);
    let object = encrypted(&a, c.device_id());
    let first = RoutePermit::issue(&a, &object, b.device_id(), 1000, 16_000).unwrap();
    let replacement = RoutePermit::issue(&a, &object, d.device_id(), 2000, 16_000).unwrap();
    assert!(first != replacement);
    assert!(first.packet == replacement.packet);
    assert!(first.issuer == replacement.issuer);
    assert!(RoutePermit::issue(&b, &object, d.device_id(), 1000, 16_000).is_err());
    let wrong = encrypted(&c, a.device_id());
    assert!(first.matches(&wrong).is_err());
    assert!(RoutePermit::issue(&a, &object, b.device_id(), 1000, 16_001).is_err());
}

#[test]
fn room_permits_are_only_for_announcements_and_role_confusion_refuses() {
    let a = device(1);
    let b = device(2);
    let c = device(3);
    let object = announcement(&a, 1);
    let permit = RoutePermit::issue(&a, &object, b.device_id(), 1000, 16_000).unwrap();
    assert!(permit.final_scope == FinalScope::Room);
    let envelope = Envelope::ToHelper {
        object: object.clone(),
        permit: permit.clone(),
    };
    assert!(
        envelope.authorize(a.device_id(), &context(b.device_id()), |_| true)
            == Ok(Disposition::Forward(FinalScope::Room))
    );
    let delivery = Envelope::HelperDelivery { object, permit };
    assert!(
        delivery.authorize(b.device_id(), &context(c.device_id()), |_| true)
            == Ok(Disposition::Deliver)
    );
    for helper in [a.device_id(), c.device_id()] {
        assert!(
            RoutePermit::issue(&a, &encrypted(&a, c.device_id()), helper, 1000, 16_000).is_err()
        );
    }
    let packet = encrypted(&a, c.device_id());
    let mut forged = RoutePermit::issue(&a, &packet, b.device_id(), 1000, 16_000).unwrap();
    forged.final_scope = FinalScope::Room;
    assert!(forged.matches(&packet).is_err());
    assert!(RoutePermit::decode(&forged.encode()).is_err());
}

#[test]
fn membership_policy_group_room_and_epoch_are_independent_admission_checks() {
    let a = device(1);
    let b = device(2);
    let c = device(3);
    let object = announcement(&a, 1);
    let permit = RoutePermit::issue(&a, &object, b.device_id(), 1000, 16_000).unwrap();
    let envelope = Envelope::HelperDelivery { object, permit };
    for removed in [a.device_id(), b.device_id(), c.device_id()] {
        assert!(envelope
            .authorize(b.device_id(), &context(c.device_id()), |id| id != removed)
            .is_err());
    }
    let mut ctx = context(c.device_id());
    ctx.indirect = false;
    assert!(envelope.authorize(b.device_id(), &ctx, |_| true).is_err());
    for dimension in 0..3 {
        let mut ctx = context(c.device_id());
        match dimension {
            0 => ctx.scope.group[0] ^= 1,
            1 => ctx.scope.policy = Some([55; 32]),
            _ => ctx.scope.room += 1,
        };
        assert!(envelope.authorize(b.device_id(), &ctx, |_| true).is_err());
    }
    let mut ctx = context(c.device_id());
    ctx.epoch = 6;
    assert!(envelope.authorize(b.device_id(), &ctx, |_| true).is_err());
    let mut unverified = scope();
    unverified.policy = None;
    let direct_only = SignedObject::issue(
        unverified,
        &a,
        [1; 32],
        1000,
        31_000,
        Body::Announcement {
            key: key(1),
            epoch: 7,
            renewal: 1,
        },
    )
    .unwrap();
    assert!(RoutePermit::issue(&a, &direct_only, b.device_id(), 1000, 16_000).is_err());
    let mut ctx = context(c.device_id());
    ctx.scope = unverified;
    ctx.indirect = false;
    assert!(
        Envelope::Direct(direct_only).authorize(a.device_id(), &ctx, |_| true)
            == Ok(Disposition::Deliver)
    );
}

#[test]
fn bounds_invalid_tags_suites_points_lengths_and_nested_envelopes_refuse() {
    assert_eq!(
        Envelope::decode(&vec![0; MAX_ENVELOPE + 1]).err(),
        Some(VoiceError::Bounds)
    );
    assert!(Envelope::decode(&[1, 1, 0, 0, 0, 0]).is_err());
    assert!(Envelope::decode(&[2, 4]).is_err());
    let a = device(1);
    let c = device(3);
    let packet = encrypted(&a, c.device_id());
    let direct = Envelope::Direct(packet.clone()).encode();
    let mut nested = Encoder::new();
    nested.put_u8(2);
    nested.put_u8(1);
    fixed(&mut nested, &direct);
    assert!(Envelope::decode(&nested.finish()).is_err());
    let mut bad = key(1);
    bad.public_key[0] = 3;
    assert!(bad.validate().is_err());
    let mut bad = key(1);
    bad.key_ref[0] ^= 1;
    assert!(bad.validate().is_err());
    for (kind, len) in [
        (EncryptedKind::Offer, 32 * 1024 + 17),
        (EncryptedKind::Ice, 2 * 1024 + 17),
        (EncryptedKind::ConfirmHello, 529),
        (EncryptedKind::Bye, 145),
    ] {
        let mut object = packet.clone();
        let Body::Encrypted { header, ciphertext } = &mut object.body else {
            panic!()
        };
        header.kind = kind;
        header.offer = match kind {
            EncryptedKind::Offer | EncryptedKind::Ice => header.offer,
            _ => None,
        };
        *ciphertext = vec![0; len];
        assert!(object.validate().is_err());
    }
    let mut object = packet.clone();
    let Body::Encrypted { header, .. } = &mut object.body else {
        panic!()
    };
    header.sequence = 0;
    assert!(object.validate().is_err());
    let mut object = packet.clone();
    let Body::Encrypted { header, .. } = &mut object.body else {
        panic!()
    };
    header.low_to_high = !header.low_to_high;
    assert!(object.validate().is_err());
    let mut object = packet;
    let Body::Encrypted { header, .. } = &mut object.body else {
        panic!()
    };
    header.offer = None;
    assert!(object.validate().is_err());
}

#[test]
fn permit_bytes_are_signed_and_cannot_extend_original_scope_or_lifetime() {
    let a = device(1);
    let b = device(2);
    let c = device(3);
    let packet = encrypted(&a, c.device_id());
    let permit = RoutePermit::issue(&a, &packet, b.device_id(), 1000, 16_000).unwrap();
    let bytes = permit.encode();
    for offset in 0..bytes.len() {
        let mut changed = bytes.clone();
        changed[offset] ^= 1;
        assert!(
            RoutePermit::decode(&changed).is_err(),
            "accepted permit mutation at {offset}"
        );
    }
    let mut early = permit.clone();
    early.issued = 999;
    assert!(early.matches(&packet).is_err());
    let mut late = permit;
    late.expires += 1;
    assert!(late.matches(&packet).is_err());
}
