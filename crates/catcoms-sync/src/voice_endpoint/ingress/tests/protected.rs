//! The receiving end is an actual Root with MLS membership, directory custody and a native
//! CallGrant. Only the remote AEAD result is mocked; these tests make no browser/media claim.
use super::*;
use automerge::{transaction::Transactable, ReadDoc, ROOT};
use catcoms_rt::RngCore;
use voice_v2::crypto_actions::{CryptoAction, CryptoInput};
use voice_v2::presence::{
    ConfirmationOutput, FlightSubmission, LocalJoin, WatchAuthority, Watcher,
};
use voice_v2::{EncryptedKind, JoinKey};

struct Fixture {
    sender: Node,
    receiver: Node,
    runtime: Arc<Runtime>,
    _sender_runtime: Arc<Runtime>,
    clock: Arc<ManualClock>,
    grant: Arc<CallGrant>,
    object: SignedObject,
    plaintext: Vec<u8>,
}
fn key(seed: u8) -> JoinKey {
    let point = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap();
    JoinKey::new([seed; 32], point.try_into().unwrap()).unwrap()
}
fn authority<'a>(context: &'a Context, lease: &'a CallLease) -> Authority<'a> {
    Authority {
        context,
        lease,
        member: &|_| true,
    }
}
async fn fixture() -> Fixture {
    fixture_in_room(1).await
}
async fn fixture_in_room(room: u128) -> Fixture {
    let (mut sender, mut receiver, runtime, clock) = super::fixture().await;
    assert!(sender.device_id() < receiver.device_id());
    // The response verifier also needs its own actual native actor, not the receiver's owner.
    let sender_runtime = Runtime::new(clock.clone());
    attach(&mut sender, &sender_runtime);
    if room != 1 {
        // A real Root document and directory observation, rather than a bare revoked flag.
        // This test reader models permanent compatibility room1 and deletable numeric rooms.
        fn rooms(doc: Option<&AutoCommit>) -> Result<BTreeSet<u128>, VoiceError> {
            let mut rooms = BTreeSet::from([1]);
            if let Some(doc) = doc {
                for (n, key) in doc.keys(ROOT).enumerate() {
                    if n >= voice_v2::directory::MAX_DIRECTORY_KEYS {
                        return Err(VoiceError::Capacity);
                    }
                    if let Ok(room) = key.parse() {
                        rooms.insert(room);
                    }
                    if rooms.len() > voice_v2::directory::MAX_DIRECTORY_ROOMS {
                        return Err(VoiceError::Capacity);
                    }
                }
            }
            Ok(rooms)
        }
        receiver
            .open_channel(DocType::ChannelIndex, 991)
            .await
            .unwrap();
        receiver
            .post(DocType::ChannelIndex, 991, |doc| {
                doc.put(ROOT, room.to_string(), true)
            })
            .await
            .unwrap();
        let group = receiver.voice_v2_context(room).unwrap().scope.group;
        receiver.voice_runtime.as_mut().unwrap().directory =
            voice_v2::directory::Directory::new(group, runtime.resources(), rooms, |_| {
                Ok(BTreeSet::new())
            });
    }
    let grant = Arc::new(receiver.voice_call_grant(room).unwrap());
    let local_key = key(81);
    receiver
        .start_voice_endpoint(grant.clone(), local_key.join, local_key.public_key)
        .unwrap();
    let context = receiver.voice_v2_context(room).unwrap();
    let remote_context = Context {
        local: sender.device_id(),
        ..context
    };
    let remote_lease = CallLease::default();
    let pool = Arc::new(voice_v2::resources::Resources::default());
    let mut remote =
        LocalJoin::new(&remote_context, key(82), remote_lease.clone(), pool.clone()).unwrap();
    let remote_authority = authority(&remote_context, &remote_lease);
    let local_lease = grant.lease();
    let local_authority = authority(&context, &local_lease);
    let remote_announcement = remote
        .announce(
            &sender.device,
            &remote_authority,
            clock.as_ref(),
            &mut sender.rng,
        )
        .unwrap();
    let voice = receiver.voice_runtime.as_mut().unwrap();
    let participant = voice.endpoints.participant.as_mut().unwrap();
    let local_announcement = participant
        .join
        .announce(
            &receiver.device,
            &local_authority,
            clock.as_ref(),
            &mut receiver.rng,
        )
        .unwrap();
    let observer = voice
        .endpoints
        .observers
        .get_mut(&participant.watch.id)
        .unwrap();
    let watch_authority = WatchAuthority {
        context: &context,
        lease: &observer.lease,
        member: &|_| true,
    };
    let challenge = voice
        .current
        .as_ref()
        .unwrap()
        .with_limits(|limits| {
            observer.watcher.challenge(
                &remote_announcement,
                &receiver.device,
                &watch_authority,
                limits,
                clock.as_ref(),
                &mut receiver.rng,
            )
        })
        .unwrap();
    let proof = remote
        .answer(
            &challenge,
            &sender.device,
            &remote_authority,
            clock.as_ref(),
            &mut sender.rng,
        )
        .unwrap();
    let fresh = observer
        .watcher
        .prove(&proof, &watch_authority, clock.as_ref())
        .unwrap();
    let setup = voice
        .current
        .as_ref()
        .unwrap()
        .with_limits(|limits| {
            participant
                .join
                .begin_pair(&fresh, &local_authority, limits, clock.as_ref())
        })
        .unwrap();
    assert!(!setup.local_is_low);
    participant
        .join
        .prepare_crypto_chain(setup.handshake, &local_authority, clock.as_ref())
        .unwrap();

    let remote_watch = WatchLease::default();
    let mut watcher = Watcher::new(
        &remote_context,
        remote_watch.clone(),
        pool.clone(),
        &mut sender.rng,
    )
    .unwrap();
    let remote_watch_authority = WatchAuthority {
        context: &remote_context,
        lease: &remote_watch,
        member: &|_| true,
    };
    let mut limits = voice_v2::limits::OriginLimits::new(context.scope.group, pool);
    let challenge = watcher
        .challenge(
            &local_announcement,
            &sender.device,
            &remote_watch_authority,
            &mut limits,
            clock.as_ref(),
            &mut sender.rng,
        )
        .unwrap();
    let proof = participant
        .join
        .answer(
            &challenge,
            &receiver.device,
            &local_authority,
            clock.as_ref(),
            &mut receiver.rng,
        )
        .unwrap();
    let fresh = watcher
        .prove(&proof, &remote_watch_authority, clock.as_ref())
        .unwrap();
    let remote_setup = remote
        .begin_pair(&fresh, &remote_authority, &mut limits, clock.as_ref())
        .unwrap();
    assert_eq!(remote_setup.handshake, setup.handshake);
    remote
        .start_confirmation(
            setup.handshake,
            &remote_authority,
            clock.as_ref(),
            &mut sender.rng,
        )
        .unwrap();
    let Some(ConfirmationOutput::Encrypt(flight)) = remote
        .prepare_confirmation(
            setup.handshake,
            &remote_authority,
            clock.as_ref(),
            &mut sender.rng,
        )
        .unwrap()
    else {
        panic!("Hello flight")
    };
    let object = remote
        .submit_confirmation(
            FlightSubmission {
                handshake: flight.handshake,
                action: flight.action,
                reservation: flight.reservation.id,
                ciphertext: &vec![22; flight.plaintext.len() + 16],
            },
            &sender.device,
            &remote_authority,
            clock.as_ref(),
        )
        .unwrap();
    // The Runtime process registry must survive independently of its retained actor handles.
    Fixture {
        sender,
        receiver,
        runtime,
        _sender_runtime: sender_runtime,
        clock,
        grant,
        object,
        plaintext: flight.plaintext,
    }
}
fn admit(f: &mut Fixture, object: SignedObject) -> Result<EndpointReceipt, VoiceError> {
    let context = f.receiver.voice_v2_context(f.grant.scope().room)?;
    let room = f.receiver.voice_room_authority(context.scope.room)?;
    let envelope = Envelope::Direct(object);
    let ticket = f
        .receiver
        .voice_transport_actor()?
        .admit_transport_verification(f.sender.device_id(), &envelope.encode())?;
    f.receiver
        .admit_voice_endpoint(ticket, envelope, &context, room)
}
fn actions(f: &mut Fixture) -> Vec<Arc<CryptoAction>> {
    let context = f.receiver.voice_v2_context(f.grant.scope().room).unwrap();
    f.receiver
        .voice_runtime
        .as_mut()
        .unwrap()
        .endpoints
        .participant
        .as_mut()
        .unwrap()
        .join
        .pull_crypto_actions(&authority(&context, &f.grant.lease()))
        .unwrap()
}
fn frame(f: &mut Fixture, object: SignedObject) -> Vec<u8> {
    let inner = Envelope::Direct(object).encode();
    let node = &mut f.sender;
    let key = node.device.public_key_bytes();
    let mut nonce = [0; 16];
    node.rng.fill_bytes(&mut nonce);
    let auth = RequestAuth {
        ts: node.voice_v2_clock().now_ms(),
        nonce,
        epoch: node.group.epoch(),
    };
    let transcript = catchup_auth_transcript(
        &node.group.group_id(),
        KIND_VOICE_TRANSPORT,
        &inner,
        &key,
        auth.ts,
        &auth.nonce,
        auth.epoch,
        Some(&node.local_peer()),
    );
    let mut frame = vec![KIND_VOICE_TRANSPORT];
    frame.extend(encode_authed_request(
        &inner,
        &key,
        auth.ts,
        &auth.nonce,
        auth.epoch,
        &node.device.sign(&transcript).unwrap(),
    ));
    frame
}

#[tokio::test]
async fn protected_root_hello_returns_signed_admission_and_exact_lost_ack_without_new_work() {
    let mut f = fixture().await;
    let object = f.object.clone();
    let request = frame(&mut f, object.clone());
    let answer = f
        .receiver
        .receive_voice_transport(f.sender.local_peer(), &request)
        .unwrap();
    assert_eq!(
        f.sender
            .verify_voice_transport_response(
                f.receiver.local_peer(),
                &request,
                object.hash(),
                &answer
            )
            .unwrap(),
        crate::VoiceTransportStatus::EndpointAdmitted
    );
    let first = actions(&mut f);
    assert_eq!(first.len(), 1);
    let input = first[0].input().unwrap();
    assert!(
        matches!(input.as_input(), CryptoInput::DecryptConfirmation(packet) if packet.object_hash() == object.hash() && packet.header.kind == EncryptedKind::ConfirmHello)
    );
    drop(answer);
    let retained = f.runtime.resources().counts();
    let receipt = admit(&mut f, object.clone()).unwrap();
    assert!(receipt.authorized());
    assert_eq!(receipt.object_hash(), object.hash());
    assert_eq!(f.runtime.resources().counts(), retained);
    assert!(Arc::ptr_eq(&first[0], &actions(&mut f)[0]));

    // Successful decryption consumes its token. Admission history must survive independently.
    first[0].admit_engine(first[0].engine_bytes()).unwrap();
    let context = f.receiver.voice_v2_context(f.grant.scope().room).unwrap();
    let result = f
        .receiver
        .voice_runtime
        .as_mut()
        .unwrap()
        .endpoints
        .participant
        .as_mut()
        .unwrap()
        .join
        .commit_confirmation_result(
            &first[0],
            &f.plaintext,
            first[0].output_tag(&f.plaintext).unwrap(),
            &authority(&context, &f.grant.lease()),
            f.clock.as_ref(),
            &mut f.receiver.rng,
        )
        .unwrap();
    first[0].settle_engine();
    first[0].acknowledge_result(&result).unwrap();
    assert!(admit(&mut f, object).unwrap().authorized());
    assert!(actions(&mut f)
        .iter()
        .all(|action| Arc::ptr_eq(action, &first[0])));
}

#[tokio::test]
async fn protected_root_shared_identity_refuses_public_encrypted_substitution_in_both_orders() {
    for encrypted_first in [false, true] {
        let mut f = fixture().await;
        let original = f.object.clone();
        let public = SignedObject::issue(
            original.scope(),
            &f.sender.device,
            original.id(),
            1000,
            31_000,
            Body::Announcement {
                key: key(82),
                epoch: f.sender.group.epoch(),
                renewal: 1,
            },
        )
        .unwrap();
        let (first, changed) = if encrypted_first {
            (original, public)
        } else {
            (public, original)
        };
        let receipt = admit(&mut f, first).unwrap();
        assert!(receipt.authorized());
        let before = f.runtime.resources().counts();
        assert_eq!(
            admit(&mut f, changed).err(),
            Some(VoiceError::RetryMismatch)
        );
        assert_eq!(f.runtime.resources().counts(), before);
        assert_eq!(actions(&mut f).len(), usize::from(encrypted_first));
    }
}

#[tokio::test]
async fn protected_root_end_and_equal_join_replacement_cannot_adopt_original_admission() {
    let mut f = fixture().await;
    let object = f.object.clone();
    let receipt = admit(&mut f, object.clone()).unwrap();
    let original_action = actions(&mut f)[0].clone();
    let old_key = f
        .receiver
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .participant
        .as_ref()
        .unwrap()
        .join
        .key()
        .clone();
    f.grant.lease().revoke();
    assert!(!receipt.authorized());
    f.receiver.reconcile_voice_actor();
    let fresh = Arc::new(f.receiver.voice_call_grant(f.grant.scope().room).unwrap());
    f.receiver
        .start_voice_endpoint(fresh.clone(), old_key.join, old_key.public_key)
        .unwrap();
    f.grant = fresh;
    assert!(admit(&mut f, object).is_err());
    assert!(actions(&mut f).is_empty());
    assert!(original_action
        .admit_engine(original_action.engine_bytes())
        .is_err());
}

#[tokio::test]
async fn protected_root_queued_ack_epoch_change_revokes_before_driver_send_and_first_result() {
    let mut f = fixture().await;
    let reserved_work = f.runtime.resources().counts().1;
    let object = f.object.clone();
    let request = frame(&mut f, object.clone());
    let reply = f
        .receiver
        .receive_voice_transport(f.sender.local_peer(), &request)
        .unwrap();
    let action = actions(&mut f).pop().unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let (responder, receiver) = catcoms_rt::Responder::channel();
    reply.respond(responder).unwrap();
    let response = receiver.recv_owned().await.unwrap();
    assert!(response.is_current());
    let epoch = f.receiver.group.epoch();
    let joiner = MlsDevice::generate().unwrap();
    let package = joiner.key_package().unwrap();
    f.receiver
        .with_observed_mls_transition(|node| node.group.add_member(&node.device, package))
        .unwrap();
    assert!(f.receiver.group.epoch() > epoch);
    assert!(f.receiver.group.contains_device(&f.sender.device_id()));
    assert!(
        !response.is_current(),
        "original Root route fences normal epoch advance even when both endpoints remain members"
    );
    assert!(admit(&mut f, object).is_err());
    let context = f.receiver.voice_v2_context(f.grant.scope().room).unwrap();
    assert_eq!(
        f.receiver
            .voice_runtime
            .as_mut()
            .unwrap()
            .endpoints
            .participant
            .as_mut()
            .unwrap()
            .join
            .commit_confirmation_result(
                &action,
                &f.plaintext,
                action.output_tag(&f.plaintext).unwrap(),
                &authority(&context, &f.grant.lease()),
                f.clock.as_ref(),
                &mut f.receiver.rng
            ),
        Err(VoiceError::Unauthorized)
    );
    assert!(action.terminal_status().receipt.is_none());
    assert_eq!(
        f.runtime.resources().counts().1,
        reserved_work + 1,
        "response verification and original protected work remain charged independently"
    );
    action.settle_engine();
    drop(response);
    assert_eq!(
        f.runtime.resources().counts().1,
        reserved_work,
        "settlement returns its loan to the still-admitted chain"
    );
    f.grant.lease().revoke();
    f.receiver.reconcile_voice_actor();
    assert_eq!(f.runtime.resources().counts().1, 0);
}

#[tokio::test]
async fn protected_root_actual_room_delete_recreate_cannot_revive_ack_or_original_call() {
    let mut f = fixture_in_room(2).await;
    let object = f.object.clone();
    let receipt = admit(&mut f, object.clone()).unwrap();
    let request = frame(&mut f, object.clone());
    let reply = f
        .receiver
        .receive_voice_transport(f.sender.local_peer(), &request)
        .unwrap();
    let action = actions(&mut f).pop().unwrap();
    f.receiver
        .post(DocType::ChannelIndex, 991, |doc| doc.delete(ROOT, "2"))
        .await
        .unwrap();
    assert!(!receipt.authorized());
    assert!(!f.grant.lease().is_live());
    let (responder, receiver) = catcoms_rt::Responder::channel();
    assert!(matches!(
        reply.respond(responder),
        Err(catcoms_rt::TransportError::Cancelled)
    ));
    assert!(receiver.recv_owned().await.is_none());
    f.receiver
        .post(DocType::ChannelIndex, 991, |doc| doc.put(ROOT, "2", true))
        .await
        .unwrap();
    let fresh = Arc::new(f.receiver.voice_call_grant(2).unwrap());
    let local_key = key(81);
    f.receiver
        .start_voice_endpoint(fresh.clone(), local_key.join, local_key.public_key)
        .unwrap();
    f.grant = fresh;
    assert!(admit(&mut f, object).is_err());
    assert!(actions(&mut f).is_empty());
    assert!(action.admit_engine(action.engine_bytes()).is_err());
    assert!(!receipt.authorized());
}
