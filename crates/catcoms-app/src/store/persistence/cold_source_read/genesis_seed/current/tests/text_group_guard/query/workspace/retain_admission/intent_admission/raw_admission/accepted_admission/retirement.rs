//! Real canonical old-root lifecycle, not deletion permission. Both actual query
//! families and independently paid observations participate; new B readers survive.
use super::*;
use crate::store::owner_evidence::canonical::text_lane::{
    PreparedV2TextRetirementRootsClose, RemovedV2TextRetirementRootGenerations,
    V2TextRetirementRootsClosed, V2TextRetirementRootsDrained,
};
use crate::store::persistence::V2AdvancedAppendBasis;
use std::sync::mpsc::sync_channel;
use std::time::Duration;

fn select_and_finalize(f: &mut AcceptedFixture) {
    let stage = stage_b(f);
    f.accepted.run_stage(stage).unwrap();
    let commit = commit_b(f);
    f.accepted.run_commit(commit).unwrap();
    advance_b(f).unwrap();
    f.accepted.drain_accepted_retired_math_if_present().unwrap();
    final_b(f).unwrap();
}
fn current_query(f: &AcceptedFixture, token: u8) -> PreparedV2TextTokenQuery {
    let selected = selected_capture(&f.original);
    let (mut query, mut reader, mut capture) = prepare_query(&f.original, &selected, [token; 32]);
    let admitted = admit_query(&f.original, &mut query, &mut reader, &mut capture);
    query.run(admitted).unwrap();
    query
}
fn prepare_close(
    f: &AcceptedFixture,
    basis: &V2AdvancedAppendBasis,
) -> PreparedV2TextRetirementRootsClose {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    PreparedV2TextRetirementRootsClose::prepare(store, owner.source(), &f.accepted, basis, &f.lane)
        .unwrap()
}
fn close(
    f: &AcceptedFixture,
    basis: &V2AdvancedAppendBasis,
    prepared: &mut Option<PreparedV2TextRetirementRootsClose>,
    closed: &mut Option<V2TextRetirementRootsClosed>,
) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let resources = f.accepted.resources().clone();
    store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        guard.close_text_retirement_roots_into(&f.lane, &f.accepted, basis, prepared, closed)
    })
}
fn detach(
    f: &AcceptedFixture,
    basis: &V2AdvancedAppendBasis,
    drained: &mut V2TextRetirementRootsDrained,
    removed: &mut Option<RemovedV2TextRetirementRootGenerations>,
) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let resources = f.accepted.resources().clone();
    store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        guard.detach_text_retirement_roots_into(&f.lane, &f.accepted, basis, drained, removed)
    })
}
fn preparation_refuses(f: &AcceptedFixture, basis: &V2AdvancedAppendBasis) {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let pool = &store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool;
    let before = pool.charged_for_test();
    assert!(PreparedV2TextRetirementRootsClose::prepare(
        store,
        owner.source(),
        &f.accepted,
        basis,
        &f.lane
    )
    .is_err());
    assert_eq!(pool.charged_for_test(), before);
}

#[test]
fn actual_old_root_close_spans_both_query_families_and_read_bodies_but_preserves_new_b_reader() {
    let mut f = accepted_fixture(0, 0x80100);
    // Two independent actual selected/read bodies observe the SAME S ciphertext.
    let old_a = current_query(&f, 90);
    let old_b = current_query(&f, 91);
    select_and_finalize(&mut f);
    let new_b = current_query(&f, 92);
    let basis = f.accepted.accepted_final_basis_data().unwrap().clone();
    assert_eq!(
        f.accepted.original_query_readers_live_for_test(),
        (true, true)
    );
    let mut prepared = Some(prepare_close(&f, &basis));
    preparation_refuses(&f, &basis);
    let mut closed = None;
    close(&f, &basis, &mut prepared, &mut closed).unwrap();
    assert!(prepared.is_none());
    assert_eq!(
        closed.as_ref().unwrap().snapshot_counts_for_test(),
        (3, 1, 4, 0)
    );
    assert_eq!(
        f.accepted.original_query_readers_live_for_test(),
        (false, false)
    );
    validate_query(&f.original, &new_b);
    let mut drained = None;
    assert!(!V2TextRetirementRootsClosed::poll_drained_into(&mut closed, &mut drained).unwrap());
    drop(old_a);
    drop(old_b);
    assert!(!V2TextRetirementRootsClosed::poll_drained_into(&mut closed, &mut drained).unwrap());
    f.accepted
        .drain_original_query_readers_outside_writers()
        .unwrap();
    assert!(V2TextRetirementRootsClosed::poll_drained_into(&mut closed, &mut drained).unwrap());
    let mut drained = drained.unwrap();
    let mut removed = None;
    detach(&f, &basis, &mut drained, &mut removed).unwrap();
    assert!(removed.is_some());
    // A lost metadata-detach reply is reobserved by SAME opaque lineage, not by
    // interpreting now-missing entries as a substitute drain proof.
    detach(&f, &basis, &mut drained, &mut removed).unwrap();
    let store = f.original.layout.checked_store().unwrap();
    assert_eq!(text_query_registry_counts_for_test(store), (1, 1));
    validate_query(&f.original, &new_b);
    preparation_refuses(&f, &basis);
    drop(drained);
    // The actual removed registry Arcs keep the SAME one-preparation claim alive.
    preparation_refuses(&f, &basis);
    drop(removed);
    drop(prepare_close(&f, &basis));
    validate_saved_b(&f).unwrap();
}

#[test]
fn actual_full_32_generation_scan_waits_for_real_ends_then_detaches_all_old_roots() {
    let mut f = accepted_fixture(0, 0x80120);
    let old: Vec<_> = (0..30)
        .map(|index| current_query(&f, 100 + index))
        .collect();
    let store = f.original.layout.checked_store().unwrap();
    assert_eq!(text_query_registry_counts_for_test(store), (32, 32));
    select_and_finalize(&mut f);
    let basis = f.accepted.accepted_final_basis_data().unwrap().clone();
    let mut prepared = Some(prepare_close(&f, &basis));
    let mut closed = None;
    close(&f, &basis, &mut prepared, &mut closed).unwrap();
    assert_eq!(
        closed.as_ref().unwrap().snapshot_counts_for_test(),
        (31, 1, 32, 0)
    );
    let (waiting_tx, waiting_rx) = sync_channel(0);
    let (done_tx, done_rx) = sync_channel(0);
    closed
        .as_mut()
        .unwrap()
        .set_before_wait_for_test(Box::new(move || waiting_tx.send(()).unwrap()));
    let worker = std::thread::spawn(move || {
        let mut drained = None;
        V2TextRetirementRootsClosed::join_drained_into(&mut closed, &mut drained).unwrap();
        done_tx.send(()).unwrap();
        drained.unwrap()
    });
    // This notification comes from the real nonzero-End predicate while holding
    // QueryGeneration.wait, immediately before Condvar.wait, not worker startup.
    waiting_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    assert!(matches!(
        done_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    drop(old);
    f.accepted
        .drain_original_query_readers_outside_writers()
        .unwrap();
    done_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    let mut drained = worker.join().unwrap();
    let mut removed = None;
    detach(&f, &basis, &mut drained, &mut removed).unwrap();
    let store = f.original.layout.checked_store().unwrap();
    assert_eq!(text_query_registry_counts_for_test(store), (0, 0));
    // Registry capacity is actually available again; no weak-expiry/slot-counter
    // setter or synthetic completion was used to make the new genuine query fit.
    let new_b = current_query(&f, 199);
    validate_query(&f.original, &new_b);
}

#[test]
fn actual_late_scope_stop_preserves_closed_or_removed_owners_and_negative_drain() {
    for after_detach in [false, true] {
        let mut f = accepted_fixture(0, 0x80140 + u64::from(after_detach));
        select_and_finalize(&mut f);
        let stopped_store = f.original.layout.checked_store().unwrap();
        let stopped_owner = f.original.owner.borrow_current().unwrap();
        let basis = f.accepted.accepted_final_basis_data().unwrap().clone();
        let mut prepared = Some(prepare_close(&f, &basis));
        let mut closed = None;
        let revoke = f
            .original
            .owner
            .borrow_current()
            .unwrap()
            .warm_scope()
            .unwrap()
            .revocation();
        if !after_detach {
            prepared
                .as_mut()
                .unwrap()
                .set_after_close_for_test(Box::new(move || revoke.revoke()));
            assert!(close(&f, &basis, &mut prepared, &mut closed).is_err());
        } else {
            close(&f, &basis, &mut prepared, &mut closed).unwrap();
        }
        assert!(prepared.is_none() && closed.is_some());
        assert_eq!(
            closed.as_ref().unwrap().snapshot_counts_for_test(),
            (1, 1, 2, 0)
        );
        f.accepted
            .drain_original_query_readers_outside_writers()
            .unwrap();
        let mut drained = None;
        assert!(V2TextRetirementRootsClosed::poll_drained_into(&mut closed, &mut drained).unwrap());
        let mut drained = drained.unwrap();
        let mut removed = None;
        if after_detach {
            let revoke = f
                .original
                .owner
                .borrow_current()
                .unwrap()
                .warm_scope()
                .unwrap()
                .revocation();
            drained.set_after_detach_for_test(Box::new(move || revoke.revoke()));
            assert!(detach(&f, &basis, &mut drained, &mut removed).is_err());
            assert!(removed.is_some());
            let store = f.original.layout.checked_store().unwrap();
            assert_eq!(text_query_registry_counts_for_test(store), (0, 0));
        } else {
            assert!(detach(&f, &basis, &mut drained, &mut removed).is_err());
            assert!(removed.is_none());
        }
        assert!(stopped_store
            .with_v2_text_current(&stopped_owner, |_| Ok(()))
            .is_err());
        assert!(f.accepted.has_saved_data());
        assert!(!f.model_dropped.load(Ordering::SeqCst));
        assert!(!f.sdk_dropped.load(Ordering::SeqCst));
    }
}

mod semantic;

mod effects;
mod worker;

mod finish;
mod resident;

mod next_create;
