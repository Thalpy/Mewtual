use super::*;

#[test]
fn registry_frozen_owner_takeover_uses_installed_baseline_and_whole_source_recovery() {
    for adopting in [false, true] {
        let mut f = Fixture::new();
        f.fill();
        let first = f.decide(None);
        f.install(&first);
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
        let next = MlsDevice::generate().unwrap();
        let welcome = f
            .group
            .add_member(&f.owner, next.key_package().unwrap())
            .unwrap()
            .welcome;
        let mut group = ServerGroup::join(&next, &welcome).unwrap();
        group.remove_member(&next, &f.owner.device_id()).unwrap();
        let tenure = group.epoch();
        let source = f.source.snapshot().unwrap();
        f.source =
            RegistryEpoch::restore(&source, &group, f.key.bucket(), next.device_id()).unwrap();
        assert!(f.source.owner_rotation_needs_adoption(tenure));
        let (decision, seed) = f
            .source
            .frozen_owner_decision(
                &group,
                &next,
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
        assert_eq!(f.source.projection().unwrap(), original);
        assert_eq!(f.source.op_count(), count);
        let (retry, retry_seed) = f
            .source
            .frozen_owner_decision(
                &group,
                &next,
                tenure,
                Some(decision.receipt()),
                Some(decision.close()),
            )
            .unwrap();
        assert_eq!(retry.receipt(), decision.receipt());
        assert_eq!(retry_seed.bytes(), seed.bytes());
        f.source
            .begin_checkpoint_adoption(decision.receipt().clone(), &group, tenure)
            .unwrap();
        let source = f.source.snapshot().unwrap();
        f.source =
            RegistryEpoch::restore(&source, &group, f.key.bucket(), next.device_id()).unwrap();
        assert!(f
            .source
            .frozen_owner_decision(&group, &next, tenure, None, None)
            .is_err());
        let (retry, retry_seed) = f
            .source
            .frozen_owner_decision(
                &group,
                &next,
                tenure,
                Some(decision.receipt()),
                Some(decision.close()),
            )
            .unwrap();
        assert_eq!(retry.receipt(), decision.receipt());
        assert_eq!(retry_seed.bytes(), seed.bytes());
        let plan = f
            .source
            .prepare_checkpoint_adoption(decision.receipt(), seed.bytes(), &group, tenure)
            .unwrap();
        let recovery = crate::registry::RegistryRecovery::from_snapshot(
            plan.recovery_snapshot().unwrap(),
            &f.source.logical,
            f.key.bucket(),
        )
        .unwrap();
        assert_eq!(recovery.projection(), &original);
        f.source = f.source.adopted_successor(&plan, &group, tenure).unwrap();
        let source = f.source.snapshot().unwrap();
        f.source =
            RegistryEpoch::restore(&source, &group, f.key.bucket(), next.device_id()).unwrap();
        assert_eq!(f.source.epoch(), 2);
        assert_eq!(f.source.phase(), EpochPhase::Open);
    }
}
