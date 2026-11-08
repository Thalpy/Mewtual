//! Genuine Clear-only workspace/control admissions. No packet, SDK signer, SavedA/I,
//! selection/acceptance, ordinary WC or complete dirty-WAL recovery is fabricated.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextWorkspaceRead, BoundV2TextCreatePredecessor, PreparedV2TextWorkspaceReadCapture,
};
use crate::store::owner_evidence::canonical::text_lane::{
    active_text_lane_count_for_test, PreparedV2TextCreateLane, PreparedV2TextLane, V2TextCreateLane,
};
use crate::store::persistence::v2_text::{PreparedV2RetainData, PreparedV2TextWorkspaceCapture};
use crate::store::persistence::PreparedV2TextSourceObserver;
use catcoms_sync::owner_v2::V2CreateRequest;

fn create_lane(
    original: &TextFixture,
    resources: &crate::store::persistence::v2_text::V2TextReadResources,
) -> V2TextCreateLane {
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let observer = PreparedV2TextSourceObserver::prepare(store, &owner).unwrap();
    let observer = store
        .with_v2_text_current(&owner, |a| observer.capture(a))
        .unwrap();
    let mut raw_prepared = PreparedV2TextLane::prepare(store, &owner, &observer).unwrap();
    let mut raw = None;
    store
        .with_v2_text_current(&owner, |a| {
            store
                .lock_v2_text_owner_namespace(a)?
                .claim_text_lane_into(&mut raw_prepared, &mut raw)
        })
        .unwrap();
    raw_prepared.drain_unused_outside_writers();
    let mut prepared =
        PreparedV2TextCreateLane::prepare(store, &owner, raw.as_ref().unwrap()).unwrap();
    let mut retained = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, resources)?;
            guard.convert_text_create_lane_into(&mut prepared, &mut raw, &mut retained)
        })
        .unwrap();
    retained.unwrap()
}

fn completed_query(
    original: &TextFixture,
    selected: &PreparedV2TextParentCapture,
    token: [u8; 32],
) -> PreparedV2TextTokenQuery {
    let (mut query, mut reader, mut capture) = prepare_query(original, selected, token);
    let admitted = admit_query(original, &mut query, &mut reader, &mut capture);
    query.run(admitted).unwrap();
    validate_query(original, &query);
    query
}

fn workspace(
    original: &TextFixture,
    selected: &PreparedV2TextParentCapture,
    query: &PreparedV2TextTokenQuery,
    text: &str,
) -> (PreparedV2RetainData, PreparedV2TextWorkspaceCapture) {
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let producer = PreparedV2RetainData::prepare(
        store,
        owner.source(),
        selected.selected_data().unwrap(),
        V2CreateRequest {
            token: [31; 32],
            channel: 7,
            text,
        },
    )
    .unwrap();
    let workspace = PreparedV2TextWorkspaceCapture::prepare(
        store,
        owner.source(),
        &producer,
        selected.selected_data().unwrap(),
        query,
    )
    .unwrap();
    (producer, workspace)
}

fn read_workspace(
    original: &TextFixture,
    lane: &V2TextCreateLane,
    prepared: &mut PreparedV2TextWorkspaceCapture,
    query: &PreparedV2TextTokenQuery,
) -> AdmittedV2TextWorkspaceRead {
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let mut capture =
        Some(PreparedV2TextWorkspaceReadCapture::prepare(store, owner.source(), prepared).unwrap());
    let resources = prepared.resources().clone();
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_workspace_read_into(lane, prepared, query, &mut capture, &mut admitted)
        })
        .unwrap();
    assert!(capture.is_none());
    admitted.unwrap()
}

#[test]
fn actual_workspace_clear_lost_return_binds_only_one_original_without_sdk_or_head_write() {
    let original = super::super::promotion::promoted_text_fixture(0, 0x7f000);
    let selected = selected_capture(&original);
    let query = completed_query(&original, &selected, [31; 32]);
    let lane = create_lane(&original, query.resources());
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let head = owner.source().source().capture.target.join("head.bin");
    let before = std::fs::read(&head).unwrap();
    let sdk_before = super::super::super::super::super::SDK_ENTRIES.with(|n| n.get());
    let (producer, mut prepared) = workspace(&original, &selected, &query, "first request");
    assert!(
        PreparedV2TextWorkspaceCapture::prepare(
            store,
            owner.source(),
            &producer,
            selected.selected_data().unwrap(),
            &query
        )
        .is_err(),
        "one WHOLE paid workspace, including idle Ready tails"
    );
    drop(owner);
    let admitted = read_workspace(&original, &lane, &mut prepared, &query);
    assert!(lane.workspace_active_for_test());
    prepared.fail_after_ready_for_test();
    assert!(prepared.run_read(admitted).is_err());
    assert!(prepared.has_ready_data());
    assert!(!lane.workspace_active_for_test());
    let (other_producer, mut other) = workspace(&original, &selected, &query, "changed request");
    let admitted = read_workspace(&original, &lane, &mut other, &query);
    other.run_read(admitted).unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let resources = prepared.resources().clone();
    let mut bound: Option<BoundV2TextCreatePredecessor> = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_create_predecessor_into(
                &lane,
                prepared.ready_clear()?,
                &query,
                &producer.observe_original(),
                &mut bound,
            )?;
            Err::<(), _>(critical_namespaces::refused())
        })
        .is_err());
    bound
        .as_ref()
        .unwrap()
        .require_same_original(&producer.observe_original())
        .unwrap();
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_create_predecessor_into(
                &lane,
                prepared.ready_clear()?,
                &query,
                &producer.observe_original(),
                &mut bound,
            )
        })
        .unwrap();
    let mut conflicting = None;
    let mut duplicate = None;
    assert!(
        store
            .with_v2_text_current(&owner, |a| {
                let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                guard.admit_text_create_predecessor_into(
                    &lane,
                    prepared.ready_clear()?,
                    &query,
                    &producer.observe_original(),
                    &mut duplicate,
                )
            })
            .is_err(),
        "a second owning slot cannot duplicate the same paid binding"
    );
    assert!(duplicate.is_none());
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_create_predecessor_into(
                &lane,
                other.ready_clear()?,
                &query,
                &other_producer.observe_original(),
                &mut conflicting,
            )
        })
        .is_err());
    assert!(conflicting.is_none());
    assert_eq!(std::fs::read(head).unwrap(), before);
    assert_eq!(
        super::super::super::super::super::SDK_ENTRIES.with(|n| n.get()),
        sdk_before
    );
    assert_eq!(
        active_text_lane_count_for_test(store),
        1,
        "logical binding is not ordinary WC retirement"
    );
}

#[test]
fn actual_unstarted_workspace_marker_blocks_predecessor_until_its_real_end_drains() {
    let original = super::super::promotion::promoted_text_fixture(0, 0x7f010);
    let selected = selected_capture(&original);
    let query = completed_query(&original, &selected, [31; 32]);
    let lane = create_lane(&original, query.resources());
    let (producer, mut prepared) = workspace(&original, &selected, &query, "request");
    let admitted = read_workspace(&original, &lane, &mut prepared, &query);
    prepared.run_read(admitted).unwrap();
    let admitted = read_workspace(&original, &lane, &mut prepared, &query);
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    assert!(PreparedV2TextWorkspaceReadCapture::prepare(store, owner.source(), &prepared).is_err());
    let resources = prepared.resources().clone();
    let mut bound = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_create_predecessor_into(
                &lane,
                prepared.ready_clear()?,
                &query,
                &producer.observe_original(),
                &mut bound,
            )
        })
        .is_err());
    assert!(bound.is_none());
    assert!(lane.workspace_active_for_test());
    drop(admitted);
    assert!(!lane.workspace_active_for_test());
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_create_predecessor_into(
                &lane,
                prepared.ready_clear()?,
                &query,
                &producer.observe_original(),
                &mut bound,
            )
        })
        .unwrap();
    assert!(bound.is_some());
    assert_eq!(
        text_query_registry_counts_for_test(store),
        (1, 1),
        "own logical query stays live"
    );
}

#[test]
fn actual_failed_workspace_capture_couples_files_os_lease_and_actual_lane_end() {
    use std::os::windows::fs::OpenOptionsExt;
    let original = super::super::promotion::promoted_text_fixture(0, 0x7f020);
    let selected = selected_capture(&original);
    let query = completed_query(&original, &selected, [31; 32]);
    let lane = create_lane(&original, query.resources());
    let (producer, mut prepared) = workspace(&original, &selected, &query, "request");
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let head = owner.source().source().capture.target.join("head.bin");
    let revocation = owner.warm_scope().unwrap().revocation();
    prepared.set_after_capture_files_for_test(Box::new(move || revocation.revoke()));
    let mut capture = Some(
        PreparedV2TextWorkspaceReadCapture::prepare(store, owner.source(), &prepared).unwrap(),
    );
    let resources = prepared.resources().clone();
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_workspace_read_into(
                &lane,
                &mut prepared,
                &query,
                &mut capture,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none());
    assert!(capture.as_ref().unwrap().has_source_for_test());
    assert!(lane.workspace_active_for_test());
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&head)
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
    assert!(!lane.workspace_active_for_test());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(head)
        .unwrap();
    drop(file);
    let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    assert!(!prepared.has_ready_data());
    drop(session);
    drop(query);
    drop(prepared);
    drop(producer);
    drop(lane);
}

#[test]
fn actual_ready_clear_is_not_reused_after_wal_appears_or_bound_on_failed_inspection() {
    let original = super::super::promotion::promoted_text_fixture(0, 0x7f030);
    let selected = selected_capture(&original);
    let query = completed_query(&original, &selected, [31; 32]);
    let lane = create_lane(&original, query.resources());
    let (producer, mut prepared) = workspace(&original, &selected, &query, "first request");
    let admitted = read_workspace(&original, &lane, &mut prepared, &query);
    prepared.run_read(admitted).unwrap();
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let wal = owner.source().source().capture.target.join("text.wal");
    let bytes = b"unselected unauthenticated test WAL; never an accepted transaction";
    std::fs::write(&wal, bytes).unwrap();
    let resources = prepared.resources().clone();
    let mut bound = None;
    assert!(
        store
            .with_v2_text_current(&owner, |a| {
                let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                guard.admit_text_create_predecessor_into(
                    &lane,
                    prepared.ready_clear()?,
                    &query,
                    &producer.observe_original(),
                    &mut bound,
                )
            })
            .is_err(),
        "a historical Clear result cannot skip fresh complete absences"
    );
    assert!(bound.is_none());
    assert!(!lane.workspace_active_for_test());
    // No first-request binding was installed on that failed proof. A genuinely separate
    // producer can still inspect through the SAME lane; its readonly run must preserve
    // and refuse this WAL, rather than converting arbitrary bytes into an empty workspace.
    let (_other_producer, mut other) = workspace(&original, &selected, &query, "other request");
    let admitted = read_workspace(&original, &lane, &mut other, &query);
    assert!(lane.workspace_active_for_test());
    assert!(other.run_read(admitted).is_err());
    assert!(!other.has_ready_data());
    assert!(other.ready_clear().is_err());
    assert!(!lane.workspace_active_for_test());
    assert_eq!(std::fs::read(wal).unwrap(), bytes);
    // Lookup and workspace classification intentionally remain different proof domains.
    // The valid readonly empty token result did not turn this dirty workspace into Clear.
    validate_query(&original, &query);
    assert!(query.token_value_data().unwrap().is_none());
}

mod custody;
mod preservation;

// Actual A stage/commit ownership and query-IO/Scope interleavings.
mod retain_admission;
