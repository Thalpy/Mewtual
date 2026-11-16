//! Genuine SDK producers -> the DISTINCT closed consumer. These are resource-only
//! reservations, never Current/Admission/source witnesses. Source creation uses the
//! unchanged legacy SDK operations; the returned paid graphs remain mutation-closed.
use super::*;
use crate::{Incoming, MlsDevice, ServerGroup};
use catcoms_wire::{DocType, Encoder};
use std::sync::atomic::{AtomicUsize, Ordering};
const GRAPH: usize = 2 * 1024 * 1024;
const RAW: usize = 8 * 1024 * 1024;
const WORK: usize = 1024 * 1024 * 1024;
struct Fee {
    used: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
    bytes: usize,
    work: usize,
}
impl TextCurrentSdkGraphReservation for Fee {
    fn limits(&self) -> TextCurrentSdkRestoreLimits {
        self.calls.fetch_add(1, Ordering::SeqCst);
        TextCurrentSdkRestoreLimits {
            max_typed_bytes: self.bytes,
            max_work: self.work,
        }
    }
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
        assert_eq!(
            self.used.fetch_sub(self.bytes, Ordering::SeqCst),
            self.bytes
        );
    }
}
fn fee(bytes: usize, work: usize) -> (Fee, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let used = Arc::new(AtomicUsize::new(bytes));
    let calls = Arc::new(AtomicUsize::new(0));
    (
        Fee {
            used: used.clone(),
            calls: calls.clone(),
            bytes,
            work,
        },
        used,
        calls,
    )
}
fn load(source: &[u8]) -> (MlsDevice, ServerGroup, Arc<AtomicUsize>) {
    let (f, used, calls) = fee(GRAPH, WORK);
    let (d, g) = restore_server_text_current_bounded(
        source,
        RAW,
        RAW,
        TextCurrentSdkRestoreAdmission::new(f).unwrap(),
    )
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    (d, g, used)
}
fn exporter(d: &MlsDevice, g: &ServerGroup) -> [u8; 32] {
    g.channel_secret(d, DocType::Channel, 19).unwrap()
}
fn roster(g: &ServerGroup) -> Vec<(u32, Vec<u8>)> {
    let mut result = g
        .member_device_ids()
        .into_iter()
        .map(|id| {
            (
                g.member_leaf_index(&id).unwrap(),
                g.member_signature_key(&id).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    result.sort_unstable();
    result
}
fn row(source: &[u8], label: &[u8]) -> Vec<u8> {
    let mut d = Decoder::new(source);
    let map = deserialize_map(d.get_bytes().unwrap()).unwrap();
    let mut rows = map.iter().filter(|(k, _)| k.starts_with(label));
    let value = rows.next().unwrap().1.clone();
    assert!(rows.next().is_none());
    value
}
fn null_current(source: &[u8]) {
    let bytes = row(source, b"MessageSecrets");
    for variant in [
        b"EncryptionRatchet".as_slice(),
        b"DecryptionRatchet".as_slice(),
        b"DualUse".as_slice(),
    ] {
        assert!(!bytes.windows(variant.len()).any(|w| w == variant));
    }
}
fn roundtrip(d: &MlsDevice, g: &ServerGroup, drop_device_first: bool) {
    let expected = exporter(d, g);
    let original = snapshot_server(d, g).unwrap();
    null_current(&original);
    let expected_roster = roster(g);
    let expected_epoch = g.epoch();
    let (device, group, used) = load(&original);
    assert_eq!(group.epoch(), expected_epoch);
    assert!(roster(&group) == expected_roster);
    assert!(exporter(&device, &group) == expected);
    let restored = snapshot_server(&device, &group).unwrap();
    for label in [
        b"ResumptionPsk".as_slice(),
        b"MessageSecrets".as_slice(),
        b"Tree".as_slice(),
    ] {
        assert!(row(&restored, label) == row(&original, label));
    }
    assert_eq!(used.load(Ordering::SeqCst), GRAPH);
    if drop_device_first {
        drop(device);
        assert_eq!(used.load(Ordering::SeqCst), GRAPH);
        drop(group);
    } else {
        drop(group);
        assert_eq!(used.load(Ordering::SeqCst), GRAPH);
        drop(device);
    }
    assert_eq!(used.load(Ordering::SeqCst), 0);
}
struct Topology {
    group: ServerGroup,
    device: MlsDevice,
    witness_group: ServerGroup,
    witness: MlsDevice,
    removable: catcoms_crypto::DeviceId,
}
fn topology(count: usize) -> Topology {
    let device = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&device).unwrap();
    let mut witness = None;
    let mut removable = None;
    for index in 1..count {
        let member = MlsDevice::generate().unwrap();
        let package = member
            .key_package_for_invite(group.group_id_ref(), (index as u128).to_be_bytes())
            .unwrap();
        let reference = member.key_package_reference(&package).unwrap();
        let add = group.add_member(&device, package).unwrap();
        if index == 1 {
            removable = Some(member.device_id());
        }
        if index + 1 == count {
            witness = Some((
                ServerGroup::join_for_key_package(&member, &add.welcome, &reference).unwrap(),
                member,
            ));
        }
    }
    let (witness_group, witness) = witness.unwrap();
    Topology {
        group,
        device,
        witness_group,
        witness,
        removable: removable.unwrap(),
    }
}
#[test]
fn text_current_twenty_and_fifty_real_members_exporters_and_both_fee_drop_orders() {
    for count in [20, 50] {
        let t = topology(count);
        assert_eq!(t.group.member_count(), count);
        assert_eq!(t.group.epoch(), count as u64 - 1);
        assert!(exporter(&t.device, &t.group) == exporter(&t.witness, &t.witness_group));
        roundtrip(&t.device, &t.group, false);
        roundtrip(&t.witness, &t.witness_group, true);
        let source = snapshot_server(&t.device, &t.group).unwrap();
        let before = source.to_vec();
        let (f, used, _) = fee(GRAPH, WORK);
        assert!(
            super::super::owner_restore::restore_server_owner_checkpoint_bounded(
                &source,
                RAW,
                RAW,
                super::super::owner_restore::OwnerSdkRestoreAdmission::new(f).unwrap()
            )
            .is_err()
        );
        assert!(source.as_slice() == before);
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn text_current_actual_blank_reuse_and_exact_resumption_first_wrap_boundaries() {
    let mut t = topology(20);
    while t.group.epoch() < 64 {
        let remove = t.group.stage_remove(&t.device, &t.removable).unwrap();
        assert!(matches!(
            t.witness_group
                .process_incoming(&t.witness, &remove.commit)
                .unwrap(),
            Incoming::CommitApplied { removed: true }
        ));
        t.group.merge_staged_self(&t.device).unwrap();
        assert!(t.group.member_leaf_index(&t.removable).is_none());
        if [31, 32, 62, 63, 64].contains(&t.group.epoch()) {
            roundtrip(&t.device, &t.group, false);
            roundtrip(&t.witness, &t.witness_group, true);
        }
        if t.group.epoch() == 20 {
            roundtrip(&t.device, &t.group, true);
        } // actual blank interior slot
        if t.group.epoch() >= 64 {
            break;
        }
        let newcomer = MlsDevice::generate().unwrap();
        let package = newcomer
            .key_package_for_invite(
                t.group.group_id_ref(),
                (1000 + t.group.epoch() as u128).to_be_bytes(),
            )
            .unwrap();
        let add = t.group.add_member(&t.device, package).unwrap();
        assert!(matches!(
            t.witness_group
                .process_incoming(&t.witness, &add.commit)
                .unwrap(),
            Incoming::CommitApplied { removed: false }
        ));
        t.removable = newcomer.device_id();
        if [31, 32, 62, 63, 64].contains(&t.group.epoch()) {
            roundtrip(&t.device, &t.group, false);
            roundtrip(&t.witness, &t.witness_group, true);
        }
    }
    assert_eq!(t.group.epoch(), 64);
    assert!(roster(&t.group) == roster(&t.witness_group));
    assert!(exporter(&t.device, &t.group) == exporter(&t.witness, &t.witness_group));
}
fn changed(source: &[u8], label: &[u8], change: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut d = Decoder::new(source);
    let mut map = deserialize_map(d.get_bytes().unwrap()).unwrap();
    let signer = d.get_bytes().unwrap();
    let group = d.get_bytes().unwrap();
    d.finish().unwrap();
    let key = map
        .keys()
        .find(|k| k.starts_with(label))
        .cloned()
        .unwrap_or_else(|| {
            assert!(matches!(label, b"OwnLeafNodes" | b"ProposalQueueRefs"));
            let tree = map.keys().find(|k| k.starts_with(b"Tree")).unwrap();
            let mut key = label.to_vec();
            key.extend_from_slice(&tree[4..]);
            key
        });
    let mut value =
        serde_json::from_slice(map.get(&key).map_or(b"[]".as_slice(), |v| v.as_slice())).unwrap();
    change(&mut value);
    map.insert(key, serde_json::to_vec(&value).unwrap());
    let mut e = Encoder::new();
    e.put_bytes(&serialize_map(&map).unwrap()).unwrap();
    e.put_bytes(signer).unwrap();
    e.put_bytes(group).unwrap();
    e.finish()
}
fn refuses_before_sdk(source: &[u8]) {
    let before = source.to_vec();
    let entries = TEXT_LOAD_ENTRIES.with(|n| n.get());
    let (f, used, calls) = fee(GRAPH, WORK);
    assert!(restore_server_text_current_bounded(
        source,
        RAW,
        RAW,
        TextCurrentSdkRestoreAdmission::new(f).unwrap()
    )
    .is_err());
    assert_eq!(TEXT_LOAD_ENTRIES.with(|n| n.get()), entries);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(used.load(Ordering::SeqCst), 0);
    assert!(source == before);
}
#[test]
fn text_current_unknown_schema_shape_caps_and_work_refuse_without_source_change() {
    let t = topology(3);
    let source = snapshot_server(&t.device, &t.group).unwrap();
    for changed in [
        changed(&source, b"MessageSecrets", |v| {
            v["message_secrets"]["secret_tree"]["handshake_sender_ratchets"][0] =
                serde_json::json!({"DualUse":null});
        }),
        changed(&source, b"MessageSecrets", |v| {
            v["message_secrets"]["secret_tree"]["application_sender_ratchets"]
                .as_array_mut()
                .unwrap()
                .pop();
        }),
        changed(&source, b"Tree", |v| {
            let leaves = v["tree"]["leaf_nodes"].as_array_mut().unwrap();
            let exemplar = leaves[0].clone();
            while leaves.len() <= 64 {
                leaves.push(exemplar.clone());
            }
        }),
        changed(&source, b"ResumptionPsk", |v| {
            v["cursor"] = serde_json::json!(999);
        }),
        changed(&source, b"OwnLeafNodes", |v| {
            *v = serde_json::json!([null]);
        }),
        changed(&source, b"GroupState", |v| {
            *v = serde_json::json!("PendingCommit");
        }),
    ] {
        refuses_before_sdk(&changed);
    }
    let mut trailing = source.to_vec();
    trailing.push(0);
    refuses_before_sdk(&trailing);
    for wrong_signer in [true, false] {
        let mut d = Decoder::new(&source);
        let storage = d.get_bytes().unwrap();
        let signer = d.get_bytes().unwrap();
        let group = d.get_bytes().unwrap();
        let mut e = Encoder::new();
        e.put_bytes(storage).unwrap();
        e.put_bytes(if wrong_signer { &[0x91; 32] } else { signer })
            .unwrap();
        e.put_bytes(if wrong_signer { group } else { &[0x92; 16] })
            .unwrap();
        refuses_before_sdk(&e.finish());
    }
    let before = source.to_vec();
    for (bytes, work) in [(1, WORK), (GRAPH, 1)] {
        let (f, used, _) = fee(bytes, work);
        let outcome = TextCurrentSdkRestoreAdmission::new(f)
            .and_then(|a| restore_server_text_current_bounded(&source, RAW, RAW, a));
        assert!(outcome.is_err());
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
    let (f, used, _) = fee(GRAPH, WORK);
    assert!(restore_server_text_current_bounded(
        &source,
        source.len() - 1,
        RAW,
        TextCurrentSdkRestoreAdmission::new(f).unwrap()
    )
    .is_err());
    assert_eq!(used.load(Ordering::SeqCst), 0);
    let (f, used, _) = fee(GRAPH, WORK);
    assert!(restore_server_text_current_bounded(
        &source,
        RAW,
        1,
        TextCurrentSdkRestoreAdmission::new(f).unwrap()
    )
    .is_err());
    assert_eq!(used.load(Ordering::SeqCst), 0);
    assert!(source.as_slice() == before);
}
#[test]
fn text_current_real_initialized_ratchets_refuse_and_restored_graph_mutations_stay_closed() {
    let mut t = topology(2);
    let message = t
        .group
        .create_application_message(&t.device, b"only legacy test input")
        .unwrap();
    assert!(matches!(
        t.witness_group
            .process_incoming(&t.witness, &message)
            .unwrap(),
        Incoming::Application(_)
    ));
    let source = snapshot_server(&t.device, &t.group).unwrap();
    refuses_before_sdk(&source);
    let remote = snapshot_server(&t.witness, &t.witness_group).unwrap();
    refuses_before_sdk(&remote);
    let clean = topology(2);
    let source = snapshot_server(&clean.device, &clean.group).unwrap();
    let (device, mut group, used) = load(&source);
    let before = snapshot_server(&device, &group).unwrap();
    // This exact Internal variant is produced by the wrapper's FIRST guard; an
    // SDK ratchet advance followed by blocked provider write returns Protocol.
    assert!(matches!(
        group.create_application_message(&device, b"must refuse before SDK"),
        Err(MlsError::Internal(
            "closed SDK graph needs explicit operation admission"
        ))
    ));
    assert!(matches!(
        group.stage_remove(&device, &clean.removable),
        Err(MlsError::Internal(
            "closed SDK graph needs explicit operation admission"
        ))
    ));
    let after = snapshot_server(&device, &group).unwrap();
    assert!(before.as_slice() == after.as_slice());
    drop(group);
    assert_eq!(used.load(Ordering::SeqCst), GRAPH);
    drop(device);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}
