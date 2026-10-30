use super::*;
use catcoms_rt::{Hub, ManualClock};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn owner_evidence_private_profile_comes_from_actual_found_and_never_mutates_or_exports() {
    let mut owner = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(985)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(985),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let original = owner.group.epoch_authenticator_id();
    let a = owner.prepare_owner_evidence_checkpoint(9).unwrap();
    let b = owner.prepare_owner_evidence_checkpoint(10).unwrap();
    assert!(a.profile.verify());
    assert_eq!(
        a.profile_id(),
        b.profile_id(),
        "channel grants cannot choose a different group root"
    );
    assert_eq!(a.group(), owner.group_id());
    assert_eq!(a.founder().as_slice(), owner.device.public_key_bytes());
    assert_eq!(a.initial_channel(), 9);
    assert_eq!(owner.group.epoch_authenticator_id(), original);
    assert!(owner.outbox.is_empty());
    assert!(a.matches_original_mls_state(&owner));
    // Ordinary snapshot restore cannot recover this process-local fresh source or self-pin.
    let snapshot = owner.snapshot().unwrap();
    let restored = ChannelSync::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(986)),
        ChaCha20Rng::seed_from_u64(986),
        Box::new(ManualClock::new(2_000)),
    )
    .unwrap();
    assert!(matches!(
        restored.prepare_owner_evidence_checkpoint(9),
        Err(OwnerEvidencePrepareError::Context)
    ));
    assert!(
        a.matches_original_mls_state(&restored),
        "MLS equality alone is intentionally not the original physical/native lease"
    );
    let _scope_source = owner.take_fresh_chat_admission().unwrap();
    assert!(a.matches_original_mls_state(&owner));
    assert!(
        matches!(
            owner.prepare_owner_evidence_checkpoint(9),
            Err(OwnerEvidencePrepareError::Context)
        ),
        "a consumed source cannot mint another preparation even at unchanged epoch"
    );
    let newcomer = MlsDevice::generate().unwrap();
    let package = newcomer
        .key_package_for_invite(&owner.group_id(), [14; 16])
        .unwrap();
    owner
        .with_observed_mls_transition(|node| node.group.add_member(&node.device, package))
        .unwrap();
    assert!(!a.matches_original_mls_state(&owner));
    assert!(matches!(
        owner.prepare_owner_evidence_checkpoint(9),
        Err(OwnerEvidencePrepareError::Context)
    ));
}

#[test]
fn owner_evidence_preparation_types_remain_send_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<PreparedOwnerEvidence>();
}

/// This is the real-key/actual-transition verifier boundary. It does not claim the absent
/// canonical Store/publication transaction or automatic invite root distribution is implemented.
#[test]
fn owner_evidence_actual_mls_admission_chain_verifies_on_member_and_fresh_restore() {
    use catcoms_replication::chat::{
        owner_facts::{OwnerBasisFacts, OwnerProfileExpectation},
        ActionSpec, ActionState, Body, ChatLedger, LedgerLimits, PendingReason, Realm, Rights,
        SignedAction,
    };
    let mut owner = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(990)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(990),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let prepared = owner.prepare_owner_evidence_checkpoint(9).unwrap();
    // Explicit trusted actual-founder handoff BEFORE join. Never derive this expectation from
    // a returned Profile/Welcome/roster or from the archive being restored below.
    let expected = OwnerProfileExpectation::from_trusted_host(
        prepared.group(),
        *prepared.founder(),
        prepared.profile_id(),
    )
    .unwrap();
    let member = MlsDevice::generate().unwrap();
    let package = member
        .key_package_for_invite(&owner.group_id(), [20; 16])
        .unwrap();
    let expected_kp = member.key_package_reference(&package).unwrap();
    let (outcome, observed) = owner
        .with_observed_mls_transition(|node| node.group.add_member_observed(&node.device, package))
        .unwrap();
    assert!(observed.matches_current(&owner.group, &owner.device));
    assert_eq!(observed.key_package_reference(), Some(expected_kp));
    let member_group =
        ServerGroup::join_for_key_package(&member, &outcome.welcome, &expected_kp).unwrap();
    assert!(member_group.is_active());
    assert!(member_group.has_local_signer(&member.public_key_bytes()));
    assert_eq!(member_group.group_id(), prepared.group());
    let sign_fact = |body| {
        SignedOwnerFact::sign_using(
            OwnerFactSpec {
                group: prepared.group().to_vec(),
                profile: Some(prepared.profile_id()),
                body,
            },
            *prepared.founder(),
            |bytes| owner.device.sign(bytes).map_err(|_| ChatError::Signature),
        )
        .unwrap()
    };
    let admission = sign_fact(OwnerFactBody::Admission {
        subject: *observed.subject(),
        predecessor: prepared.profile_id(),
        origin: AdmissionOrigin::Add {
            commit: observed.commit_digest(),
            before_epoch: observed.before_epoch(),
            after_epoch: observed.after_epoch(),
            key_package: observed.key_package_reference().unwrap(),
            credential: observed.credential_digest().unwrap(),
        },
    });
    let grant = sign_fact(OwnerFactBody::RealmGrant {
        channel: 9,
        admission: admission.id(),
        subject: *observed.subject(),
        rights: Rights::MEMBER,
    });
    let basis = sign_fact(OwnerFactBody::Basis {
        channel: 9,
        admission: admission.id(),
        subject: *observed.subject(),
        grant: grant.grant_id().unwrap(),
        covered: vec![],
    });
    let action = SignedAction::sign_using(
        ActionSpec {
            realm: Realm::new(&member_group.group_id(), 9).unwrap(),
            basis: basis.basis_id().unwrap(),
            grant: grant.grant_id().unwrap(),
            nonce: [20; 16],
            presented_ms: 1_000,
            parents: vec![],
            body: Body::Create {
                text: "actual member original evidence".into(),
                reply_to: None,
            },
        },
        *observed.subject(),
        |bytes| member.sign(bytes).map_err(|_| ChatError::Signature),
    )
    .unwrap();
    let witness = expected
        .verify_basis(
            &prepared.profile,
            OwnerBasisFacts {
                admission: &admission,
                grant: &grant,
                basis: &basis,
            },
            &[],
        )
        .unwrap();
    let limits = LedgerLimits {
        max_actions: 64,
        max_action_bytes: 1024 * 1024,
    };
    let mut ledger = ChatLedger::new(action.spec().realm.clone(), limits).unwrap();
    ledger.import(action.clone()).unwrap();
    ledger.authorize(action.id(), witness).unwrap();
    assert!(!ledger.advance(32).remaining);
    assert_eq!(ledger.state(action.id()), Some(ActionState::Valid));

    // Original public facts/action and the actual member provider are independently restored.
    // These bytes carry no expected-root or saved semantic-authority flag.
    let frames = [&prepared.profile, &admission, &grant, &basis].map(SignedOwnerFact::encode);
    let mls = catcoms_mls::snapshot_server(&member, &member_group).unwrap();
    let (restored_member, restored_group) = catcoms_mls::restore_server(&mls).unwrap();
    assert!(restored_group.has_local_signer(&restored_member.public_key_bytes()));
    let original = ledger.snapshot().unwrap().encode();
    let mut restored = ChatLedger::restore(&original, limits, &[]).unwrap();
    assert!(!restored.advance(32).remaining);
    assert_eq!(
        restored.state(action.id()),
        Some(ActionState::Pending(PendingReason::Authority))
    );
    let facts = frames
        .iter()
        .map(|frame| SignedOwnerFact::decode(frame).unwrap())
        .collect::<Vec<_>>();
    restored
        .authorize(
            action.id(),
            expected
                .verify_basis(
                    &facts[0],
                    OwnerBasisFacts {
                        admission: &facts[1],
                        grant: &facts[2],
                        basis: &facts[3],
                    },
                    &[],
                )
                .unwrap(),
        )
        .unwrap();
    assert!(!restored.advance(32).remaining);
    assert_eq!(restored.state(action.id()), Some(ActionState::Valid));
    assert_eq!(
        restored.action(action.id()).unwrap().encode(),
        action.encode()
    );
    assert_eq!(
        restored_member.public_key_bytes(),
        observed.subject().as_slice()
    );

    // Current membership removal does not rewrite historical semantic authorization. Exact
    // action acceptance/held projection must be decided separately from these ordinary rights.
    let removed = owner
        .with_observed_mls_transition(|node| {
            node.group
                .remove_member_observed(&node.device, &member.device_id())
        })
        .unwrap();
    assert!(removed.matches_current(&owner.group, &owner.device));
    assert!(expected
        .verify_basis(
            &facts[0],
            OwnerBasisFacts {
                admission: &facts[1],
                grant: &facts[2],
                basis: &facts[3],
            },
            &[]
        )
        .is_ok());
}

/// The critical tail is original evidence only. Neither raw restore nor public legacy owner
/// controls can use it as Current/canonical authority; refusals precede mutating/signing work.
#[tokio::test]
async fn owner_evidence_raw_tail_is_strict_unbound_and_refuses_positive_owner_controls() {
    let mut owner = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(0xe001)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0xe001),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    owner.initialize_new_group_policy().unwrap();
    let prepared = owner.prepare_owner_evidence_checkpoint(9).unwrap();
    owner.stage_managed_chat_scope([1; 32], [2; 32]).unwrap();
    owner.stage_owner_evidence_checkpoint(&prepared).unwrap();
    let bytes = owner.snapshot().unwrap();
    let mut restored = ChannelSync::restore(
        &bytes,
        Hub::new().join(PeerId::from_u64(0xe002)),
        ChaCha20Rng::seed_from_u64(0xe002),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert!(restored.owner_evidence_unbound());
    assert!(restored.durable_send_context().is_err());
    let epoch = restored.group.epoch();
    let admin = restored.device.device_id();
    assert!(restored.open_channel(DocType::Channel, 9).await.is_err());
    let edited = std::cell::Cell::new(false);
    assert!(restored
        .post(DocType::Channel, 9, |_| {
            edited.set(true);
            Ok(())
        })
        .await
        .is_err());
    assert!(!edited.get());
    assert!(restored.subscribe_control().await.is_err());
    assert!(restored.run_once().await.is_err());
    assert!(restored.set_admin(&admin, true).await.is_err());
    assert!(restored.mint_invite([2; 16], 10_000, vec![]).is_err());
    assert!(restored.publish_group_policy().is_err());
    assert!(restored.request_remove(&admin).await.is_err());
    assert!(restored.remove(&admin).await.is_err());
    assert_eq!(restored.group.epoch(), epoch);
    assert!(restored.docs.is_empty());
    assert!(restored.admin_roster.is_empty());
    assert!(restored.outbox.is_empty());
    assert_eq!(restored.snapshot().unwrap().as_slice(), bytes.as_slice());
    let mut trailing = bytes.to_vec();
    trailing.extend_from_slice(&0u32.to_be_bytes());
    assert!(ChannelSync::restore(
        &trailing,
        Hub::new().join(PeerId::from_u64(0xe003)),
        ChaCha20Rng::seed_from_u64(0xe003),
        Box::new(ManualClock::new(1_000)),
    )
    .is_err());
    let mut unknown = bytes.to_vec();
    let tag = b"catcoms/owner-evidence-initial/v1";
    let at = unknown
        .windows(tag.len())
        .position(|window| window == tag)
        .unwrap();
    unknown[at + tag.len() - 1] = b'9';
    assert!(ChannelSync::restore(
        &unknown,
        Hub::new().join(PeerId::from_u64(0xe004)),
        ChaCha20Rng::seed_from_u64(0xe004),
        Box::new(ManualClock::new(1_000)),
    )
    .is_err());
}
