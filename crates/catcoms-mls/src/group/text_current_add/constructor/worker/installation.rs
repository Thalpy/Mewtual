//! Real completed candidates and exact receiver ownership; no Root authority
//! or covering save is fabricated by this SDK-only fixture.
use super::*;

#[test]
fn install_rejects_partial_and_foreign_receivers_without_taking_or_mutating_any_input() {
    let bytes = source(3, false);
    let (mut d, mut g, _) = parent(&bytes);
    let (mut other_d, mut other_g, _) = parent(&bytes);
    let (_, cell, used, _, _, _) = prepared(&d, &g);
    let mut slot = Some(cell);
    let before = crate::snapshot_server(&d, &g).unwrap();
    assert!(PreparedTextCurrentAdd::prepare_install_into(&mut slot, &mut d, &mut g).is_err());
    assert!(slot.is_some());
    assert_eq!(slot.as_ref().unwrap().faults.sdk_dispatches, 0);
    slot.as_mut().unwrap().execute_private().unwrap();
    let exact = digest_outputs(slot.as_ref().unwrap());
    assert!(
        PreparedTextCurrentAdd::prepare_install_into(&mut slot, &mut other_d, &mut other_g)
            .is_err()
    );
    // Even identical state decoded into another independent pair is not the
    // original accounting receiver. Mixing one half must refuse as well.
    assert!(PreparedTextCurrentAdd::prepare_install_into(&mut slot, &mut d, &mut other_g).is_err());
    assert!(digest_outputs(slot.as_ref().unwrap()) == exact);
    assert!(crate::snapshot_server(&d, &g).unwrap().as_slice() == before.as_slice());
    drop(slot);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn cancelling_or_dropping_install_preparation_preserves_the_same_exact_completed_candidate() {
    let bytes = source(3, false);
    let (mut d, mut g, _) = parent(&bytes);
    let (_, mut cell, _, _, _, _) = prepared(&d, &g);
    cell.execute_private().unwrap();
    let exact = digest_outputs(&cell);
    let pointers = [
        cell.commit.as_ref().unwrap().as_ptr(),
        cell.welcome.as_ref().unwrap().as_ptr(),
        cell.snapshot.as_ref().unwrap().as_ptr(),
    ];
    let before = crate::snapshot_server(&d, &g).unwrap();
    let mut slot = Some(cell);
    PreparedTextCurrentAdd::prepare_install_into(&mut slot, &mut d, &mut g)
        .unwrap()
        .cancel();
    {
        let _cancelled_at_scope_end =
            PreparedTextCurrentAdd::prepare_install_into(&mut slot, &mut d, &mut g).unwrap();
    }
    let cell = slot.as_mut().unwrap();
    cell.execute_private().unwrap();
    assert!(digest_outputs(cell) == exact);
    assert_eq!(
        pointers,
        [
            cell.commit.as_ref().unwrap().as_ptr(),
            cell.welcome.as_ref().unwrap().as_ptr(),
            cell.snapshot.as_ref().unwrap().as_ptr()
        ]
    );
    assert_eq!(
        (cell.faults.sdk_dispatches, cell.faults.merge_dispatches),
        (1, 1)
    );
    assert!(crate::snapshot_server(&d, &g).unwrap().as_slice() == before.as_slice());
}

#[test]
fn install_moves_the_real_pair_keeps_both_payments_and_never_reopens_public_sdk_mutation() {
    for device_first in [false, true] {
        let bytes = source(3, false);
        let (mut d, mut g, old_used) = parent(&bytes);
        let old_auth = g.epoch_authenticator_id();
        let (peer, mut cell, used, _, _, _) = prepared(&d, &g);
        cell.execute_private().unwrap();
        let joined =
            ServerGroup::join(&peer, cell.completed_result_data().unwrap().welcome_data()).unwrap();
        let mut slot = Some(cell);
        let plan = PreparedTextCurrentAdd::prepare_install_into(&mut slot, &mut d, &mut g).unwrap();
        let retired = plan.install();
        assert!(slot.is_none());
        assert!(g.epoch_authenticator_id() != old_auth);
        assert!(g.epoch_authenticator_id() == joined.epoch_authenticator_id());
        assert_eq!(
            old_used.load(Ordering::SeqCst),
            TYPED,
            "swap must not destroy the old pair or its last fee"
        );
        assert!(used.load(Ordering::SeqCst) > 0);
        assert!(d.key_package().is_err());
        assert!(g.create_application_message(&d, b"still closed").is_err());
        assert!(g.remove_member(&d, &peer.device_id()).is_err());
        assert_eq!(format!("{retired:?}"), "RetiredTextCurrentAdd { .. }");
        drop(retired);
        assert_eq!(old_used.load(Ordering::SeqCst), 0);
        assert!(
            used.load(Ordering::SeqCst) > 0,
            "installed pair keeps its own full debit"
        );
        if device_first {
            drop(d);
            assert!(used.load(Ordering::SeqCst) > 0);
            drop(g);
        } else {
            drop(g);
            assert!(used.load(Ordering::SeqCst) > 0);
            drop(d);
        }
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn repeated_installs_release_previous_graph_fees_instead_of_retaining_an_ancestry_chain() {
    let bytes = source(3, false);
    let (mut d, mut g, old_used) = parent(&bytes);
    let mut previous = old_used;
    for _ in 0..3 {
        let (_, mut cell, used, _, _, _) = prepared(&d, &g);
        cell.execute_private().unwrap();
        let mut slot = Some(cell);
        let retired = PreparedTextCurrentAdd::prepare_install_into(&mut slot, &mut d, &mut g)
            .unwrap()
            .install();
        assert!(previous.load(Ordering::SeqCst) > 0);
        drop(retired);
        assert_eq!(previous.load(Ordering::SeqCst), 0);
        assert!(used.load(Ordering::SeqCst) > 0);
        previous = used;
    }
    drop(g);
    drop(d);
    assert_eq!(previous.load(Ordering::SeqCst), 0);
}

#[test]
fn stale_sibling_candidate_cannot_install_after_another_add_won_on_the_same_receiver() {
    let bytes = source(3, false);
    let (mut d, mut g, _) = parent(&bytes);
    let (_, mut first, _, _, _, _) = prepared(&d, &g);
    let (_, mut second, used, _, _, _) = prepared(&d, &g);
    first.execute_private().unwrap();
    second.execute_private().unwrap();
    let mut first = Some(first);
    let mut second = Some(second);
    drop(
        PreparedTextCurrentAdd::prepare_install_into(&mut first, &mut d, &mut g)
            .unwrap()
            .install(),
    );
    let before = crate::snapshot_server(&d, &g).unwrap();
    assert!(PreparedTextCurrentAdd::prepare_install_into(&mut second, &mut d, &mut g).is_err());
    assert!(second.as_ref().unwrap().is_complete_data());
    assert!(crate::snapshot_server(&d, &g).unwrap().as_slice() == before.as_slice());
    drop(second);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}
