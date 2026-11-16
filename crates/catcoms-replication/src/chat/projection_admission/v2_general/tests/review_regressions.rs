//! Regression cases from independent review of d9e3b8d8. All acceptance and
//! removal evidence goes through the actual signed governing replay.
use super::*;

/// One genuine four-leaf Append, with exact inclusion siblings from the
/// production frontier's created nodes. Repeated one-leaf fixture calls would
/// not establish a cumulative accepted root or these multi-leaf proofs.
fn accept_four(
    f: &mut Fixture,
    admission: &V2AuthenticatedAdmission,
    actions: [&SignedAction; 4],
) -> Vec<(AcceptanceRecord, Vec<TreeHash>)> {
    let context = TreeContext {
        profile: admission.core.body.spec().profile,
        admission: admission.core.body.admission_id(),
    };
    let mut records: Vec<_> = actions
        .into_iter()
        .map(|action| {
            AcceptanceRecord::new(AcceptanceSpec {
                group: GROUP.to_vec(),
                context,
                channel: action.spec().realm.channel(),
                action: action.id(),
                subject: *action.signer(),
                grant: action.spec().grant,
                basis: action.spec().basis,
                create: action.spec().body.create(),
            })
            .unwrap()
        })
        .collect();
    records.sort_by_key(AcceptanceRecord::leaf);
    let base = f.verifier.replay_data();
    let slot = base.slot(admission.core.body.id()).unwrap();
    assert_eq!(slot.frontier.tip().count, 0);
    let leaves: Vec<_> = records.iter().map(AcceptanceRecord::leaf).collect();
    let prepared = slot.frontier.prepare_append(&leaves).unwrap();
    let proofs: Vec<_> = (0u64..4)
        .map(|index| {
            let node = |first, count| {
                prepared
                    .created_nodes()
                    .iter()
                    .find(|node| node.first == first && node.count == count)
                    .unwrap()
                    .hash
            };
            vec![node(index ^ 1, 1), node(if index < 2 { 2 } else { 0 }, 2)]
        })
        .collect();
    let body = AppendBody::new(AppendSpec {
        group: GROUP.to_vec(),
        previous_tip: slot.tip_header,
        old_tip: prepared.old_tip(),
        new_tip: prepared.new_tip(),
        leaves,
    })
    .unwrap();
    let mut slots: Vec<_> = base.slots().map(|slot| slot.image).collect();
    let selected = slots
        .iter_mut()
        .find(|slot| slot.admission == admission.core.body.id())
        .unwrap();
    selected.tip_body = body.id();
    selected.accepted_count = prepared.new_tip().count;
    selected.accepted_root = prepared.new_tip().root;
    let image = f.image(HeaderKind::AcceptanceAppend, body.id(), f.epoch(), slots);
    let header = f.owner.header(&image);
    drop(f.advance(&header, ReplayBody::Append(&body), &image));
    records.into_iter().zip(proofs).collect()
}

#[test]
fn removed_nonwinning_provisional_edit_invalidates_captured_body_selection() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[173; 32]);
    let admission = f.add(&key, 73);
    let c = create(&key, &admission, 1);
    let unrelated = create(&key, &admission, 2);
    // Find a deterministic CID ordering using genuinely signed actions. The
    // presentation timestamps must not decide the winner or the removal cut.
    let mut concurrent: Vec<_> = (3..11)
        .map(|nonce| {
            action(
                &key,
                &admission,
                nonce,
                Body::Edit {
                    create: c.id(),
                    text: format!("accepted branch {nonce}"),
                },
                vec![c.id()],
            )
        })
        .collect();
    concurrent.sort_by_key(SignedAction::id);
    let e1 = concurrent.pop().unwrap();
    let e3 = concurrent.remove(3);
    let e2 = (0..=255)
        .map(|nonce| {
            action(
                &key,
                &admission,
                nonce,
                Body::Edit {
                    create: c.id(),
                    text: "provisional descendant".into(),
                },
                vec![e1.id()],
            )
        })
        .find(|edit| edit.id() < e3.id())
        .unwrap();
    assert!(e1.id() > e3.id() && e3.id() > e2.id());
    let (mut ledger, view) = f.ledger();
    retain(&f, &mut ledger, &c, &admission, &[]);
    retain(&f, &mut ledger, &unrelated, &admission, &[]);
    for edit in [&e1, &e3] {
        let dependencies = [V2OrdinaryDependency::new(&c, &admission)];
        retain(&f, &mut ledger, edit, &admission, &dependencies);
        install(&f, &mut ledger, &view, edit, &admission, &dependencies);
    }
    let e2_deps = ordered_dependencies(vec![(&c, &admission), (&e1, &admission)]);
    retain(&f, &mut ledger, &e2, &admission, &e2_deps);
    install(&f, &mut ledger, &view, &e2, &admission, &e2_deps);
    let accepted = accept_four(&mut f, &admission, [&c, &e1, &e3, &unrelated]);
    for (index, (record, proof)) in accepted.iter().enumerate() {
        let action = [&c, &e1, &e3, &unrelated]
            .into_iter()
            .find(|action| action.id() == record.spec().action)
            .unwrap();
        let dependencies = if action.spec().body.create().is_some() {
            vec![V2OrdinaryDependency::new(&c, &admission)]
        } else {
            vec![]
        };
        let token = f
            .verifier
            .accepted_action(
                action,
                V2GeneralAcceptanceSource::Current(&admission),
                &dependencies,
                record,
                index as u64,
                proof,
                &view,
            )
            .unwrap();
        ledger.admit_projection(&token, 512).unwrap();
    }
    let captured = ledger.guarded_projection(c.id(), 512).unwrap();
    let independent = ledger.guarded_projection(unrelated.id(), 512).unwrap();
    let mut completion = None;
    // Already-admitted publication is allowed to finish. Removal closes new
    // admission immediately, while its completion waits for this original reader.
    captured.with_publication(|result| {
        assert!(matches!(result, GuardedProjection::Visible { selected, .. } if *selected == e3.id()));
        let mut retired = f.remove(&admission);
        let mut close = retired.take_close_completion().unwrap();
        assert!(!close.try_complete());
        completion = Some(close);
    }).unwrap();
    assert!(completion.unwrap().try_complete());
    assert!(captured.with_publication(|_| ()).is_err());
    assert!(ledger
        .guarded_projection(c.id(), 512)
        .unwrap()
        .with_publication(|result| matches!(result,
            GuardedProjection::Visible { selected, .. } if *selected == e1.id()))
        .unwrap());
    assert!(independent
        .with_publication(|result| matches!(result,
        GuardedProjection::Visible { selected, .. } if *selected == unrelated.id()))
        .unwrap());
    for original in [&c, &e1, &e2, &e3, &unrelated] {
        assert_eq!(ledger.state(original.id()), Some(ActionState::Valid));
    }
}

#[test]
fn completed_original_group_covers_old_admission_without_waiting_on_replacement_readers() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[174; 32]);
    let admission = f.add(&key, 74);
    let c = create(&key, &admission, 1);
    let (mut ledger, view) = f.ledger();
    retain(&f, &mut ledger, &c, &admission, &[]);
    install(&f, &mut ledger, &view, &c, &admission, &[]);
    let captured = ledger.guarded_projection(c.id(), 128).unwrap();
    let mut close = None;
    captured
        .with_publication(|_| {
            let mut retired = f.remove(&admission);
            let mut removed = retired.take_close_completion().unwrap();
            assert!(
                !removed.try_complete(),
                "original admitted reader has not drained"
            );
            close = Some(removed);
        })
        .unwrap();
    let mut original_drain = f.group_view.revoker().close();
    assert!(original_drain.try_complete());
    let mut replacement = f._group.begin_view();
    assert!(replacement.try_ready());
    // This is the actual shared publication gate retained by the genuinely ready
    // replacement. No fake completion bit or equal-label replacement is supplied.
    let _replacement_reader = replacement.life.gate.read().unwrap();
    let mut close = close.unwrap();
    assert!(
        close.try_complete(),
        "G1's completed drain covers its own Admission"
    );
    assert!(close.try_complete());
}
