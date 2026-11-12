//! Genuine completed-WC namespace and independent metadata refusal regressions.
//! No fake saved source/Resident/Bound is constructed by this test family.
use super::*;
use std::os::windows::fs::OpenOptionsExt;
fn read_ready(t: &mut NextCreateFixture) {
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let mut capture = Some(
        PreparedV2TextCompletedWorkspaceReadCapture::prepare(
            store,
            owner.source(),
            t.workspace.as_ref().unwrap(),
        )
        .unwrap(),
    );
    drop(owner);
    let mut admitted = None;
    read_completed(t, &mut capture, &mut admitted).unwrap();
    assert!(capture.is_none());
    t.workspace
        .as_mut()
        .unwrap()
        .run_read(admitted.take().unwrap())
        .unwrap();
    assert!(t.predecessor.is_none());
}
#[test]
fn actual_completed_predecessor_rechecks_same_wal_bytes_after_final_source_before_binding() {
    let mut t = next_create_fixture(0, 0x83400);
    read_ready(&mut t);
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let wal = owner.source().source().capture.target.join("text.wal");
    let original = std::fs::read(&wal).unwrap();
    let resources = t.workspace.as_ref().unwrap().resources().clone();
    let identity = t.producer.as_ref().unwrap().observe_original();
    let called = Arc::new(AtomicBool::new(false));
    let hit = called.clone();
    let mut wrong = original.clone();
    let at = wrong.len() / 2;
    wrong[at] ^= 1;
    let target = wal.clone();
    let changed = wrong.clone();
    let result = store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        guard.after_original_source_for_test(Box::new(move || {
            assert!(
                std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(&target)
                    .is_err(),
                "actual WC held before final source seam"
            );
            let held = std::fs::File::open(&target).unwrap();
            let native = catcoms_storage::kept::session_id(&held).unwrap();
            let len = held.metadata().unwrap().len();
            std::fs::write(&target, changed).unwrap();
            let after = std::fs::File::open(&target).unwrap();
            assert_eq!(catcoms_storage::kept::session_id(&after).unwrap(), native);
            assert_eq!(after.metadata().unwrap().len(), len);
            assert_eq!(held.metadata().unwrap().len(), len);
            hit.store(true, Ordering::SeqCst);
        }));
        guard.admit_text_completed_create_predecessor_into(
            &t.lane,
            t.workspace.as_ref().unwrap().ready_completed()?,
            t.query.as_ref().unwrap(),
            t.resident.as_ref().unwrap(),
            &identity,
            &mut t.predecessor,
        )
    });
    assert!(called.load(Ordering::SeqCst));
    assert!(result.is_err());
    assert!(t.predecessor.is_none());
    assert_eq!(std::fs::read(&wal).unwrap(), wrong);
    assert!(!t.lane.workspace_active_for_test());
    drop(
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&wal)
            .unwrap(),
    );
    std::fs::write(&wal, &original).unwrap();
    drop(owner);
    bind_completed(&mut t, false).unwrap();
    assert!(t.predecessor.is_some());
}
pub(super) fn known_files(target: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut paths: Vec<_> = std::fs::read_dir(target)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    paths.extend(
        std::fs::read_dir(target.join("text.pages"))
            .unwrap()
            .map(|e| e.unwrap().path()),
    );
    assert!(paths.len() < 128);
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let bytes = std::fs::read(&p).unwrap();
            (p, bytes)
        })
        .collect()
}
#[test]
fn actual_completed_parent_metadata_shortfall_preserves_original_inputs_then_same_request_succeeds()
{
    let mut t = next_create_fixture(0, 0x83410);
    // No Source/Ready/Bound exists. Drop the previous unstarted preparation outside
    // writers so the same producer's central claim and completed-only debit drain.
    drop(t.workspace.take());
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let files = known_files(&target);
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let before = pool.charged_for_test();
    let required =
        PreparedV2TextCompletedWorkspaceCapture::completed_parent_metadata_fee_bytes_for_test();
    let held_bytes = CAPACITY
        .checked_sub(before)
        .unwrap()
        .checked_sub(required - 1)
        .unwrap();
    let held = pool.try_reserve(held_bytes).unwrap();
    assert_eq!(CAPACITY - pool.charged_for_test(), required - 1);
    let identity = t.producer.as_ref().unwrap().observe_original();
    assert!(PreparedV2TextCompletedWorkspaceCapture::prepare(
        store,
        owner.source(),
        t.producer.as_ref().unwrap(),
        t.selected.selected_data().unwrap(),
        t.query.as_ref().unwrap(),
        t.resident.as_ref().unwrap()
    )
    .is_err());
    identity
        .require_same_original(&t.producer.as_ref().unwrap().observe_original())
        .unwrap();
    t.producer
        .as_ref()
        .unwrap()
        .require_unmaterialized_data()
        .unwrap();
    assert!(t.predecessor.is_none() && t.transaction.is_none());
    assert!(t
        .query
        .as_ref()
        .unwrap()
        .token_value_data()
        .unwrap()
        .is_none());
    assert_eq!(pool.charged_for_test(), before + held_bytes);
    assert_eq!(known_files(&target), files);
    drop(held);
    assert_eq!(pool.charged_for_test(), before);
    t.workspace = Some(
        PreparedV2TextCompletedWorkspaceCapture::prepare(
            store,
            owner.source(),
            t.producer.as_ref().unwrap(),
            t.selected.selected_data().unwrap(),
            t.query.as_ref().unwrap(),
            t.resident.as_ref().unwrap(),
        )
        .unwrap(),
    );
    assert_eq!(pool.charged_for_test(), before + required);
    drop(owner);
    read_ready(&mut t);
    bind_completed(&mut t, false).unwrap();
    assert!(t.predecessor.is_some());
    assert_eq!(known_files(&target), files);
}

#[test]
fn actual_completed_parent_and_opaque_observer_keep_their_independent_fee_until_last_tail() {
    let mut t = next_create_fixture(0, 0x83420);
    read_ready(&mut t);
    let store = t.original.layout.checked_store().unwrap();
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let parent = t
        .workspace
        .as_ref()
        .unwrap()
        .ready_completed()
        .unwrap()
        .completed_parent_data()
        .unwrap()
        .clone();
    let original = t
        .workspace
        .as_ref()
        .unwrap()
        .ready_completed()
        .unwrap()
        .observe_completed_original();
    let before = pool.charged_for_test();
    let required =
        PreparedV2TextCompletedWorkspaceCapture::completed_parent_metadata_fee_bytes_for_test();
    // Producer/read/query/model stay alive, so their own independent credits cannot
    // masquerade as this class's refund. No actual Source File/lease remains here.
    drop(t.workspace.take());
    assert_eq!(pool.charged_for_test(), before);
    drop(parent);
    assert_eq!(pool.charged_for_test(), before);
    drop(original);
    assert_eq!(pool.charged_for_test(), before - required);
    t.producer
        .as_ref()
        .unwrap()
        .require_unmaterialized_data()
        .unwrap();
    assert!(t
        .query
        .as_ref()
        .unwrap()
        .token_value_data()
        .unwrap()
        .is_none());
    assert!(!t.model_dropped.load(Ordering::SeqCst));
}
