//! Actual signed public replay fixtures. These prove pre-sign/signed equivalence, not an
//! observed MLS Add, source authentication, owner signing permission or durable installation.
use super::*;

fn complete(
    base: &ReplayMath,
    key: &DeviceKeypair,
    body: &AdmissionBody,
) -> (SignedOwnerHeader, ReplayMath) {
    let plan = base.prepare_admission_data(body).unwrap();
    let header = SignedOwnerHeader::sign_using(plan.header_spec_data().clone(), |bytes| {
        assert_eq!(bytes, plan.signing_bytes_data());
        Ok(key.sign(bytes))
    })
    .unwrap();
    let next = base
        .derive_successor(&header, ReplayBody::Admission(body), plan.image_data())
        .unwrap()
        .next;
    (header, next)
}

#[test]
fn unsigned_add_matches_independent_received_image_and_real_signed_replay() {
    let (key, base) = genesis();
    let (body, expected, image) = add(&base, &key, [81; 32], 82);
    let original = base.image().to_vec();
    let original_header = base.header().clone();
    let founder = base.slots().next().unwrap().image.admission;
    let founder_tip = base.slot(founder).unwrap().tip_header;
    let first = base.prepare_admission_data(&body).unwrap();
    assert_eq!(first.image_data(), image);
    assert_eq!(first.header_spec_data(), expected.spec());
    assert_eq!(first.signing_bytes_data(), expected.semantic_bytes());
    for _ in 0..3 {
        let retry = base.prepare_admission_data(&body).unwrap();
        assert_eq!(
            retry.intent_commitment_data(),
            first.intent_commitment_data()
        );
        assert_eq!(retry.signing_bytes_data(), first.signing_bytes_data());
        assert_eq!(retry.image_data(), first.image_data());
    }
    let (header, next) = complete(&base, &key, &body);
    let added = next.slot(body.id()).unwrap();
    assert_eq!(added.tip_header, header.id());
    assert_eq!(added.image.tip_body, body.id());
    assert_eq!(added.image.accepted_count, 0);
    assert_eq!(
        added.image.accepted_root,
        TreeContext {
            profile: base.profile().id(),
            admission: body.admission_id()
        }
        .empty_hash()
    );
    assert_eq!(next.slot(founder).unwrap().tip_header, founder_tip);
    assert_eq!(base.image(), original);
    assert_eq!(base.header(), &original_header);
    assert_eq!(base.slots().len(), 1);
    assert_eq!(format!("{first:?}"), "UnsignedAdmissionStepData { .. }");
    const { assert!(UnsignedAdmissionStepData::named_layout_bytes() <= UNSIGNED_ADMISSION_DATA_INVOICE) };
}

#[test]
fn unsigned_add_refuses_founder_foreign_context_bad_epoch_duplicate_and_signed_wrong_predecessor() {
    let (key, base) = genesis();
    let (body, _, _) = add(&base, &key, [83; 32], 84);
    let original = base.image().to_vec();
    for field in 0..5 {
        let mut spec = body.spec().clone();
        match field {
            0 => spec.group = b"foreign-group".to_vec(),
            1 => spec.profile = OwnerFactId::from_bytes([85; 32]),
            2 => spec.origin = AdmissionOrigin::Founder { creation: [86; 32] },
            3 => {
                spec.origin = AdmissionOrigin::Add {
                    commit: [84; 32],
                    before_epoch: 1,
                    after_epoch: 2,
                    key_package: [85; 32],
                    credential: [86; 32],
                }
            }
            _ => spec.subject = base.profile().spec().founder,
        }
        assert!(base
            .prepare_admission_data(&AdmissionBody::new(spec).unwrap())
            .is_err());
        assert_eq!(base.image(), original);
    }
    let plan = base.prepare_admission_data(&body).unwrap();
    for field in 0..8 {
        let mut spec = plan.header_spec_data().clone();
        match field {
            0 => spec.parent = OwnerHeaderId::from_bytes([87; 32]),
            1 => spec.sequence += 1,
            2 => spec.tenure = OwnerTenureId::from_bytes([88; 32]),
            3 => spec.profile = OwnerFactId::from_bytes([89; 32]),
            4 => spec.body = OwnerBodyId::from_bytes([90; 32]),
            5 => spec.kind = HeaderKind::AcceptanceAppend,
            6 => spec.state = OwnerStateCommitment::from_bytes([91; 32]),
            _ => spec.group = b"foreign-group".to_vec(),
        }
        let signed = SignedOwnerHeader::sign_using(spec, |bytes| Ok(key.sign(bytes))).unwrap();
        assert!(base
            .derive_successor(&signed, ReplayBody::Admission(&body), plan.image_data())
            .is_err());
        assert_eq!(base.image(), original);
    }
    let mut invalid =
        SignedOwnerHeader::sign_using(plan.header_spec_data().clone(), |bytes| Ok(key.sign(bytes)))
            .unwrap()
            .encode();
    *invalid.last_mut().unwrap() ^= 1;
    let invalid = SignedOwnerHeader::decode(&invalid).unwrap();
    assert!(matches!(
        base.derive_successor(&invalid, ReplayBody::Admission(&body), plan.image_data()),
        Err(ChatError::Signature)
    ));
    let (_, next) = complete(&base, &key, &body);
    assert!(matches!(
        next.prepare_admission_data(&body),
        Err(ChatError::Dependency) | Err(ChatError::AuthorityConflict)
    ));
    assert_eq!(base.image(), original);
}

#[test]
fn interleaved_append_makes_old_add_parent_stale_and_preserves_other_admissions_own_tip() {
    let (key, base) = genesis();
    let founder = base.slots().next().unwrap().image.admission;
    let (first, _, _) = add(&base, &key, [91; 32], 92);
    let (first_header, both) = complete(&base, &key, &first);
    let (second, _, _) = add(&both, &key, [93; 32], 94);
    let stale = both.prepare_admission_data(&second).unwrap();
    let stale_header =
        SignedOwnerHeader::sign_using(
            stale.header_spec_data().clone(),
            |bytes| Ok(key.sign(bytes)),
        )
        .unwrap();
    let (append_body, append_header, append_image) = append(&both, &key, founder, 95);
    let after_append = both
        .derive_successor(
            &append_header,
            ReplayBody::Append(&append_body),
            &append_image,
        )
        .unwrap()
        .next;
    assert!(after_append
        .derive_successor(
            &stale_header,
            ReplayBody::Admission(&second),
            stale.image_data()
        )
        .is_err());
    let fresh = after_append.prepare_admission_data(&second).unwrap();
    assert_ne!(
        fresh.intent_commitment_data(),
        stale.intent_commitment_data()
    );
    assert_eq!(fresh.header_spec_data().parent, append_header.id());
    assert_eq!(stale.header_spec_data().parent, first_header.id());
    let (_, three) = complete(&after_append, &key, &second);
    assert_eq!(
        three.slot(first.id()).unwrap().tip_header,
        first_header.id()
    );
    assert_eq!(three.slot(founder).unwrap().tip_header, append_header.id());
    assert!(three.slot(first.id()).unwrap().image == after_append.slot(first.id()).unwrap().image);
    assert_eq!(both.slot(founder).unwrap().image.accepted_count, 0);
    assert_eq!(after_append.slot(founder).unwrap().image.accepted_count, 1);
}

#[test]
fn unsigned_add_full_64_frame_refuses_growth_and_same_key_rejoin_is_new_incarnation() {
    let (key, mut source) = genesis();
    let mut first = None;
    for marker in 1..64 {
        // The neighboring genuine genesis fixture uses seed8 for the founder. Keep this
        // member range disjoint, otherwise marker8 would correctly refuse as a duplicate.
        let member = DeviceKeypair::from_seed(&[marker + 64; 32]);
        assert_ne!(
            member.verifying_key().to_bytes(),
            source.profile().spec().founder
        );
        let (body, expected, image) = add(&source, &key, member.verifying_key().to_bytes(), marker);
        let plan = source.prepare_admission_data(&body).unwrap();
        assert_eq!(plan.image_data(), image);
        assert_eq!(plan.header_spec_data(), expected.spec());
        let (_, next) = complete(&source, &key, &body);
        if marker == 1 {
            first = Some(body);
        };
        source = next;
        assert_eq!(source.slots.capacity(), MAX_SLOTS);
    }
    assert_eq!(source.slots().len(), 64);
    let before = source.image().to_vec();
    let full = AdmissionBody::new(AdmissionSpec {
        group: GROUP.to_vec(),
        profile: source.profile().id(),
        subject: [101; 32],
        origin: AdmissionOrigin::Add {
            commit: [102; 32],
            before_epoch: source.epoch,
            after_epoch: source.epoch + 1,
            key_package: [103; 32],
            credential: [104; 32],
        },
    })
    .unwrap();
    assert!(matches!(
        source.prepare_admission_data(&full),
        Err(ChatError::Bound)
    ));
    assert_eq!(source.image(), before);
    let first = first.unwrap();
    let (cut, header, image) = remove(&source, &key, first.id());
    let after_remove = source
        .derive_successor(&header, ReplayBody::Removal(&cut), &image)
        .unwrap()
        .next;
    let (rejoin, _, _) = add(&after_remove, &key, first.spec().subject, 105);
    assert_ne!(first.id(), rejoin.id());
    let plan = after_remove.prepare_admission_data(&rejoin).unwrap();
    assert_eq!(
        StateFrame::decode(plan.image_data())
            .unwrap()
            .slots()
            .count(),
        64
    );
    assert!(plan.image_data().len() <= MAX_STATE_BYTES);
    let (_, rejoined) = complete(&after_remove, &key, &rejoin);
    assert!(rejoined.slot(first.id()).is_none());
    assert!(rejoined.slot(rejoin.id()).is_some());
    assert_eq!(rejoined.slots.capacity(), MAX_SLOTS);
    assert_eq!(source.image(), before);
}

#[test]
fn unsigned_add_checked_sequence_and_epoch_exhaustion_refuse_without_touching_source() {
    let (key, mut source) = genesis();
    let (body, _, _) = add(&source, &key, [106; 32], 107);
    // Explicit arithmetic boundary instrumentation, not a claim to replay2^64 Headers.
    let original_header = source.header.clone();
    let mut spec = source.header.spec().clone();
    spec.sequence = u64::MAX;
    source.header = SignedOwnerHeader::sign_using(spec, |bytes| Ok(key.sign(bytes))).unwrap();
    let before = source.image().to_vec();
    let header = source.header.id();
    assert!(matches!(
        source.prepare_admission_data(&body),
        Err(ChatError::Dependency)
    ));
    assert_eq!(source.header.id(), header);
    assert_eq!(source.image(), before);
    assert!(matches!(
        AdmissionBody::new(AdmissionSpec {
            group: GROUP.to_vec(),
            profile: source.profile().id(),
            subject: [108; 32],
            origin: AdmissionOrigin::Add {
                commit: [109; 32],
                before_epoch: u64::MAX,
                after_epoch: 0,
                key_package: [110; 32],
                credential: [111; 32]
            }
        }),
        Err(ChatError::Dependency)
    ));
    source.header = original_header;
    source.epoch = u64::MAX;
    assert!(source.prepare_admission_data(&body).is_err());
    assert_eq!(source.image(), before);
}
