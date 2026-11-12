//! Actual selected-parent token readers. Empty is readonly DATA, never a workspace-clear
//! result or a Create admission. No artificial selected root or caller count is accepted.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextTokenQuery, PreparedV2TextQueryLease, PreparedV2TextTokenQueryCapture,
};
use crate::store::owner_evidence::canonical::text_lane::text_query_registry_counts_for_test;
use crate::store::persistence::v2_text::PreparedV2TextTokenQuery;
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;

fn selected_capture(original: &TextFixture) -> PreparedV2TextParentCapture {
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let mut capture = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
    let resources = capture.resources().clone();
    store
        .with_v2_text_current(&owner, |a| {
            let guard = store.lock_v2_text_current_group(a, &resources)?;
            capture.capture(&guard)?;
            Ok(())
        })
        .unwrap();
    capture
}

fn prepare_query(
    original: &TextFixture,
    selected: &PreparedV2TextParentCapture,
    token: [u8; 32],
) -> (
    PreparedV2TextTokenQuery,
    PreparedV2TextQueryLease,
    Option<PreparedV2TextTokenQueryCapture>,
) {
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let query = PreparedV2TextTokenQuery::prepare(
        store,
        owner.source(),
        selected.selected_data().unwrap(),
        token,
    )
    .unwrap();
    let reader = PreparedV2TextQueryLease::prepare(store, owner.source(), &query).unwrap();
    let capture = PreparedV2TextTokenQueryCapture::prepare(store, owner.source(), &query).unwrap();
    (query, reader, Some(capture))
}

fn admit_query(
    original: &TextFixture,
    query: &mut PreparedV2TextTokenQuery,
    reader: &mut PreparedV2TextQueryLease,
    capture: &mut Option<PreparedV2TextTokenQueryCapture>,
) -> AdmittedV2TextTokenQuery {
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let resources = query.resources().clone();
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_token_query_into(query, reader, capture, &mut admitted)
        })
        .unwrap();
    reader.drain_unused_outside_writers();
    assert!(capture.is_none());
    admitted.unwrap()
}

fn validate_query(original: &TextFixture, query: &PreparedV2TextTokenQuery) {
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let resources = query.resources().clone();
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            query.validate_completed_under_current_guard(&mut guard)
        })
        .unwrap();
}

#[test]
fn actual_token_query_caches_same_affine_reader_before_lost_return_and_final_use() {
    let original = super::promotion::promoted_text_fixture(0, 0x7ef00);
    let selected = selected_capture(&original);
    let (mut query, mut reader, mut capture) = prepare_query(&original, &selected, [1; 32]);
    let store = original.layout.checked_store().unwrap();
    assert!(query.token_value_data().is_err());
    let owner = original.owner.borrow_current().unwrap();
    assert!(PreparedV2TextQueryLease::prepare(store, owner.source(), &query).is_err());
    assert!(PreparedV2TextTokenQueryCapture::prepare(store, owner.source(), &query).is_err());
    drop(owner);
    let admitted = admit_query(&original, &mut query, &mut reader, &mut capture);
    assert_eq!(text_query_registry_counts_for_test(store), (1, 1));
    let owner = original.owner.borrow_current().unwrap();
    assert!(PreparedV2TextQueryLease::prepare(store, owner.source(), &query).is_err());
    assert!(PreparedV2TextTokenQueryCapture::prepare(store, owner.source(), &query).is_err());
    drop(owner);
    query.fail_after_completed_for_test();
    assert!(query.run(admitted).is_err());
    assert!(query.has_completed_data());
    assert!(query.token_value_data().unwrap().is_none());
    assert!(query.path_data().unwrap().is_none());
    validate_query(&original, &query);
    let owner = original.owner.borrow_current().unwrap();
    assert!(PreparedV2TextQueryLease::prepare(store, owner.source(), &query).is_err());
    assert!(PreparedV2TextTokenQueryCapture::prepare(store, owner.source(), &query).is_err());
    drop(owner);
    assert_eq!(text_query_registry_counts_for_test(store), (1, 1));
    drop(reader);
    assert_eq!(text_query_registry_counts_for_test(store), (1, 1));
    drop(query);
    assert_eq!(text_query_registry_counts_for_test(store), (1, 0));

    // A dropped UNSTARTED marker ends its actual pin/Files, but its preparer's paid
    // metadata still owns the SAME reader claim. Do not admit another preparation under
    // that original query fee until that precise preparer has also drained.
    let (mut query, mut reader, mut capture) = prepare_query(&original, &selected, [99; 32]);
    let admitted = admit_query(&original, &mut query, &mut reader, &mut capture);
    drop(admitted);
    assert_eq!(text_query_registry_counts_for_test(store), (1, 0));
    let owner = original.owner.borrow_current().unwrap();
    assert!(PreparedV2TextQueryLease::prepare(store, owner.source(), &query).is_err());
    let unused_capture =
        PreparedV2TextTokenQueryCapture::prepare(store, owner.source(), &query).unwrap();
    drop(unused_capture);
    drop(reader);
    let mut reader = PreparedV2TextQueryLease::prepare(store, owner.source(), &query).unwrap();
    let mut capture =
        Some(PreparedV2TextTokenQueryCapture::prepare(store, owner.source(), &query).unwrap());
    drop(owner);
    let admitted = admit_query(&original, &mut query, &mut reader, &mut capture);
    query.run(admitted).unwrap();
    validate_query(&original, &query);
}

#[test]
fn actual_late_query_capture_refusal_keeps_files_and_os_lease_in_one_outside_bundle() {
    use std::os::windows::fs::OpenOptionsExt;
    let original = super::promotion::promoted_text_fixture(0, 0x7ef10);
    let selected = selected_capture(&original);
    let (mut query, mut reader, mut capture) = prepare_query(&original, &selected, [2; 32]);
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let head_path = owner.source().source().capture.target.join("head.bin");
    let revocation = owner.warm_scope().unwrap().revocation();
    query.set_after_capture_files_for_test(Box::new(move || revocation.revoke()));
    let resources = query.resources().clone();
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_token_query_into(&mut query, &mut reader, &mut capture, &mut admitted)
        })
        .is_err());
    assert!(admitted.is_none());
    assert!(capture.as_ref().unwrap().has_source_for_test());
    assert_eq!(
        text_query_registry_counts_for_test(store),
        (1, 1),
        "failed Source capture retains its actual negative root/File pin"
    );
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&head_path)
        .is_err());
    drop(owner);
    drop(resources);
    drop(selected);
    let TextFixture {
        owner,
        layout,
        _directory: directory,
    } = original;
    drop(owner);
    drop(layout);
    assert!(matches!(
        catcoms_storage::acquire_vault_session(directory.path()),
        Err(catcoms_storage::StorageError::VaultBusy)
    ));
    drop(capture.take());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&head_path)
        .unwrap();
    drop(file);
    let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    // The inert query and unused reader preparer deliberately remain alive. Their
    // original fee/negative metadata cannot hide an actual File or physical OS lease.
    assert!(!query.has_completed_data());
    drop(session);
    drop(reader);
    drop(query);
}

#[test]
fn actual_query_admitted_before_scope_stop_finishes_read_but_cannot_validate_current() {
    let original = super::promotion::promoted_text_fixture(0, 0x7ef20);
    let selected = selected_capture(&original);
    let (mut query, mut reader, mut capture) = prepare_query(&original, &selected, [3; 32]);
    let admitted = admit_query(&original, &mut query, &mut reader, &mut capture);
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    owner.warm_scope().unwrap().revocation().revoke();
    query.run(admitted).unwrap();
    assert!(query.has_completed_data());
    assert!(query.token_value_data().unwrap().is_none());
    let resources = query.resources().clone();
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            query.validate_completed_under_current_guard(&mut guard)
        })
        .is_err());
    assert_eq!(text_query_registry_counts_for_test(store), (1, 1));
    drop(query);
    assert_eq!(text_query_registry_counts_for_test(store), (1, 0));
}

#[test]
fn actual_query_rechecks_selected_parent_before_pin_or_retained_source_files_open() {
    let original = super::promotion::promoted_text_fixture(0, 0x7ef28);
    let selected = selected_capture(&original);
    let (mut query, mut reader, mut capture) = prepare_query(&original, &selected, [8; 32]);
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let path = owner.source().source().capture.target.join("head.bin");
    let actual = std::fs::read(&path).unwrap();
    // This deliberately malformed replacement is NOT a later selected generation or
    // a forged Saved receipt. It isolates the mandatory fresh selected-parent proof.
    std::fs::write(&path, b"malformed replacement, never a selected O").unwrap();
    let resources = query.resources().clone();
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_token_query_into(&mut query, &mut reader, &mut capture, &mut admitted)
        })
        .is_err());
    assert!(admitted.is_none());
    assert!(!capture.as_ref().unwrap().has_source_for_test());
    assert_eq!(text_query_registry_counts_for_test(store), (0, 0));
    assert!(
        query.token_value_data().is_err(),
        "broken selection is never a miss"
    );
    std::fs::write(path, actual).unwrap();
    drop(owner);
    let admitted = admit_query(&original, &mut query, &mut reader, &mut capture);
    query.run(admitted).unwrap();
    validate_query(&original, &query);
}

#[test]
fn actual_query_registry_reuses_only_drained_metadata_after_more_than_32_read_origins() {
    let original = super::promotion::promoted_text_fixture(0, 0x7ef30);
    let store = original.layout.checked_store().unwrap();
    for number in 0..40u8 {
        // Each iteration independently pays/captures actual O/H. The immutable O bytes
        // stay equal, but the exact native read-resource identity is genuinely distinct.
        // These are40 actual read origins, not fabricated later private generations.
        let selected = selected_capture(&original);
        let (mut query, mut reader, mut capture) =
            prepare_query(&original, &selected, [number; 32]);
        let admitted = admit_query(&original, &mut query, &mut reader, &mut capture);
        query.run(admitted).unwrap();
        validate_query(&original, &query);
        assert_eq!(
            text_query_registry_counts_for_test(store),
            ((number as usize + 1).min(32), 1)
        );
        drop(query);
        drop(reader);
        assert_eq!(text_query_registry_counts_for_test(store).1, 0);
    }
}

#[test]
fn actual_query_reader_cap_refuses_65th_until_a_real_completed_reader_drops() {
    let original = super::promotion::promoted_text_fixture(0, 0x7ef40);
    let selected = selected_capture(&original);
    let store = original.layout.checked_store().unwrap();
    let mut live = Vec::with_capacity(64);
    for number in 0..64u8 {
        let (mut query, mut reader, mut capture) =
            prepare_query(&original, &selected, [number; 32]);
        let admitted = admit_query(&original, &mut query, &mut reader, &mut capture);
        query.run(admitted).unwrap();
        live.push((query, reader));
    }
    assert_eq!(text_query_registry_counts_for_test(store), (1, 64));
    let (mut waiting, mut reader, mut capture) = prepare_query(&original, &selected, [255; 32]);
    let owner = original.owner.borrow_current().unwrap();
    let resources = waiting.resources().clone();
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_token_query_into(
                &mut waiting,
                &mut reader,
                &mut capture,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none());
    assert!(
        !capture.as_ref().unwrap().has_source_for_test(),
        "reader pressure refuses BEFORE any retained Source File/key opens"
    );
    assert_eq!(text_query_registry_counts_for_test(store), (1, 64));
    drop(live.pop());
    assert_eq!(text_query_registry_counts_for_test(store), (1, 63));
    let admitted = admit_query(&original, &mut waiting, &mut reader, &mut capture);
    waiting.run(admitted).unwrap();
    validate_query(&original, &waiting);
    assert_eq!(text_query_registry_counts_for_test(store), (1, 64));
    drop(live);
    drop(waiting);
    assert_eq!(text_query_registry_counts_for_test(store), (1, 0));
}

#[test]
fn actual_32_live_read_origins_cannot_be_replaced_until_an_actual_affine_end_drains() {
    let original = super::promotion::promoted_text_fixture(0, 0x7ef50);
    let store = original.layout.checked_store().unwrap();
    let mut live = Vec::with_capacity(32);
    for number in 0..32u8 {
        let selected = selected_capture(&original);
        let (mut query, mut reader, mut capture) =
            prepare_query(&original, &selected, [number; 32]);
        let admitted = admit_query(&original, &mut query, &mut reader, &mut capture);
        query.run(admitted).unwrap();
        live.push((query, reader));
    }
    assert_eq!(text_query_registry_counts_for_test(store), (32, 32));
    let selected = selected_capture(&original);
    let (mut waiting, mut reader, mut capture) = prepare_query(&original, &selected, [255; 32]);
    let owner = original.owner.borrow_current().unwrap();
    let resources = waiting.resources().clone();
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_token_query_into(
                &mut waiting,
                &mut reader,
                &mut capture,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none());
    assert_eq!(text_query_registry_counts_for_test(store), (32, 32));
    let (last, preparation) = live.pop().unwrap();
    let old_negative = last.observe_original();
    drop(last);
    drop(preparation);
    assert_eq!(text_query_registry_counts_for_test(store), (32, 31));
    let admitted = admit_query(&original, &mut waiting, &mut reader, &mut capture);
    assert!(
        admitted.require_same_query(&old_negative).is_err(),
        "a successor slot never adopts the old opaque query identity"
    );
    waiting.run(admitted).unwrap();
    validate_query(&original, &waiting);
    assert_eq!(text_query_registry_counts_for_test(store), (32, 32));
    drop(live);
    drop(waiting);
    assert_eq!(text_query_registry_counts_for_test(store), (32, 0));
}

#[test]
fn actual_open_query_files_keep_vault_busy_until_source_and_worker_cleanup_drain() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let original = super::promotion::promoted_text_fixture(0, 0x7ef60);
    let selected = selected_capture(&original);
    let (mut query, mut reader, mut capture) = prepare_query(&original, &selected, [9; 32]);
    let admitted = admit_query(&original, &mut query, &mut reader, &mut capture);
    let owner = original.owner.borrow_current().unwrap();
    let head = owner.source().source().capture.target.join("head.bin");
    drop(owner);
    let (started_tx, started_rx) = sync_channel(0);
    let (release_tx, release_rx) = sync_channel(0);
    query.set_after_run_source_for_test(Box::new(move || {
        started_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    }));
    let worker = std::thread::spawn(move || {
        let result = query.run(admitted);
        (query, result)
    });
    started_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&head)
        .is_err());
    drop(selected);
    let TextFixture {
        owner,
        layout,
        _directory: directory,
    } = original;
    drop(owner);
    drop(layout);
    assert!(matches!(
        catcoms_storage::acquire_vault_session(directory.path()),
        Err(catcoms_storage::StorageError::VaultBusy)
    ));
    release_tx.send(()).unwrap();
    let (query, result) = worker.join().unwrap();
    result.unwrap();
    assert!(query.has_completed_data());
    let head_file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&head)
        .unwrap();
    drop(head_file);
    // This actual retained root-reader is logical cleanup custody, never a hidden OS
    // session after finite IO ended. Fresh Current/result use remains independently fenced.
    let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    drop(session);
    drop(query);
    drop(reader);
}

// Actual finite control-IO cleanup is distinct from retained logical root readers.
mod io;

// Genuine Clear-only workspace/control admissions before any SDK packet.
mod workspace;
