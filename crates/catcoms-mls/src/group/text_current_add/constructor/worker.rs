//! Genuine SDK/provider execution through the opaque Cell, not synthetic output
//! adopters or a Source/publication fixture. Every original remains mutation-closed.
use super::*;
use openmls_memory_storage::owner_restore::{TextAddTestEvent as Event, TextAddTestObservation};

fn prepared(
    d: &MlsDevice,
    g: &ServerGroup,
) -> (
    MlsDevice,
    PreparedTextCurrentAdd,
    Arc<AtomicUsize>,
    Vec<u8>,
    [u8; 32],
    [u8; 32],
) {
    let (peer, frame, subject, reference) = request(g.group_id_ref());
    let (paid, used, _) = admission(limits());
    let mut paid = Some(paid);
    let mut cell = None;
    PreparedTextCurrentAdd::prepare_into(
        d,
        g,
        input(&frame, subject, reference),
        &mut paid,
        &mut cell,
    )
    .unwrap();
    assert!(paid.is_none());
    (peer, cell.unwrap(), used, frame, subject, reference)
}
fn unchanged(d: &MlsDevice, g: &ServerGroup, before: &[u8], epoch: u64, auth: [u8; 32]) {
    assert_eq!(g.epoch(), epoch);
    assert!(g.epoch_authenticator_id() == auth);
    assert!(crate::snapshot_server(d, g).unwrap().as_slice() == before);
}
fn completed(cell: &PreparedTextCurrentAdd, peer: &MlsDevice) {
    assert!(cell.is_complete_data() && cell.provider_closed);
    assert_eq!(cell.faults.sdk_dispatches, 1);
    assert_eq!(cell.faults.merge_dispatches, 1);
    let joined = ServerGroup::join(peer, cell.welcome.as_deref().unwrap()).unwrap();
    assert!(joined.epoch_authenticator_id() == cell.graph.group.epoch_authenticator_id());
    assert_eq!(joined.epoch(), cell.original.epoch + 1);
    assert!(
        joined
            .channel_secret(peer, catcoms_wire::DocType::Channel, 37)
            .unwrap()
            == cell
                .graph
                .group
                .channel_secret(&cell.graph.device, catcoms_wire::DocType::Channel, 37)
                .unwrap()
    );
    let observation = cell.observation.as_ref().unwrap();
    assert!(observation.matches_current(&cell.graph.group, &cell.graph.device));
    assert_eq!(observation.kind(), LocalMembershipChangeKind::Add);
    assert_eq!(observation.subject(), &cell.subject);
    assert_eq!(observation.key_package_reference(), Some(cell.reference));
    assert_eq!(
        observation.credential_digest(),
        Some(cell.credential_digest)
    );
    assert!(observation.commit() == cell.commit.as_deref().unwrap());
    let (cold_device, cold_group, _fee) = parent(cell.snapshot.as_deref().unwrap());
    assert!(cold_group.epoch_authenticator_id() == cell.graph.group.epoch_authenticator_id());
    assert_eq!(cold_group.epoch(), joined.epoch());
    assert!(
        cold_group
            .channel_secret(&cold_device, catcoms_wire::DocType::Channel, 37)
            .unwrap()
            == joined
                .channel_secret(peer, catcoms_wire::DocType::Channel, 37)
                .unwrap()
    );
    // The cold decoder grants no follow-up mutation. Output graph ownership
    // remains inside Cell until a separate saved-governance adapter exists.
    assert!(matches!(
        cold_device.key_package(),
        Err(MlsError::Internal(_))
    ));
}
fn digest_outputs(cell: &PreparedTextCurrentAdd) -> [[u8; 32]; 4] {
    [
        cell.commit.as_deref().unwrap(),
        cell.welcome.as_deref().unwrap(),
        cell.group_info.as_deref().unwrap(),
        cell.snapshot.as_deref().unwrap(),
    ]
    .map(|bytes| *blake3::hash(bytes).as_bytes())
}

#[test]
fn actual20_and50_member_adds_cache_real_welcome_observation_and_cold_candidate_without_mutating_parent(
) {
    for members in [20, 50] {
        let bytes = source(members, false);
        let (d, g, old_used) = parent(&bytes);
        let before = crate::snapshot_server(&d, &g).unwrap();
        let epoch = g.epoch();
        let auth = g.epoch_authenticator_id();
        let (peer, mut cell, used, _, _, _) = prepared(&d, &g);
        cell.execute_private().unwrap();
        completed(&cell, &peer);
        unchanged(&d, &g, &before, epoch, auth);
        assert!(matches!(
            cell.graph
                .group
                .create_application_message(&cell.graph.device, b"closed"),
            Err(MlsError::Internal(_))
        ));
        assert!(matches!(
            cell.graph
                .group
                .remove_member(&cell.graph.device, &peer.device_id()),
            Err(MlsError::Internal(_))
        ));
        let exact = digest_outputs(&cell);
        cell.execute_private().unwrap();
        assert!(digest_outputs(&cell) == exact);
        assert_eq!(cell.faults.sdk_dispatches, 1);
        drop(g);
        drop(d);
        assert_eq!(old_used.load(Ordering::SeqCst), TYPED);
        assert!(used.load(Ordering::SeqCst) > 0);
        drop(cell);
        assert_eq!(used.load(Ordering::SeqCst), 0);
        assert_eq!(old_used.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn every_real_known_output_cache_lost_return_resumes_without_second_add_or_merge_dispatch() {
    let bytes = source(5, false);
    let (d, g, _) = parent(&bytes);
    let before = crate::snapshot_server(&d, &g).unwrap();
    let epoch = g.epoch();
    let auth = g.epoch_authenticator_id();
    for cut in [
        TestCut::OutputsCached,
        TestCut::CommitSerialized,
        TestCut::WelcomeSerialized,
        TestCut::GroupInfoSerialized,
        TestCut::Serialized,
        TestCut::Merged,
        TestCut::ProviderClosed,
        TestCut::ObservationCached,
        TestCut::SnapshotCached,
        TestCut::Complete,
    ] {
        let (peer, mut cell, used, _, _, _) = prepared(&d, &g);
        cell.faults.fail_at = Some(cut);
        assert!(matches!(
            cell.execute_private(),
            Err(MlsError::Internal("injected private Add cache return loss"))
        ));
        assert!(cell.outputs.is_some());
        assert_eq!(cell.faults.sdk_dispatches, 1);
        let commit_object_size = cell.outputs.as_ref().unwrap().commit.tls_serialized_len();
        let already_commit = cell
            .commit
            .as_ref()
            .map(|bytes| *blake3::hash(bytes).as_bytes());
        cell.execute_private().unwrap();
        completed(&cell, &peer);
        assert_eq!(cell.commit.as_ref().unwrap().len(), commit_object_size);
        if let Some(digest) = already_commit {
            assert!(*blake3::hash(cell.commit.as_ref().unwrap()).as_bytes() == digest);
        }
        let exact = digest_outputs(&cell);
        cell.execute_private().unwrap();
        assert!(digest_outputs(&cell) == exact);
        unchanged(&d, &g, &before, epoch, auth);
        drop(cell);
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn entered_but_unknown_work_and_real_provider_phase_errors_stick_and_never_claim_noop_success() {
    let bytes = source(4, false);
    let (d, g, _) = parent(&bytes);
    let before = crate::snapshot_server(&d, &g).unwrap();
    let epoch = g.epoch();
    let auth = g.epoch_authenticator_id();
    for event in [
        Event::PendingWritten,
        Event::OldRatchetsWritten,
        Event::MergeStateWritten,
        Event::MergeTreeWritten,
        Event::MergeEpochRead,
        Event::MergeEpochWritten,
        Event::MergeEpochDeleted,
    ] {
        let (_peer, mut cell, used, _, _, _) = prepared(&d, &g);
        let fault = TextAddTestObservation::install(Some((event, 1)), None);
        assert!(cell.execute_private().is_err());
        assert_eq!(fault.count(event), 1);
        assert!(cell.phase == Phase::Failed && !cell.is_complete_data());
        let add = cell.faults.sdk_dispatches;
        let merge = cell.faults.merge_dispatches;
        let candidate = crate::snapshot_server(&cell.graph.device, &cell.graph.group).unwrap();
        drop(fault);
        assert!(cell.execute_private().is_err());
        assert_eq!(cell.faults.sdk_dispatches, add);
        assert_eq!(cell.faults.merge_dispatches, merge);
        assert!(
            crate::snapshot_server(&cell.graph.device, &cell.graph.group)
                .unwrap()
                .as_slice()
                == candidate.as_slice()
        );
        // A first merge write fault has already consumed Pending in the SDK.
        // Cached Commit/Welcome/GI survive, but Operational+noPending is NOT
        // enough to relabel this incomplete operation Complete.
        if event == Event::MergeStateWritten {
            assert!(cell.graph.group.group.pending_commit().is_none());
            assert!(cell.outputs.is_some() && cell.commit.is_some() && cell.welcome.is_some());
            assert_eq!(cell.graph.group.epoch(), epoch);
        }
        unchanged(&d, &g, &before, epoch, auth);
        drop(cell);
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
    for cut in [TestCut::AddEntered, TestCut::MergeEntered] {
        let (_peer, mut cell, _, _, _, _) = prepared(&d, &g);
        cell.faults.fail_at = Some(cut);
        assert!(cell.execute_private().is_err());
        let counts = (cell.faults.sdk_dispatches, cell.faults.merge_dispatches);
        assert!(cell.execute_private().is_err());
        assert_eq!(
            counts,
            (cell.faults.sdk_dispatches, cell.faults.merge_dispatches)
        );
        assert!(!cell.is_complete_data());
        unchanged(&d, &g, &before, epoch, auth);
    }
}

#[test]
fn genuine_pending_queue_serializer_prepays_temporaries_and_old_epoch_initializes_both_own_ratchets(
) {
    let bytes = source(4, false);
    let (d, g, _) = parent(&bytes);
    let fee = TextAddTestObservation::serializer_temporary_bytes();
    for credit in [fee - 1, fee] {
        let (_peer, mut cell, _, _, _, _) = prepared(&d, &g);
        let observation = TextAddTestObservation::install(None, Some(credit));
        assert!(cell.execute_private().is_err());
        assert_eq!(observation.count(Event::PendingSerialize), 1);
        // Actual SDK PendingCommit owns the genuine one-Add ProposalQueue.
        // Short typed credit fails before either real serde/vector_converter
        // entry; exact credit permits both passes before the next typed debit.
        assert_eq!(
            observation.count(Event::SerializeEntered),
            if credit < fee { 0 } else { 2 }
        );
        assert!(cell.phase == Phase::Failed);
        assert_eq!(cell.faults.sdk_dispatches, 1);
    }
    let (_peer, mut cell, _, _, _, _) = prepared(&d, &g);
    let observation = TextAddTestObservation::install(Some((Event::OldRatchetsWritten, 1)), None);
    assert!(cell.execute_private().is_err());
    assert_eq!(observation.count(Event::OldRatchetsWritten), 1);
    let map = cell
        .graph
        .device
        .provider()
        .storage()
        .values
        .read()
        .unwrap();
    let row = map
        .iter()
        .find(|(key, _)| key.starts_with(b"MessageSecrets"))
        .unwrap()
        .1;
    let value: serde_json::Value = serde_json::from_slice(row).unwrap();
    let handshake = find(&value, "handshake_sender_ratchets")
        .unwrap()
        .as_array()
        .unwrap();
    let application = find(&value, "application_sender_ratchets")
        .unwrap()
        .as_array()
        .unwrap();
    let own = cell.original.own as usize;
    assert_eq!(
        handshake[own]["EncryptionRatchet"]["generation"].as_u64(),
        Some(1)
    );
    assert_eq!(
        application[own]["EncryptionRatchet"]["generation"].as_u64(),
        Some(0)
    );
    for (i, ratchet) in handshake.iter().enumerate() {
        if i != own {
            assert!(ratchet.is_null());
        }
    }
    for (i, ratchet) in application.iter().enumerate() {
        if i != own {
            assert!(ratchet.is_null());
        }
    }
}

fn find<'a>(value: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map
            .get(name)
            .or_else(|| map.values().find_map(|v| find(v, name))),
        serde_json::Value::Array(values) => values.iter().find_map(|v| find(v, name)),
        _ => None,
    }
}

#[test]
fn invoice_pressure_and_future_table_headroom_refuse_before_sdk_then_genuine_blank_reuse_completes()
{
    let bytes = source(4, false);
    let (d, g, _) = parent(&bytes);
    let (_peer, frame, subject, reference) = request(g.group_id_ref());
    let before = crate::snapshot_server(&d, &g).unwrap();
    let needed = invoice::required().unwrap();
    for mode in 0..3 {
        let mut l = limits();
        match mode {
            0 => l.sdk_typed_bytes = needed.typed - 1,
            1 => l.sdk_work = needed.work - 1,
            _ => {
                l.max_provider_backing_bytes = d
                    .provider()
                    .storage()
                    .values
                    .read()
                    .unwrap()
                    .allocation_size()
            }
        }
        let (paid, used, _) = admission(l);
        let mut paid = Some(paid);
        let mut cell = None;
        assert!(PreparedTextCurrentAdd::prepare_into(
            &d,
            &g,
            input(&frame, subject, reference),
            &mut paid,
            &mut cell
        )
        .is_err());
        assert!(paid.is_some() && cell.is_none());
        assert!(crate::snapshot_server(&d, &g).unwrap().as_slice() == before.as_slice());
        drop(paid);
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
    let bytes = source(64, true);
    let (d, g, _) = parent(&bytes);
    let (peer, mut cell, used, _, _, _) = prepared(&d, &g);
    assert_eq!(g.member_count(), 63);
    cell.execute_private().unwrap();
    completed(&cell, &peer);
    assert_eq!(cell.graph.group.member_count(), 64);
    drop(cell);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}
