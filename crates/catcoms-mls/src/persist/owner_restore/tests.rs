//! Real SDK producer/closed-loader regressions. Fee fixtures are bounded resource accounts,
//! not saved flags, fresh proofs or source authority; actual App issuance is tested separately.
use super::*;
use std::cell::Cell;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

const GRAPH: usize = 128 * 1024;
const WORK: usize = 512 * 1024 * 1024;
#[derive(Default)]
struct Pool {
    used: usize,
    cap: usize,
}
struct Fee {
    pool: Arc<Mutex<Pool>>,
    bytes: usize,
    work: usize,
    calls: Arc<AtomicUsize>,
}
impl OwnerSdkGraphReservation for Fee {
    fn limits(&self) -> OwnerSdkRestoreLimits {
        self.calls.fetch_add(1, Ordering::SeqCst);
        OwnerSdkRestoreLimits {
            max_typed_bytes: self.bytes,
            max_work: self.work,
        }
    }
}
impl Drop for Fee {
    fn drop(&mut self) {
        let mut pool = self.pool.lock().unwrap();
        pool.used = pool.used.checked_sub(self.bytes).unwrap();
    }
}
fn pool() -> Arc<Mutex<Pool>> {
    Arc::new(Mutex::new(Pool {
        used: 0,
        cap: 2 * GRAPH,
    }))
}
fn admission(
    pool: &Arc<Mutex<Pool>>,
    bytes: usize,
    work: usize,
    calls: Arc<AtomicUsize>,
) -> Result<OwnerSdkRestoreAdmission, MlsError> {
    {
        let mut p = pool.lock().unwrap();
        p.used = p
            .used
            .checked_add(bytes)
            .filter(|n| *n <= p.cap)
            .ok_or(MlsError::Internal("fixture resource capacity"))?;
    }
    OwnerSdkRestoreAdmission::new(Fee {
        pool: pool.clone(),
        bytes,
        work,
        calls,
    })
}
fn load(
    snapshot: &[u8],
    pool: &Arc<Mutex<Pool>>,
    bytes: usize,
) -> Result<(MlsDevice, ServerGroup), MlsError> {
    restore_server_owner_checkpoint_bounded(
        snapshot,
        1024 * 1024,
        1024 * 1024,
        admission(pool, bytes, WORK, Arc::new(AtomicUsize::new(0)))?,
    )
}
fn snapshot() -> Zeroizing<Vec<u8>> {
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    snapshot_server(&device, &group).unwrap()
}
fn changed(source: &[u8], label: &[u8], value: &[u8]) -> Vec<u8> {
    let mut d = Decoder::new(source);
    let mut map = deserialize_map(d.get_bytes().unwrap()).unwrap();
    let signer = d.get_bytes().unwrap();
    let group = d.get_bytes().unwrap();
    d.finish().unwrap();
    let key = map
        .keys()
        .find(|key| key.starts_with(label))
        .cloned()
        .unwrap_or_else(|| {
            assert!(matches!(label, b"OwnLeafNodes" | b"ProposalQueueRefs"));
            let tree = map.keys().find(|key| key.starts_with(b"Tree")).unwrap();
            let mut key = label.to_vec();
            key.extend_from_slice(&tree[4..]);
            key
        });
    map.insert(key, value.to_vec());
    let mut e = Encoder::new();
    e.put_bytes(&serialize_map(&map).unwrap()).unwrap();
    e.put_bytes(signer).unwrap();
    e.put_bytes(group).unwrap();
    e.finish()
}
#[test]
fn actual_founder_policy_precedes_signer_and_counts_every_real_generic_direct_list_getter() {
    let source = snapshot();
    let resources = pool();
    let calls = Arc::new(AtomicUsize::new(0));
    SDK_LOAD_ENTRIES.with(|n| n.set(0));
    let (device, group) = restore_server_owner_checkpoint_bounded(
        &source,
        1024 * 1024,
        1024 * 1024,
        admission(&resources, GRAPH, WORK, calls.clone()).unwrap(),
    )
    .unwrap();
    assert!(group.is_active());
    assert!(group.has_local_signer(device.public_key_ref()));
    assert_eq!(SDK_LOAD_ENTRIES.with(Cell::get), 1);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "immutable caps queried exactly once before SDK locks"
    );
    // complete() requires all13 observations. A missed direct/list path cannot report success.
    assert!(openmls::prelude::MlsGroup::load(
        device.provider().storage(),
        &openmls::prelude::GroupId::from_slice(group.group_id_ref())
    )
    .is_err());
    assert_eq!(resources.lock().unwrap().used, GRAPH);
    drop(device);
    assert_eq!(
        resources.lock().unwrap().used,
        GRAPH,
        "group still retains SAME fee"
    );
    drop(group);
    assert_eq!(resources.lock().unwrap().used, 0);
}
#[test]
fn actual_first_add_and_selected_package_member_roundtrip_under_independent_graph_fees() {
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    let kp = member
        .key_package_for_invite(group.group_id_ref(), [7; 16])
        .unwrap();
    let selected = member.key_package_reference(&kp).unwrap();
    let welcome = group.add_member(&owner, kp).unwrap().welcome;
    let joined = ServerGroup::join_for_key_package(&member, &welcome, &selected).unwrap();
    let source = snapshot_server(&owner, &group).unwrap();
    let member_source = snapshot_server(&member, &joined).unwrap();
    let resources = pool();
    let (owner, group) = load(&source, &resources, GRAPH).unwrap();
    let (member, joined) = load(&member_source, &resources, GRAPH).unwrap();
    assert_eq!(group.member_count(), 2);
    assert_eq!(joined.epoch(), 1);
    assert_eq!(resources.lock().unwrap().used, 2 * GRAPH);
    drop(group);
    assert_eq!(
        resources.lock().unwrap().used,
        2 * GRAPH,
        "owner still holds first graph"
    );
    drop(owner);
    assert_eq!(resources.lock().unwrap().used, GRAPH);
    drop(member);
    assert_eq!(
        resources.lock().unwrap().used,
        GRAPH,
        "member group outlives device"
    );
    drop(joined);
    assert_eq!(resources.lock().unwrap().used, 0);
}
#[test]
fn every_late_malformed_row_refuses_before_any_actual_signer_sdk_entry_and_preserves_input() {
    let source = snapshot();
    let resources = pool();
    for label in [
        b"SignatureKeyPair".as_slice(),
        b"Tree",
        b"ProposalQueueRefs",
        b"GroupContext",
        b"InterimTranscriptHash",
        b"ConfirmationTag",
        b"EpochSecrets",
        b"OwnLeafNodeIndex",
        b"MessageSecrets",
        b"ResumptionPsk",
        b"MlsGroupJoinConfig",
        b"OwnLeafNodes",
        b"GroupState",
    ] {
        let damaged = changed(&source, label, b"{");
        let retained = damaged.clone();
        SDK_LOAD_ENTRIES.with(|n| n.set(0));
        assert!(load(&damaged, &resources, GRAPH).is_err());
        assert_eq!(
            SDK_LOAD_ENTRIES.with(Cell::get),
            0,
            "complete row preflight must precede SIGNER"
        );
        assert_eq!(damaged, retained);
        assert_eq!(resources.lock().unwrap().used, 0);
    }
}
#[test]
fn completed_graph_refuses_mutation_duplicate_kp_and_storage_before_any_state_change() {
    let source = snapshot();
    let resources = pool();
    let (device, mut group) = load(&source, &resources, GRAPH).unwrap();
    let before = snapshot_server(&device, &group).unwrap();
    assert!(device.duplicate().is_err());
    assert!(device.key_package().is_err());
    assert!(device
        .key_package_for_invite(group.group_id_ref(), [9; 16])
        .is_err());
    assert!(ServerGroup::create(&device).is_err());
    let outsider = MlsDevice::generate().unwrap();
    let kp = outsider.key_package().unwrap();
    assert!(group.add_member_observed(&device, kp).is_err());
    assert!(group
        .remove_member_observed(&device, &outsider.device_id())
        .is_err());
    assert!(group
        .create_application_message(&device, b"no unadmitted ratchet mutation")
        .is_err());
    assert!(group
        .process_incoming(&device, b"no parse or SDK work")
        .is_err());
    assert!(group.stage_remove(&device, &outsider.device_id()).is_err());
    assert!(group.merge_staged_self(&device).is_err());
    assert!(group.abort_staged(&device).is_err());
    assert_eq!(
        snapshot_server(&device, &group).unwrap().as_slice(),
        before.as_slice()
    );
    // Wrong unclosed device must not bypass the GROUP's independent early guard.
    assert!(group
        .create_application_message(&outsider, b"wrong device")
        .is_err());
    assert_eq!(
        snapshot_server(&device, &group).unwrap().as_slice(),
        before.as_slice()
    );
}
#[test]
fn exact_typed_cap_one_byte_short_and_work_exhaustion_do_not_refill_or_mutate_source() {
    let source = snapshot();
    let retained = source.to_vec();
    let resources = pool();
    let mut low = 1usize;
    let mut high = GRAPH;
    assert!(load(&source, &resources, high).is_ok());
    assert_eq!(resources.lock().unwrap().used, 0);
    while low < high {
        let mid = low + (high - low) / 2;
        let result = load(&source, &resources, mid);
        let success = result.is_ok();
        drop(result);
        assert_eq!(resources.lock().unwrap().used, 0);
        if success {
            high = mid
        } else {
            low = mid + 1
        }
    }
    let exact = load(&source, &resources, low).unwrap();
    drop(exact);
    SDK_LOAD_ENTRIES.with(|n| n.set(0));
    assert!(load(&source, &resources, low - 1).is_err());
    assert_eq!(
        SDK_LOAD_ENTRIES.with(Cell::get),
        1,
        "one-byte-short actual graph fails DURING typed load, after the fixed invoice/preflight"
    );
    assert_eq!(
        resources.lock().unwrap().used,
        0,
        "partial graph/getter cleanup precedes final fee refund"
    );
    SDK_LOAD_ENTRIES.with(|n| n.set(0));
    let fee = admission(&resources, GRAPH, 1, Arc::new(AtomicUsize::new(0))).unwrap();
    assert!(
        restore_server_owner_checkpoint_bounded(&source, 1024 * 1024, 1024 * 1024, fee).is_err()
    );
    assert_eq!(SDK_LOAD_ENTRIES.with(Cell::get), 0);
    assert_eq!(resources.lock().unwrap().used, 0);
    assert_eq!(source.as_slice(), retained);
}
#[test]
fn unsupported_history_and_frozen_legacy_profile_are_preserved_without_legacy_fallback() {
    let legacy = include_bytes!("../fixtures/openmls-0.8.1/founder.snapshot");
    assert!(
        restore_server(legacy).is_ok(),
        "ordinary legacy compatibility is retained"
    );
    let resources = pool();
    assert!(load(legacy, &resources, GRAPH).is_err());
    assert_eq!(resources.lock().unwrap().used, 0);
    let source = snapshot();
    let mut d = Decoder::new(&source);
    let map = deserialize_map(d.get_bytes().unwrap()).unwrap();
    let value = map
        .iter()
        .find(|(key, _)| key.starts_with(b"MessageSecrets"))
        .unwrap()
        .1;
    let mut row: serde_json::Value = serde_json::from_slice(value).unwrap();
    row["message_secrets"]["secret_tree"]["parent_nodes"] = serde_json::json!([]);
    let damaged = changed(
        &source,
        b"MessageSecrets",
        &serde_json::to_vec(&row).unwrap(),
    );
    SDK_LOAD_ENTRIES.with(|n| n.set(0));
    assert!(load(&damaged, &resources, GRAPH).is_err());
    assert_eq!(SDK_LOAD_ENTRIES.with(Cell::get), 0);
}

#[test]
fn native_parent_indices_are_checked_before_signer_even_when_tree_fields_arrive_late() {
    let device = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&device).unwrap();
    group
        .add_member(&device, member.key_package().unwrap())
        .unwrap();
    let source = snapshot_server(&device, &group).unwrap();
    let mut d = Decoder::new(&source);
    let map = deserialize_map(d.get_bytes().unwrap()).unwrap();
    let bytes = map
        .iter()
        .find(|(key, _)| key.starts_with(b"Tree"))
        .unwrap()
        .1;
    let original: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let resources = pool();
    for indices in [
        serde_json::json!([u32::MAX]),
        serde_json::json!([u64::MAX]),
        serde_json::json!([2]),
        serde_json::json!([1, 0]),
        serde_json::json!([0, 0]),
    ] {
        let mut row = original.clone();
        // A real bounded key shape from the actual producer; only native index DATA is altered.
        let key = row["tree"]["leaf_nodes"][0]["node"]["payload"]["encryption_key"].clone();
        row["tree"]["parent_nodes"][0] = serde_json::json!({"node":{"encryption_key":key,"parent_hash":{"vec":[]},"unmerged_leaves":{"list":indices}}});
        let damaged = changed(&source, b"Tree", &serde_json::to_vec(&row).unwrap());
        SDK_LOAD_ENTRIES.with(|n| n.set(0));
        assert!(load(&damaged, &resources, GRAPH).is_err());
        assert_eq!(SDK_LOAD_ENTRIES.with(Cell::get), 0);
        assert_eq!(resources.lock().unwrap().used, 0);
    }
    assert!(
        load(&source, &resources, GRAPH).is_ok(),
        "genuine first-Add still passes"
    );
}
