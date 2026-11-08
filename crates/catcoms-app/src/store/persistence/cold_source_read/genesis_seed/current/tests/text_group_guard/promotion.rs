//! Genuine promotion admissions, not fabricated Ready/Saved or scalar permit flags.
//! Declared only with the paired concrete physical promotion and its actual host caller.
use super::*;
use crate::store::critical_namespaces::current_layout::PreparedV2TextPhysicalLease;
use crate::store::owner_evidence::canonical::text_lane::{
    active_text_lane_count_for_test, PreparedV2TextLane, V2TextLane,
};
use crate::store::persistence::v2_text::{
    PreparedV2TextPromotion, PreparedV2TextPromotionCapture, V2TextPromotionTestCut,
    V2TextQuotaPolicy, V2TextReadResources,
};
use crate::store::persistence::PreparedV2TextSourceObserver;
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
use std::sync::atomic::{AtomicBool, Ordering};

struct PromotionFixture {
    prepared: PreparedV2TextPromotion,
    lane: V2TextLane,
    // LAST: all operation metadata drains before the original owner/store/directory.
    original: TextFixture,
}

fn promotion_fixture(family: usize, number: u64) -> PromotionFixture {
    let original = text_fixture(family, number);
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let mut capture = PreparedV2TextPromotionCapture::prepare(store, owner.source()).unwrap();
    let resources = capture.resources().clone();
    store
        .with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            capture.capture_genesis_parent(&guard)
        })
        .unwrap();
    let mut prepared = None;
    capture
        .take_promotion_into(
            V2TextQuotaPolicy::new(1024 * 1024 * 1024, 1024 * 1024).unwrap(),
            &mut prepared,
        )
        .unwrap();
    let mut prepared = prepared.unwrap();
    prepared
        .prepare_candidates(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(number),
        )
        .unwrap();
    let observer = PreparedV2TextSourceObserver::prepare(store, &owner).unwrap();
    let observer = store
        .with_v2_text_current(&owner, |admission| observer.capture(admission))
        .unwrap();
    let mut lane_preparation = PreparedV2TextLane::prepare(store, &owner, &observer).unwrap();
    let mut lane = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut namespace = store.lock_v2_text_owner_namespace(admission)?;
            namespace.claim_text_lane_into(&mut lane_preparation, &mut lane)
        })
        .unwrap();
    lane_preparation.drain_unused_outside_writers();
    PromotionFixture {
        prepared,
        lane: lane.unwrap(),
        original,
    }
}

fn finish_promotion_lane(
    original: &TextFixture,
    prepared: &PreparedV2TextPromotion,
    lane: &mut Option<V2TextLane>,
) {
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let resources = prepared.resources().clone();
    let saved = prepared.saved_promotion().unwrap();
    let mut closed = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.close_text_promotion_lane_into(lane.as_ref().unwrap(), saved, &mut closed)
        })
        .unwrap();
    let drained = closed.take().unwrap().join().unwrap();
    let mut retired = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.retire_text_promotion_lane_into(lane, saved, &drained, &mut retired)
        })
        .unwrap();
    assert!(lane.is_none());
    assert!(retired.is_some());
    assert_eq!(active_text_lane_count_for_test(store), 0);
    // A lost local return reobserves ONLY this exact retired owner. It must not require
    // a now-absent registry entry or remove any newer same-group entry as a substitute.
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.retire_text_promotion_lane_into(lane, saved, &drained, &mut retired)
        })
        .unwrap();
    assert_eq!(active_text_lane_count_for_test(store), 0);
    // All actual removed ownership and closed metadata drain outside the real writers.
    drop(retired);
    drop(drained);
}

fn stage_and_commit(f: &mut PromotionFixture) {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.prepared.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_stage_into(&f.lane, &f.prepared, &mut lease, &mut admitted)
        })
        .unwrap();
    f.prepared.run_stage(admitted.take().unwrap()).unwrap();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_commit_into(
                &f.lane,
                f.prepared.ready_selection()?,
                &mut lease,
                &mut admitted,
            )
        })
        .unwrap();
    f.prepared.run_commit(admitted.take().unwrap()).unwrap();
}

#[test]
fn actual_three_family_promotion_stages_before_fresh_commit_and_then_reads_selected_o() {
    for family in 0..3 {
        let mut f = promotion_fixture(family, 0x7e900 + family as u64);
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.join("head.bin");
        let genesis = std::fs::read(&target).unwrap();
        let resources = f.prepared.resources().clone();
        let mut lease =
            Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
        let mut staged = None;
        store
            .with_v2_text_current(&owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_promotion_stage_into(&f.lane, &f.prepared, &mut lease, &mut staged)
            })
            .unwrap();
        assert!(lease.is_none());
        // Even an UNSTARTED actual marker occupies the SAME lane's one active-work slot.
        let mut duplicate_lease =
            Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
        let mut duplicate = None;
        assert!(store
            .with_v2_text_current(&owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_promotion_stage_into(
                    &f.lane,
                    &f.prepared,
                    &mut duplicate_lease,
                    &mut duplicate,
                )
            })
            .is_err());
        assert!(duplicate.is_none());
        assert!(duplicate_lease.is_some());
        drop(duplicate_lease);
        f.prepared.run_stage(staged.take().unwrap()).unwrap();
        assert!(f.prepared.ready_selection().is_ok());
        assert!(f.prepared.saved_promotion().is_err());
        assert_eq!(
            std::fs::read(&target).unwrap(),
            genesis,
            "staging cannot select O"
        );

        // A NEW same-Store OS lease and actual Current/Ready/lane proof precede CAS.
        let mut lease =
            Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
        let mut admitted = None;
        store
            .with_v2_text_current(&owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_promotion_commit_into(
                    &f.lane,
                    f.prepared.ready_selection()?,
                    &mut lease,
                    &mut admitted,
                )
            })
            .unwrap();
        assert!(lease.is_none());
        f.prepared.run_commit(admitted.take().unwrap()).unwrap();
        assert!(f.prepared.saved_promotion().is_ok());
        assert_ne!(std::fs::read(target).unwrap(), genesis);
        let mut selected = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
        let resources = selected.resources().clone();
        store
            .with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                let selected = selected.capture(&guard)?;
                assert_eq!(selected.head_data().generation, 1);
                assert_eq!(
                    selected.head_data().context.spec_data().nonce,
                    admission.scope_nonce_data()?
                );
                assert_eq!(
                    selected.archive_data().spec_data().header.id(),
                    owner.source().data().header.id()
                );
                Ok(())
            })
            .unwrap();
        drop(selected);
        drop(resources);
        drop(owner);
        let mut lane = Some(f.lane);
        finish_promotion_lane(&f.original, &f.prepared, &mut lane);
    }
}

#[test]
fn actual_stage_admission_is_retained_before_late_scope_refusal_and_does_not_grant_commit() {
    let mut f = promotion_fixture(0, 0x7e910);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.join("head.bin");
    let genesis = std::fs::read(&target).unwrap();
    let resources = f.prepared.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_stage_into(
                &f.lane,
                &f.prepared,
                &mut lease,
                &mut admitted,
            )?;
            owner.warm_scope().unwrap().revocation().revoke();
            Ok(())
        })
        .is_err());
    assert!(
        admitted.is_some(),
        "the actual outer slot survives Scope's late Err"
    );
    assert!(
        lease.is_none(),
        "the SAME preacquired OS lease moved with admission"
    );
    // Cancellation after admission may not replace this finite work or erase its result.
    f.prepared.run_stage(admitted.take().unwrap()).unwrap();
    assert!(f.prepared.ready_selection().is_ok());
    assert_eq!(std::fs::read(target).unwrap(), genesis);
    let callbacks = std::cell::Cell::new(0);
    assert!(store
        .with_v2_text_current(&owner, |_| {
            callbacks.set(callbacks.get() + 1);
            Ok(())
        })
        .is_err());
    assert_eq!(
        callbacks.get(),
        0,
        "Ready never revives a revoked Current owner"
    );
    assert!(f.prepared.saved_promotion().is_err());
}

#[test]
fn actual_scope_cancellation_before_admission_keeps_lease_and_performs_no_stage_io() {
    let f = promotion_fixture(0, 0x7e920);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let resources = f.prepared.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    owner.warm_scope().unwrap().revocation().revoke();
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_stage_into(&f.lane, &f.prepared, &mut lease, &mut admitted)
        })
        .is_err());
    assert!(admitted.is_none());
    assert!(
        lease.is_some(),
        "failed admission cannot consume the outside original"
    );
    assert!(!target.join("text.seed").exists());
    assert!(!target.join("text.pages").exists());
    assert!(f.prepared.ready_selection().is_err());
    assert!(f.prepared.saved_promotion().is_err());
}

#[test]
fn actual_promotion_admission_retains_same_physical_session_after_original_store_drop() {
    let f = promotion_fixture(0, 0x7e930);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.prepared.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_stage_into(&f.lane, &f.prepared, &mut lease, &mut admitted)
        })
        .unwrap();
    assert!(lease.is_none());
    drop(owner);
    let PromotionFixture {
        prepared,
        lane,
        original,
    } = f;
    let TextFixture {
        owner,
        layout,
        _directory: directory,
    } = original;
    drop(owner);
    drop(lane);
    drop(layout);
    assert!(
        matches!(
            catcoms_storage::acquire_vault_session(directory.path()),
            Err(catcoms_storage::StorageError::VaultBusy)
        ),
        "the admitted owner must already retain the original OS lease before any worker starts"
    );
    // This was an admitted but UNSTARTED finite stage. No file was created and there is
    // no cleanup debt to outlive this explicit outside-writer abandonment of permission.
    drop(admitted.take());
    let next_session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    assert!(prepared.ready_selection().is_err());
    assert!(prepared.saved_promotion().is_err());
    drop(next_session);
}

#[test]
fn actual_sole_promotion_saved_destroys_facts_before_its_exact_read_fee_refunds() {
    let exact_fee_dropped = Arc::new(AtomicBool::new(false));
    let facts_dropped = Arc::new(AtomicBool::new(false));
    let mut f =
        V2TextReadResources::with_fee_drop_notice_for_test(exact_fee_dropped.clone(), || {
            promotion_fixture(0, 0x7e940)
        });
    f.prepared
        .observe_saved_facts_drop_for_test(facts_dropped.clone(), exact_fee_dropped.clone());
    stage_and_commit(&mut f);
    let saved = f.prepared.saved_promotion().unwrap().clone();
    let mut lane = Some(f.lane);
    finish_promotion_lane(&f.original, &f.prepared, &mut lane);
    drop(f.prepared);
    assert!(!exact_fee_dropped.load(Ordering::SeqCst));
    assert!(!facts_dropped.load(Ordering::SeqCst));
    // The real original Store/SDK remains alive, so unrelated source refunds cannot pass.
    // LAST notice inside actual facts asserts exact_fee_dropped is still false AFTER all
    // frame/path fields have dropped. Only the sole Saved's final read loan follows it.
    drop(saved);
    assert!(facts_dropped.load(Ordering::SeqCst));
    assert!(exact_fee_dropped.load(Ordering::SeqCst));
    assert!(f.original.layout.checked_store().is_ok());
}

#[test]
fn actual_old_saved_cannot_close_a_new_same_group_lane_after_original_retirement() {
    let mut f = promotion_fixture(0, 0x7e950);
    stage_and_commit(&mut f);
    let mut old_lane = Some(f.lane);
    finish_promotion_lane(&f.original, &f.prepared, &mut old_lane);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.prepared.resources().clone();
    let observer = PreparedV2TextSourceObserver::prepare(store, &owner).unwrap();
    let observer = store
        .with_v2_text_current(&owner, |a| observer.capture(a))
        .unwrap();
    let mut prepared_lane = PreparedV2TextLane::prepare(store, &owner, &observer).unwrap();
    let mut new_lane = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut namespace = store.lock_v2_text_owner_namespace(admission)?;
            namespace.claim_text_lane_into(&mut prepared_lane, &mut new_lane)
        })
        .unwrap();
    prepared_lane.drain_unused_outside_writers();
    assert_eq!(active_text_lane_count_for_test(store), 1);
    let mut closed = None;
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.close_text_promotion_lane_into(
                new_lane.as_ref().unwrap(),
                f.prepared.saved_promotion()?,
                &mut closed,
            )
        })
        .is_err());
    assert!(closed.is_none());
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    // An already-Saved old plan cannot bind the fresh lane by replaying stage admission.
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_stage_into(
                new_lane.as_ref().unwrap(),
                &f.prepared,
                &mut lease,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none());
    assert!(lease.is_some());
    assert_eq!(active_text_lane_count_for_test(store), 1);
}

#[test]
fn actual_success_close_is_cached_before_late_scope_refusal_without_faking_retirement() {
    let mut f = promotion_fixture(0, 0x7e960);
    stage_and_commit(&mut f);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.prepared.resources().clone();
    let saved = f.prepared.saved_promotion().unwrap();
    let mut lane = Some(f.lane);
    let mut closed = None;
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.close_text_promotion_lane_into(lane.as_ref().unwrap(), saved, &mut closed)?;
            owner.warm_scope().unwrap().revocation().revoke();
            Ok(())
        })
        .is_err());
    assert!(closed.is_some());
    let drained = closed.take().unwrap().join().unwrap();
    let mut retired = None;
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.retire_text_promotion_lane_into(&mut lane, saved, &drained, &mut retired)
        })
        .is_err());
    assert!(lane.is_some());
    assert!(retired.is_none());
    assert_eq!(
        active_text_lane_count_for_test(store),
        1,
        "actual work end and cached closing DATA cannot replace fresh Current retirement"
    );
}

#[test]
fn actual_open_stage_file_and_failed_worker_hold_session_until_all_file_cleanup_drains() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let mut f = promotion_fixture(0, 0x7e970);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let stage_path = owner
        .source()
        .source()
        .capture
        .target
        .join("text.seed.stage.v1");
    let resources = f.prepared.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_stage_into(&f.lane, &f.prepared, &mut lease, &mut admitted)
        })
        .unwrap();
    drop(owner);
    drop(resources);
    let (started_tx, started_rx) = sync_channel(0);
    let (release_tx, release_rx) = sync_channel(0);
    f.prepared.set_work_pause_for_test(
        V2TextPromotionTestCut::SeedCreated,
        Box::new(move || {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        }),
    );
    f.prepared
        .set_work_fault_for_test(V2TextPromotionTestCut::SeedCreated);
    let PromotionFixture {
        mut prepared,
        lane,
        original,
    } = f;
    let worker = std::thread::spawn(move || {
        let result = prepared.run_stage(admitted.take().unwrap());
        (prepared, result)
    });
    started_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    assert!(
        stage_path.is_file(),
        "the actual stage File must already be open"
    );
    assert!(
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&stage_path)
            .is_err(),
        "a real live stage File, not a synthetic work counter, is parked"
    );
    let TextFixture {
        owner,
        layout,
        _directory: directory,
    } = original;
    drop(owner);
    drop(lane);
    drop(layout);
    assert!(matches!(
        catcoms_storage::acquire_vault_session(directory.path()),
        Err(catcoms_storage::StorageError::VaultBusy)
    ));
    release_tx.send(()).unwrap();
    let (prepared, result) = worker.join().unwrap();
    assert!(result.is_err());
    assert!(prepared.ready_selection().is_err());
    assert!(prepared.saved_promotion().is_err());
    let closed_stage = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&stage_path)
        .unwrap();
    drop(closed_stage);
    // Metadata may remain alive after a failed attempt; no workspace File or physical
    // lease is allowed to hide in that dormant state or block this real next session.
    let next_session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    drop(next_session);
    drop(prepared);
}

// Distinct complete-seed repair and pre-MAIN scratch admission regressions.
mod recovery;

// Actual promotion binding permanently excludes ordinary conversion.
mod create_lane;

mod query_fixture;
pub(super) use query_fixture::promoted_text_fixture;
