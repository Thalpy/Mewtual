use super::*;
use automerge::{transaction::Transactable, ReadDoc, ROOT};
use catcoms_rt::ManualClock;
use voice_v2::directory::{MAX_DIRECTORY_KEYS, MAX_DIRECTORY_ROOMS};
const INDEX: u128 = 991;
include!("transport_tests.rs");
#[path = "ready_tests.rs"]
mod ready_tests;
fn reader(doc: Option<&AutoCommit>) -> Result<BTreeSet<u128>, VoiceError> {
    let mut rooms = BTreeSet::from([1]);
    if let Some(doc) = doc {
        for (index, key) in doc.keys(ROOT).enumerate() {
            if index >= MAX_DIRECTORY_KEYS {
                return Err(VoiceError::Capacity);
            }
            if let Ok(id) = key.parse::<u128>() {
                rooms.insert(id);
            }
            if rooms.len() > MAX_DIRECTORY_ROOMS {
                return Err(VoiceError::Capacity);
            }
        }
    }
    Ok(rooms)
}
fn deletions(change: &automerge::Change) -> Result<BTreeSet<u128>, VoiceError> {
    use automerge::legacy::{Key, ObjectId, OpType};
    let expanded =
        catcoms_replication::decode_change_projection(change).map_err(|_| VoiceError::Bounds)?;
    let mut removed = BTreeSet::new();
    for op in expanded.operations {
        if op.obj == ObjectId::Root && matches!(op.action, OpType::Delete) {
            if let Key::Map(key) = op.key {
                if let Ok(id) = key.parse::<u128>() {
                    if id != 1 {
                        removed.insert(id);
                    }
                }
            }
        }
    }
    Ok(removed)
}
fn attach<T: MeshTransport, R: CryptoRngCore>(
    node: &mut ChannelSync<T, R>,
    runtime: &Arc<Runtime>,
) -> ActorRegistration {
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let installation = node
        .prepare_voice_actor(runtime, &ticket, INDEX, reader, deletions)
        .unwrap();
    installation.register().unwrap();
    installation
}

#[tokio::test]
async fn retained_root_actor_lock_unlock_uses_fresh_owner_and_same_process_group_protection() {
    let (_hub, mut nodes, _) = crate::tests::build_members(1).await;
    let node = &mut nodes[0];
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(node, &runtime);
    let old = node.voice_call_lease(1).unwrap();
    let helper = node
        .voice_runtime
        .as_ref()
        .unwrap()
        .current
        .as_ref()
        .unwrap()
        .helper()
        .unwrap();
    runtime.revoke_session();
    assert!(!old.is_live() && node.voice_call_lease(1).is_err());
    let ticket = runtime.session_ticket();
    let installation = node
        .prepare_voice_actor(&runtime, &ticket, INDEX, reader, deletions)
        .unwrap();
    installation.register().unwrap();
    assert!(node.voice_call_lease(1).is_err());
    runtime.activate_session(&ticket).unwrap();
    let new = node.voice_call_lease(1).unwrap();
    assert!(new.is_live() && !old.is_live());
    assert!(helper.is_same(
        &node
            .voice_runtime
            .as_ref()
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .helper()
            .unwrap()
    ));
}

#[tokio::test]
async fn asynchronous_child_grant_custody_revokes_when_reply_or_last_native_owner_is_dropped() {
    let (_hub, mut nodes, _) = crate::tests::build_members(1).await;
    let node = &mut nodes[0];
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(node, &runtime);
    let grant = node.voice_call_grant(1).unwrap();
    let lease = grant.lease();
    assert_eq!(grant.scope().room, 1);
    let (reply, rx) = tokio::sync::oneshot::channel();
    drop(rx);
    drop(reply.send(grant));
    assert!(
        !lease.is_live(),
        "failed reply delivery retires the exact child"
    );
    let native = Arc::new(node.voice_call_grant(1).unwrap());
    let lease = native.lease();
    let borrowed = native.clone();
    drop(native);
    assert!(lease.is_live(), "actual native custody remains");
    drop(borrowed);
    assert!(
        !lease.is_live(),
        "a worker's bare lease clone grants no custody"
    );
    let current = node.voice_call_grant(1).unwrap();
    runtime.revoke_session();
    assert!(
        !current.lease().is_live(),
        "custody cannot defeat actor lock revocation"
    );
}

#[tokio::test]
async fn lost_prepare_reply_and_lock_during_reattachment_cannot_activate_old_root_candidates() {
    let (_hub, mut nodes, _) = crate::tests::build_members(1).await;
    let node = &mut nodes[0];
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let lost = node
        .prepare_voice_actor(&runtime, &ticket, INDEX, reader, deletions)
        .unwrap();
    drop(lost);
    assert!(node.voice_call_lease(1).is_err());
    let late = node
        .prepare_voice_actor(&runtime, &ticket, INDEX, reader, deletions)
        .unwrap();
    runtime.revoke_session();
    assert_eq!(late.register(), Err(VoiceError::Unauthorized));
    attach(node, &runtime);
    let new = node.voice_call_lease(1).unwrap();
    assert!(new.is_live());
    assert_eq!(late.register(), Err(VoiceError::Unauthorized));
    assert!(new.is_live());
}

#[tokio::test]
async fn local_directory_edit_that_mutates_then_errors_revokes_before_returning_failure() {
    let (_hub, mut nodes, _) = crate::tests::build_members(1).await;
    let node = &mut nodes[0];
    node.open_channel(DocType::ChannelIndex, INDEX)
        .await
        .unwrap();
    node.post(DocType::ChannelIndex, INDEX, |doc| doc.put(ROOT, "2", true))
        .await
        .unwrap();
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(node, &runtime);
    let old = node.voice_call_lease(2).unwrap();
    let room = node.voice_room_authority(2).unwrap();
    let result = node
        .post(DocType::ChannelIndex, INDEX, |doc| {
            doc.delete(ROOT, "2")?;
            doc.insert(ROOT, 0, "root is a map, so this fails")
        })
        .await;
    assert!(result.is_err());
    assert!(!old.is_live() && !room.lease.is_live());
    assert!(node.voice_call_lease(2).is_err());
}

#[tokio::test]
async fn actual_signed_catchup_delete_recreate_and_bad_tail_revoke_captured_lower_flight() {
    let (_hub, mut nodes, ids) = crate::tests::build_members(3).await;
    crate::tests::converge_and_publish_test_routes(&mut nodes);
    nodes[0]
        .initialize_group_policy(GroupMode::PeerToPeer)
        .unwrap();
    nodes[0].publish_group_policy().unwrap();
    let policy = nodes[0].group_policy.as_ref().unwrap().encode();
    nodes[1].on_group_policy(&policy);
    let mut nodes = nodes.into_iter();
    let mut source = nodes.next().unwrap();
    let mut helper_node = nodes.next().unwrap();
    let recipient = nodes.next().unwrap();
    source
        .open_channel(DocType::ChannelIndex, INDEX)
        .await
        .unwrap();
    helper_node
        .open_channel(DocType::ChannelIndex, INDEX)
        .await
        .unwrap();
    let create = source
        .docs
        .get_mut(&(DocType::ChannelIndex, INDEX))
        .unwrap()
        .edit_tracked(&source.device, &source.group, &mut source.rng, |doc| {
            doc.put(ROOT, "2", true)
        })
        .unwrap()
        .0;
    helper_node
        .apply_catchup_response(DocType::ChannelIndex, INDEX, &encode_bundle(&[create]))
        .unwrap();
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(&mut helper_node, &runtime);
    let old_child = helper_node.voice_call_lease(2).unwrap();
    let room = helper_node.voice_room_authority(2).unwrap();
    let context = helper_node.voice_v2_context(2).unwrap();
    let object = voice_v2::SignedObject::issue(
        context.scope,
        &source.device,
        [51; 32],
        1000,
        16_000,
        voice_v2::Body::WatchChallenge {
            watcher: [2; 32],
            target: ids[2],
            join: [3; 32],
            key_ref: [4; 32],
            challenge: [5; 32],
        },
    )
    .unwrap();
    let permit =
        voice_v2::RoutePermit::issue(&source.device, &object, ids[1], 1000, 16_000).unwrap();
    let owner = helper_node
        .voice_runtime
        .as_ref()
        .unwrap()
        .current
        .as_ref()
        .unwrap();
    let ledger = owner.helper().unwrap();
    let routes = [voice_v2::helper::BoundRoute {
        device: ids[2],
        peer: recipient.local_peer(),
    }];
    ledger
        .admit(
            owner,
            ids[0],
            voice_v2::Envelope::ToHelper { object, permit },
            &context,
            &room,
            &routes,
        )
        .unwrap();
    let job = ledger.take_ready(owner, &routes).unwrap().pop().unwrap();
    let lower = job.keepalive();
    let deleted = source
        .docs
        .get_mut(&(DocType::ChannelIndex, INDEX))
        .unwrap()
        .edit_tracked(&source.device, &source.group, &mut source.rng, |doc| {
            doc.delete(ROOT, "2")
        })
        .unwrap()
        .0;
    let recreated = source
        .docs
        .get_mut(&(DocType::ChannelIndex, INDEX))
        .unwrap()
        .edit_tracked(&source.device, &source.group, &mut source.rng, |doc| {
            doc.put(ROOT, "2", true)
        })
        .unwrap()
        .0;
    let mut bad = recreated.clone();
    bad.epoch += 1;
    assert!(helper_node
        .apply_catchup_response(
            DocType::ChannelIndex,
            INDEX,
            &encode_bundle(&[deleted, recreated, bad])
        )
        .is_err());
    assert!(helper_node
        .doc(DocType::ChannelIndex, INDEX)
        .unwrap()
        .doc()
        .get(ROOT, "2")
        .unwrap()
        .is_some());
    assert!(!old_child.is_live() && !room.lease.is_live() && !job.authority().authorized());
    let new = helper_node.voice_call_lease(2).unwrap();
    assert!(new.is_live());
    ledger.complete(job.finish(true)).unwrap();
    assert_eq!(ledger.counts(), (1, 1, 1));
    drop(lower);
    assert_eq!(ledger.counts(), (1, 1, 0));
}

#[tokio::test]
async fn policy_pin_initialization_retires_legacy_direct_join_before_publication() {
    let (_hub, mut nodes, _) = crate::tests::build_members(1).await;
    let node = &mut nodes[0];
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(node, &runtime);
    let direct = node.voice_call_lease(1).unwrap();
    assert!(node.voice_v2_context(1).unwrap().scope.policy.is_none());
    node.initialize_group_policy(GroupMode::PeerToPeer).unwrap();
    assert!(!direct.is_live() && node.voice_call_lease(1).is_err());
    node.publish_group_policy().unwrap();
    let upgraded = node.voice_call_lease(1).unwrap();
    assert!(upgraded.is_live());
    assert!(node.voice_v2_context(1).unwrap().scope.policy.is_some());
}

#[tokio::test]
async fn compound_local_directory_edit_retires_exact_room_even_when_final_projection_is_equal() {
    let (_hub, mut nodes, _) = crate::tests::build_members(1).await;
    let node = &mut nodes[0];
    node.open_channel(DocType::ChannelIndex, INDEX)
        .await
        .unwrap();
    node.post(DocType::ChannelIndex, INDEX, |doc| doc.put(ROOT, "2", true))
        .await
        .unwrap();
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(node, &runtime);
    let old = node.voice_call_lease(2).unwrap();
    node.post(DocType::ChannelIndex, INDEX, |doc| {
        doc.delete(ROOT, "2")?;
        doc.put(ROOT, "2", true)
    })
    .await
    .unwrap();
    assert!(!old.is_live());
    let replacement = node.voice_call_lease(2).unwrap();
    assert!(replacement.is_live());
    node.post(DocType::ChannelIndex, INDEX, |_| Ok(()))
        .await
        .unwrap();
    assert!(
        replacement.is_live(),
        "a no-op publication must not replay the previous compound deletion"
    );
    // Permanent compatibility room remains available and keeps its own unchanged incarnation.
    let general = node.voice_call_lease(1).unwrap();
    node.post(DocType::ChannelIndex, INDEX, |doc| {
        doc.put(ROOT, "1", true)?;
        doc.delete(ROOT, "1")
    })
    .await
    .unwrap();
    assert!(general.is_live());
}

#[tokio::test]
async fn buffered_directory_deletion_retires_once_but_duplicate_and_forged_deletions_do_not() {
    let (_hub, mut nodes, _) = crate::tests::build_members(2).await;
    let mut replica = nodes.pop().unwrap();
    let mut source = nodes.pop().unwrap();
    source
        .open_channel(DocType::ChannelIndex, INDEX)
        .await
        .unwrap();
    replica
        .open_channel(DocType::ChannelIndex, INDEX)
        .await
        .unwrap();
    let create = source
        .docs
        .get_mut(&(DocType::ChannelIndex, INDEX))
        .unwrap()
        .edit_tracked(&source.device, &source.group, &mut source.rng, |doc| {
            doc.put(ROOT, "2", true)
        })
        .unwrap()
        .0;
    replica
        .apply_catchup_response(DocType::ChannelIndex, INDEX, &encode_bundle(&[create]))
        .unwrap();
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(&mut replica, &runtime);
    let old = replica.voice_call_lease(2).unwrap();
    let dependency = source
        .docs
        .get_mut(&(DocType::ChannelIndex, INDEX))
        .unwrap()
        .edit_tracked(&source.device, &source.group, &mut source.rng, |doc| {
            doc.put(ROOT, "other", true)
        })
        .unwrap()
        .0;
    let deleted = source
        .docs
        .get_mut(&(DocType::ChannelIndex, INDEX))
        .unwrap()
        .edit_tracked(&source.device, &source.group, &mut source.rng, |doc| {
            doc.delete(ROOT, "2")
        })
        .unwrap()
        .0;
    let recreated = source
        .docs
        .get_mut(&(DocType::ChannelIndex, INDEX))
        .unwrap()
        .edit_tracked(&source.device, &source.group, &mut source.rng, |doc| {
            doc.put(ROOT, "2", true)
        })
        .unwrap()
        .0;
    replica
        .apply_catchup_response(
            DocType::ChannelIndex,
            INDEX,
            &encode_bundle(&[recreated, dependency]),
        )
        .unwrap();
    assert!(old.is_live(), "no deletion was admitted yet");
    replica
        .apply_catchup_response(
            DocType::ChannelIndex,
            INDEX,
            &encode_bundle(std::slice::from_ref(&deleted)),
        )
        .unwrap();
    assert!(replica
        .doc(DocType::ChannelIndex, INDEX)
        .unwrap()
        .doc()
        .get(ROOT, "2")
        .unwrap()
        .is_some());
    assert!(!old.is_live(),"buffered deletion/recreation can materialize together, but its exact accepted deletion still retires");
    let new = replica.voice_call_lease(2).unwrap();
    assert_eq!(
        replica
            .apply_catchup_response(
                DocType::ChannelIndex,
                INDEX,
                &encode_bundle(std::slice::from_ref(&deleted))
            )
            .unwrap(),
        0
    );
    assert!(
        new.is_live(),
        "duplicate historical deletion grants no new revocation"
    );
    let key = source
        .group
        .channel_secret(&source.device, DocType::ChannelIndex, INDEX)
        .unwrap();
    let mut forged = deleted.open(&key).unwrap();
    forged.signature[0] ^= 1;
    let forged = SealedOp::seal(&forged, &source.group, &source.device, &mut source.rng).unwrap();
    assert!(replica
        .apply_catchup_response(DocType::ChannelIndex, INDEX, &encode_bundle(&[forged]))
        .is_err());
    assert!(
        new.is_live(),
        "an unaccepted forged deletion grants no revocation authority"
    );
    // The same authenticated-change observation applies to current and retained-key ingress.
    for retained in [false, true] {
        let delta = source
            .docs
            .get_mut(&(DocType::ChannelIndex, INDEX))
            .unwrap()
            .edit_tracked(&source.device, &source.group, &mut source.rng, |doc| {
                doc.delete(ROOT, "2")?;
                doc.put(ROOT, "2", true)
            })
            .unwrap()
            .0;
        let prior = replica.voice_call_lease(2).unwrap();
        if retained {
            replica.past_keys.insert(
                (DocType::ChannelIndex, INDEX, delta.epoch),
                Zeroizing::new(key),
            );
            replica.ingest_past(&delta);
        } else {
            replica.ingest_current(&delta);
        }
        assert!(!prior.is_live());
        let current = replica.voice_call_lease(2).unwrap();
        if retained {
            replica.ingest_past(&delta);
        } else {
            replica.ingest_current(&delta);
        }
        assert!(current.is_live());
    }
}
