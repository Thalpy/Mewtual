//! Genuine current-A accepted lookup. Pending retained custody and a token hit are
//! deliberately different from the public accepted Directory/Actions lookup result.
//! These tests never manufacture SavedA, an Admission, roots, a lease or result DATA.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextAcceptedQuery, PreparedV2TextAcceptedQueryCapture,
    PreparedV2TextAcceptedQueryLease,
};
use crate::store::persistence::v2_text::{
    PreparedV2TextAcceptedQuery, V2TextAcceptedQueryResultData,
};

fn saved_fixture(family: usize, number: u64) -> RetainFixture {
    let mut f = retain_fixture(family, number);
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    let commit = commit_admission(&f);
    f.transaction.run_commit(commit).unwrap();
    f
}

fn prepare_accepted(
    f: &RetainFixture,
) -> (
    PreparedV2TextAcceptedQuery,
    PreparedV2TextAcceptedQueryLease,
    Option<PreparedV2TextAcceptedQueryCapture>,
) {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let query =
        PreparedV2TextAcceptedQuery::prepare(store, owner.source(), &f.transaction).unwrap();
    let reader = PreparedV2TextAcceptedQueryLease::prepare(store, owner.source(), &query).unwrap();
    let capture =
        PreparedV2TextAcceptedQueryCapture::prepare(store, owner.source(), &query).unwrap();
    (query, reader, Some(capture))
}

fn admit_accepted(
    f: &RetainFixture,
    query: &mut PreparedV2TextAcceptedQuery,
    reader: &mut PreparedV2TextAcceptedQueryLease,
    capture: &mut Option<PreparedV2TextAcceptedQueryCapture>,
) -> AdmittedV2TextAcceptedQuery {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = query.resources().clone();
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_accepted_query_into(query, reader, capture, &mut admitted)
        })
        .unwrap();
    assert!(capture.is_none());
    admitted.unwrap()
}

fn validate_accepted(
    f: &RetainFixture,
    query: &PreparedV2TextAcceptedQuery,
) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let resources = query.resources().clone();
    store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        query.validate_completed_under_current_guard(&mut guard)
    })
}

#[test]
fn actual_three_family_pending_a_token_hit_is_separate_from_verified_empty_accepted_directory() {
    for family in 0..3 {
        let f = saved_fixture(family, 0x7f800 + family as u64 * 16);
        let (mut query, mut reader, mut capture) = prepare_accepted(&f);
        if family == 0 {
            let store = f.original.layout.checked_store().unwrap();
            let owner = f.original.owner.borrow_current().unwrap();
            let (_, archive, _, _) = f
                .transaction
                .saved_retained_action()
                .unwrap()
                .selected_paths_for_test();
            let original = std::fs::read(archive).unwrap();
            let mut changed = original.clone();
            let at = changed.len() / 2;
            changed[at] ^= 0x80;
            std::fs::write(archive, changed).unwrap();
            let resources = query.resources().clone();
            let mut refused = None;
            assert!(store
                .with_v2_text_current(&owner, |a| {
                    let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                    guard.admit_text_accepted_query_into(
                        &mut query,
                        &mut reader,
                        &mut capture,
                        &mut refused,
                    )
                })
                .is_err());
            assert!(refused.is_none());
            assert!(!capture.as_ref().unwrap().has_source_for_test());
            assert_eq!(
                crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(
                    store
                ),
                0,
                "actual SavedA closure must be fresh BEFORE negative pin/source opens"
            );
            assert!(query.result_data().is_err());
            // Restore the SAME file's original bytes, then retry the unspent original
            // query. A damaged current manifest never falls back to its genesis copy.
            std::fs::write(archive, original).unwrap();
        }
        let admitted = admit_accepted(&f, &mut query, &mut reader, &mut capture);
        query.run(admitted).unwrap();
        assert!(query.has_completed_data());
        assert!(matches!(
            query.result_data().unwrap(),
            V2TextAcceptedQueryResultData::Absent
        ));
        validate_accepted(&f, &query).unwrap();
        // Actual A has a nonempty token tree for retained pending custody, while its
        // unchanged accepted root is still anchored to the genuine H_G public prefix.
        let selected = selected_capture(&f.original);
        let token = completed_query(&f.original, &selected, [31; 32]);
        assert!(token.token_value_data().unwrap().is_some());
        validate_query(&f.original, &token);
        assert!(selected
            .selected_data()
            .unwrap()
            .head_data()
            .pending
            .transaction_data()
            .is_some());
        assert!(matches!(
            query.result_data().unwrap(),
            V2TextAcceptedQueryResultData::Absent
        ));
    }
}

#[test]
fn actual_accepted_source_io_blocks_control_writer_then_logical_reader_survives_cleanup() {
    let f = saved_fixture(0, 0x7f840);
    let (mut query, mut reader, mut capture) = prepare_accepted(&f);
    let admitted = admit_accepted(&f, &mut query, &mut reader, &mut capture);
    let store = f.original.layout.checked_store().unwrap();
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        1
    );
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.transaction.resources().clone();
    let mut physical =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut commit = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_commit_into(
                &f.lane,
                &f.predecessor,
                f.transaction.ready_selection()?,
                &mut physical,
                &mut commit,
            )
        })
        .is_err());
    assert!(commit.is_none() && physical.is_some());
    query.run(admitted).unwrap();
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
    validate_accepted(&f, &query).unwrap();
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_commit_into(
                &f.lane,
                &f.predecessor,
                f.transaction.ready_selection()?,
                &mut physical,
                &mut commit,
            )
        })
        .unwrap();
    assert!(physical.is_none());
    // A REAL unstarted selector owner closes the read gate before any ephemeral O/H
    // final-use proof can open. The completed logical reader is neither a writer permit
    // nor a reason to strand the control writer after finite capture cleanup.
    assert!(validate_accepted(&f, &query).is_err());
    drop(commit.take());
    validate_accepted(&f, &query).unwrap();
    assert!(query.has_completed_data());
}

#[test]
fn actual_accepted_query_claims_and_lost_result_preserve_finite_data_across_scope_stop() {
    let f = saved_fixture(0, 0x7f850);
    let (mut query, mut reader, mut capture) = prepare_accepted(&f);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    assert!(PreparedV2TextAcceptedQueryLease::prepare(store, owner.source(), &query).is_err());
    assert!(PreparedV2TextAcceptedQueryCapture::prepare(store, owner.source(), &query).is_err());
    drop(owner);
    let admitted = admit_accepted(&f, &mut query, &mut reader, &mut capture);
    let owner = f.original.owner.borrow_current().unwrap();
    assert!(PreparedV2TextAcceptedQueryLease::prepare(store, owner.source(), &query).is_err());
    assert!(PreparedV2TextAcceptedQueryCapture::prepare(store, owner.source(), &query).is_err());
    drop(owner);
    query.fail_after_completed_for_test();
    assert!(query.run(admitted).is_err());
    assert!(query.has_completed_data());
    assert!(matches!(
        query.result_data().unwrap(),
        V2TextAcceptedQueryResultData::Absent
    ));
    validate_accepted(&f, &query).unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    assert!(PreparedV2TextAcceptedQueryLease::prepare(store, owner.source(), &query).is_err());
    assert!(PreparedV2TextAcceptedQueryCapture::prepare(store, owner.source(), &query).is_err());
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
    // The original preparer remains alive after real result completion. Its second
    // private claim alias must not duplicate the actual registered affine reader.
    reader.drain_unused_outside_writers();
    drop(owner);
    let (mut stopped, mut stopped_reader, mut stopped_capture) = prepare_accepted(&f);
    let admitted = admit_accepted(&f, &mut stopped, &mut stopped_reader, &mut stopped_capture);
    let owner = f.original.owner.borrow_current().unwrap();
    let revocation = owner.warm_scope().unwrap().revocation();
    stopped.set_after_run_source_for_test(Box::new(move || revocation.revoke()));
    stopped.run(admitted).unwrap();
    assert!(stopped.has_completed_data());
    assert!(matches!(
        stopped.result_data().unwrap(),
        V2TextAcceptedQueryResultData::Absent
    ));
    assert!(validate_accepted(&f, &stopped).is_err());
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
}

#[test]
fn actual_accepted_late_capture_error_keeps_files_native_lease_and_finite_io_coupled() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = saved_fixture(0, 0x7f860);
    let (mut query, mut reader, mut capture) = prepare_accepted(&f);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let head = owner.source().source().capture.target.join("head.bin");
    let revocation = owner.warm_scope().unwrap().revocation();
    query.set_after_capture_files_for_test(Box::new(move || revocation.revoke()));
    let resources = query.resources().clone();
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_accepted_query_into(
                &mut query,
                &mut reader,
                &mut capture,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none() && capture.as_ref().unwrap().has_source_for_test());
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        1
    );
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&head)
        .is_err());
    drop(owner);
    drop(resources);
    let RetainFixture {
        transaction,
        lane,
        predecessor,
        original,
    } = f;
    let TextFixture {
        owner,
        layout,
        _directory: directory,
    } = original;
    drop(transaction);
    drop(lane);
    drop(predecessor);
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
        .open(head)
        .unwrap();
    drop(file);
    let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    assert!(!query.has_completed_data());
    assert!(
        query.result_data().is_err(),
        "a read error cannot become Absent"
    );
    // Retain actual negative query/preparer debt after all Files/OS/IO have ended.
    drop(session);
    drop(reader);
    drop(query);
}

#[test]
fn actual_accepted_constructor_late_scope_stop_drains_copied_parent_before_exact_query_refund() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let f = saved_fixture(0, 0x7f870);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let head = owner.source().source().capture.target.join("head.bin");
    let before_head = std::fs::read(&head).unwrap();
    let before_readers = text_query_registry_counts_for_test(store);
    let fee_dropped = Arc::new(AtomicBool::new(false));
    let parent_dropped = Arc::new(AtomicBool::new(false));
    let copied = Arc::new(AtomicBool::new(false));
    let fee_at_copy = fee_dropped.clone();
    let parent_at_copy = parent_dropped.clone();
    let copied_at_hook = copied.clone();
    let revocation = owner.warm_scope().unwrap().revocation();
    let prepared = PreparedV2TextAcceptedQuery::with_prepare_observation_for_test(
        fee_dropped.clone(),
        parent_dropped.clone(),
        Box::new(move || {
            // This hook belongs to the REAL constructor after its paid parent copies,
            // not a manufactured drop order or a cancellation before allocation.
            assert!(!fee_at_copy.load(Ordering::SeqCst));
            assert!(!parent_at_copy.load(Ordering::SeqCst));
            copied_at_hook.store(true, Ordering::SeqCst);
            revocation.revoke();
        }),
        || PreparedV2TextAcceptedQuery::prepare(store, owner.source(), &f.transaction),
    );
    assert!(prepared.is_err());
    assert!(copied.load(Ordering::SeqCst));
    // The actual parent LAST notice asserts that the exact query debit is still held
    // after every copied frame/path/context field drains. Its refund then follows.
    assert!(parent_dropped.load(Ordering::SeqCst));
    assert!(fee_dropped.load(Ordering::SeqCst));
    assert_eq!(text_query_registry_counts_for_test(store), before_readers);
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
    assert_eq!(std::fs::read(head).unwrap(), before_head);
    assert!(f.transaction.saved_retained_action().is_ok());
    assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
}

mod completed;
