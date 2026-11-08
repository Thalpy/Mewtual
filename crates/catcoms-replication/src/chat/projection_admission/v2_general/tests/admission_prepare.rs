//! Unsigned Add DATA reaches the actual general verifier only through a genuinely signed
//! successor and explicit HOST selection. HOST fixture selection is not App Saved authority.
use super::*;

#[test]
fn unsigned_add_reaches_general_admission_only_after_real_header_and_exact_host_selection() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[128; 32]);
    let body = AdmissionBody::new(AdmissionSpec {
        group: GROUP.to_vec(),
        profile: f.verifier.replay_data().profile().id(),
        subject: key.verifying_key().to_bytes(),
        origin: AdmissionOrigin::Add {
            commit: [54; 32],
            before_epoch: 0,
            after_epoch: 1,
            key_package: [55; 32],
            credential: [56; 32],
        },
    })
    .unwrap();
    let old = f.verifier.replay_data().header().id();
    let plan = f
        .verifier
        .replay_data()
        .prepare_admission_data(&body)
        .unwrap();
    assert!(f.verifier.admission(body.admission_id()).is_err());
    assert_eq!(f.verifier.replay_data().header().id(), old);
    let header = SignedOwnerHeader::sign_using(plan.header_spec_data().clone(), |bytes| {
        assert_eq!(bytes, plan.signing_bytes_data());
        f.owner.calls.set(f.owner.calls.get() + 1);
        Ok(f.owner.key.sign(bytes))
    })
    .unwrap();
    let mut candidate = Some(
        f.verifier
            .derive_successor(&header, ReplayBody::Admission(&body), plan.image_data())
            .unwrap(),
    );
    assert!(f.verifier.admission(body.admission_id()).is_err());
    let wrong = V2HostSelectionExpectation::from_trusted_host(old, header.spec().state);
    assert!(f
        .verifier
        .advance_host_selected(&wrong, &mut candidate)
        .is_err());
    assert!(candidate.is_some());
    assert_eq!(f.verifier.replay_data().header().id(), old);
    let exact = V2HostSelectionExpectation::from_trusted_host(header.id(), header.spec().state);
    let retired = f
        .verifier
        .advance_host_selected(&exact, &mut candidate)
        .unwrap();
    assert!(candidate.is_none());
    assert_eq!(retired.parent_data().header().id(), old);
    drop(retired);
    let admission = f.verifier.admission(body.admission_id()).unwrap();
    assert_eq!(admission.admission_data(), &body);
    let (mut ledger, view) = f.ledger();
    let action = create(&key, &admission, 57);
    let owner_calls = f.owner.calls.get();
    retain(&f, &mut ledger, &action, &admission, &[]);
    install(&f, &mut ledger, &view, &action, &admission, &[]);
    assert!(ledger
        .guarded_projection(action.id(), 128)
        .unwrap()
        .with_publication(|p| matches!(
            p,
            GuardedProjection::Visible {
                certainty: ProjectionCertainty::Provisional,
                ..
            }
        ))
        .unwrap());
    assert_eq!(
        f.owner.calls.get(),
        owner_calls,
        "member action needs no further owner Header"
    );
}
