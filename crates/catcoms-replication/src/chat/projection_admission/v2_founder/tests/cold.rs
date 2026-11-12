//! Historical replay is nonissuing until a genuine new owner binds the exact target.
//! HOST expectations here are deliberate trusted fixtures, not App cold-Current evidence.
use super::*;

fn cursor(f: &Fixture) -> V2FounderReplayCursor {
    V2FounderReplayCursor::from_genesis(
        V2FounderExpectation::from_trusted_host(f.anchor()).unwrap(),
        &f.profile,
        &f.founder,
        &f.header,
        &f.image,
    )
    .unwrap()
}

fn signed_append(
    f: &Fixture,
    base: &ReplayMath,
    record: &AcceptanceRecord,
) -> (SignedOwnerHeader, AppendBody, Vec<u8>) {
    let slot = base.slot(f.founder.id()).unwrap();
    let leaves = vec![record.leaf()];
    let appended = slot.frontier.prepare_append(&leaves).unwrap();
    let body = AppendBody::new(AppendSpec {
        group: GROUP.to_vec(),
        previous_tip: slot.tip_header,
        old_tip: slot.frontier.tip(),
        new_tip: appended.new_tip(),
        leaves,
    })
    .unwrap();
    let plan = base.prepare_append_data(&body).unwrap();
    let header =
        SignedOwnerHeader::sign_using(plan.header_spec_data().clone(), |b| Ok(f.key.sign(b)))
            .unwrap();
    (header, body, plan.image_data().to_vec())
}

fn fresh_group(group: &[u8]) -> (ProjectionGroup, ProjectionGroupView) {
    let mut owner = ProjectionGroup::new(group).unwrap();
    let mut view = owner.begin_view();
    assert!(view.try_ready());
    (owner, view)
}

#[test]
fn cold_history_replays_without_an_effect_owner_then_binds_exact_target_once() {
    let f = Fixture::new();
    f.negative.live.store(false, Ordering::Release);
    assert!(!f.origin.live());
    // There is deliberately no group/origin parameter or issuer API on this cursor.
    let mut replay = cursor(&f);
    let action = f.create(81);
    let record = f.record(&action);
    let (header, body, image) = signed_append(&f, replay.replay_data(), &record);
    let retired = replay.replay_append(&header, &body, &image).unwrap();
    assert_eq!(retired.parent_data().header().id(), f.header.id());
    assert!(retired.append_data().is_some());
    drop(retired);
    let (_new_owner, new_group) = fresh_group(GROUP);
    let negative = Negative::new();
    let origin = V2ProjectionOrigin::new(negative.clone());
    let selected = V2HostSelectionExpectation::from_trusted_host(header.id(), header.spec().state);
    let mut replay = Some(replay);
    let bound = V2FounderPrefixVerifier::bind_replayed(&mut replay, &selected, &new_group, &origin)
        .unwrap();
    assert!(replay.is_none());
    assert_eq!(bound.replay_data().image(), image);
    assert!(
        V2FounderPrefixVerifier::bind_replayed(&mut replay, &selected, &new_group, &origin)
            .is_err()
    );
    let (mut ledger, old_view) = f.ledger();
    assert!(bound
        .accepted_create(&action, &record, 0, &[], &old_view)
        .is_err());
    let mut view = ledger.begin_projection_view_in(&new_group).unwrap();
    assert!(view.try_ready());
    assert!(bound
        .accepted_create(&action, &record, 0, &[], &view)
        .is_ok());
    negative.live.store(false, Ordering::Release);
    assert!(bound
        .accepted_create(&action, &record, 0, &[], &view)
        .is_err());
}

#[test]
fn cold_bind_refusals_preserve_same_cursor_and_preallocated_identity() {
    let f = Fixture::new();
    for case in 0..6 {
        let mut replay = Some(cursor(&f));
        let identity = Arc::downgrade(&replay.as_ref().unwrap().identity);
        let (_owner, view) = fresh_group(if case == 2 { b"wrong-group" } else { GROUP });
        if case == 5 {
            drop(view.revoker().close());
        }
        let negative = Negative::new();
        if case == 3 {
            negative.live.store(false, Ordering::Release);
        } else if case == 4 {
            // Preliminary source check passes; the FINAL cached source check refuses.
            negative.checks_left.store(1, Ordering::Release);
        }
        let selected = V2HostSelectionExpectation::from_trusted_host(
            if case == 0 {
                OwnerHeaderId::from_bytes([0; 32])
            } else {
                f.header.id()
            },
            if case == 1 {
                OwnerStateCommitment::from_bytes([0; 32])
            } else {
                f.header.spec().state
            },
        );
        assert!(
            V2FounderPrefixVerifier::bind_replayed(
                &mut replay,
                &selected,
                &view,
                &V2ProjectionOrigin::new(negative),
            )
            .is_err(),
            "case {case}"
        );
        let original = replay.as_ref().unwrap();
        assert!(identity.ptr_eq(&Arc::downgrade(&original.identity)));
        assert_eq!(original.replay_data().header().id(), f.header.id());
        assert_eq!(original.replay_data().image(), f.image);
    }
}

#[test]
fn cold_replay_rejects_wrong_signature_image_duplicate_and_skipped_successor() {
    let f = Fixture::new();
    let mut replay = cursor(&f);
    let (header, body, image) = signed_append(&f, replay.replay_data(), &f.record(&f.create(82)));
    let mut bytes = header.encode();
    *bytes.last_mut().unwrap() ^= 1;
    let invalid_signature = SignedOwnerHeader::decode(&bytes).unwrap();
    assert_eq!(invalid_signature.id(), header.id());
    assert!(replay
        .replay_append(&invalid_signature, &body, &image)
        .is_err());
    let mut changed = image.clone();
    *changed.last_mut().unwrap() ^= 1;
    assert!(replay.replay_append(&header, &body, &changed).is_err());
    assert_eq!(replay.replay_data().header().id(), f.header.id());

    let mut ahead = cursor(&f);
    drop(ahead.replay_append(&header, &body, &image).unwrap());
    let (next, next_body, next_image) =
        signed_append(&f, ahead.replay_data(), &f.record(&f.create(83)));
    assert!(replay
        .replay_append(&next, &next_body, &next_image)
        .is_err());
    assert_eq!(replay.replay_data().header().id(), f.header.id());
    drop(replay.replay_append(&header, &body, &image).unwrap());
    assert!(replay.replay_append(&header, &body, &image).is_err());
    assert_eq!(replay.replay_data().header().id(), header.id());
    drop(
        replay
            .replay_append(&next, &next_body, &next_image)
            .unwrap(),
    );
    assert_eq!(replay.replay_data().header().spec().sequence, 3);
}

#[test]
fn cold_cursor_requires_full_authentic_genesis_and_cannot_rebind_old_candidates() {
    let f = Fixture::new();
    let mut anchor = f.anchor();
    anchor.profile = OwnerFactId::from_bytes([0; 32]);
    assert!(V2FounderReplayCursor::from_genesis(
        V2FounderExpectation::from_trusted_host(anchor).unwrap(),
        &f.profile,
        &f.founder,
        &f.header,
        &f.image,
    )
    .is_err());
    let mut raw = f.header.encode();
    *raw.last_mut().unwrap() ^= 1;
    assert!(V2FounderReplayCursor::from_genesis(
        V2FounderExpectation::from_trusted_host(f.anchor()).unwrap(),
        &f.profile,
        &f.founder,
        &SignedOwnerHeader::decode(&raw).unwrap(),
        &f.image,
    )
    .is_err());
    let old = f.verifier();
    let candidate = f.append(&old, &[f.record(&f.create(84))]);
    let selected_child = V2HostSelectionExpectation::from_trusted_host(
        candidate.header_data().id(),
        candidate.header_data().spec().state,
    );
    f.negative.live.store(false, Ordering::Release);
    let (_new_owner, new_group) = fresh_group(GROUP);
    let mut replay = Some(cursor(&f));
    let selected_genesis =
        V2HostSelectionExpectation::from_trusted_host(f.header.id(), f.header.spec().state);
    let mut bound = V2FounderPrefixVerifier::bind_replayed(
        &mut replay,
        &selected_genesis,
        &new_group,
        &V2ProjectionOrigin::new(Negative::new()),
    )
    .unwrap();
    let mut candidate = Some(candidate);
    assert!(bound
        .advance_host_selected(&selected_child, &mut candidate)
        .is_err());
    assert!(
        candidate.is_some(),
        "old unconsumed candidate stays owned on refusal"
    );
    assert_eq!(bound.replay_data().header().id(), f.header.id());
}
