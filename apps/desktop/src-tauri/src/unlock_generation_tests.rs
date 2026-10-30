//! Deterministic tests of the RPC's actual shared authentication path. No Wry window is built.
//! Cold refusal uses the production admission future; the controlled cold continuation below
//! exercises real Store open/finalization, not AppHandle routing/reload/sweep behavior.
use super::*;
use libp2p::futures::FutureExt;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &str = "correct horse";
fn open_store(path: &Path, seed: u64) -> ServerStore {
    ServerStore::open(
        path,
        SECRET.as_bytes(),
        &mut ChaCha20Rng::seed_from_u64(seed),
    )
    .unwrap()
}
fn is_session_refusal(error: &AppError) {
    assert_eq!(
        serde_json::to_value(error).unwrap()["code"],
        "SESSION.LOCKED"
    );
}

#[test]
fn unlock_generation_accepts_only_bounded_canonical_u64_text() {
    assert_eq!(parse_unlock_generation("0"), Ok(0));
    assert_eq!(
        parse_unlock_generation("18446744073709551615"),
        Ok(u64::MAX)
    );
    for bad in [
        "",
        "00",
        "01",
        "+1",
        "-1",
        " 1",
        "1 ",
        "1.0",
        "1e0",
        "?",
        "18446744073709551616",
        "000000000000000000000",
    ] {
        assert!(parse_unlock_generation(bad).is_err(), "accepted {bad:?}");
    }
    assert!(parse_unlock_generation(&"9".repeat(1024 * 1024)).is_err());
}

#[tokio::test]
async fn native_lock_outcome_returns_its_registered_generation_even_after_newer_lock_and_error() {
    let state = AppState::default();
    let commit = state.ui_session_commit.lock().await;
    let mut first = Box::pin(lock_session_outcome_inner(&state, None));
    assert!(first.as_mut().now_or_never().is_none());
    let first_registered = state.ui_session_generation.load(Ordering::Acquire);
    let mut second = Box::pin(lock_session_outcome_inner(&state, Some("not json".into())));
    assert!(second.as_mut().now_or_never().is_none());
    let second_registered = state.ui_session_generation.load(Ordering::Acquire);
    assert_ne!(first_registered, second_registered);
    drop(commit);
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.generation, first_registered.to_string());
    assert_eq!(second.generation, second_registered.to_string());
    // Whichever lock consumes the malformed pending bytes, this exact registered token remains
    // the response's correlation. A continuity failure is not an unregistered/failed lock.
    assert!(first.continuity_error.is_some() || second.continuity_error.is_some());
    let json = serde_json::to_value(second).unwrap();
    assert!(json["generation"].is_string());
    assert!(require_unlocked_session(&state).await.is_err());
}

#[tokio::test]
async fn entirely_unpolled_cold_unlock_cannot_adopt_a_newer_completed_lock() {
    let root = tempfile::tempdir().unwrap();
    // A genuine cold vault with a valid secret exists. No mounted-store helper or fake success
    // substitutes for cold admission; the stale future must refuse before any cold work/KDF.
    drop(open_store(root.path(), 601));
    let state = AppState::default();
    let original = lock_session_outcome_inner(&state, None).await;
    let old = unlock_authentication(
        &state,
        SECRET.into(),
        original.generation,
        None,
        |_, _, _, _| async { panic!("stale cold command reached its cold continuation") },
    );
    // Intentionally do not poll `old` before the later lock is COMPLETELY committed.
    let newer = lock_session_outcome_inner(&state, None).await;
    let mount = state.vault_mount.lock().await;
    let error = old
        .now_or_never()
        .expect("first poll must refuse before awaiting mount/KDF")
        .err()
        .expect("authentication must refuse");
    is_session_refusal(&error);
    assert_eq!(
        state
            .ui_session_generation
            .load(Ordering::Acquire)
            .to_string(),
        newer.generation
    );
    assert!(state.store.lock().await.is_none());
    assert!(require_unlocked_session(&state).await.is_err());
    drop(mount);
    drop(open_store(root.path(), 602)); // the original actual cold vault/secret stays usable.
}

#[tokio::test]
async fn entirely_unpolled_mounted_unlock_cannot_adopt_a_newer_completed_lock() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::default();
    *state.store.lock().await = Some(open_store(root.path(), 603));
    let original = lock_session_outcome_inner(&state, None).await;
    let old = unlock_authentication(
        &state,
        SECRET.into(),
        original.generation,
        None,
        |_, _, _, _| async { panic!("mounted stale command cannot use a cold fallback") },
    );
    // The actual shared RPC body has not run yet. Capturing current inside it would be wrong.
    let newer = lock_session_outcome_inner(&state, None).await;
    let mount = state.vault_mount.lock().await;
    let error = old
        .now_or_never()
        .expect("first poll must refuse before awaiting mount/KDF")
        .err()
        .expect("authentication must refuse");
    is_session_refusal(&error);
    assert!(state.store.lock().await.is_some());
    assert_eq!(
        state
            .ui_session_generation
            .load(Ordering::Acquire)
            .to_string(),
        newer.generation
    );
    assert!(require_unlocked_session(&state).await.is_err());
    drop(mount);
}

#[tokio::test]
async fn malformed_generation_refuses_in_actual_authentication_before_await_or_kdf() {
    let state = AppState::default();
    let mount = state.vault_mount.lock().await;
    for bad in ["", "00", "+1", "18446744073709551616"] {
        let result = unlock_authentication(
            &state,
            SECRET.into(),
            bad.into(),
            None,
            |_, _, _, _| async { panic!("malformed token reached cold work") },
        )
        .now_or_never()
        .expect("malformed input must not await mount/authentication");
        is_session_refusal(&result.err().expect("authentication must refuse"));
    }
    drop(mount);
}

#[tokio::test]
async fn actual_mounted_authentication_keeps_secret_checks_and_uses_the_exact_barrier() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::default();
    *state.store.lock().await = Some(open_store(root.path(), 604));
    let barrier = lock_session_outcome_inner(&state, None).await;
    let error = unlock_authentication(
        &state,
        "wrong horse".into(),
        barrier.generation.clone(),
        None,
        |_, _, _, _| async {
            panic!("mounted authentication cannot fall through after wrong secret")
        },
    )
    .await
    .err()
    .expect("authentication must refuse");
    assert_eq!(
        serde_json::to_value(error).unwrap()["code"],
        "VAULT.OPEN.REFUSED"
    );
    assert!(require_unlocked_session(&state).await.is_err());
    let servers = unlock_authentication(
        &state,
        SECRET.into(),
        barrier.generation.clone(),
        None,
        |_, _, _, _| async { panic!("real mounted vault must authenticate without cold fallback") },
    )
    .await
    .unwrap();
    assert!(servers.is_empty());
    assert!(require_unlocked_session(&state).await.is_ok());
    assert_eq!(
        state
            .ui_authorized_generation
            .load(Ordering::Acquire)
            .to_string(),
        barrier.generation
    );
}

#[tokio::test]
async fn mounted_unlock_polled_before_lock_still_refuses_at_original_finalization() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::default();
    *state.store.lock().await = Some(open_store(root.path(), 605));
    let barrier = lock_session_outcome_inner(&state, None).await;
    let mount = state.vault_mount.lock().await;
    let mut old = Box::pin(unlock_authentication(
        &state,
        SECRET.into(),
        barrier.generation,
        None,
        |_, _, _, _| async { panic!("mounted authentication cannot use cold continuation") },
    ));
    assert!(
        old.as_mut().now_or_never().is_none(),
        "original gate passes then waits on real mount"
    );
    lock_session_outcome_inner(&state, None).await;
    drop(mount);
    let error = old.await.err().expect("authentication must refuse");
    is_session_refusal(&error);
    assert!(require_unlocked_session(&state).await.is_err());
}

#[tokio::test]
async fn actual_cold_store_continuation_retains_original_generation_through_finalization() {
    let root = tempfile::tempdir().unwrap();
    drop(open_store(root.path(), 606));
    let state = AppState::default();
    let barrier = lock_session_outcome_inner(&state, None).await;
    let entered = AtomicBool::new(false);
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let cold_state = &state;
    let cold_root = root.path();
    let cold_entered = &entered;
    let mut old = Box::pin(unlock_authentication(
        &state,
        SECRET.into(),
        barrier.generation,
        None,
        |generation, ticket, op, passphrase| async move {
            cold_entered.store(true, Ordering::Release);
            released.await.unwrap();
            let store = ServerStore::open(
                cold_root,
                passphrase.as_bytes(),
                &mut ChaCha20Rng::seed_from_u64(607),
            )
            .map_err(|error| op.fail(codes::VAULT_LOCKED_OUT, error.to_string()))?;
            *cold_state.store.lock().await = Some(store);
            finalize_unlock_session_with_voice(cold_state, generation, false, &ticket)
                .await
                .map_err(|error| op.fail(codes::SESSION_LOCKED, error))?;
            Ok(Vec::new())
        },
    ));
    assert!(old.as_mut().now_or_never().is_none());
    assert!(
        entered.load(Ordering::Acquire),
        "the production helper admitted its cold path under mount custody"
    );
    assert!(
        state.vault_mount.try_lock().is_err(),
        "same mount remains held through cold work"
    );
    lock_session_outcome_inner(&state, None).await;
    release.send(()).unwrap();
    is_session_refusal(&old.await.err().expect("authentication must refuse"));
    assert!(
        state.store.lock().await.is_some(),
        "real cold vault opened, but stale work never authorizes IPC"
    );
    assert!(require_unlocked_session(&state).await.is_err());
}
