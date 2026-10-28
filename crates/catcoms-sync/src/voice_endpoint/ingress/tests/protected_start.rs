//! Actual Root low producer, using real signed discovery and the actual affine call owner.
//! This establishes no native IPC, WebView execution or physical network/audio acceptance.
use super::*;
use voice_v2::crypto_actions::{CryptoAction, CryptoInput};
use voice_v2::presence::{LocalJoin, WatchAuthority};
use voice_v2::JoinKey;
mod admission;
mod derivation;
mod pair;
mod progress;

fn key(seed: u8) -> JoinKey {
    JoinKey::new([seed; 32], hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()).unwrap()
}
fn authority<'a>(context: &'a Context, lease: &'a CallLease) -> Authority<'a> {
    Authority {
        context,
        lease,
        member: &|_| true,
    }
}

type LowFixture = (
    Node,
    Node,
    Arc<Runtime>,
    Arc<ManualClock>,
    Arc<CallGrant>,
    voice_v2::Nonce,
);

async fn low_fixture() -> LowFixture {
    low_fixture_room(1).await
}

async fn low_fixture_room(room: u128) -> LowFixture {
    let (mut local, remote, runtime, clock, grant) = low_discovery_fixture(room).await;
    let context = local.voice_v2_context(room).unwrap();
    let pair = local
        .prepare_voice_pair(
            &grant,
            voice_v2::presence::PairSelector {
                peer: remote.device_id(),
                join: key(82).join,
                key_ref: key(82).key_ref,
                epoch: context.epoch,
            },
        )
        .unwrap();
    assert!(pair.local_is_low);
    (local, remote, runtime, clock, grant, pair.handshake)
}

// Shared real signed discovery without privately constructing a Pair. Positive protected Root
// tests now traverse the production pair API, while admission regressions can stop beforehand.
type DiscoveredFixture = (Node, Node, Arc<Runtime>, Arc<ManualClock>, Arc<CallGrant>);
async fn low_discovery_fixture(room: u128) -> DiscoveredFixture {
    low_discovery_for_watch(room, false).await.0
}
async fn low_discovery_for_watch(
    room: u128,
    explicit: bool,
) -> (DiscoveredFixture, Option<Arc<crate::VoiceWatchGrant>>) {
    low_discovery_with_clock(room, explicit, |clock| clock).await
}
async fn low_discovery_with_clock(
    room: u128,
    explicit: bool,
    make_clock: impl FnOnce(Arc<ManualClock>) -> Arc<dyn Clock>,
) -> (DiscoveredFixture, Option<Arc<crate::VoiceWatchGrant>>) {
    let (mut local, mut remote, _remote_runtime, clock) = super::fixture().await;
    assert!(local.device_id() < remote.device_id());
    let runtime = Runtime::new(make_clock(clock.clone()));
    if room == 1 {
        attach(&mut local, &runtime);
    } else {
        use automerge::{transaction::Transactable, ROOT};
        assert_eq!(room, 2);
        local
            .open_channel(DocType::ChannelIndex, 991)
            .await
            .unwrap();
        local
            .post(DocType::ChannelIndex, 991, |doc| doc.put(ROOT, "2", true))
            .await
            .unwrap();
        let ticket = runtime.session_ticket();
        runtime.activate_session(&ticket).unwrap();
        local
            .prepare_voice_actor(
                &runtime,
                &ticket,
                991,
                derivation::read_rooms,
                derivation::deleted_rooms,
            )
            .unwrap()
            .register()
            .unwrap();
    }
    let grant = Arc::new(local.voice_call_grant(room).unwrap());
    let local_key = key(81);
    local
        .start_voice_endpoint(grant.clone(), local_key.join, local_key.public_key)
        .unwrap();
    let explicit_watch =
        explicit.then(|| local.start_voice_watch(room, CallLease::default()).unwrap());
    let context = local.voice_v2_context(room).unwrap();
    let remote_context = Context {
        local: remote.device_id(),
        ..context
    };
    let remote_lease = CallLease::default();
    let mut remote_join = LocalJoin::new(
        &remote_context,
        key(82),
        remote_lease.clone(),
        Arc::new(voice_v2::resources::Resources::default()),
    )
    .unwrap();
    let remote_authority = authority(&remote_context, &remote_lease);
    let remote_announcement = remote_join
        .announce(
            &remote.device,
            &remote_authority,
            clock.as_ref(),
            &mut remote.rng,
        )
        .unwrap();
    let lease = grant.lease();
    let local_authority = authority(&context, &lease);
    let voice = local.voice_runtime.as_mut().unwrap();
    let participant = voice.endpoints.participant.as_mut().unwrap();
    participant
        .join
        .announce(
            &local.device,
            &local_authority,
            clock.as_ref(),
            &mut local.rng,
        )
        .unwrap();
    let observer = voice
        .endpoints
        .observers
        .get_mut(
            &explicit_watch
                .as_ref()
                .map_or(participant.watch.id, |watch| watch.id),
        )
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
                &local.device,
                &watch_authority,
                limits,
                clock.as_ref(),
                &mut local.rng,
            )
        })
        .unwrap();
    let proof = remote_join
        .answer(
            &challenge,
            &remote.device,
            &remote_authority,
            clock.as_ref(),
            &mut remote.rng,
        )
        .unwrap();
    observer
        .watcher
        .prove(&proof, &watch_authority, clock.as_ref())
        .unwrap();
    ((local, remote, runtime, clock, grant), explicit_watch)
}

fn message(action: &CryptoAction) -> voice_v2::Nonce {
    let input = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("Hello seal")
    };
    flight.reservation.id
}

#[tokio::test]
async fn root_protected_low_start_recovers_exact_history_and_refuses_a_foreign_call_owner() {
    let (mut root, _remote, _runtime, _clock, grant, handshake) = low_fixture().await;
    let first = root
        .start_voice_confirmation(&grant, handshake, None)
        .unwrap();
    let first_id = message(&first);
    let input = first.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("seal")
    };
    let ciphertext = vec![22; flight.plaintext.len() + 16];
    let tag = first.output_tag(&ciphertext).unwrap();
    // The Root result path must reject an unadmitted engine before signing, then publish its
    // original packet and terminal metadata through the actual directory/session fence.
    assert_eq!(
        root.submit_voice_confirmation_result(&grant, &first, &ciphertext, tag)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    first.admit_engine(first.engine_bytes()).unwrap();
    let sealed = root
        .submit_voice_confirmation_result(&grant, &first, &ciphertext, tag)
        .unwrap();
    assert!(matches!(
        sealed.outcome,
        voice_v2::crypto_actions::CryptoOutcome::Sealed { .. }
    ));
    first.settle_engine();
    first.acknowledge_result(&sealed).unwrap();
    assert!(first.input().is_err());
    assert!(sealed.same_transition(
        &root
            .submit_voice_confirmation_result(&grant, &first, &ciphertext, tag)
            .unwrap()
    ));
    let next = root
        .start_voice_confirmation(&grant, handshake, Some(first_id))
        .unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &root
            .start_voice_confirmation(&grant, handshake, None)
            .unwrap()
    ));
    assert!(Arc::ptr_eq(
        &next,
        &root
            .start_voice_confirmation(&grant, handshake, Some(first_id))
            .unwrap()
    ));
    let foreign = Arc::new(CallGrant::from_lease(CallLease::default(), grant.scope()));
    assert_eq!(
        root.start_voice_confirmation(&foreign, handshake, None)
            .err(),
        Some(VoiceError::Unauthorized)
    );
}

#[tokio::test]
async fn root_protected_low_equal_join_replacement_never_adopts_old_action_or_predecessor() {
    let (mut root, _remote, _runtime, _clock, grant, handshake) = low_fixture().await;
    let old = root
        .start_voice_confirmation(&grant, handshake, None)
        .unwrap();
    let predecessor = message(&old);
    let input = old.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("seal")
    };
    let ciphertext = vec![22; flight.plaintext.len() + 16];
    let tag = old.output_tag(&ciphertext).unwrap();
    old.admit_engine(old.engine_bytes()).unwrap();
    grant.lease().revoke();
    root.reconcile_voice_actor();
    let replacement = Arc::new(root.voice_call_grant(1).unwrap());
    let same_key = key(81);
    root.start_voice_endpoint(replacement.clone(), same_key.join, same_key.public_key)
        .unwrap();
    assert_eq!(
        root.start_voice_confirmation(&grant, handshake, None).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        root.start_voice_confirmation(&replacement, handshake, Some(predecessor))
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(!old.is_pending());
    for owner in [&grant, &replacement] {
        assert_eq!(
            root.submit_voice_confirmation_result(owner, &old, &ciphertext, tag)
                .err(),
            Some(VoiceError::Unauthorized)
        );
    }
    assert!(old.terminal_status().receipt.is_none());
    assert!(old.admit_engine(old.engine_bytes()).is_err());
    old.settle_engine();
}

#[tokio::test]
async fn root_protected_low_mls_epoch_addition_retires_old_start_without_removing_its_peer() {
    let (mut root, remote, _runtime, clock, grant, handshake) = low_fixture().await;
    let old = root
        .start_voice_confirmation(&grant, handshake, None)
        .unwrap();
    let input = old.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("seal")
    };
    let ciphertext = vec![22; flight.plaintext.len() + 16];
    let tag = old.output_tag(&ciphertext).unwrap();
    old.admit_engine(old.engine_bytes()).unwrap();
    let before = root.voice_v2_context(1).unwrap().epoch;
    let third = catcoms_mls::MlsDevice::generate().unwrap();
    let package = third.key_package().unwrap();
    root.with_observed_mls_transition(|node| node.group.add_member(&node.device, package))
        .unwrap();
    assert!(root.group.contains_device(&remote.device_id()));
    assert!(root.voice_v2_context(1).unwrap().epoch > before);
    assert!(grant.lease().is_live());
    assert_eq!(
        root.start_voice_confirmation(&grant, handshake, None).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(!old.is_pending());
    assert_eq!(
        root.submit_voice_confirmation_result(&grant, &old, &ciphertext, tag)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(old.terminal_status().receipt.is_none());
    old.settle_engine();
    assert_eq!(clock.now_ms(), 1000, "the test changes epoch, not expiry");
}
