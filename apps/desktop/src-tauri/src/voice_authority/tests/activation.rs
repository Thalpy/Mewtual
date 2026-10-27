//! Native identity tests; synthetic lifecycle grants here are not Root/RTC acceptance.
use super::*;

#[test]
fn accepted_activation_recovery_preserves_nonce_allocations_serial_and_revision() {
    let mut c = action_coordinator();
    let controller_id = controller(&mut c);
    let controller_receipt = c.accepted_activation(&controller_id).unwrap();
    let owner_id = call(&mut c, &controller_id, 1, 1);
    let accepted = c.accepted_activation(&owner_id).unwrap();
    let original = c.owner(&owner_id).unwrap();
    let counters = (c.serial, c.revision, c.resources.counts());
    assert!(
        c.candidate(&owner_id, 0).is_none(),
        "activation consumed its candidate"
    );
    for _ in 0..4 {
        // Recovery does not sample time: an accepted call outlives its candidate's TTL.
        c.validate_accepted_activation(&accepted).unwrap();
        let recovered = c.accepted_activation(&owner_id).unwrap();
        c.validate_accepted_activation(&recovered).unwrap();
        let owner = recovered.owner().unwrap();
        assert_eq!(owner.join, original.join);
        assert!(Arc::ptr_eq(&owner._grant, &original._grant));
        assert!(owner.lease.is_same(&original.lease));
    }
    // Recovering the original controller cannot replay its old retire-owner side effect.
    c.validate_accepted_activation(&controller_receipt).unwrap();
    assert!(c.owner(&owner_id).unwrap().lease.is_live());
    assert_eq!((c.serial, c.revision, c.resources.counts()), counters);
}

#[test]
fn accepted_activation_never_recovers_ended_or_replaced_equal_scope_owners() {
    let mut c = action_coordinator();
    let controller_id = controller(&mut c);
    let first = call(&mut c, &controller_id, 1, 1);
    let accepted = c.accepted_activation(&first).unwrap();
    c.end(&first);
    assert!(c.accepted_activation(&first).is_none());
    assert!(c.validate_accepted_activation(&accepted).is_err());
    let second = call(&mut c, &controller_id, 1, 1);
    let second_receipt = c.accepted_activation(&second).unwrap();
    assert!(accepted.owner().unwrap().binding == second_receipt.owner().unwrap().binding);
    assert!(c.validate_accepted_activation(&accepted).is_err());
    assert!(c.accepted_activation(&first).is_none());
    c.validate_accepted_activation(&second_receipt).unwrap();
    let second_owner = c.owner(&second).unwrap();
    c.end(&first);
    assert!(
        second_owner.lease.is_live(),
        "late old End cannot affect replacement"
    );
}

#[test]
fn accepted_activation_rechecks_exact_controller_and_actual_actor_lease() {
    let mut c = action_coordinator();
    let controller_id = controller(&mut c);
    let controller_receipt = c.accepted_activation(&controller_id).unwrap();
    let first = call(&mut c, &controller_id, 1, 1);
    let accepted = c.accepted_activation(&first).unwrap();
    accepted.owner().unwrap().lease.revoke();
    assert!(c.accepted_activation(&first).is_none());
    assert!(c.validate_accepted_activation(&accepted).is_err());
    let second = call(&mut c, &controller_id, 1, 1);
    let second_receipt = c.accepted_activation(&second).unwrap();
    let replacement = controller(&mut c);
    assert_ne!(replacement, controller_id);
    assert!(c.validate_accepted_activation(&controller_receipt).is_err());
    assert!(c.validate_accepted_activation(&second_receipt).is_err());
    assert!(c.accepted_activation(&controller_id).is_none());
    assert!(c.accepted_activation(&second).is_none());
    c.validate_accepted_activation(&c.accepted_activation(&replacement).unwrap())
        .unwrap();
}

#[test]
fn accepted_activation_rejects_same_id_scope_and_nonce_with_substituted_grant() {
    let mut c = action_coordinator();
    let controller_id = controller(&mut c);
    let owner_id = call(&mut c, &controller_id, 1, 1);
    let accepted = c.accepted_activation(&owner_id).unwrap();
    let original = c.owner(&owner_id).unwrap();
    // Only this private unit fixture can forge an equal-id replacement. The public bridge
    // cannot construct AcceptedActivation or replace its private original grant allocation.
    let foreign = Arc::new(CallGrant::from_lease(
        original.lease.clone(),
        original.binding.scope,
    ));
    c.owner.as_mut().unwrap()._grant = foreign;
    assert!(original.lease.is_live());
    assert!(c.validate_accepted_activation(&accepted).is_err());
}

#[test]
fn refused_activation_keeps_temporary_grant_until_native_guards_have_left() {
    let mut c = action_coordinator();
    let controller_id = controller(&mut c);
    let original = call(&mut c, &controller_id, 1, 1);
    let original_lease = c.owner(&original).unwrap().lease;
    let pending = c
        .prepare(
            CandidateKind::Call {
                controller: controller_id,
                binding: binding(1, 1),
            },
            0,
        )
        .unwrap();
    let temporary = CallLease::default();
    let mut grant = Some(CallGrant::from_lease(
        temporary.clone(),
        binding(1, 1).scope,
    ));
    // The outer grant predates both real mutex guards, as it does in the async bridge. Revoking
    // the pending candidate models End after grant acquisition, before final admission.
    let coordinator = std::sync::Mutex::new(c);
    let ui = std::sync::Mutex::new(());
    {
        let _commit = ui.lock().unwrap();
        let mut c = coordinator.lock().unwrap();
        c.end(&pending);
        assert!(c.activate(&pending, [9; 32], 0, &mut grant).is_err());
        assert!(
            grant.is_some() && temporary.is_live(),
            "refusal has not destroyed the input"
        );
        assert!(original_lease.is_live());
    }
    assert!(coordinator.try_lock().is_ok() && ui.try_lock().is_ok());
    drop(grant);
    assert!(
        !temporary.is_live(),
        "actual affine destructor runs only after both guards leave"
    );
    assert!(original_lease.is_live());
}
