use super::*;

fn handoff(f: &mut Fixture) -> u64 {
    let next = MlsDevice::generate().unwrap();
    let welcome = f
        .group
        .add_member(&f.owner, next.key_package().unwrap())
        .unwrap()
        .welcome;
    let mut group = ServerGroup::join(&next, &welcome).unwrap();
    group.remove_member(&next, &f.owner.device_id()).unwrap();
    f.owner = next;
    f.group = group;
    f.restart();
    f.group.epoch()
}

#[test]
fn studio_frozen_owner_takeover_preserves_source_resumes_journal_and_recovers_whole_version() {
    for art in [false, true] {
        for adopting in [false, true] {
            let mut f = Fixture::new(art);
            f.fill();
            let first = f.decide(None);
            let plan = f.plan(&first);
            f.source = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
            f.fill();
            let old = f.decide(Some(first.receipt()));
            if adopting {
                f.source
                    .begin_checkpoint_adoption(old.receipt().clone(), &f.group, 0)
                    .unwrap();
            } else {
                f.source.seal(old.receipt().clone(), &f.group, 0).unwrap();
            }
            assert!(f
                .source
                .frozen_owner_decision(&f.group, &f.owner, 0, None, None)
                .is_err());
            let original = f.source.projection().unwrap();
            let count = f.source.op_count();
            let tenure = handoff(&mut f);
            assert!(f.source.owner_rotation_needs_adoption(tenure));
            assert!(f
                .source
                .new_owner_decision(&f.group, &f.owner, tenure, Some(old.receipt()))
                .is_err());
            let (decision, seed) = f
                .source
                .frozen_owner_decision(
                    &f.group,
                    &f.owner,
                    tenure,
                    Some(old.receipt()),
                    Some(old.close()),
                )
                .unwrap();
            assert_eq!(
                decision.receipt().inherited,
                InheritedCheckpoint::Checkpoint {
                    epoch: 1,
                    close_record_hash: first.receipt().close_record_hash,
                    seed_change_hash: first.receipt().seed_change_hash,
                }
            );
            assert_eq!(f.source.phase(), EpochPhase::Closing);
            assert_eq!(f.source.projection().unwrap(), original);
            assert_eq!(f.source.op_count(), count);
            // Journal saved, but new seal not saved: exact retry must still use the persisted
            // close. The old head cannot authorize recomputing an irrevocable choice.
            f.restart();
            let (retry, retry_seed) = f
                .source
                .frozen_owner_decision(
                    &f.group,
                    &f.owner,
                    tenure,
                    Some(decision.receipt()),
                    Some(decision.close()),
                )
                .unwrap();
            assert_eq!(retry.receipt(), decision.receipt());
            assert_eq!(retry_seed.bytes(), seed.bytes());
            assert!(f
                .source
                .frozen_owner_decision(&f.group, &f.owner, tenure, Some(decision.receipt()), None)
                .is_err());
            f.source
                .begin_checkpoint_adoption(decision.receipt().clone(), &f.group, tenure)
                .unwrap();
            f.restart();
            assert!(
                f.source
                    .frozen_owner_decision(&f.group, &f.owner, tenure, None, None)
                    .is_err(),
                "lost current journal is not permission to sign a replacement"
            );
            let (retry, retry_seed) = f
                .source
                .frozen_owner_decision(
                    &f.group,
                    &f.owner,
                    tenure,
                    Some(decision.receipt()),
                    Some(decision.close()),
                )
                .unwrap();
            assert_eq!(retry.receipt(), decision.receipt());
            assert_eq!(retry_seed.bytes(), seed.bytes());
            let plan = f
                .source
                .prepare_checkpoint_adoption(decision.receipt(), seed.bytes(), &f.group, tenure)
                .unwrap();
            let recovered = StudioRecovery::from_snapshot(
                plan.recovery_snapshot().unwrap(),
                &f.source.logical,
                f.source.target.channel(),
            )
            .unwrap();
            assert_eq!(recovered.projection(), &original);
            assert_eq!(f.source.op_count(), count);
            f.source = f.source.adopted_successor(&plan, &f.group, tenure).unwrap();
            f.restart();
            assert_eq!(f.source.epoch(), 2);
            assert_eq!(f.source.phase(), EpochPhase::Open);
            assert!(!f.source.owner_rotation_needs_adoption(tenure));
            f.fill();
            let later = f
                .source
                .new_owner_decision(&f.group, &f.owner, tenure, Some(decision.receipt()))
                .unwrap();
            assert_eq!(later.receipt().inherited, decision.receipt().inherited);
        }
    }
}

#[test]
fn studio_frozen_owner_takeover_refuses_fault_and_underfull_held_source() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        f.fill();
        let old = f.decide(None);
        f.source.seal(old.receipt().clone(), &f.group, 0).unwrap();
        let alt = Receipt::sign(
            f.source.logical.clone(),
            0,
            [8; 32],
            [9; 32],
            0,
            InheritedCheckpoint::EpochZero,
            &f.owner,
        )
        .unwrap();
        assert_eq!(
            f.source.seal(alt, &f.group, 0).unwrap(),
            ReceiptIngest::Fault
        );
        let tenure = handoff(&mut f);
        assert!(f
            .source
            .frozen_owner_decision(
                &f.group,
                &f.owner,
                tenure,
                Some(old.receipt()),
                Some(old.close())
            )
            .is_err());
        assert_eq!(f.source.phase(), EpochPhase::Fault);

        let mut f = Fixture::new(art);
        let hint = Receipt::sign(
            f.source.logical.clone(),
            3,
            [8; 32],
            [9; 32],
            0,
            InheritedCheckpoint::EpochZero,
            &f.owner,
        )
        .unwrap();
        f.source
            .begin_checkpoint_adoption(hint, &f.group, 0)
            .unwrap();
        let tenure = handoff(&mut f);
        assert!(matches!(
            f.source
                .frozen_owner_decision(&f.group, &f.owner, tenure, None, None),
            Err(ReplError::EpochBound)
        ));
        assert_eq!(f.source.phase(), EpochPhase::Closing);
        assert_eq!(
            f.source.op_count(),
            1,
            "a missing remote seed never permits clearing held work"
        );
    }
}
