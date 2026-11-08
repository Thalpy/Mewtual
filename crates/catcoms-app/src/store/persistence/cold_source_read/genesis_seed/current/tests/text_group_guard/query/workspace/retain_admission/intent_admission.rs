//! Genuine first-I ownership and Current edges. None of these tests constructs a
//! SavedI, SDK plan, accepted-absence flag or marker; I remains unsigned throughout.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextIntentCommit, AdmittedV2TextIntentStage, PreparedV2TextAcceptedQueryCapture,
    PreparedV2TextAcceptedQueryLease,
};
use crate::store::persistence::v2_text::{
    PreparedV2TextAcceptedQuery, PreparedV2TextUnsignedIntent, V2TextIntentTestCut,
};
use crate::store::persistence::PreparedV2UnsignedOriginalCustody;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct IntentFixture {
    intent: PreparedV2TextUnsignedIntent,
    lane: V2TextCreateLane,
    predecessor: BoundV2TextCreatePredecessor,
    model_dropped: Arc<AtomicBool>,
    sdk_dropped: Arc<AtomicBool>,
    original: TextFixture,
}

fn intent_fixture(family: usize, number: u64) -> IntentFixture {
    let (mut f, selected_genesis) = retain_fixture_with_selected(family, number);
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    let commit = commit_admission(&f);
    f.transaction.run_commit(commit).unwrap();
    let model_dropped = Arc::new(AtomicBool::new(false));
    let sdk_dropped = Arc::new(AtomicBool::new(false));
    let (query, unsigned) = {
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        let mut query =
            PreparedV2TextAcceptedQuery::prepare(store, owner.source(), &f.transaction).unwrap();
        let mut reader =
            PreparedV2TextAcceptedQueryLease::prepare(store, owner.source(), &query).unwrap();
        let mut capture = Some(
            PreparedV2TextAcceptedQueryCapture::prepare(store, owner.source(), &query).unwrap(),
        );
        let resources = query.resources().clone();
        let mut admitted = None;
        store
            .with_v2_text_current(&owner, |a| {
                let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                guard.admit_text_accepted_query_into(
                    &mut query,
                    &mut reader,
                    &mut capture,
                    &mut admitted,
                )
            })
            .unwrap();
        reader.drain_unused_outside_writers();
        query.run(admitted.take().unwrap()).unwrap();
        drop(owner);
        let unsigned = PreparedV2UnsignedOriginalCustody::prepare_for_test(
            store,
            &f.original.owner,
            selected_genesis.selected_data().unwrap(),
            &f.transaction,
            model_dropped.clone(),
            sdk_dropped.clone(),
        )
        .unwrap();
        (query, unsigned)
    };
    let RetainFixture {
        transaction,
        lane,
        predecessor,
        original,
    } = f;
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let mut retained = Some(transaction);
    let mut unsigned = Some(unsigned);
    let mut query = Some(query);
    let mut intent = None;
    PreparedV2TextUnsignedIntent::take_into(
        store,
        owner.source(),
        &mut retained,
        &mut unsigned,
        &mut query,
        &mut intent,
    )
    .unwrap();
    assert!(retained.is_none() && unsigned.is_none() && query.is_none());
    let mut intent = intent.unwrap();
    intent
        .prepare_candidates(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(number + 3),
        )
        .unwrap();
    drop(owner);
    IntentFixture {
        intent,
        lane,
        predecessor,
        model_dropped,
        sdk_dropped,
        original,
    }
}

fn stage_i(f: &IntentFixture) -> AdmittedV2TextIntentStage {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.intent.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_intent_stage_into(
                &f.lane,
                &f.predecessor,
                &f.intent,
                &mut lease,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(lease.is_none());
    admitted.unwrap()
}

fn commit_i(f: &IntentFixture) -> AdmittedV2TextIntentCommit {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let ready = f.intent.ready_selection().unwrap();
    let resources = ready.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_intent_commit_into(
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

fn validate_saved_i(f: &IntentFixture) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let saved = f.intent.saved_unsigned_intent()?;
    let resources = saved.resources().clone();
    store.with_v2_text_current(&owner, |a| {
        let guard = store.lock_v2_text_current_group(a, &resources)?;
        saved.validate_selected_under_current_guard(&guard)
    })
}

fn i_target(f: &IntentFixture) -> std::path::PathBuf {
    f.original
        .owner
        .borrow_current()
        .unwrap()
        .source()
        .source()
        .capture
        .target
        .to_path_buf()
}

#[test]
fn actual_three_family_i_stage_keeps_a_selected_then_fresh_commit_selects_only_unsigned_intent() {
    for family in 0..3 {
        let mut f = intent_fixture(family, 0x7f900 + family as u64 * 16);
        let dir = i_target(&f);
        let a = std::fs::read(dir.join("head.bin")).unwrap();
        let wal_a = std::fs::read(dir.join("text.wal")).unwrap();
        let admitted = stage_i(&f);
        // A real unstarted I writer closes the same query gate before O/H opens.
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        let mut read = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
        let resources = read.resources().clone();
        assert!(store
            .with_v2_text_current(&owner, |a| {
                let guard = store.lock_v2_text_current_group(a, &resources)?;
                read.capture(&guard).map(|_| ())
            })
            .is_err());
        assert!(read.selected_data().is_none());
        drop(owner);
        f.intent.run_stage(admitted).unwrap();
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), a);
        assert_ne!(std::fs::read(dir.join("text.wal")).unwrap(), wal_a);
        assert!(f.intent.ready_selection().is_ok());
        assert!(f.intent.saved_unsigned_intent().is_err());
        let commit = commit_i(&f);
        f.intent.run_commit(commit).unwrap();
        validate_saved_i(&f).unwrap();
        assert_ne!(std::fs::read(dir.join("head.bin")).unwrap(), a);
        assert!(!f.model_dropped.load(Ordering::SeqCst));
        assert!(!f.sdk_dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn actual_i_ready_and_saved_survive_lost_returns_and_retries_use_no_new_nonce() {
    let mut f = intent_fixture(0, 0x7f940);
    let dir = i_target(&f);
    let a = std::fs::read(dir.join("head.bin")).unwrap();
    f.intent
        .set_work_fault_for_test(V2TextIntentTestCut::ReadyCached);
    let stage = stage_i(&f);
    assert!(f.intent.run_stage(stage).is_err());
    assert!(f.intent.has_ready_data() && !f.intent.has_saved_data());
    let wal = std::fs::read(dir.join("text.wal")).unwrap();
    let staged = std::fs::read(dir.join("head.stage")).unwrap();
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    f.intent
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .unwrap();
    drop(owner);
    let stage = stage_i(&f);
    f.intent.run_stage(stage).unwrap();
    assert_eq!(std::fs::read(dir.join("text.wal")).unwrap(), wal);
    assert_eq!(std::fs::read(dir.join("head.stage")).unwrap(), staged);
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), a);
    f.intent
        .set_work_fault_for_test(V2TextIntentTestCut::SavedCached);
    let commit = commit_i(&f);
    assert!(f.intent.run_commit(commit).is_err());
    assert!(f.intent.has_saved_data());
    validate_saved_i(&f).unwrap();
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), staged);
    assert_eq!(std::fs::read(dir.join("text.wal")).unwrap(), wal);
    let owner = f.original.owner.borrow_current().unwrap();
    f.intent
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .unwrap();
    validate_saved_i(&f).unwrap();
}

#[test]
fn actual_i_stage_or_commit_admitted_before_scope_stop_finishes_without_reviving_current() {
    for commit in [false, true] {
        let mut f = intent_fixture(0, 0x7f950 + u64::from(commit) * 16);
        let dir = i_target(&f);
        let a = std::fs::read(dir.join("head.bin")).unwrap();
        let stage = stage_i(&f);
        if commit {
            f.intent.run_stage(stage).unwrap();
            let admitted = commit_i(&f);
            let owner = f.original.owner.borrow_current().unwrap();
            owner.warm_scope().unwrap().revocation().revoke();
            f.intent.run_commit(admitted).unwrap();
            assert!(f.intent.has_saved_data());
            assert_ne!(std::fs::read(dir.join("head.bin")).unwrap(), a);
        } else {
            let owner = f.original.owner.borrow_current().unwrap();
            owner.warm_scope().unwrap().revocation().revoke();
            f.intent.run_stage(stage).unwrap();
            assert!(f.intent.has_ready_data() && !f.intent.has_saved_data());
            assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), a);
        }
        assert!(!f.lane.workspace_active_for_test());
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
        assert!(!f.model_dropped.load(Ordering::SeqCst));
        assert!(!f.sdk_dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn actual_i_commit_compares_full_wi_after_admission_before_any_selector_change() {
    let mut f = intent_fixture(0, 0x7f970);
    let stage = stage_i(&f);
    f.intent.run_stage(stage).unwrap();
    let dir = i_target(&f);
    let a = std::fs::read(dir.join("head.bin")).unwrap();
    let wal = dir.join("text.wal");
    let expected = std::fs::read(&wal).unwrap();
    let mut corrupt = expected.clone();
    let at = corrupt.len() / 2;
    corrupt[at] ^= 0x20;
    let commit = commit_i(&f);
    std::fs::write(&wal, &corrupt).unwrap();
    assert!(f.intent.run_commit(commit).is_err());
    assert!(!f.intent.has_saved_data());
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), a);
    assert_eq!(std::fs::read(&wal).unwrap(), corrupt);
    std::fs::write(&wal, expected).unwrap();
    let commit = commit_i(&f);
    f.intent.run_commit(commit).unwrap();
    validate_saved_i(&f).unwrap();
}

#[test]
fn actual_i_intent_file_keeps_vault_busy_until_worker_lease_end_cleanup_and_preserves_sdk_debt() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let mut f = intent_fixture(0, 0x7f980);
    let stage = stage_i(&f);
    let dir = i_target(&f);
    let model_dropped = f.model_dropped.clone();
    let sdk_dropped = f.sdk_dropped.clone();
    let (paused_tx, paused_rx) = sync_channel(0);
    let (release_tx, release_rx) = sync_channel(0);
    f.intent.set_work_pause_for_test(
        V2TextIntentTestCut::IntentCreated,
        Box::new(move || {
            paused_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        }),
    );
    f.intent
        .set_work_fault_for_test(V2TextIntentTestCut::IntentCreated);
    let IntentFixture {
        mut intent,
        lane,
        predecessor,
        model_dropped: _,
        sdk_dropped: _,
        original,
    } = f;
    let worker = std::thread::spawn(move || {
        let result = intent.run_stage(stage);
        (intent, result)
    });
    paused_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    let path = std::fs::read_dir(dir.join("text.pages"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("i-")
                && std::fs::metadata(path).unwrap().len() == 0
        })
        .expect("actual IntentCreated cut has created its named File");
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
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
    let (intent, result) = worker.join().unwrap();
    assert!(result.is_err());
    assert!(!intent.has_ready_data() && !intent.has_saved_data());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap();
    drop(file);
    let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    assert!(!model_dropped.load(Ordering::SeqCst));
    assert!(!sdk_dropped.load(Ordering::SeqCst));
    drop(session);
    // Failed finite work keeps the SAME original SDK/model custody, but no dormant
    // File/session. Only dropping its actual owner refunds those exact independent fees.
    drop(intent);
    assert!(model_dropped.load(Ordering::SeqCst));
    assert!(sdk_dropped.load(Ordering::SeqCst));
}

// Actual Ready-A/Ready-I held-control ordering at Source/ID3 (no signing).
mod held_proof;

// Genuine recorded-original I prefix continuation and preservation regressions.
mod constructor;
mod semantic;
mod warm;

mod raw_call;

mod raw_admission;
