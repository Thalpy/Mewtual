//! Root registration identity and lifecycle, using the surrounding signed-discovery fixture.
//! No IPC/browser execution or physical media acceptance is implied by an empty Derived marker.
use super::*;
use voice_v2::crypto_actions::CryptoOutcome;

#[tokio::test]
async fn root_handoff_expiry_refuses_new_capacity_but_preserves_the_actual_old_loan() {
    for admitted in [false, true] {
        let (mut root, _remote, runtime, clock, grant, handshake) = low_fixture().await;
        let action = root.prepare_voice_derivation(&grant, handshake).unwrap();
        let bound = action.protected_binding().unwrap();
        let loan = admitted.then(|| action.borrow_handoff(16 * 1024).unwrap());
        clock.advance_ms(bound.deadline);
        let before = runtime.resources().counts();
        assert_eq!(
            action.borrow_handoff(16 * 1024).err(),
            Some(VoiceError::Unauthorized)
        );
        assert_eq!(
            runtime.resources().counts(),
            before,
            "expiry refusal spends no capacity"
        );
        runtime.collect_expired();
        let collected = runtime.resources().counts();
        drop(loan);
        assert_eq!(
            runtime.resources().counts().0,
            collected.0 - if admitted { 16 * 1024 } else { 0 },
            "actual old copy ownership, not expiry, returns its handoff loan"
        );
        assert_eq!(action.protected_binding().unwrap(), bound);
    }
}

#[tokio::test]
async fn root_handoff_binding_and_lane_are_original_and_shared_between_distinct_rows() {
    let (mut root, remote, runtime, _clock, grant, handshake) = low_fixture().await;
    let action = root.prepare_voice_derivation(&grant, handshake).unwrap();
    let bound = action.protected_binding().unwrap();
    let context = root.voice_v2_context(1).unwrap();
    assert_eq!(bound.scope, grant.scope());
    assert_eq!(bound.epoch, context.epoch);
    assert_eq!(bound.local, root.device_id());
    assert_eq!(bound.peer, remote.device_id());
    assert_eq!(bound.local_join, key(81).join);
    assert_eq!(bound.local_key, key(81).key_ref);
    assert_eq!(bound.peer_join, key(82).join);
    assert_eq!(bound.peer_key, key(82).key_ref);
    assert_eq!(bound.handshake, handshake);
    assert!(bound.local_is_low);
    let before = runtime.resources().counts();
    let loan = action.borrow_handoff(16 * 1024).unwrap();
    assert_eq!(runtime.resources().counts(), before);
    assert!(Arc::ptr_eq(
        &action,
        &root.prepare_voice_derivation(&grant, handshake).unwrap(),
    ));
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = root
        .commit_voice_derivation_result(&grant, &action, action.output_tag(&[]).unwrap())
        .unwrap();
    action.settle_engine();
    action.acknowledge_result(&receipt).unwrap();
    assert!(action.input().is_err());
    let next = root
        .start_voice_confirmation(&grant, handshake, None)
        .unwrap();
    assert_eq!(next.protected_binding().unwrap(), bound);
    assert_eq!(
        next.borrow_handoff(16 * 1024).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(
        action.borrow_handoff(16 * 1024).err(),
        Some(VoiceError::Unauthorized)
    );
    drop(loan);
    let next_loan = next.borrow_handoff(16 * 1024).unwrap();
    grant.lease().revoke();
    root.reconcile_voice_actor();
    assert_eq!(next.borrow_handoff(1).err(), Some(VoiceError::Unauthorized));
    assert_eq!(action.protected_binding().unwrap(), bound);
    drop(next_loan);
}

pub(super) fn read_rooms(
    doc: Option<&automerge::AutoCommit>,
) -> Result<BTreeSet<u128>, VoiceError> {
    use automerge::{ReadDoc, ROOT};
    // Closed fixture schema: the permanent room and one explicit test room.
    let mut rooms = BTreeSet::from([1]);
    if let Some(doc) = doc {
        if doc
            .get(ROOT, "2")
            .map_err(|_| VoiceError::Bounds)?
            .is_some()
        {
            rooms.insert(2);
        }
    }
    Ok(rooms)
}
pub(super) fn deleted_rooms(change: &automerge::Change) -> Result<BTreeSet<u128>, VoiceError> {
    use automerge::legacy::{Key, ObjectId, OpType};
    let expanded =
        catcoms_replication::decode_change_projection(change).map_err(|_| VoiceError::Bounds)?;
    Ok(expanded
        .operations
        .into_iter()
        .filter_map(|op| {
            (op.obj == ObjectId::Root
                && matches!(op.action, OpType::Delete)
                && matches!(op.key, Key::Map(ref key) if key == "2"))
            .then_some(2)
        })
        .collect())
}

#[tokio::test]
async fn root_derivation_recovers_original_action_and_result_without_recreating_key_input() {
    let (mut root, _remote, _runtime, _clock, grant, handshake) = low_fixture().await;
    let action = root.prepare_voice_derivation(&grant, handshake).unwrap();
    let tag = action.output_tag(&[]).unwrap();
    assert!(Arc::ptr_eq(
        &action,
        &root.prepare_voice_derivation(&grant, handshake).unwrap()
    ));
    assert_eq!(
        root.commit_voice_derivation_result(&grant, &action, tag)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = root
        .commit_voice_derivation_result(&grant, &action, tag)
        .unwrap();
    assert_eq!(receipt.outcome, CryptoOutcome::Derived);
    action.settle_engine();
    action.acknowledge_result(&receipt).unwrap();
    assert!(action.input().is_err());
    assert!(Arc::ptr_eq(
        &action,
        &root.prepare_voice_derivation(&grant, handshake).unwrap()
    ));
    assert!(receipt.same_transition(
        &root
            .commit_voice_derivation_result(&grant, &action, tag)
            .unwrap()
    ));
    assert_eq!(
        root.commit_voice_derivation_result(&grant, &action, [3; 32])
            .err(),
        Some(VoiceError::RetryMismatch)
    );
    assert!(action.admit_engine(action.engine_bytes()).is_err());
}

#[tokio::test]
async fn root_derivation_rejects_foreign_call_and_actual_registered_wrong_operation() {
    let (mut root, _remote, _runtime, _clock, grant, handshake) = low_fixture().await;
    let action = root.prepare_voice_derivation(&grant, handshake).unwrap();
    let foreign = Arc::new(CallGrant::from_lease(CallLease::default(), grant.scope()));
    assert_eq!(
        root.prepare_voice_derivation(&foreign, handshake).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        root.commit_voice_derivation_result(&foreign, &action, action.output_tag(&[]).unwrap())
            .err(),
        Some(VoiceError::Unauthorized)
    );
    let seal = root
        .start_voice_confirmation(&grant, handshake, None)
        .unwrap();
    assert_eq!(
        root.commit_voice_derivation_result(&grant, &seal, [0; 32])
            .err(),
        Some(VoiceError::Unauthorized),
        "registered action from another operation is not row zero"
    );
    assert!(action.terminal_status().receipt.is_none());
}

#[tokio::test]
async fn root_derivation_equal_join_replacement_never_adopts_original_action() {
    let (mut root, _remote, _runtime, _clock, grant, handshake) = low_fixture().await;
    let action = root.prepare_voice_derivation(&grant, handshake).unwrap();
    let tag = action.output_tag(&[]).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    grant.lease().revoke();
    root.reconcile_voice_actor();
    let replacement = Arc::new(root.voice_call_grant(1).unwrap());
    let same_key = key(81);
    root.start_voice_endpoint(replacement.clone(), same_key.join, same_key.public_key)
        .unwrap();
    for owner in [&grant, &replacement] {
        assert_eq!(
            root.prepare_voice_derivation(owner, handshake).err(),
            Some(VoiceError::Unauthorized)
        );
        assert_eq!(
            root.commit_voice_derivation_result(owner, &action, tag)
                .err(),
            Some(VoiceError::Unauthorized)
        );
    }
    assert!(action.terminal_status().receipt.is_none());
    action.settle_engine();
}

#[tokio::test]
async fn root_derivation_mls_epoch_change_rejects_first_result_with_original_peer_still_present() {
    let (mut root, remote, _runtime, _clock, grant, handshake) = low_fixture().await;
    let action = root.prepare_voice_derivation(&grant, handshake).unwrap();
    let tag = action.output_tag(&[]).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let before = root.voice_v2_context(1).unwrap().epoch;
    let third = catcoms_mls::MlsDevice::generate().unwrap();
    root.with_observed_mls_transition(|node| {
        node.group
            .add_member(&node.device, third.key_package().unwrap())
    })
    .unwrap();
    assert!(root.group.contains_device(&remote.device_id()));
    assert!(root.voice_v2_context(1).unwrap().epoch > before);
    assert!(grant.lease().is_live());
    assert_eq!(
        root.prepare_voice_derivation(&grant, handshake).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        root.commit_voice_derivation_result(&grant, &action, tag)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(action.terminal_status().receipt.is_none());
    action.settle_engine();
}

#[tokio::test]
async fn root_derivation_native_room_and_session_revocation_refuse_first_result() {
    for session in [false, true] {
        let (mut root, _remote, runtime, _clock, grant, handshake) = low_fixture().await;
        let action = root.prepare_voice_derivation(&grant, handshake).unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        if session {
            runtime.revoke_session();
        } else {
            // Revoke the actual Directory incarnation, not an equal-ID RoomAuthority fixture.
            // Signed deletion/recreation itself is covered by voice_runtime/directory tests.
            root.voice_room_authority(1).unwrap().lease.revoke();
        }
        assert!(root.prepare_voice_derivation(&grant, handshake).is_err());
        assert!(root
            .commit_voice_derivation_result(&grant, &action, action.output_tag(&[]).unwrap())
            .is_err());
        assert!(action.terminal_status().receipt.is_none());
        action.settle_engine();
    }
}

#[tokio::test]
async fn root_derivation_signed_compound_room_delete_recreate_never_adopts_old_owner() {
    use automerge::{transaction::Transactable, ROOT};
    let (mut root, _remote, _runtime, _clock, grant, handshake) = low_fixture_room(2).await;
    let action = root.prepare_voice_derivation(&grant, handshake).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    // The actual signed local change deletes then recreates the same key, so the projected
    // room list alone is unchanged. Change observation must still retire its old incarnation.
    root.post(DocType::ChannelIndex, 991, |doc| {
        doc.delete(ROOT, "2")?;
        doc.put(ROOT, "2", true)
    })
    .await
    .unwrap();
    assert!(!grant.lease().is_live());
    let replacement = Arc::new(root.voice_call_grant(2).unwrap());
    let same_key = key(81);
    root.start_voice_endpoint(replacement.clone(), same_key.join, same_key.public_key)
        .unwrap();
    for owner in [&grant, &replacement] {
        assert!(root.prepare_voice_derivation(owner, handshake).is_err());
        assert!(root
            .commit_voice_derivation_result(owner, &action, action.output_tag(&[]).unwrap())
            .is_err());
    }
    assert!(action.terminal_status().receipt.is_none());
    action.settle_engine();
}

#[tokio::test]
async fn terminal_root_voice_retirement_fences_actual_action_without_refunding_running_custody() {
    let (mut root, _remote, runtime, _clock, grant, handshake) = low_fixture().await;
    let action = root.prepare_voice_derivation(&grant, handshake).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let input = action.input().unwrap();
    let changes = root.voice_endpoint_changes().unwrap();
    assert_eq!(runtime.resources().counts().1, 1);
    root.retire_voice_actor();
    assert!(!grant.lease().is_live() && !action.is_pending());
    assert!(changes.has_changed().is_err());
    assert_eq!(
        runtime.resources().counts().1,
        1,
        "actual engine still owns its original slot"
    );
    assert!(matches!(input.as_input(), CryptoInput::Derive(_)));
    assert!(root
        .commit_voice_derivation_result(&grant, &action, action.output_tag(&[]).unwrap())
        .is_err());
    action.settle_engine();
    assert_eq!(runtime.resources().counts().1, 0);
    let held = runtime.resources().counts().0;
    drop(input);
    assert!(
        runtime.resources().counts().0 < held,
        "last native input owner releases only its own bytes"
    );
    assert!(action.terminal_status().receipt.is_none());
}
