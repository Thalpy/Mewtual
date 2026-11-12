//! Actual retained-A admissions from the same original SDK and native predecessor.
//! These tests never fabricate Saved/Ready, set a writer flag, or claim accepted/public
//! history: A only installs durable pending custody, with the signed public header unchanged.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextRetainCommit, AdmittedV2TextRetainStage, PreparedV2TextPhysicalLease,
};
use crate::store::persistence::v2_text::{
    PreparedV2RetainOriginalCustody, PreparedV2TextRetainTransaction, V2TextQuotaPolicy,
    V2TextRetainTestCut,
};
use catcoms_sync::owner_v2::CREATE_DATA_BUFFER_BYTES;

struct RetainFixture {
    transaction: PreparedV2TextRetainTransaction,
    lane: V2TextCreateLane,
    predecessor: BoundV2TextCreatePredecessor,
    // LAST: the whole actual owner/store and its native directory outlive every field.
    original: TextFixture,
}

fn retain_fixture(family: usize, number: u64) -> RetainFixture {
    retain_fixture_with_selected(family, number).0
}

// Keep the ORIGINAL captured read body when a later genuine unsigned-SDK fixture
// needs it. Reopening equal O_G bytes would create a different paid body and must not
// adopt the actual Retain producer's source association.
fn retain_fixture_with_selected(
    family: usize,
    number: u64,
) -> (RetainFixture, PreparedV2TextParentCapture) {
    let mut original = super::super::super::promotion::promoted_text_fixture(family, number);
    let selected = selected_capture(&original);
    let query = completed_query(&original, &selected, [31; 32]);
    let lane = create_lane(&original, query.resources());
    let (mut producer, mut workspace) =
        workspace(&original, &selected, &query, "retained before acceptance");
    let read = read_workspace(&original, &lane, &mut workspace, &query);
    workspace.run_read(read).unwrap();
    let store = original.layout.checked_store().unwrap();
    let resources = producer.resources().clone();
    let mut predecessor = None;
    {
        let owner = original.owner.borrow_current().unwrap();
        store
            .with_v2_text_current(&owner, |a| {
                let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                guard.admit_text_create_predecessor_into(
                    &lane,
                    workspace.ready_clear()?,
                    &query,
                    &producer.observe_original(),
                    &mut predecessor,
                )
            })
            .unwrap();
    }
    let predecessor = predecessor.unwrap();
    drop(workspace);
    let mut sdk_fee = Some(
        store
            .archive_bootstrap_policy
            .as_ref()
            .unwrap()
            .pending
            .pool
            .try_reserve(CREATE_DATA_BUFFER_BYTES)
            .unwrap(),
    );
    let mut sdk = None;
    original
        .owner
        .prepare_create_data_for_test(
            store,
            V2CreateRequest {
                token: [31; 32],
                channel: 7,
                text: "retained before acceptance",
            },
            &mut sdk,
        )
        .unwrap();
    let mut packet = Some(sdk.take().unwrap().into_handoff_data().unwrap());
    let owner = original.owner.borrow_current().unwrap();
    producer
        .attach_packet(store, owner.source(), &mut packet)
        .unwrap();
    assert!(packet.is_none());
    producer
        .prepare_pages(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(number + 1),
        )
        .unwrap();
    let mut producer = Some(producer);
    let mut custody = None;
    PreparedV2RetainOriginalCustody::take_complete_into(&mut producer, &mut sdk_fee, &mut custody)
        .unwrap();
    assert!(producer.is_none() && sdk_fee.is_none());
    let mut query = Some(query);
    let mut transaction = None;
    PreparedV2TextRetainTransaction::take_into(
        store,
        owner.source(),
        selected.selected_data().unwrap(),
        &predecessor,
        V2TextQuotaPolicy::new(1024 * 1024 * 1024, 1024 * 1024).unwrap(),
        &mut custody,
        &mut query,
        &mut transaction,
    )
    .unwrap();
    assert!(custody.is_none() && query.is_none());
    let mut transaction = transaction.unwrap();
    transaction
        .prepare_candidates(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(number + 2),
        )
        .unwrap();
    drop(owner);
    let fixture = RetainFixture {
        transaction,
        lane,
        predecessor,
        original,
    };
    (fixture, selected)
}

fn stage_admission(f: &RetainFixture) -> AdmittedV2TextRetainStage {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.transaction.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_stage_into(
                &f.lane,
                &f.predecessor,
                &f.transaction,
                &mut lease,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(lease.is_none());
    admitted.unwrap()
}

fn commit_admission(f: &RetainFixture) -> AdmittedV2TextRetainCommit {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let ready = f.transaction.ready_selection().unwrap();
    let resources = ready.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_commit_into(
                &f.lane,
                &f.predecessor,
                ready,
                &mut lease,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(lease.is_none());
    admitted.unwrap()
}

#[test]
fn actual_three_family_a_stage_preserves_o_and_fresh_commit_installs_only_pending_custody() {
    for family in 0..3 {
        let mut f = retain_fixture(family, 0x7f600 + family as u64 * 16);
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.to_path_buf();
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let old = selected_capture(&f.original);
        let old = old.selected_data().unwrap();
        let header = old.archive_data().spec_data().header.id();
        let accepted = old.head_data().directory;
        let publications = old.head_data().publications;
        assert!(old.head_data().pending.transaction_data().is_none());
        drop(owner);
        let admitted = stage_admission(&f);
        f.transaction.run_stage(admitted).unwrap();
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert!(target.join("text.wal").is_file());
        assert!(f.transaction.ready_selection().is_ok());
        assert!(f.transaction.saved_retained_action().is_err());
        let admitted = commit_admission(&f);
        f.transaction.run_commit(admitted).unwrap();
        assert!(f.transaction.saved_retained_action().is_ok());
        let current = selected_capture(&f.original);
        let current = current.selected_data().unwrap();
        assert_eq!(
            current
                .head_data()
                .pending
                .transaction_data()
                .unwrap()
                .token,
            [31; 32]
        );
        assert_eq!(current.head_data().directory, accepted);
        assert_eq!(current.head_data().publications, publications);
        assert_eq!(current.archive_data().spec_data().header.id(), header);
        assert_ne!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(
            active_text_lane_count_for_test(store),
            1,
            "pending A never substitutes for ordinary WC retirement"
        );
    }
}

#[test]
fn actual_query_io_blocks_a_then_its_cleanup_allows_stage_while_logical_reader_survives() {
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let mut f = retain_fixture(0, 0x7f640);
    let selected = selected_capture(&f.original);
    let (mut query, mut reader, mut capture) = prepare_query(&f.original, &selected, [72; 32]);
    let query_work = admit_query(&f.original, &mut query, &mut reader, &mut capture);
    let (started_tx, started_rx) = sync_channel(0);
    let (release_tx, release_rx) = sync_channel(0);
    query.set_after_run_source_for_test(Box::new(move || {
        started_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    }));
    let worker = std::thread::spawn(move || {
        let result = query.run(query_work);
        (query, result)
    });
    started_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.transaction.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_stage_into(
                &f.lane,
                &f.predecessor,
                &f.transaction,
                &mut lease,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none() && lease.is_some());
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        1
    );
    release_tx.send(()).unwrap();
    let (query, result) = worker.join().unwrap();
    result.unwrap();
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
    assert!(text_query_registry_counts_for_test(store).1 >= 2);
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_stage_into(
                &f.lane,
                &f.predecessor,
                &f.transaction,
                &mut lease,
                &mut admitted,
            )
        })
        .unwrap();
    // The admitted but UNSTARTED actual writer gate blocks every new ephemeral O/H read.
    let mut blocked = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
    let read_resources = blocked.resources().clone();
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let guard = store.lock_v2_text_current_group(a, &read_resources)?;
            blocked.capture(&guard)?;
            Ok(())
        })
        .is_err());
    assert!(blocked.selected_data().is_none());
    drop(owner);
    f.transaction.run_stage(admitted.take().unwrap()).unwrap();
    validate_query(&f.original, &query);
    let commit = commit_admission(&f);
    f.transaction.run_commit(commit).unwrap();
    assert!(f.transaction.saved_retained_action().is_ok());
}

#[test]
fn actual_scope_stop_after_a_stage_admission_finishes_only_that_stage() {
    let mut f = retain_fixture(0, 0x7f650);
    let admitted = stage_admission(&f);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let head = owner.source().source().capture.target.join("head.bin");
    let before = std::fs::read(&head).unwrap();
    owner.warm_scope().unwrap().revocation().revoke();
    drop(owner);
    f.transaction.run_stage(admitted).unwrap();
    assert!(f.transaction.ready_selection().is_ok());
    assert!(f.transaction.saved_retained_action().is_err());
    assert_eq!(std::fs::read(head).unwrap(), before);
    let owner = f.original.owner.borrow_current().unwrap();
    assert!(
        PreparedV2TextPhysicalLease::prepare(store, owner.source(), f.transaction.resources())
            .is_err()
    );
    assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
}

#[test]
fn actual_scope_stop_after_a_commit_admission_cannot_erase_its_exact_saved_pending_result() {
    let mut f = retain_fixture(0, 0x7f660);
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    let commit = commit_admission(&f);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let head = owner.source().source().capture.target.join("head.bin");
    let before = std::fs::read(&head).unwrap();
    owner.warm_scope().unwrap().revocation().revoke();
    drop(owner);
    f.transaction.run_commit(commit).unwrap();
    assert!(f.transaction.saved_retained_action().is_ok());
    assert_ne!(std::fs::read(head).unwrap(), before);
    let owner = f.original.owner.borrow_current().unwrap();
    assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
}

#[test]
fn actual_open_a_wal_file_keeps_store_busy_until_files_lease_and_writer_end_all_drain() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let mut f = retain_fixture(0, 0x7f670);
    let admitted = stage_admission(&f);
    let owner = f.original.owner.borrow_current().unwrap();
    let wal = owner
        .source()
        .source()
        .capture
        .target
        .join("text.wal.stage.v1");
    drop(owner);
    let (started_tx, started_rx) = sync_channel(0);
    let (release_tx, release_rx) = sync_channel(0);
    f.transaction.set_work_pause_for_test(
        V2TextRetainTestCut::WalCreated,
        Box::new(move || {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        }),
    );
    f.transaction
        .set_work_fault_for_test(V2TextRetainTestCut::WalCreated);
    let RetainFixture {
        mut transaction,
        lane,
        predecessor,
        original,
    } = f;
    let worker = std::thread::spawn(move || {
        let result = transaction.run_stage(admitted);
        (transaction, result)
    });
    started_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    assert!(
        wal.is_file(),
        "the actual worker has already created its WAL-stage File"
    );
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&wal)
        .is_err());
    let TextFixture {
        owner,
        layout,
        _directory: directory,
    } = original;
    drop(owner);
    drop(lane);
    drop(predecessor);
    drop(layout);
    assert!(matches!(
        catcoms_storage::acquire_vault_session(directory.path()),
        Err(catcoms_storage::StorageError::VaultBusy)
    ));
    release_tx.send(()).unwrap();
    let (transaction, result) = worker.join().unwrap();
    assert!(result.is_err());
    assert!(transaction.ready_selection().is_err());
    assert!(transaction.saved_retained_action().is_err());
    let wal_file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(wal)
        .unwrap();
    drop(wal_file);
    // Original packet/candidate debt is deliberately still alive. It must not contain
    // an idle File/session or release its actual gate ahead of worker cleanup.
    let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    drop(session);
    drop(transaction);
}

#[test]
fn actual_admitted_a_commit_compares_full_original_wal_before_selecting_o() {
    let mut f = retain_fixture(0, 0x7f680);
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    let commit = commit_admission(&f);
    let owner = f.original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let wal = target.join("text.wal");
    let mut changed = std::fs::read(&wal).unwrap();
    assert!(changed.len() > 64);
    let offset = changed.len() / 2;
    changed[offset] ^= 0x80;
    // Deliberately change the SAME file after the real fresh admission. The exact
    // length/native identity can stay equal; the finite worker must compare full bytes.
    std::fs::write(&wal, &changed).unwrap();
    drop(owner);
    assert!(f.transaction.run_commit(commit).is_err());
    assert!(f.transaction.saved_retained_action().is_err());
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
    assert_eq!(
        std::fs::read(wal).unwrap(),
        changed,
        "unknown changed WAL is preserved"
    );
}

#[test]
fn actual_writer_gate_clear_interleaving_cannot_skip_a_new_query_before_next_commit() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let f = retain_fixture(0, 0x7f690);
    let selected = selected_capture(&f.original);
    let (mut query, mut reader, mut capture) = prepare_query(&f.original, &selected, [73; 32]);
    let mut stage = stage_admission(&f);
    let (gate_clear_tx, gate_clear_rx) = sync_channel(0);
    let (release_tx, release_rx) = sync_channel(0);
    stage.set_after_writer_gate_clear_for_test(Box::new(move || {
        gate_clear_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    }));
    let RetainFixture {
        mut transaction,
        lane,
        predecessor,
        original,
    } = f;
    let worker = std::thread::spawn(move || {
        let result = transaction.run_stage(stage);
        (transaction, result)
    });
    gate_clear_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    assert!(
        lane.workspace_active_for_test(),
        "real original end has not cleared cap yet"
    );
    let owner = original.owner.borrow_current().unwrap();
    let wal = owner.source().source().capture.target.join("text.wal");
    let wal_file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(wal)
        .unwrap();
    drop(wal_file);
    drop(owner);
    // Gate has cleared only AFTER actual WAL/helper/File cleanup. A readonly capture
    // may start, but the original control cap remains active during this deterministic
    // interleaving. Its actual finite IO must be observed by the next write admission.
    let query_work = admit_query(&original, &mut query, &mut reader, &mut capture);
    let store = original.layout.checked_store().unwrap();
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        1
    );
    release_tx.send(()).unwrap();
    let (mut transaction, result) = worker.join().unwrap();
    result.unwrap();
    assert!(!lane.workspace_active_for_test());
    let owner = original.owner.borrow_current().unwrap();
    let resources = transaction.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut commit = None;
    assert!(
        store
            .with_v2_text_current(&owner, |a| {
                let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                guard.admit_text_retain_commit_into(
                    &lane,
                    &predecessor,
                    transaction.ready_selection()?,
                    &mut lease,
                    &mut commit,
                )
            })
            .is_err(),
        "the new actual query IO cannot be skipped between write owners"
    );
    assert!(commit.is_none() && lease.is_some());
    query.run(query_work).unwrap();
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_commit_into(
                &lane,
                &predecessor,
                transaction.ready_selection()?,
                &mut lease,
                &mut commit,
            )
        })
        .unwrap();
    drop(owner);
    transaction.run_commit(commit.take().unwrap()).unwrap();
    assert!(transaction.saved_retained_action().is_ok());
}

mod semantics;
// Actual lost-return, selected-closure, namespace and exact-fee preservation cases.
mod preservation;

// Actual same-Cell warm WAL/output recovery, distinct from ordinary Stage/Commit.
mod recovery;

// Actual selected-A accepted Directory/Actions query and shared reader/IO boundaries.
mod accepted_query;

// Genuine whole unsigned SDK/model ownership, cancellation and exact-payment tests.
mod unsigned_custody;

// Actual unsigned-I owning custody, stage/commit and finite native cleanup regressions.
mod intent_admission;
