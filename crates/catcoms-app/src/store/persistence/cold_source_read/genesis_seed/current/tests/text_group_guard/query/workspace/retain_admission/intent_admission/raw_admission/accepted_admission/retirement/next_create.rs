//! A genuine completed first transaction is disposed before request two starts.
//! These fixtures never reconstruct G, fabricate a WC, set a count, or relabel a
//! present WAL as Clear. Next-A has its own SDK/work/query/metadata obligations.
use super::finish::{completed_lane_fixture, LaneFinishFixture};
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextCompletedWorkspaceRead, BoundV2TextCompletedCreatePredecessor,
    PreparedV2TextCompletedWorkspaceReadCapture,
};
use crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test;
use crate::store::persistence::v2_text::{
    PreparedV2RetainData, PreparedV2TextCompletedWorkspaceCapture,
};
use crate::store::persistence::PreparedV2TextResidentProjection;

const NEXT_TOKEN: [u8; 32] = [43; 32];
const NEXT_TEXT: &str = "second founder create after actual WC";

pub(super) struct NextCreateFixture {
    pub(super) transaction: Option<PreparedV2TextRetainTransaction>,
    pub(super) predecessor: Option<BoundV2TextCompletedCreatePredecessor>,
    pub(super) workspace: Option<PreparedV2TextCompletedWorkspaceCapture>,
    pub(super) producer: Option<PreparedV2RetainData>,
    pub(super) query: Option<PreparedV2TextTokenQuery>,
    pub(super) selected: PreparedV2TextParentCapture,
    pub(super) lane: V2TextCreateLane,
    pub(super) resident: Option<PreparedV2TextResidentProjection>,
    pub(super) model_dropped: Arc<AtomicBool>,
    // Original native owner is whole and LAST; no Root/model/SDK extractor.
    pub(super) original: TextFixture,
}

fn take_actual_resident(
    family: usize,
    number: u64,
) -> (
    TextFixture,
    PreparedV2TextResidentProjection,
    Arc<AtomicBool>,
) {
    let mut t = completed_lane_fixture(family, number);
    let mut resident = None;
    let mut disposal = None;
    {
        let accepted = t.accepted.as_ref().unwrap();
        let saved = accepted.saved_retirement_completion().unwrap().clone();
        let resources = accepted.resources().clone();
        let store = t.original.layout.checked_store().unwrap();
        let owner = t.original.owner.borrow_current().unwrap();
        store
            .with_v2_text_current(&owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                let finish = guard.admit_completed_finish(
                    t.retired.as_ref().unwrap(),
                    &saved,
                    t.accepted.as_ref().unwrap(),
                )?;
                PreparedV2TextAcceptedCreate::finish_resident_into(
                    &finish,
                    &mut t.accepted,
                    &mut resident,
                    &mut disposal,
                )
            })
            .unwrap();
    }
    let LaneFinishFixture {
        accepted,
        predecessor,
        original,
        model_dropped,
        sdk_dropped,
        basis,
        lane,
        drained,
        retired,
    } = t;
    assert!(accepted.is_none() && lane.is_none() && drained.is_none());
    drop(predecessor);
    drop(basis);
    drop(retired);
    drop(disposal); // actual old SDK/query/read/WAL/transaction tails, outside writers
    assert!(sdk_dropped.load(Ordering::SeqCst));
    assert!(!model_dropped.load(Ordering::SeqCst));
    assert_eq!(
        active_text_lane_count_for_test(original.layout.checked_store().unwrap()),
        0
    );
    (original, resident.unwrap(), model_dropped)
}

pub(super) fn next_create_fixture(family: usize, number: u64) -> NextCreateFixture {
    let (original, resident, model_dropped) = take_actual_resident(family, number);
    let selected = selected_capture(&original);
    let query = completed_query(&original, &selected, NEXT_TOKEN);
    assert!(query.token_value_data().unwrap().is_none());
    let lane = create_lane(&original, query.resources());
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let producer = PreparedV2RetainData::prepare(
        store,
        owner.source(),
        selected.selected_data().unwrap(),
        V2CreateRequest {
            token: NEXT_TOKEN,
            channel: 7,
            text: NEXT_TEXT,
        },
    )
    .unwrap();
    let workspace = PreparedV2TextCompletedWorkspaceCapture::prepare(
        store,
        owner.source(),
        &producer,
        selected.selected_data().unwrap(),
        &query,
        &resident,
    )
    .unwrap();
    drop(owner);
    NextCreateFixture {
        transaction: None,
        predecessor: None,
        workspace: Some(workspace),
        producer: Some(producer),
        query: Some(query),
        selected,
        lane,
        resident: Some(resident),
        model_dropped,
        original,
    }
}

fn read_completed(
    t: &mut NextCreateFixture,
    capture: &mut Option<PreparedV2TextCompletedWorkspaceReadCapture>,
    admitted: &mut Option<AdmittedV2TextCompletedWorkspaceRead>,
) -> Result<(), AppError> {
    let store = t.original.layout.checked_store()?;
    let owner = t.original.owner.borrow_current()?;
    let prepared = t.workspace.as_mut().unwrap();
    let resources = prepared.resources().clone();
    store.with_v2_text_current(&owner, |admission| {
        let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
        guard.admit_text_completed_workspace_read_into(
            &t.lane,
            prepared,
            t.query.as_ref().unwrap(),
            t.resident.as_ref().unwrap(),
            capture,
            admitted,
        )
    })
}

pub(super) fn prepare_completed_predecessor(t: &mut NextCreateFixture) {
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
    bind_completed(t, false).unwrap();
}

fn bind_completed(t: &mut NextCreateFixture, lose_return: bool) -> Result<(), AppError> {
    let store = t.original.layout.checked_store()?;
    let owner = t.original.owner.borrow_current()?;
    let prepared = t.workspace.as_ref().unwrap();
    let resources = prepared.resources().clone();
    let original = t.producer.as_ref().unwrap().observe_original();
    let mut lost = Some(AppError::Invalid(
        "injected completed predecessor lost return".into(),
    ));
    store.with_v2_text_current(&owner, |admission| {
        let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
        guard.admit_text_completed_create_predecessor_into(
            &t.lane,
            prepared.ready_completed()?,
            t.query.as_ref().unwrap(),
            t.resident.as_ref().unwrap(),
            &original,
            &mut t.predecessor,
        )?;
        if lose_return {
            return Err(lost.take().unwrap());
        }
        Ok(())
    })
}

pub(super) fn prepare_next_a(t: &mut NextCreateFixture, number: u64) {
    if t.predecessor.is_none() {
        prepare_completed_predecessor(t);
    }
    let store = t.original.layout.checked_store().unwrap();
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
    t.resident
        .as_ref()
        .unwrap()
        .validate_negative(store)
        .unwrap();
    t.predecessor.as_ref().unwrap().validate_negative().unwrap();
    let mut sdk = None;
    t.original
        .owner
        .prepare_create_data_for_test(
            store,
            V2CreateRequest {
                token: NEXT_TOKEN,
                channel: 7,
                text: NEXT_TEXT,
            },
            &mut sdk,
        )
        .unwrap();
    let mut packet = Some(sdk.take().unwrap().into_handoff_data().unwrap());
    let owner = t.original.owner.borrow_current().unwrap();
    let producer = t.producer.as_mut().unwrap();
    producer
        .attach_packet(store, owner.source(), &mut packet)
        .unwrap();
    producer
        .prepare_pages(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(number + 1),
        )
        .unwrap();
    let mut custody = None;
    PreparedV2RetainOriginalCustody::take_complete_into(
        &mut t.producer,
        &mut sdk_fee,
        &mut custody,
    )
    .unwrap();
    PreparedV2TextRetainTransaction::take_from_completed_into(
        store,
        owner.source(),
        t.selected.selected_data().unwrap(),
        t.predecessor.as_ref().unwrap(),
        t.workspace.as_ref().unwrap().ready_completed().unwrap(),
        V2TextQuotaPolicy::new(1024 * 1024 * 1024, 1024 * 1024).unwrap(),
        &mut custody,
        &mut t.query,
        &mut t.transaction,
    )
    .unwrap();
    assert!(custody.is_none() && t.query.is_none());
    t.transaction
        .as_mut()
        .unwrap()
        .prepare_candidates(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(number + 2),
        )
        .unwrap();
    drop(owner);
    // Actual next-A has its own exact completed predecessor. No large read-plan
    // alias or obsolete-WC source remains necessary for continuation after WA.
    drop(t.workspace.take());
}

pub(super) fn next_stage(t: &NextCreateFixture) -> AdmittedV2TextRetainStage {
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let transaction = t.transaction.as_ref().unwrap();
    let resources = transaction.resources().clone();
    let mut physical =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_completed_retain_stage_into(
                &t.lane,
                t.predecessor.as_ref().unwrap(),
                transaction,
                &mut physical,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(physical.is_none());
    admitted.unwrap()
}

pub(super) fn next_commit(t: &NextCreateFixture) -> AdmittedV2TextRetainCommit {
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let ready = t.transaction.as_ref().unwrap().ready_selection().unwrap();
    let resources = ready.resources().clone();
    let mut physical =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_completed_retain_commit_into(
                &t.lane,
                t.predecessor.as_ref().unwrap(),
                ready,
                &mut physical,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(physical.is_none());
    admitted.unwrap()
}

pub(super) fn next_saved_a_fixture(family: usize, number: u64) -> NextCreateFixture {
    let mut t = next_create_fixture(family, number);
    prepare_next_a(&mut t, number);
    let stage = next_stage(&t);
    t.transaction.as_mut().unwrap().run_stage(stage).unwrap();
    let commit = next_commit(&t);
    t.transaction.as_mut().unwrap().run_commit(commit).unwrap();
    t
}

#[test]
fn actual_completed_wc_role_keeps_clear_strict_and_retains_bound_after_lost_return() {
    let mut t = next_create_fixture(0, 0x82b00);
    let target = t
        .original
        .owner
        .borrow_current()
        .unwrap()
        .source()
        .source()
        .capture
        .target
        .to_path_buf();
    let wc = std::fs::read(target.join("text.wal")).unwrap();
    // Release the actual single plan claim first. Refusal below must arise from
    // present completed WC, not merely a competing maximum-one preparation cap.
    drop(t.workspace.take());
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let mut clear = PreparedV2TextWorkspaceCapture::prepare(
        store,
        owner.source(),
        t.producer.as_ref().unwrap(),
        t.selected.selected_data().unwrap(),
        t.query.as_ref().unwrap(),
    )
    .unwrap();
    drop(owner);
    let clear_read = read_workspace(&t.original, &t.lane, &mut clear, t.query.as_ref().unwrap());
    assert!(clear.run_read(clear_read).is_err());
    assert!(!clear.has_ready_data());
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wc);
    drop(clear);
    let owner = t.original.owner.borrow_current().unwrap();
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
    read_completed(&mut t, &mut capture, &mut admitted).unwrap();
    t.workspace.as_mut().unwrap().fail_after_ready_for_test();
    assert!(t
        .workspace
        .as_mut()
        .unwrap()
        .run_read(admitted.take().unwrap())
        .is_err());
    assert!(t.workspace.as_ref().unwrap().has_ready_data());
    assert!(!t.lane.workspace_active_for_test());
    assert!(bind_completed(&mut t, true).is_err());
    assert!(t.predecessor.is_some());
    bind_completed(&mut t, false).unwrap();
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wc);
    assert!(t.transaction.is_none());
    assert_eq!(
        text_query_control_io_for_test(t.original.layout.checked_store().unwrap()),
        0
    );
}

#[test]
fn actual_completed_parent_late_capture_stop_retains_files_lease_and_real_end_until_drop() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut t = next_create_fixture(0, 0x82b10);
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let wal = owner.source().source().capture.target.join("text.wal");
    let revoke = owner.warm_scope().unwrap().revocation();
    t.workspace
        .as_mut()
        .unwrap()
        .set_after_capture_files_for_test(Box::new(move || revoke.revoke()));
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
    assert!(read_completed(&mut t, &mut capture, &mut admitted).is_err());
    assert!(admitted.is_none() && capture.as_ref().unwrap().has_source_for_test());
    assert!(t.lane.workspace_active_for_test());
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&wal)
        .is_err());
    drop(capture);
    assert!(!t.lane.workspace_active_for_test());
    drop(
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&wal)
            .unwrap(),
    );
    assert!(t.predecessor.is_none());
}

#[test]
fn actual_next_a_lost_wa_rename_continues_from_same_wa_without_requiring_obsolete_wc() {
    let mut t = next_create_fixture(0, 0x82b20);
    prepare_next_a(&mut t, 0x82b20);
    let target = t
        .original
        .owner
        .borrow_current()
        .unwrap()
        .source()
        .source()
        .capture
        .target
        .to_path_buf();
    let wc = std::fs::read(target.join("text.wal")).unwrap();
    t.transaction
        .as_mut()
        .unwrap()
        .set_work_fault_for_test(V2TextRetainTestCut::WalRenamed);
    let stage = next_stage(&t);
    assert!(t.transaction.as_mut().unwrap().run_stage(stage).is_err());
    let wa = std::fs::read(target.join("text.wal")).unwrap();
    assert_ne!(wa, wc);
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    t.transaction
        .as_mut()
        .unwrap()
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .unwrap();
    drop(owner);
    let stage = next_stage(&t);
    t.transaction.as_mut().unwrap().run_stage(stage).unwrap();
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wa);
    let commit = next_commit(&t);
    let owner = t.original.owner.borrow_current().unwrap();
    owner.warm_scope().unwrap().revocation().revoke();
    // Already-admitted finite selector work still finishes, without renewing a
    // Current or the completed parent that its own WA has legitimately replaced.
    t.transaction.as_mut().unwrap().run_commit(commit).unwrap();
    assert!(t
        .transaction
        .as_ref()
        .unwrap()
        .saved_retained_action()
        .is_ok());
    assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wa);
    assert!(!t.lane.workspace_active_for_test());
}

#[test]
fn actual_completed_workspace_rejects_foreign_live_resident_and_independent_domain_stop() {
    let mut a = next_create_fixture(0, 0x82b30);
    let b = next_create_fixture(1, 0x82b31);
    let store = a.original.layout.checked_store().unwrap();
    let owner = a.original.owner.borrow_current().unwrap();
    let workspace = a.workspace.as_mut().unwrap();
    let resources = workspace.resources().clone();
    let mut capture = Some(
        PreparedV2TextCompletedWorkspaceReadCapture::prepare(store, owner.source(), workspace)
            .unwrap(),
    );
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_completed_workspace_read_into(
                &a.lane,
                workspace,
                a.query.as_ref().unwrap(),
                b.resident.as_ref().unwrap(),
                &mut capture,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none() && !capture.as_ref().unwrap().has_source_for_test());
    assert!(!a.lane.workspace_active_for_test());
    a.resident
        .as_ref()
        .unwrap()
        .cancel_original_domain_for_test();
    store.with_v2_text_current(&owner, |_| Ok(())).unwrap();
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_completed_workspace_read_into(
                &a.lane,
                workspace,
                a.query.as_ref().unwrap(),
                a.resident.as_ref().unwrap(),
                &mut capture,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none() && !capture.as_ref().unwrap().has_source_for_test());
    assert!(!a.lane.workspace_active_for_test());
}

mod physical;
mod staged;

mod unsigned;
