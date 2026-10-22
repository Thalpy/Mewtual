use super::*;
use crate::voice_activation_tests::Fixture;
use catcoms_rt::{CryptoRng, RngCore};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const POINT: &str = "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
const EXECUTOR: &str = "0909090909090909090909090909090909090909090909090909090909090909";
struct Rng {
    inner: ChaCha20Rng,
    draws: usize,
}
impl Rng {
    fn new() -> Self {
        Self {
            inner: ChaCha20Rng::seed_from_u64(217),
            draws: 0,
        }
    }
}
impl CryptoRng for Rng {}
impl RngCore for Rng {
    fn next_u32(&mut self) -> u32 {
        self.draws += 1;
        self.inner.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.draws += 1;
        self.inner.next_u64()
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        self.draws += 1;
        self.inner.fill_bytes(bytes);
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), rand_core::Error> {
        self.draws += 1;
        self.inner.try_fill_bytes(bytes)
    }
}
async fn selected(f: &Fixture) -> String {
    let owner = f.candidate().await;
    crate::activate_voice_owner_inner(&f.state, owner.clone())
        .await
        .unwrap();
    let admission = crate::voice_join_key::admit_inner(&f.state, &owner, EXECUTOR)
        .await
        .unwrap();
    crate::voice_join_key::publish_inner(
        &f.state,
        &owner,
        EXECUTOR,
        &admission.permit,
        hex::decode(POINT).unwrap().try_into().unwrap(),
    )
    .await
    .unwrap();
    crate::voice_join_key::settle_inner(&f.state, &owner, [9; 32])
        .await
        .unwrap();
    owner
}
fn host(state: &AppState) -> Host {
    let mut config = serde_json::from_str(include_str!("../../../tauri.conf.json")).unwrap();
    let host = Host::prepare(state, &mut config, false).unwrap();
    host.start_in_tauri_runtime().unwrap();
    host
}

#[test]
fn identity_validation_is_canonical_bounded_and_allocation_free() {
    assert_eq!(
        identity("voice-0000000000000001", EXECUTOR).unwrap(),
        [9; 32]
    );
    for owner in [
        "",
        "voice-1",
        "voice-000000000000000A",
        "voice-0000000000000001\n",
        "voice-000000000000000\"",
        "voice-000000000000000é",
    ] {
        assert!(identity(owner, EXECUTOR).is_err());
    }
    for executor in ["", "09", &"A".repeat(64), &"g".repeat(64), &"0".repeat(65)] {
        assert!(identity("voice-0000000000000001", executor).is_err());
    }
}

#[test]
fn only_native_closed_and_actually_drained_is_a_void_cleanup_success() {
    assert!(drained(Ok(Status::Closed {
        native_setup_drained: true
    }))
    .is_ok());
    assert!(drained(Ok(Status::Closed {
        native_setup_drained: false
    }))
    .is_err());
    assert!(drained(Ok(Status::Pending)).is_err());
    assert!(drained(Err(Refused::Binding)).is_err());
}

#[tokio::test]
async fn unavailable_service_refuses_before_registry_admission_or_rng() {
    let f = Fixture::new().await;
    let owner = selected(&f).await;
    let resources = crate::voice_runtime(&f.state).resources();
    let before = resources.counts();
    let host = Host::unavailable(super::super::service::Refused::Io);
    let mut rng = Rng::new();
    assert!(
        setup_inner(&f.state, &host, owner, EXECUTOR.into(), &mut rng)
            .await
            .is_err()
    );
    assert_eq!(rng.draws, 0);
    assert_eq!(resources.counts(), before);
    f.close().await;
}

#[tokio::test]
async fn public_reply_is_exact_shared_and_projection_retries_never_repeat_ecdh() {
    let f = Fixture::new().await;
    let owner = selected(&f).await;
    let host = host(&f.state);
    let mut rng = Rng::new();
    let first = setup_inner(&f.state, &host, owner.clone(), EXECUTOR.into(), &mut rng)
        .await
        .unwrap();
    let draws = rng.draws;
    assert!(draws > 0);
    let json = first.json().unwrap();
    assert_eq!(
        json.capacity(),
        PUBLIC_REPLY_BYTES,
        "fixed allocation, not just final length"
    );
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert_eq!(value["owner"], owner);
    assert_eq!(value["executor"], EXECUTOR);
    assert_eq!(value["kind"], "ready");
    assert_eq!(value["port"], host.port().unwrap());
    assert!(value["transcript"].as_str().unwrap().len() <= 2048);
    assert!(
        setup_inner(&f.state, &host, owner.clone(), EXECUTOR.into(), &mut rng)
            .await
            .is_err(),
        "no overlapping native public projections"
    );
    assert_eq!(rng.draws, draws);
    // The first native-to-framework conversion was lost. Drop the actual projection then
    // recover the exact immutable metadata; no new scalar, Session or physical counter exists.
    drop(first);
    let recovered = setup_inner(&f.state, &host, owner.clone(), EXECUTOR.into(), &mut rng)
        .await
        .unwrap();
    assert_eq!(recovered.json().unwrap(), json);
    assert_eq!(rng.draws, draws);
    drop(recovered);
    f.state.voice_authority.lock().unwrap().end(&owner);
    settle_inner(&f.state, &owner, EXECUTOR).await.unwrap();
    collect_inner(&f.state, &owner, EXECUTOR).await.unwrap();
    host.finish().await.unwrap();
    f.close().await;
}

#[test]
fn hard_json_writer_refuses_growth_without_reallocating() {
    let mut buffer = JsonBuffer {
        bytes: Vec::with_capacity(PUBLIC_REPLY_BYTES),
    };
    buffer.write_all(&[b'x'; PUBLIC_REPLY_BYTES]).unwrap();
    assert!(buffer.write_all(b"x").is_err());
    assert_eq!(buffer.bytes.capacity(), PUBLIC_REPLY_BYTES);
    assert_eq!(buffer.bytes.len(), PUBLIC_REPLY_BYTES);
}

#[tokio::test]
async fn ready_and_closed_share_one_guard_and_delayed_closed_body_remains_paid_after_collection() {
    let f = Fixture::new().await;
    let owner = selected(&f).await;
    let selected = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .selected_handoff_owner(&owner, [9; 32])
        .unwrap();
    let resources = selected.resources();
    let host = host(&f.state);
    let mut rng = Rng::new();
    let ready = setup_inner(&f.state, &host, owner.clone(), EXECUTOR.into(), &mut rng)
        .await
        .unwrap();
    settle_inner(&f.state, &owner, EXECUTOR).await.unwrap();
    assert!(
        setup_inner(&f.state, &host, owner.clone(), EXECUTOR.into(), &mut rng)
            .await
            .is_err(),
        "Closed cannot overlap a delayed Ready projection"
    );
    drop(ready);
    let closed = setup_inner(&f.state, &host, owner.clone(), EXECUTOR.into(), &mut rng)
        .await
        .unwrap();
    assert!(
        setup_inner(&f.state, &host, owner.clone(), EXECUTOR.into(), &mut rng)
            .await
            .is_err(),
        "Closed cannot overlap another Closed projection"
    );
    f.state.voice_authority.lock().unwrap().end(&owner);
    collect_inner(&f.state, &owner, EXECUTOR).await.unwrap();
    let retained = resources.counts();
    // All Session/SDK work is gone, but this delayed native reply still owns its original
    // Row payment and projection position through actual fixed-buffer serialization.
    let json = closed.json().unwrap();
    assert_eq!(json.capacity(), PUBLIC_REPLY_BYTES);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap()["kind"],
        "closed"
    );
    assert_eq!(resources.counts(), retained);
    drop(closed);
    collect_inner(&f.state, &owner, EXECUTOR).await.unwrap();
    assert_eq!(resources.counts().0, retained.0 - 8192);
    drop(selected);
    host.finish().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn pending_metadata_also_owns_the_original_serial_projection_payment() {
    let f = Fixture::new().await;
    let owner = selected(&f).await;
    let host = host(&f.state);
    let mut rng = Rng::new();
    drop(
        setup_inner(&f.state, &host, owner.clone(), EXECUTOR.into(), &mut rng)
            .await
            .unwrap(),
    );
    let projection = f.state.voice_channels.project(&owner, [9; 32]).unwrap();
    // Exercise the Pending response contract independently of native scheduling. The same
    // registered original Row funds it; constructing an unpaid response is impossible.
    let pending = SetupReply::new(
        Status::Pending,
        owner.clone(),
        EXECUTOR.into(),
        host.port().unwrap(),
        projection,
    )
    .unwrap();
    assert!(
        setup_inner(&f.state, &host, owner.clone(), EXECUTOR.into(), &mut rng)
            .await
            .is_err()
    );
    let json = pending.json().unwrap();
    assert_eq!(json.capacity(), PUBLIC_REPLY_BYTES);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap()["kind"],
        "pending"
    );
    drop(pending);
    drop(
        setup_inner(&f.state, &host, owner.clone(), EXECUTOR.into(), &mut rng)
            .await
            .unwrap(),
    );
    f.state.voice_authority.lock().unwrap().end(&owner);
    collect_inner(&f.state, &owner, EXECUTOR).await.unwrap();
    assert!(
        f.state.voice_channels.project(&owner, [9; 32]).is_err(),
        "collected history cannot create unpaid metadata"
    );
    host.finish().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn cleanup_remains_available_after_end_lock_and_listener_shutdown() {
    let f = Fixture::new().await;
    let owner = selected(&f).await;
    let host = host(&f.state);
    drop(
        setup_inner(
            &f.state,
            &host,
            owner.clone(),
            EXECUTOR.into(),
            &mut Rng::new(),
        )
        .await
        .unwrap(),
    );
    crate::register_ui_lock_request(&f.state);
    f.state.voice_authority.lock().unwrap().revoke_all();
    host.finish().await.unwrap();
    settle_inner(&f.state, &owner, EXECUTOR).await.unwrap();
    collect_inner(&f.state, &owner, EXECUTOR).await.unwrap();
    collect_inner(&f.state, &owner, EXECUTOR).await.unwrap(); // Lost final reply.
    let mut rng = Rng::new();
    assert!(
        setup_inner(&f.state, &host, owner, EXECUTOR.into(), &mut rng)
            .await
            .is_err()
    );
    assert_eq!(rng.draws, 0);
    f.close().await;
}
