//! Actual observed Add and authenticated exact-KP finisher. These crypto-boundary tests do not
//! claim the owner's still-separate canonical save-before-Welcome transaction has executed.
use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn actual_selected_kp_member_initialization_restores_through_closed_provider_strategy_without_authority(
) {
    let mut f = fixture();
    let verified = verify(&f.member, &f.expected, &f.facts).unwrap();
    f.member
        .stage_managed_chat_scope([21; 32], [22; 32])
        .unwrap();
    f.member
        .stage_joined_owner_evidence_checkpoint(&verified)
        .unwrap();
    let limits = crate::OwnerInitializationRestoreLimits {
        max_snapshot_bytes: 1024 * 1024,
        max_mls_bytes: 512 * 1024,
        max_mls_backing_bytes: 512 * 1024,
    };
    let bytes = f
        .member
        .snapshot_owner_initialization(
            limits.max_snapshot_bytes,
            limits.max_mls_bytes,
            limits.max_mls_backing_bytes,
        )
        .unwrap();
    let original = f.member.group.epoch_authenticator_id();
    let restored = ChannelSync::restore_owner_initialization(
        &bytes,
        Hub::new().join(PeerId::from_u64(0xf101)),
        ChaCha20Rng::seed_from_u64(0xf101),
        Box::new(ManualClock::new(2_000)),
        limits,
    )
    .unwrap();
    assert_eq!(restored.group.epoch_authenticator_id(), original);
    assert_eq!(restored.device.device_id(), f.member.device.device_id());
    assert_eq!(
        restored.owner_evidence_profile_id(),
        Some(verified.profile_id())
    );
    assert!(restored.fresh_chat_admission().is_none());
    assert!(restored.durable_send_context().is_err());
    // This proves exact-KP/codec restoration only; no remote fsync, local Current or native
    // host resource admission is inferred from these test-supplied negative caps.
}

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
struct Fixture {
    owner: Node,
    member: Node,
    before_join: MlsDevice,
    expected: OwnerProfileExpectation,
    facts: Vec<SignedOwnerFact>,
}
fn sign(owner: &Node, profile: &SignedOwnerFact, body: OwnerFactBody) -> SignedOwnerFact {
    SignedOwnerFact::sign_using(
        OwnerFactSpec {
            group: profile.spec().group.clone(),
            profile: Some(profile.id()),
            body,
        },
        *profile.issuer(),
        |bytes| owner.device.sign(bytes).map_err(|_| ChatError::Signature),
    )
    .unwrap()
}
fn admit(
    owner: &mut Node,
    device: MlsDevice,
    profile: &SignedOwnerFact,
    nonce: u8,
) -> (Node, Vec<SignedOwnerFact>) {
    let invite = owner.mint_invite([nonce; 16], 10_000, Vec::new()).unwrap();
    let package = device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let reference = device.key_package_reference(&package).unwrap();
    let (outcome, observed) = owner
        .with_observed_mls_transition(|node| node.group.add_member_observed(&node.device, package))
        .unwrap();
    let routing = owner.seal_routing_state();
    let signature = owner
        .device
        .sign(&join_transcript(
            &invite.group_id,
            &invite.invite_nonce,
            &outcome.welcome,
            &routing,
        ))
        .unwrap();
    let (group, routing) = finish_join(
        &device,
        &invite,
        &outcome.welcome,
        &signature,
        &routing,
        &reference,
    )
    .unwrap();
    let mut member = ChannelSync::new_joined(
        Hub::new().join(PeerId::from_u64(0xe100 + u64::from(nonce))),
        group,
        device,
        ChaCha20Rng::seed_from_u64(0xe100 + u64::from(nonce)),
        Box::new(ManualClock::new(1_000)),
        routing,
    );
    member.require_managed_chat_scope();
    let admission = sign(
        owner,
        profile,
        OwnerFactBody::Admission {
            subject: *observed.subject(),
            predecessor: profile.id(),
            origin: AdmissionOrigin::Add {
                commit: observed.commit_digest(),
                before_epoch: observed.before_epoch(),
                after_epoch: observed.after_epoch(),
                key_package: reference,
                credential: observed.credential_digest().unwrap(),
            },
        },
    );
    let grant = sign(
        owner,
        profile,
        OwnerFactBody::RealmGrant {
            channel: 9,
            admission: admission.id(),
            subject: *observed.subject(),
            rights: Rights::MEMBER,
        },
    );
    let basis = sign(
        owner,
        profile,
        OwnerFactBody::Basis {
            channel: 9,
            admission: admission.id(),
            grant: grant.grant_id().unwrap(),
            subject: *observed.subject(),
            covered: vec![],
        },
    );
    let bootstrap = sign(
        owner,
        profile,
        OwnerFactBody::JoinBootstrap {
            key_package: reference,
            subject: *observed.subject(),
            welcome: *blake3::hash(&outcome.welcome).as_bytes(),
            expected_founder: *profile.issuer(),
        },
    );
    (
        member,
        vec![profile.clone(), admission, grant, basis, bootstrap],
    )
}
fn fixture() -> Fixture {
    let mut owner = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(0xe100)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0xe100),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let prepared = owner.prepare_owner_evidence_checkpoint(9).unwrap();
    // Explicit actual-founder trust handoff BEFORE the join. Downloaded facts never choose it.
    let expected = OwnerProfileExpectation::from_trusted_host(
        prepared.group(),
        *prepared.founder(),
        prepared.profile_id(),
    )
    .unwrap();
    let device = MlsDevice::generate().unwrap();
    let before_join = device.duplicate().unwrap();
    let (member, facts) = admit(&mut owner, device, &prepared.profile, 1);
    Fixture {
        owner,
        member,
        before_join,
        expected,
        facts,
    }
}
fn verify(
    node: &Node,
    expected: &OwnerProfileExpectation,
    facts: &[SignedOwnerFact],
) -> Result<VerifiedJoinedOwnerEvidence, OwnerEvidencePrepareError> {
    let frames: Vec<_> = facts.iter().map(SignedOwnerFact::encode).collect();
    let borrowed: Vec<_> = frames.iter().map(Vec::as_slice).collect();
    node.verify_joined_owner_evidence(expected, &borrowed, 9)
}

#[test]
fn actual_finished_member_bundle_is_private_exact_and_verification_does_not_mutate() {
    let f = fixture();
    let original = f.member.group.epoch_authenticator_id();
    let evidence = verify(&f.member, &f.expected, &f.facts).unwrap();
    assert_eq!(evidence.group(), f.member.group_id());
    assert_eq!(
        evidence.subject().as_slice(),
        f.member.device.public_key_bytes()
    );
    assert_eq!(evidence.profile_id(), f.facts[0].id());
    assert_eq!(evidence.initial_channel(), 9);
    assert_eq!(evidence.facts.len(), 5);
    assert!(evidence.matches_original(&f.member));
    assert!(!evidence.matches_original(&f.owner));
    let exact_retry = verify(&f.member, &f.expected, &f.facts).unwrap();
    assert!(evidence.equivalent_proof(&exact_retry));
    let reordered: Vec<_> = f.facts.iter().rev().cloned().collect();
    let equivalent_retry = verify(&f.member, &f.expected, &reordered).unwrap();
    assert!(evidence.equivalent_proof(&equivalent_retry));
    assert!(equivalent_retry.equivalent_proof(&evidence));
    assert_eq!(f.member.group.epoch_authenticator_id(), original);
    assert!(f.member.outbox.is_empty());
    assert!(f.member.durable_send_context().is_err());
    assert_eq!(
        format!("{evidence:?}"),
        "VerifiedJoinedOwnerEvidence { .. }"
    );
}

#[test]
fn wrong_expected_root_or_same_key_other_actual_admission_cannot_borrow_old_bundle() {
    let mut f = fixture();
    let evidence = verify(&f.member, &f.expected, &f.facts).unwrap();
    let wrong = OwnerProfileExpectation::from_trusted_host(
        evidence.group(),
        [99; 32],
        evidence.profile_id(),
    )
    .unwrap();
    assert!(verify(&f.member, &wrong, &f.facts).is_err());
    f.owner
        .with_observed_mls_transition(|node| {
            node.group
                .remove_member_observed(&node.device, &f.member.device.device_id())
        })
        .unwrap();
    // This provider was captured BEFORE A1 stored the group's state. Reuse the credential
    // nonce in this low-level crypto fixture to pin same-key AND same-credential separation;
    // the real invite service still enforces its separate consumed-nonce admission ledger.
    let (second, second_facts) = admit(&mut f.owner, f.before_join, &f.facts[0], 1);
    assert_eq!(
        second.device.public_key_bytes(),
        f.member.device.public_key_bytes()
    );
    assert_eq!(
        second.group.own_credential_digest(&second.device).unwrap(),
        f.member
            .group
            .own_credential_digest(&f.member.device)
            .unwrap()
    );
    assert_ne!(
        second
            .fresh_chat_admission()
            .unwrap()
            .key_package_reference(),
        f.member
            .fresh_chat_admission()
            .unwrap()
            .key_package_reference()
    );
    assert!(!evidence.matches_original(&second));
    assert!(verify(&second, &f.expected, &f.facts).is_err());
    let second_evidence = verify(&second, &f.expected, &second_facts).unwrap();
    assert!(second_evidence.matches_original(&second));
    assert!(!evidence.equivalent_proof(&second_evidence));
}

#[test]
fn owner_signed_kp_credential_welcome_subject_and_epoch_substitutions_refuse() {
    let f = fixture();
    for field in 0..6 {
        let mut facts = f.facts.clone();
        let index = if field < 3 { 1 } else { 4 };
        let mut body = facts[index].spec().body.clone();
        match (&mut body, field) {
            (
                OwnerFactBody::Admission {
                    origin: AdmissionOrigin::Add { key_package, .. },
                    ..
                },
                0,
            ) => *key_package = [90; 32],
            (
                OwnerFactBody::Admission {
                    origin: AdmissionOrigin::Add { credential, .. },
                    ..
                },
                1,
            ) => *credential = [91; 32],
            (
                OwnerFactBody::Admission {
                    origin:
                        AdmissionOrigin::Add {
                            before_epoch,
                            after_epoch,
                            ..
                        },
                    ..
                },
                2,
            ) => {
                *before_epoch += 1;
                *after_epoch += 1;
            }
            (OwnerFactBody::JoinBootstrap { welcome, .. }, 3) => *welcome = [92; 32],
            (OwnerFactBody::JoinBootstrap { key_package, .. }, 4) => *key_package = [93; 32],
            (OwnerFactBody::JoinBootstrap { subject, .. }, 5) => *subject = [94; 32],
            _ => unreachable!(),
        }
        facts[index] = sign(&f.owner, &facts[0], body);
        if index == 1 {
            // Keep every signed dependency internally coherent. Otherwise this would merely
            // test an old AdmissionCID dangling from Grant/Basis and miss removal of the
            // actual local credential/epoch/KP comparisons in the verifier.
            let mut grant = facts[2].spec().body.clone();
            let OwnerFactBody::RealmGrant { admission, .. } = &mut grant else {
                unreachable!()
            };
            *admission = facts[1].id();
            facts[2] = sign(&f.owner, &facts[0], grant);
            let mut basis = facts[3].spec().body.clone();
            let OwnerFactBody::Basis {
                admission, grant, ..
            } = &mut basis
            else {
                unreachable!()
            };
            *admission = facts[1].id();
            *grant = facts[2].grant_id().unwrap();
            facts[3] = sign(&f.owner, &facts[0], basis);
            if field == 0 {
                let mut bootstrap = facts[4].spec().body.clone();
                let OwnerFactBody::JoinBootstrap { key_package, .. } = &mut bootstrap else {
                    unreachable!()
                };
                *key_package = [90; 32];
                facts[4] = sign(&f.owner, &facts[0], bootstrap);
            }
            assert!(
                f.expected
                    .verify_basis(
                        &facts[0],
                        OwnerBasisFacts {
                            admission: &facts[1],
                            grant: &facts[2],
                            basis: &facts[3]
                        },
                        &[],
                    )
                    .is_ok(),
                "the wrong-source record still has a valid complete owner-signed semantic chain"
            );
        }
        assert!(
            facts[index].verify(),
            "even a real owner's valid signature cannot substitute local source field{field}"
        );
        assert!(verify(&f.member, &f.expected, &facts).is_err());
        if field == 0 {
            // A genuine source-matching Bootstrap cannot lend its KP identity to a different
            // owner-signed Admission either. This independently pins the Admission KP check.
            facts[4] = f.facts[4].clone();
            assert!(verify(&f.member, &f.expected, &facts).is_err());
        }
        assert!(
            verify(&f.member, &f.expected, &f.facts).is_ok(),
            "refusal leaves the original private source unchanged"
        );
    }
}

#[test]
fn bounded_bundle_rejects_missing_duplicate_extra_and_oversized_originals() {
    let f = fixture();
    assert!(verify(&f.member, &f.expected, &f.facts[..4]).is_err());
    let mut duplicate = f.facts.clone();
    duplicate.push(f.facts[1].clone());
    assert!(verify(&f.member, &f.expected, &duplicate).is_err());
    let mut extra = f.facts.clone();
    extra.push(sign(
        &f.owner,
        &f.facts[0],
        OwnerFactBody::Basis {
            channel: 10,
            admission: f.facts[1].id(),
            grant: f.facts[2].grant_id().unwrap(),
            subject: *f.member.fresh_chat_admission().unwrap().signer(),
            covered: vec![],
        },
    ));
    assert!(verify(&f.member, &f.expected, &extra).is_err());
    let oversized = vec![0; MAX_SMALL_FACT_BYTES + 1];
    let frames = vec![oversized.as_slice(); 5];
    assert!(matches!(
        f.member
            .verify_joined_owner_evidence(&f.expected, &frames, 9),
        Err(OwnerEvidencePrepareError::Fact(ChatError::Bound))
    ));
    // Bootstrap is outside verify_basis's three-fact authority chain, so specifically pin
    // its OWN signature check even when every KP/subject/Welcome/root field matches the source.
    let mut signed: Vec<_> = f.facts.iter().map(SignedOwnerFact::encode).collect();
    *signed[4].last_mut().unwrap() ^= 1;
    assert!(!SignedOwnerFact::decode(&signed[4]).unwrap().verify());
    let borrowed: Vec<_> = signed.iter().map(Vec::as_slice).collect();
    assert!(f
        .member
        .verify_joined_owner_evidence(&f.expected, &borrowed, 9)
        .is_err());
}

#[test]
fn only_complete_same_realm_verified_founder_coverage_is_retained() {
    let f = fixture();
    let OwnerFactBody::Profile { creation, founder } = f.facts[0].spec().body else {
        unreachable!()
    };
    let admission = sign(
        &f.owner,
        &f.facts[0],
        OwnerFactBody::Admission {
            subject: founder,
            origin: AdmissionOrigin::Founder { creation },
            predecessor: f.facts[0].id(),
        },
    );
    let grant = sign(
        &f.owner,
        &f.facts[0],
        OwnerFactBody::RealmGrant {
            channel: 9,
            admission: admission.id(),
            subject: founder,
            rights: Rights::MEMBER,
        },
    );
    let basis = sign(
        &f.owner,
        &f.facts[0],
        OwnerFactBody::Basis {
            channel: 9,
            admission: admission.id(),
            grant: grant.grant_id().unwrap(),
            subject: founder,
            covered: vec![],
        },
    );
    let mut facts = f.facts.clone();
    let mut own = facts[3].spec().body.clone();
    let OwnerFactBody::Basis { covered, .. } = &mut own else {
        unreachable!()
    };
    *covered = vec![basis.basis_id().unwrap()];
    facts[3] = sign(&f.owner, &facts[0], own);
    facts.extend([admission, grant, basis]);
    let covered = verify(&f.member, &f.expected, &facts).unwrap();
    assert_eq!(covered.facts.len(), 8);
    let original = verify(&f.member, &f.expected, &f.facts).unwrap();
    assert!(
        !original.equivalent_proof(&covered),
        "a different valid signed proof set cannot replace the cached one"
    );
    assert!(verify(&f.member, &f.expected, &facts[..7]).is_err());
    assert!(verify(&f.member, &f.expected, &facts[1..]).is_err());
    let too_many: [&[u8]; 9] = [&[]; 9];
    assert!(f
        .member
        .verify_joined_owner_evidence(&f.expected, &too_many, 9)
        .is_err());
}

#[test]
fn actual_applied_removal_and_raw_restore_cannot_mint_joined_staging_provenance() {
    let mut f = fixture();
    let evidence = verify(&f.member, &f.expected, &f.facts).unwrap();
    let snapshot = f.member.snapshot().unwrap();
    let restored = ChannelSync::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(0xe200)),
        ChaCha20Rng::seed_from_u64(0xe200),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert!(verify(&restored, &f.expected, &f.facts).is_err());
    assert!(!evidence.matches_original(&restored));
    let removed = f
        .owner
        .with_observed_mls_transition(|node| {
            node.group
                .remove_member_observed(&node.device, &f.member.device.device_id())
        })
        .unwrap();
    f.member
        .with_observed_mls_transition(|node| {
            node.group.process_incoming(&node.device, removed.commit())
        })
        .unwrap();
    assert!(!f.member.group.is_active());
    assert!(f
        .member
        .group
        .own_credential_digest(&f.member.device)
        .is_err());
    assert!(!evidence.matches_original(&f.member));
    assert!(verify(&f.member, &f.expected, &f.facts).is_err());
}

#[test]
fn decoded_local_chain_still_matches_actual_restored_credential_and_epoch() {
    let mut f = fixture();
    let snapshot = f.member.snapshot().unwrap();
    let restored = ChannelSync::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(0xe199)),
        ChaCha20Rng::seed_from_u64(0xe199),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert!(
        restored.fresh_chat_admission().is_none(),
        "restore supplies no fresh admission"
    );
    let decode = |facts: &[SignedOwnerFact]| {
        let mut encoder = Encoder::new();
        encoder
            .put_str("catcoms/owner-evidence-initial/v1")
            .unwrap();
        encoder.put_bytes(f.facts[0].id().as_bytes()).unwrap();
        encoder.put_u32(facts.len() as u32);
        let ordered: BTreeMap<_, _> = facts.iter().map(|fact| (fact.id(), fact)).collect();
        for fact in ordered.values() {
            encoder.put_bytes(&fact.encode()).unwrap();
        }
        InitialOwnerEvidence::decode(&encoder.finish(), f.member.group_id().as_slice()).unwrap()
    };
    let valid = decode(&f.facts);
    assert!(valid
        .verify_local_chain(
            &restored.group,
            &restored.device,
            f.facts[0].issuer(),
            &restored.device.public_key_bytes().try_into().unwrap()
        )
        .is_ok());
    for credential_mismatch in [true, false] {
        let mut facts = f.facts.clone();
        let mut body = facts[1].spec().body.clone();
        let OwnerFactBody::Admission {
            origin:
                AdmissionOrigin::Add {
                    credential,
                    before_epoch,
                    after_epoch,
                    ..
                },
            ..
        } = &mut body
        else {
            unreachable!()
        };
        if credential_mismatch {
            *credential = [211; 32];
        } else {
            *before_epoch += 1;
            *after_epoch += 1;
        }
        facts[1] = sign(&f.owner, &facts[0], body);
        let mut body = facts[2].spec().body.clone();
        let OwnerFactBody::RealmGrant { admission, .. } = &mut body else {
            unreachable!()
        };
        *admission = facts[1].id();
        facts[2] = sign(&f.owner, &facts[0], body);
        let mut body = facts[3].spec().body.clone();
        let OwnerFactBody::Basis {
            admission, grant, ..
        } = &mut body
        else {
            unreachable!()
        };
        *admission = facts[1].id();
        *grant = facts[2].grant_id().unwrap();
        facts[3] = sign(&f.owner, &facts[0], body);
        assert!(
            f.expected
                .verify_basis(
                    &facts[0],
                    OwnerBasisFacts {
                        admission: &facts[1],
                        grant: &facts[2],
                        basis: &facts[3]
                    },
                    &[]
                )
                .is_ok(),
            "complete signed semantic chain is valid before local-source comparison"
        );
        let decoded = decode(&facts);
        assert!(decoded
            .verify_local_chain(
                &restored.group,
                &restored.device,
                facts[0].issuer(),
                &restored.device.public_key_bytes().try_into().unwrap()
            )
            .is_err());
        assert!(!decoded.live(), "decoding never supplies runtime authority");
    }
}
