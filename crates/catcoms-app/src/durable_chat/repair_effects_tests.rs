//! Real Store faults and actual Actor/peer paths pin post-save bookkeeping and UI effects.
use super::*;
use crate::{channel_id, spawn, AppEvent, ServerActor, TracedEvent};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{timeout, Duration};

type Node = Server<catcoms_rt::MemNetwork, ChaCha20Rng>;
const A: u128 = 35;
const B: u128 = 36;
async fn node() -> Node {
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(901)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(901),
        Box::new(ManualClock::new(1_000)),
        "alice",
    )
    .unwrap();
    server.open_channel(A).await.unwrap();
    server.open_channel(B).await.unwrap();
    server
}
fn send(server: &Node, token: u8, channel: u128, text: &str) -> DurableChatRequest {
    DurableChatRequest::Send(DurableSendRequest {
        token: [token; 16],
        expected_context: server.sync.durable_send_context().unwrap(),
        channel,
        text: text.into(),
        reply_to: String::new(),
    })
}
fn original_from_store(store: &ServerStore, server_id: u64) -> Node {
    Server::restore(
        &store.load_server(server_id).unwrap(),
        Hub::new().join(PeerId::from_u64(902)),
        ChaCha20Rng::seed_from_u64(902),
        Box::new(ManualClock::new(2_000)),
        "alice",
    )
    .unwrap()
}

#[tokio::test]
async fn implicit_stop_repair_reports_exact_send_and_tracks_before_any_retry() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        ServerStore::open(dir.path(), b"repair", &mut ChaCha20Rng::seed_from_u64(903)).unwrap();
    let mut server = node().await;
    let original = send(&server, 1, A, "uncertain A");
    store.fail_next_chat_parent_sync();
    let (pending, effects) = server
        .commit_chat_request_with_effects(&original, &store, 9)
        .unwrap();
    assert!(!pending.durable);
    assert!(effects.newly_committed_sends.is_empty() && effects.affected_channels.is_empty());
    assert!(server.delivery_snapshot(A).states.is_empty());
    assert!(server.messages(A).is_empty());
    assert_eq!(
        original_from_store(&store, 9).messages(A)[0].text,
        "uncertain A"
    );

    let stop = DurableStopRequest {
        original: send(&server, 2, B, "never authored B"),
    };
    let (stopped, effects) = server
        .stop_chat_request_with_effects(&stop, &store, 9)
        .unwrap();
    assert!(stopped.durable);
    assert_eq!(effects.affected_channels, vec![A]);
    assert_eq!(effects.newly_committed_sends.len(), 1);
    let committed = &effects.newly_committed_sends[0];
    let (token, binding, context, channel, message_id, _, _) = original.identity();
    assert_eq!(
        (
            committed.token,
            committed.binding,
            committed.context,
            committed.channel
        ),
        (token, binding, context, channel)
    );
    assert_eq!(committed.message_id, message_id);
    assert_eq!(committed.message_id, pending.message_id);
    assert_eq!(
        server
            .sync
            .durable_chat_lookup(token, binding, context, A)
            .unwrap()
            .unwrap()
            .change,
        Some(committed.change)
    );
    assert_eq!(
        server.own_message_changes[&A].front(),
        Some(&(message_id.clone(), committed.change))
    );
    assert_eq!(server.delivery_snapshot(A).states[0].id, message_id);
    assert!(server.messages(B).is_empty());
    let (replay, effects) = server
        .commit_chat_request_with_effects(&original, &store, 9)
        .unwrap();
    assert!(replay.durable && replay.replayed);
    assert!(effects.newly_committed_sends.is_empty() && effects.affected_channels.is_empty());
    assert_eq!(server.own_message_changes[&A].len(), 1);
    assert_eq!(
        original_from_store(&store, 9).messages(A),
        server.messages(A)
    );
}

async fn commit_actor(
    actor: &ServerActor,
    store: &Arc<Mutex<Option<ServerStore>>>,
    request: DurableChatRequest,
    stop: bool,
) -> DurableSendOutcome {
    let ready = if stop {
        actor
            .prepare_durable_stop(DurableStopRequest { original: request })
            .await
            .unwrap()
    } else {
        match request {
            DurableChatRequest::Send(request) => actor.prepare_durable_send(request).await.unwrap(),
            DurableChatRequest::Mutation(request) => {
                actor.prepare_durable_mutation(request).await.unwrap()
            }
        }
    };
    ready
        .commit(DurableSendLease::new(
            store.clone().lock_owned().await,
            9,
            (),
        ))
        .await
        .unwrap()
}
fn channel_updates(events: &mut mpsc::Receiver<TracedEvent>) -> Vec<u128> {
    let mut channels = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let AppEvent::ChannelUpdated { channel, .. } = event.event {
            channels.push(channel);
        }
    }
    channels
}

/// There is no connected peer, incoming traffic, frontend refresh or automatic host worker in
/// this fixture. The ONLY save which can repair A is the selected Stop/Send in B.
async fn implicit_actor_repair(delete: bool, stop_b: bool) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Some(
        ServerStore::open(dir.path(), b"repair", &mut ChaCha20Rng::seed_from_u64(904)).unwrap(),
    )));
    let (actor, mut events, task) = spawn(node().await);
    actor.open_channel(A).await;
    actor.open_channel(B).await;
    let context = actor.durable_send_context().await.unwrap();
    let posted = commit_actor(
        &actor,
        &store,
        DurableChatRequest::Send(DurableSendRequest {
            token: [3; 16],
            expected_context: context,
            channel: A,
            text: "original A".into(),
            reply_to: String::new(),
        }),
        false,
    )
    .await;
    actor.messages(A).await; // command barrier: initial notification has finished.
    channel_updates(&mut events);
    let target = actor
        .durable_mutation_context(A, posted.message_id.clone())
        .await
        .unwrap();
    let mutation = DurableChatRequest::Mutation(DurableMutationRequest {
        token: [4; 16],
        expected_context: target.context,
        channel: A,
        message_id: posted.message_id.clone(),
        target: target.target,
        action: if delete {
            DurableMutationAction::Delete
        } else {
            DurableMutationAction::Edit {
                text: "repaired A".into(),
            }
        },
    });
    store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .fail_next_chat_parent_sync();
    let pending = commit_actor(&actor, &store, mutation, false).await;
    assert!(!pending.durable);
    assert_eq!(actor.messages(A).await[0].text, "original A");
    assert!(channel_updates(&mut events).is_empty());

    let request_b = DurableChatRequest::Send(DurableSendRequest {
        token: [5; 16],
        expected_context: context,
        channel: B,
        text: "B commits only when selected".into(),
        reply_to: String::new(),
    });
    // A second actual parent-sync failure must not emit A's withheld effect, even though its
    // bytes reached rename again. Failed-save SnapshotNeeded scheduling remains present.
    store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .fail_next_chat_parent_sync();
    assert!(
        !commit_actor(&actor, &store, request_b.clone(), stop_b)
            .await
            .durable
    );
    assert_eq!(actor.messages(A).await[0].text, "original A");
    assert!(channel_updates(&mut events).is_empty());

    assert!(
        commit_actor(&actor, &store, request_b, stop_b)
            .await
            .durable
    );
    let current = actor.messages(A).await; // barrier after all handler notifications.
    let updates = channel_updates(&mut events);
    assert_eq!(
        updates.iter().filter(|c| **c == A).count(),
        1,
        "A must get its own exact event"
    );
    assert_eq!(
        updates.iter().filter(|c| **c == B).count(),
        usize::from(!stop_b)
    );
    if delete {
        assert!(current.is_empty());
    } else {
        assert_eq!(current[0].text, "repaired A");
    }
    let reopened = original_from_store(store.lock().await.as_ref().unwrap(), 9);
    assert_eq!(reopened.messages(A), current);
    assert_eq!(reopened.messages(B).len(), usize::from(!stop_b));
    actor.shutdown().await;
    task.await.unwrap();
}

#[tokio::test]
async fn implicit_stop_after_uncertain_edit_notifies_other_channel_without_network() {
    implicit_actor_repair(false, true).await;
}
#[tokio::test]
async fn implicit_commit_after_uncertain_delete_notifies_both_channels_without_network() {
    implicit_actor_repair(true, false).await;
}

#[tokio::test]
async fn stopping_uncertain_signed_send_keeps_exact_history_and_tracking_but_never_authors_stop() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        ServerStore::open(dir.path(), b"repair", &mut ChaCha20Rng::seed_from_u64(905)).unwrap();
    let mut server = node().await;
    let request = send(&server, 6, A, "signed history retained by Stop");
    store.fail_next_chat_parent_sync();
    assert!(
        !server
            .commit_chat_request_with_effects(&request, &store, 9)
            .unwrap()
            .0
            .durable
    );
    let (stopped, effects) = server
        .stop_chat_request_with_effects(
            &DurableStopRequest {
                original: request.clone(),
            },
            &store,
            9,
        )
        .unwrap();
    assert!(stopped.durable);
    assert_eq!(stopped.disposition, ChatDisposition::StoppedAfterCommit);
    assert_eq!(effects.newly_committed_sends.len(), 1);
    assert_eq!(effects.affected_channels, vec![A]);
    assert_eq!(server.messages(A).len(), 1);
    assert_eq!(server.delivery_snapshot(A).states[0].id, stopped.message_id);
    assert!(!server.sync.durable_chat_needs_repair());
    let (_, replay) = server
        .stop_chat_request_with_effects(&DurableStopRequest { original: request }, &store, 9)
        .unwrap();
    assert!(replay.newly_committed_sends.is_empty() && replay.affected_channels.is_empty());
    // Exact duplicate effect consumption is inert and cannot reorder/retarget the recent ring.
    let prior = server.own_message_changes[&A].clone();
    server.track_committed_chat_sends(&effects);
    assert_eq!(server.own_message_changes[&A], prior);
}

/// Automatic host repair is driven by SnapshotNeeded, not another positive request or frontend
/// retry. Bob authors NO channel descendant, so a positive confirmation must come from his actual
/// authenticated receipt. This is the same Ready/lease path the native persistence worker uses.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_repair_tracks_send_before_real_peer_receipt_without_frontend_retry() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Some(
        ServerStore::open(dir.path(), b"repair", &mut ChaCha20Rng::seed_from_u64(906)).unwrap(),
    )));
    let hub = Hub::new();
    let channel = channel_id("general");
    let alice_peer = PeerId::from_u64(906);
    let mut alice = Server::found(
        hub.join(alice_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(906),
        Box::new(ManualClock::new(1_000)),
        "alice",
    )
    .unwrap();
    alice.subscribe_control().await.unwrap();
    alice.open_channel(channel).await.unwrap();
    alice
        .publish_self_record(vec!["/ip4/203.0.113.1/tcp/22487".into()], 1)
        .unwrap();
    let owner_record = alice.sync.self_record().unwrap().clone();
    let invite = alice.mint_invite([7; 16], u64::MAX, vec![]).unwrap();
    let (alice, mut alice_events, alice_task) = spawn(alice);
    let mut bob = timeout(
        Duration::from_secs(10),
        Server::join(
            hub.join(PeerId::from_u64(907)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(907),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &invite,
        ),
    )
    .await
    .expect("actual member join progressed")
    .unwrap();
    bob.subscribe_control().await.unwrap();
    bob.open_channel(channel).await.unwrap();
    assert!(bob.sync.ingest_peer_record(owner_record));
    let mut connections = catcoms_sync::PreOwnerConnectionHandoff::default();
    connections.observe(&catcoms_rt::TransportEvent::PeerConnected(alice_peer));
    bob.sync.adopt_pre_owner_connections(connections);
    timeout(
        Duration::from_secs(10),
        bob.request_channel_catchup(alice_peer, channel),
    )
    .await
    .expect("actual signed empty catch-up progressed")
    .unwrap();
    assert!(
        bob.sync.has_proven_connected_member_peer(),
        "descriptor/connection claims cannot replace a real authenticated endpoint exchange"
    );
    assert!(bob.messages(channel).is_empty());
    let (bob, mut bob_events, bob_task) = spawn(bob);
    let bob_drain = tokio::spawn(async move { while bob_events.recv().await.is_some() {} });
    alice.messages(channel).await;
    while alice_events.try_recv().is_ok() {}

    let request = DurableSendRequest {
        token: [8; 16],
        expected_context: alice.durable_send_context().await.unwrap(),
        channel,
        text: "repair then receive, no retry".into(),
        reply_to: String::new(),
    };
    store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .fail_next_chat_parent_sync();
    let pending = alice
        .prepare_durable_send(request)
        .await
        .unwrap()
        .commit(DurableSendLease::new(
            store.clone().lock_owned().await,
            9,
            (),
        ))
        .await
        .unwrap();
    assert!(!pending.durable);
    assert!(alice
        .delivery_snapshot(channel)
        .await
        .unwrap()
        .states
        .is_empty());
    assert!(alice.messages(channel).await.is_empty());

    let worker_actor = alice.clone();
    let worker_store = store.clone();
    let (repaired, repaired_rx) = tokio::sync::oneshot::channel();
    let host = tokio::spawn(async move {
        let mut repaired = Some(repaired);
        while let Some(event) = alice_events.recv().await {
            if matches!(event.event, AppEvent::SnapshotNeeded) {
                if let Some(ready) = worker_actor.prepare_durable_chat_repair().await.unwrap() {
                    ready
                        .commit(DurableSendLease::new(
                            worker_store.clone().lock_owned().await,
                            9,
                            (),
                        ))
                        .await
                        .unwrap();
                    if let Some(done) = repaired.take() {
                        let _ = done.send(());
                    }
                }
            }
        }
    });
    timeout(Duration::from_secs(10), repaired_rx)
        .await
        .expect("host repaired actual uncertain save")
        .unwrap();
    // No retry of the original token occurs anywhere after the failed request.
    timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = alice.delivery_snapshot(channel).await.unwrap();
            if snapshot
                .states
                .iter()
                .any(|state| state.id == pending.message_id && state.delivered == 1)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("actual quiet peer receipt must confirm repaired original");
    assert_eq!(bob.messages(channel).await[0].id, pending.message_id);
    let saved = original_from_store(store.lock().await.as_ref().unwrap(), 9);
    assert_eq!(saved.messages(channel)[0].id, pending.message_id);
    alice.shutdown().await;
    bob.shutdown().await;
    alice_task.await.unwrap();
    bob_task.await.unwrap();
    host.await.unwrap();
    bob_drain.await.unwrap();
}

/// Seed terminal slots from one real encoded NeverAuthored Stop. This avoids 65,535 disk
/// transactions while preserving the exact closed record grammar, MLS/provider snapshot and
/// encrypted Store path. These fixture-only repeated terminal bodies grant no signing authority.
fn snapshot_with_stopped_slots(
    server: &mut Node,
    template_token: [u8; 16],
    slots: usize,
) -> Vec<u8> {
    use catcoms_wire::{Decoder, Encoder};
    let snapshot = server.snapshot().unwrap();
    let mut prefix = vec![2, 0, 0, 0, 1, 0, 0, 0, 16];
    prefix.extend_from_slice(&template_token);
    let positions: Vec<_> = snapshot
        .windows(prefix.len())
        .enumerate()
        .filter_map(|(position, bytes)| (bytes == prefix).then_some(position))
        .collect();
    assert_eq!(
        positions.len(),
        1,
        "locate the exact singleton durable-chat field"
    );
    let start = positions[0];
    let length = u32::from_be_bytes(snapshot[start - 4..start].try_into().unwrap()) as usize;
    let field = &snapshot[start..start + length];
    let mut decoder = Decoder::new(field);
    assert_eq!(decoder.get_u8().unwrap(), 2);
    assert_eq!(decoder.get_u32().unwrap(), 1);
    assert_eq!(decoder.get_bytes().unwrap(), template_token);
    let record_body = &field[length - decoder.remaining()..];
    let mut header = Encoder::new();
    header.put_u8(2);
    header.put_u32(u32::try_from(slots).unwrap());
    let mut seeded = header.finish();
    for index in 0..slots {
        let mut token = [0xa1; 16];
        token[8..].copy_from_slice(&(index as u64).to_be_bytes());
        seeded.extend_from_slice(&16u32.to_be_bytes());
        seeded.extend_from_slice(&token);
        seeded.extend_from_slice(record_body);
    }
    let mut replaced = snapshot[..start - 4].to_vec();
    replaced.extend_from_slice(&u32::try_from(seeded.len()).unwrap().to_be_bytes());
    replaced.extend_from_slice(&seeded);
    replaced.extend_from_slice(&snapshot[start + length..]);
    replaced
}

#[tokio::test]
async fn uncertain_stop_owns_last_receipt_slot_and_exact_repair_remains_admissible_at_cap() {
    use catcoms_sync::durable_chat::MAX_CHAT_RECEIPTS;
    let dir = tempfile::tempdir().unwrap();
    let store = ServerStore::open(
        dir.path(),
        b"capacity",
        &mut ChaCha20Rng::seed_from_u64(908),
    )
    .unwrap();
    let mut original = node().await;
    let template = send(&original, 0xe7, A, "fixture terminal");
    assert!(
        original
            .stop_chat_request(&DurableStopRequest { original: template }, &store, 9,)
            .unwrap()
            .durable
    );
    let seeded = snapshot_with_stopped_slots(&mut original, [0xe7; 16], MAX_CHAT_RECEIPTS - 1);
    let mut server = Server::restore(
        &seeded,
        Hub::new().join(PeerId::from_u64(908)),
        ChaCha20Rng::seed_from_u64(908),
        Box::new(ManualClock::new(2_000)),
        "alice",
    )
    .unwrap();
    store
        .save_server(9, &seeded, &mut ChaCha20Rng::seed_from_u64(909))
        .unwrap();
    let last = send(&server, 0xfe, B, "never authored last slot");
    let extra = send(&server, 0xff, A, "must not allocate a receipt");
    store.fail_next_chat_parent_sync();
    let (pending, effects) = server
        .stop_chat_request_with_effects(
            &DurableStopRequest {
                original: last.clone(),
            },
            &store,
            9,
        )
        .unwrap();
    assert!(!pending.durable);
    assert!(effects.newly_committed_sends.is_empty() && effects.affected_channels.is_empty());
    assert!(server.sync.durable_chat_needs_repair());
    assert!(
        server.snapshot().is_err(),
        "uncertainty cannot be snapshotted away"
    );
    let path = dir.path().join("servers").join("9.bin");
    let previous_file = std::fs::read(&path).unwrap();
    let (refused, effects) = server
        .stop_chat_request_with_effects(
            &DurableStopRequest {
                original: extra.clone(),
            },
            &store,
            9,
        )
        .unwrap();
    assert!(!refused.durable);
    assert!(refused
        .storage_error
        .as_deref()
        .unwrap()
        .contains("CHAT_SEND_CAPACITY"));
    assert!(effects.newly_committed_sends.is_empty() && effects.affected_channels.is_empty());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        previous_file,
        "refusal must not replace ciphertext"
    );
    let (token, binding, context, channel, id, _, _) = extra.identity();
    assert!(server
        .sync
        .durable_chat_lookup(token, binding, context, channel)
        .unwrap()
        .is_none());
    let error = server
        .sync
        .prepare_durable_chat(token, binding, context, channel, id, |_| {
            panic!("a new positive candidate must not be allocated at the union cap")
        })
        .unwrap_err();
    assert!(error.contains("CHAT_SEND_CAPACITY"));
    assert_eq!(std::fs::read(&path).unwrap(), previous_file);
    assert!(server.messages(A).is_empty() && server.messages(B).is_empty());
    assert!(server.own_message_changes.is_empty());
    let (token, binding, context, channel, id, _, _) = last.identity();
    let same_token = server
        .sync
        .prepare_durable_chat(token, binding, context, channel, id, |_| {
            panic!("an uncertain Stop must never remint positively, including at capacity")
        })
        .unwrap();
    assert!(!same_token.durable);
    assert_eq!(same_token.disposition, ChatDisposition::StoppedBeforeCommit);

    // Existing debt is still allowed into the actual Store barrier at the declared cap.
    // Another real rename+parent-sync failure distinguishes this from a capacity refusal.
    store.fail_next_chat_parent_sync();
    let error = server
        .sync
        .repair_durable_chat_with_effects(|bytes, rng| {
            store.save_server(9, bytes, rng).map_err(save_error)
        })
        .unwrap_err();
    assert!(matches!(error, ChatSaveError::ReplacedNotDurable(_)));
    assert_ne!(std::fs::read(&path).unwrap(), previous_file);
    assert!(server.sync.durable_chat_needs_repair());
    let (stopped, effects) = server
        .stop_chat_request_with_effects(
            &DurableStopRequest {
                original: last.clone(),
            },
            &store,
            9,
        )
        .unwrap();
    assert!(stopped.durable);
    assert_eq!(stopped.disposition, ChatDisposition::StoppedBeforeCommit);
    assert!(effects.newly_committed_sends.is_empty() && effects.affected_channels.is_empty());
    assert!(!server.sync.durable_chat_needs_repair());
    let reopened = original_from_store(&store, 9);
    let (token, binding, context, channel, _, _, _) = last.identity();
    assert!(
        reopened
            .sync
            .durable_chat_lookup(token, binding, context, channel)
            .unwrap()
            .unwrap()
            .durable
    );
    let (token, binding, context, channel, _, _, _) = extra.identity();
    assert!(reopened
        .sync
        .durable_chat_lookup(token, binding, context, channel)
        .unwrap()
        .is_none());
    assert!(reopened.messages(A).is_empty() && reopened.messages(B).is_empty());
}

#[tokio::test]
async fn real_committed_effect_cannot_retarget_an_existing_message_id_to_a_different_hash() {
    let dir = tempfile::tempdir().unwrap();
    let store = ServerStore::open(
        dir.path(),
        b"conflict",
        &mut ChaCha20Rng::seed_from_u64(910),
    )
    .unwrap();
    let mut server = node().await;
    let first = send(&server, 9, A, "first real change");
    let (_, effects) = server
        .commit_chat_request_with_effects(&first, &store, 9)
        .unwrap();
    let first_hash = effects.newly_committed_sends[0].change;
    let second = send(&server, 10, A, "second real change");
    let (_, _, _, _, second_id, _, _) = second.identity();
    server
        .own_message_changes
        .get_mut(&A)
        .unwrap()
        .push_back((second_id, first_hash));
    let retained = server.own_message_changes[&A].clone();
    let (committed, effects) = server
        .commit_chat_request_with_effects(&second, &store, 9)
        .unwrap();
    assert!(committed.durable);
    assert_eq!(effects.newly_committed_sends.len(), 1);
    assert_ne!(effects.newly_committed_sends[0].change, first_hash);
    assert_eq!(
        server.own_message_changes[&A], retained,
        "foreign hash must never replace/reorder retained identity"
    );
    server.track_committed_chat_sends(&effects);
    assert_eq!(server.own_message_changes[&A], retained);
    assert_eq!(
        server.messages(A).len(),
        2,
        "both genuine saved bodies remain in history"
    );
}
