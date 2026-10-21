//! Coordinator fixtures use synthetic grants; actual actor barriers live in the native tests.
use super::*;

fn point() -> [u8; 65] {
    let mut point = [1; 65];
    point[0] = 4;
    point
}

#[test]
fn selected_join_key_lost_admission_recovers_one_paid_operation_under_pressure() {
    let mut c = action_coordinator();
    let controller = controller(&mut c);
    let owner = call(&mut c, &controller, 1, 1);
    let first = c.admit_join_key(&owner, [9; 32]).unwrap();
    let baseline = (c.serial, c.resources.counts());
    let group = c.owner(&owner).unwrap().binding.scope.group;
    let pressure = c
        .resources
        .charge(group, 256 * 1024 - baseline.1 .0, ResourceKind::Retained)
        .unwrap();
    for _ in 0..4 {
        let retry = c.admit_join_key(&owner, [9; 32]).unwrap();
        assert_eq!(
            (retry.permit, retry.join, retry.kind),
            (first.permit.clone(), first.join.clone(), "admitted")
        );
        assert_eq!(c.serial, baseline.0);
    }
    assert!(c.admit_join_key(&owner, [8; 32]).is_err());
    drop(pressure);
    assert_eq!(c.resources.counts(), baseline.1);
    assert!(c.settle_join_key(&owner, [9; 32]).unwrap().is_none());
    assert_eq!(c.admit_join_key(&owner, [9; 32]).unwrap().kind, "closed");
}

#[test]
fn selected_join_key_settlement_before_admission_irreversibly_closes_one_selection() {
    let mut c = action_coordinator();
    let controller = controller(&mut c);
    let owner = call(&mut c, &controller, 1, 1);
    c.settle_join_key(&owner, [9; 32]).unwrap();
    let baseline = (c.serial, c.resources.counts());
    for _ in 0..4 {
        assert_eq!(c.admit_join_key(&owner, [9; 32]).unwrap().kind, "closed");
        c.settle_join_key(&owner, [9; 32]).unwrap();
        assert!(c.admit_join_key(&owner, [8; 32]).is_err());
        assert!(c.settle_join_key(&owner, [8; 32]).is_err());
    }
    assert_eq!((c.serial, c.resources.counts()), baseline);
    assert_eq!(c.join_generations.len(), 1);
    c.end(&owner);
    assert_eq!(c.resources.counts(), (0, 0));
    c.settle_join_key("unknown", [1; 32]).unwrap();
    assert!(c.join_generations.is_empty());
    assert!(c.admit_join_key(&owner, [9; 32]).is_err());
}

#[test]
fn selected_join_key_end_preserves_uncertain_credit_and_cannot_refund_replacement() {
    let mut c = action_coordinator();
    let controller = controller(&mut c);
    let owner = call(&mut c, &controller, 1, 1);
    c.admit_join_key(&owner, [9; 32]).unwrap();
    let original = c.resources.counts();
    c.end(&owner);
    assert_eq!(c.resources.counts(), original);
    let next = call(&mut c, &controller, 1, 2);
    let second = c.admit_join_key(&next, [9; 32]).unwrap();
    assert!(c.settle_join_key(&owner, [8; 32]).is_err());
    c.settle_join_key(&owner, [9; 32]).unwrap();
    assert_eq!(c.resources.counts(), original);
    assert_eq!(
        c.admit_join_key(&next, [9; 32]).unwrap().permit,
        second.permit
    );
    c.settle_join_key(&owner, [9; 32]).unwrap();
    assert_eq!(c.resources.counts(), original);
    c.revoke_all();
    assert_eq!(c.resources.counts(), original);
    c.settle_join_key(&next, [9; 32]).unwrap();
    assert_eq!(c.resources.counts(), (0, 0));
}

#[test]
fn selected_join_key_stale_publication_completion_cannot_retire_a_new_flight() {
    let mut c = action_coordinator();
    let controller = controller(&mut c);
    let owner = call(&mut c, &controller, 1, 1);
    let admitted = c.admit_join_key(&owner, [9; 32]).unwrap();
    let PublicationPlan::Start(mut old) = c
        .prepare_join_key_publication(&owner, [9; 32], &admitted.permit, point())
        .unwrap()
    else {
        panic!("first work")
    };
    assert!(c.finish_join_key_publication(&mut old, false).is_err());
    let PublicationPlan::Start(mut next) = c
        .prepare_join_key_publication(&owner, [9; 32], &admitted.permit, point())
        .unwrap()
    else {
        panic!("retry work")
    };
    let mut changed = point();
    changed[1] ^= 1;
    assert!(c
        .prepare_join_key_publication(&owner, [9; 32], &admitted.permit, changed)
        .is_err());
    let drained = c.settle_join_key(&owner, [9; 32]).unwrap().unwrap();
    let baseline = c.resources.counts();
    assert!(!*drained.borrow());
    assert!(c.finish_join_key_publication(&mut old, true).is_err());
    assert_eq!(c.resources.counts(), baseline);
    assert!(!*drained.borrow());
    assert!(c.finish_join_key_publication(&mut next, false).is_err());
    assert!(*drained.borrow());
    assert_eq!(c.resources.counts().0, baseline.0 - 128);
    assert!(c
        .prepare_join_key_publication(&owner, [9; 32], &admitted.permit, point())
        .is_err());
}

#[test]
fn selected_join_key_preparation_capacity_does_not_burn_generation_or_serial() {
    let mut c = action_coordinator();
    let controller = controller(&mut c);
    let owner = call(&mut c, &controller, 1, 1);
    let group = c.owner(&owner).unwrap().binding.scope.group;
    let pressure = c
        .resources
        .charge(group, 256 * 1024 - 64, ResourceKind::Retained)
        .unwrap();
    let serial = c.serial;
    assert!(c.admit_join_key(&owner, [9; 32]).is_err());
    assert_eq!(c.serial, serial);
    assert!(c.join_generations.is_empty());
    drop(pressure);
    c.admit_join_key(&owner, [9; 32]).unwrap();
    assert_eq!(c.serial, serial + 1);
}
