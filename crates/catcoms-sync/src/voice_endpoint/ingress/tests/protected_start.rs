//! Actual Root low producer, using real signed discovery and the actual affine call owner.
//! This establishes no native IPC, WebView execution or physical network/audio acceptance.
use super::*;
use voice_v2::crypto_actions::{CryptoAction, CryptoInput};
use voice_v2::presence::{LocalJoin, WatchAuthority};
use voice_v2::JoinKey;

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

async fn low_fixture() -> (
    Node,
    Node,
    Arc<Runtime>,
    Arc<ManualClock>,
    Arc<CallGrant>,
    voice_v2::Nonce,
) {
    let (mut local, mut remote, _remote_runtime, clock) = super::fixture().await;
    assert!(local.device_id() < remote.device_id());
    let runtime = Runtime::new(clock.clone());
    attach(&mut local, &runtime);
    let grant = Arc::new(local.voice_call_grant(1).unwrap());
    let local_key = key(81);
    local
        .start_voice_endpoint(grant.clone(), local_key.join, local_key.public_key)
        .unwrap();
    let context = local.voice_v2_context(1).unwrap();
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
    let fresh = observer
        .watcher
        .prove(&proof, &watch_authority, clock.as_ref())
        .unwrap();
    let pair = voice
        .current
        .as_ref()
        .unwrap()
        .with_limits(|limits| {
            participant
                .join
                .begin_pair(&fresh, &local_authority, limits, clock.as_ref())
        })
        .unwrap();
    assert!(pair.local_is_low);
    (local, remote, runtime, clock, grant, pair.handshake)
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
    assert!(old.admit_engine(old.engine_bytes()).is_err());
    old.settle_engine();
}

#[tokio::test]
async fn root_protected_low_mls_epoch_addition_retires_old_start_without_removing_its_peer() {
    let (mut root, remote, _runtime, clock, grant, handshake) = low_fixture().await;
    let old = root
        .start_voice_confirmation(&grant, handshake, None)
        .unwrap();
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
    assert_eq!(clock.now_ms(), 1000, "the test changes epoch, not expiry");
}
