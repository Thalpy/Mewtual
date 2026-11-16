//! Coordinator fixtures use synthetic grants; actual actor barriers live in the native tests.
use super::*;
// Pressure must track the actual native policy, including future reviewed policy revisions.
// A historical256KiB filler no longer exercises capacity refusal at the current512KiB limit.
use catcoms_sync::voice_v2::resources::GROUP_BYTES;

fn point() -> [u8; 65] {
    let mut point = [1; 65];
    point[0] = 4;
    point
}

#[test]
fn native_handoff_selection_requires_actual_published_generation_and_reuses_its_owner() {
    let mut c = action_coordinator();
    let controller = controller(&mut c);
    let owner = call(&mut c, &controller, 1, 1);
    assert!(c.selected_handoff_owner(&owner, [9; 32]).is_err());
    let admission = c.admit_join_key(&owner, [9; 32]).unwrap();
    assert!(c.selected_handoff_owner(&owner, [9; 32]).is_err());
    let PublicationPlan::Start(mut work) = c
        .prepare_join_key_publication(&owner, [9; 32], &admission.permit, point())
        .unwrap()
    else {
        panic!("first publication")
    };
    assert!(c.selected_handoff_owner(&owner, [9; 32]).is_err());
    c.finish_join_key_publication(&mut work, true).unwrap();
    assert!(c.selected_handoff_owner(&owner, [8; 32]).is_err());
    let selected = c.selected_handoff_owner(&owner, [9; 32]).unwrap();
    assert!(c.is_current_handoff_owner(&selected));
    let before = c.resources.counts();
    let group = selected.binding().scope.group;
    let pressure = c
        .resources
        .charge(group, GROUP_BYTES - before.0, ResourceKind::Retained)
        .unwrap();
    assert!(Arc::ptr_eq(
        &selected,
        &c.selected_handoff_owner(&owner, [9; 32]).unwrap()
    ));
    drop(pressure);
    assert_eq!(c.resources.counts(), before);
    c.settle_join_key(&owner, [9; 32]).unwrap();
    assert!(Arc::ptr_eq(
        &selected,
        &c.selected_handoff_owner(&owner, [9; 32]).unwrap()
    ));
    assert_eq!(selected.executor(), [9; 32]);
    assert_eq!(selected.point(), point());
    assert!(Arc::ptr_eq(
        &selected.grant().unwrap(),
        &c.owner(&owner).unwrap()._grant
    ));
    c.end(&owner);
    assert!(!c.is_current_handoff_owner(&selected));
    assert!(!selected.lease().is_live());
    assert!(selected.grant().is_err());
    let identity = selected.grant_identity();
    drop(work);
    assert!(
        identity.upgrade().is_none(),
        "the cached snapshot must never own an affine grant"
    );
    assert!(c.selected_handoff_owner(&owner, [9; 32]).is_err());
    let replacement = call(&mut c, &controller, 1, 2);
    assert!(c.selected_handoff_owner(&replacement, [9; 32]).is_err());
}

#[test]
fn native_handoff_selection_capacity_refusal_does_not_consume_its_published_generation() {
    let mut c = action_coordinator();
    let controller = controller(&mut c);
    let owner = call(&mut c, &controller, 1, 1);
    let admission = c.admit_join_key(&owner, [9; 32]).unwrap();
    let PublicationPlan::Start(mut work) = c
        .prepare_join_key_publication(&owner, [9; 32], &admission.permit, point())
        .unwrap()
    else {
        panic!("publication")
    };
    c.finish_join_key_publication(&mut work, true).unwrap();
    let before = c.resources.counts();
    let pressure = c
        .resources
        .charge(
            c.owner(&owner).unwrap().binding.scope.group,
            GROUP_BYTES - before.0,
            ResourceKind::Retained,
        )
        .unwrap();
    assert!(c.selected_handoff_owner(&owner, [9; 32]).is_err());
    drop(pressure);
    assert_eq!(c.resources.counts(), before);
    assert!(c.selected_handoff_owner(&owner, [9; 32]).is_ok());
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
        .charge(group, GROUP_BYTES - baseline.1 .0, ResourceKind::Retained)
        .unwrap();
    for _ in 0..4 {
        let retry = c.admit_join_key(&owner, [9; 32]).unwrap();
        assert_eq!(
            (retry.permit, retry.join, retry.kind),
            (first.permit.clone(), first.join.clone(), "admitted")
        );
        assert_eq!(c.serial, baseline.0);
    }
    let refused = c.admit_join_key(&owner, [8; 32]).unwrap();
    assert_eq!(refused.kind, "refused");
    assert!(refused.permit.is_empty() && refused.join.is_empty());
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
        assert_eq!(c.admit_join_key(&owner, [8; 32]).unwrap().kind, "refused");
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
        .charge(
            group,
            GROUP_BYTES - c.resources.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let serial = c.serial;
    assert!(c.admit_join_key(&owner, [9; 32]).is_err());
    assert_eq!(c.serial, serial);
    assert!(c.join_generations.is_empty());
    drop(pressure);
    c.admit_join_key(&owner, [9; 32]).unwrap();
    assert_eq!(c.serial, serial + 1);
}

#[test]
fn supplied_selection_check_never_materializes_a_local_cache_for_foreign_equal_labels() {
    fn published(c: &mut Coordinator) -> String {
        let controller = controller(c);
        let owner = call(c, &controller, 1, 1);
        let admission = c.admit_join_key(&owner, [9; 32]).unwrap();
        let PublicationPlan::Start(mut work) = c
            .prepare_join_key_publication(&owner, [9; 32], &admission.permit, point())
            .unwrap()
        else {
            panic!("publication")
        };
        c.finish_join_key_publication(&mut work, true).unwrap();
        c.settle_join_key(&owner, [9; 32]).unwrap();
        owner
    }
    let mut local = action_coordinator();
    let local_id = published(&mut local); // no selected snapshot has been requested here
    let mut foreign = action_coordinator();
    let foreign_id = published(&mut foreign);
    assert_eq!(local_id, foreign_id);
    let other = foreign
        .selected_handoff_owner(&foreign_id, [9; 32])
        .unwrap();
    let before = local.resources.counts();
    for _ in 0..4 {
        assert!(!local.is_current_handoff_owner(&other));
        assert_eq!(local.resources.counts(), before);
    }
    let group = local.owner(&local_id).unwrap().binding.scope.group;
    let pressure = local
        .resources
        .charge(group, GROUP_BYTES - before.0, ResourceKind::Retained)
        .unwrap();
    let full = local.resources.counts();
    assert!(!local.is_current_handoff_owner(&other));
    assert_eq!(
        local.resources.counts(),
        full,
        "no allocation or capacity dependency in a check"
    );
    drop(pressure);
    let selected = local.selected_handoff_owner(&local_id, [9; 32]).unwrap();
    assert!(local.is_current_handoff_owner(&selected));
    assert!(!local.is_current_handoff_owner(&other));
    let cached = local.resources.counts();
    assert!(local.is_current_handoff_owner(&selected));
    assert_eq!(local.resources.counts(), cached);
}
