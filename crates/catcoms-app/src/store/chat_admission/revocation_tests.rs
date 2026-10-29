//! Storage fixtures below model trusted issuance, not a production Fresh proof constructor.
//! They exercise actual sealed files and original-owner revocation independently of native UI.
use super::*;
use std::future::Future;

#[test]
fn revoked_current_owner_closes_siblings_and_delayed_capture_before_fence_io() {
    let (_dir, store, mut rng) = mounted();
    let identity = key(41);
    let current = issue(&store, identity.clone(), &mut rng);
    let delayed = store.capture_chat_admission(identity.clone()).unwrap();
    let sibling = match store
        .resolve_chat_admission(&identity, current.nonce())
        .unwrap()
    {
        ChatAdmissionResolution::Current(binding) => binding,
        _ => panic!("Current fixture"),
    };
    let unregistered = sibling.publication_authority();
    let revoker = current.revoker();
    // The negative path has no Store/runtime reference and must finish while this writer is
    // held. It cannot wait for the very Actor/storage operation it is supposed to invalidate.
    let writer = store.chat_mount.runtime.lock().unwrap();
    revoker.revoke();
    assert!(!current.is_live());
    assert!(!sibling.is_live());
    assert!(unregistered.is_cancelled());
    drop(writer);
    assert!(store
        .prepare_fresh_chat_admission(&delayed, [41; 32], &mut rng)
        .is_err());
    assert!(store.capture_chat_admission(identity.clone()).is_err());
    assert!(store
        .resolve_chat_admission(&identity, current.nonce())
        .is_err());
    store.retire_chat_admission(&current, &mut rng).unwrap();
    let capture = store.capture_chat_admission(identity.clone()).unwrap();
    let next = store
        .prepare_fresh_chat_admission(&capture, [42; 32], &mut rng)
        .unwrap();
    let next = store
        .finish_chat_admission(&next, &mut rng, |store, rng| {
            store.save_server(41, b"new admission covering bytes", rng)
        })
        .unwrap()
        .into_binding();
    revoker.revoke();
    assert!(
        next.is_live(),
        "old handle never looks up the replacement logical owner"
    );
    assert!(matches!(
        store
            .resolve_chat_admission(&identity, current.nonce())
            .unwrap(),
        ChatAdmissionResolution::Retired
    ));
}

#[test]
fn initializing_negative_owner_refuses_finish_and_remains_durably_retirable() {
    let (dir, store, mut rng) = mounted();
    let identity = key(42);
    let capture = store.capture_chat_admission(identity.clone()).unwrap();
    let preparation = store
        .prepare_legacy_chat_admission(&capture, b"original source", &mut rng)
        .unwrap();
    preparation.revoker().revoke();
    assert!(store
        .finish_chat_admission(&preparation, &mut rng, |_, _| panic!(
            "revoked init cannot save Current"
        ))
        .is_err());
    assert!(store
        .prepare_legacy_chat_admission(&capture, b"original source", &mut rng)
        .is_err());
    assert!(store.capture_chat_admission(identity.clone()).is_err());
    store
        .retire_chat_preparation(&preparation, &mut rng)
        .unwrap();
    drop(store);
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    reopened
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    assert!(matches!(
        reopened
            .resolve_chat_admission(&identity, preparation.nonce())
            .unwrap(),
        ChatAdmissionResolution::Retired
    ));
}

#[test]
fn own_issued_pre_rename_debt_retries_same_candidate_but_cannot_skip_later_revocation() {
    let (_dir, store, mut rng) = mounted();
    let identity = key(43);
    let old = issue(&store, identity.clone(), &mut rng);
    let capture = store.capture_chat_admission(identity.clone()).unwrap();
    store
        .chat_fence_pre_rename_failure
        .store(true, Ordering::SeqCst);
    assert!(store
        .prepare_fresh_chat_admission(&capture, [43; 32], &mut rng)
        .is_err());
    assert!(
        !old.is_live(),
        "issuing N2 already revoked N1 before the real failed rename"
    );
    let issued = capture.issued.lock().unwrap().as_ref().unwrap().clone();
    let nonce = issued.nonce();
    store
        .chat_fence_pre_rename_failure
        .store(false, Ordering::SeqCst);
    let mut repeated = ChaCha20Rng::seed_from_u64(1);
    let retry = store
        .prepare_fresh_chat_admission(&capture, [43; 32], &mut repeated)
        .unwrap();
    assert_eq!(retry.nonce(), nonce);
    assert!(Arc::ptr_eq(&issued.owner_identity, &retry.owner_identity));
    retry.revoker().revoke();
    assert!(store
        .prepare_fresh_chat_admission(&capture, [43; 32], &mut rng)
        .is_err());
    assert!(store
        .finish_chat_admission(&retry, &mut rng, |_, _| panic!(
            "revoked own-issued candidate"
        ))
        .is_err());
    store.retire_chat_preparation(&retry, &mut rng).unwrap();
}

#[test]
fn simultaneous_same_capture_retries_share_one_nonce_and_one_live_owner() {
    // Wall-time watchdogs only bound a regressed test harness. Protocol ordering is proven by
    // the real writer/nonce entry hooks; no behavior is scheduled by an ambient sleep/clock.
    const PROBE_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(5);
    const GATE_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(20);
    struct GateRng {
        inner: ChaCha20Rng,
        entered: Option<std::sync::mpsc::SyncSender<bool>>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl rand_core::RngCore for GateRng {
        fn next_u32(&mut self) -> u32 {
            self.inner.next_u32()
        }
        fn next_u64(&mut self) -> u64 {
            self.inner.next_u64()
        }
        fn fill_bytes(&mut self, dest: &mut [u8]) {
            if let Some(entered) = self.entered.take() {
                let _ = entered.try_send(dest.len() == 32);
                self.release.recv_timeout(GATE_WATCHDOG).expect(
                    "test must release the original writer even when contention proof fails",
                );
            }
            self.inner.fill_bytes(dest);
        }
        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
            self.fill_bytes(dest);
            Ok(())
        }
    }
    impl rand_core::CryptoRng for GateRng {}
    let (_dir, store, _rng) = mounted();
    let identity = key(44);
    let capture = store.capture_chat_admission(identity).unwrap();
    let (a, b) = std::thread::scope(|scope| {
        let (entered, at_nonce) = std::sync::mpsc::sync_channel(1);
        let (release, continue_nonce) = std::sync::mpsc::sync_channel(1);
        let (contended, saw_contention) = std::sync::mpsc::sync_channel(1);
        let original = &store;
        let captured = &capture;
        let a = scope.spawn(move || {
            let mut rng = GateRng {
                inner: ChaCha20Rng::seed_from_u64(44),
                entered: Some(entered),
                release: continue_nonce,
            };
            original
                .prepare_legacy_chat_admission(captured, b"same source", &mut rng)
                .unwrap()
        });
        let entered = at_nonce.recv_timeout(PROBE_WATCHDOG);
        *store.chat_mount.before_writer.lock().unwrap() = Some(Box::new(move |writer| {
            let held = matches!(writer.try_lock(), Err(std::sync::TryLockError::WouldBlock));
            let _ = contended.try_send(held);
        }));
        let b = scope.spawn(|| {
            store
                .prepare_legacy_chat_admission(
                    &capture,
                    b"same source",
                    &mut ChaCha20Rng::seed_from_u64(45),
                )
                .unwrap()
        });
        let contention = saw_contention.recv_timeout(PROBE_WATCHDOG);
        // ALWAYS release A before asserting or joining B. If the hook regresses to AFTER
        // writer acquisition, the failed proof times out and teardown can still complete.
        let _ = release.try_send(());
        let a = a.join();
        let b = b.join();
        assert!(
            matches!(entered, Ok(true)),
            "first worker did not reach its real nonce draw"
        );
        assert!(
            matches!(contention, Ok(true)),
            "second worker did not prove writer contention before release"
        );
        (a.unwrap(), b.unwrap())
    });
    assert_eq!(a.nonce(), b.nonce());
    assert!(Arc::ptr_eq(&a.owner_identity, &b.owner_identity));
    assert!(!a.admission.is_cancelled() && !b.admission.is_cancelled());
}

#[tokio::test]
async fn weak_mount_and_admission_observers_wake_on_store_drop_without_retaining_session() {
    let (dir, store, mut rng) = mounted();
    let identity = key(45);
    let current = issue(&store, identity.clone(), &mut rng);
    let old_mount = store.chat_mount_revoker();
    let old_admission = current.revoker();
    let mut observer = current.publication_authority();
    let mut waiting = std::pin::pin!(observer.cancelled());
    std::future::poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(store);
    std::future::poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_ready());
        std::task::Poll::Ready(())
    })
    .await;
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    reopened
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    let new = match reopened
        .resolve_chat_admission(&identity, current.nonce())
        .unwrap()
    {
        ChatAdmissionResolution::Current(binding) => binding,
        _ => panic!("normal reopen preserves the durable admission"),
    };
    old_mount.revoke();
    old_admission.revoke();
    assert!(
        new.is_live(),
        "old weak handles cannot affect a new physical owner"
    );
    assert!(current.publication_authority().is_cancelled());
}

#[tokio::test]
async fn actual_store_reservation_survives_current_and_scope_retirement_reaches_other_instances() {
    use crate::Server;
    use catcoms_rt::{Hub, ManualClock, MeshTransport, PeerId};
    let (_dir, store, mut rng) = mounted();
    let hub = Hub::new();
    let mut original = Server::found(
        hub.join(PeerId::from_u64(45)),
        catcoms_mls::MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(45),
        Box::new(ManualClock::new(1_000)),
        "retained history",
    )
    .unwrap();
    let source = original.snapshot().unwrap();
    store.save_server(45, &source, &mut rng).unwrap();
    let identity = ChatAdmissionKey::new(
        &original.group_id(),
        original.sync.my_public_key().try_into().unwrap(),
    )
    .unwrap();
    let capture = store.capture_chat_admission(identity.clone()).unwrap();
    let preparation = store
        .prepare_legacy_chat_admission(&capture, &source, &mut rng)
        .unwrap();
    original
        .sync
        .reserve_managed_chat_scope(preparation.domain(), preparation.nonce())
        .unwrap();
    original
        .sync
        .attach_managed_chat_reservation(&preparation)
        .unwrap();
    assert!(original.sync.durable_send_context().is_err());
    original
        .sync
        .stage_managed_chat_scope(preparation.domain(), preparation.nonce())
        .unwrap();
    let snapshot = original.snapshot().unwrap();
    let binding = store
        .finish_chat_admission(&preparation, &mut rng, |store, rng| {
            store.save_server(45, &snapshot, rng)
        })
        .unwrap()
        .into_binding();
    assert!(Arc::ptr_eq(
        &preparation.owner_identity,
        &binding.owner_identity
    ));
    original.sync.bind_managed_chat_scope(&binding).unwrap();
    let mut sibling = Server::restore(
        &snapshot,
        hub.join(PeerId::from_u64(46)),
        ChaCha20Rng::seed_from_u64(46),
        Box::new(ManualClock::new(1_000)),
        "same original admission",
    )
    .unwrap();
    let sibling_binding = match store
        .resolve_chat_admission(&identity, binding.nonce())
        .unwrap()
    {
        ChatAdmissionResolution::Current(binding) => binding,
        _ => panic!("Current"),
    };
    sibling
        .sync
        .bind_managed_chat_scope(&sibling_binding)
        .unwrap();
    let held = sibling_binding.publication_authority();
    original.sync.retire_managed_chat_scope();
    assert!(held.is_cancelled());
    assert!(sibling.sync.durable_send_context().is_err());
    assert!(store
        .with_chat_admission_storage(&binding, |_| Ok(()))
        .is_err());
    let result = hub
        .join(PeerId::from_u64(47))
        .publish_once_authorized(
            catcoms_rt::Topic::new("held-original"),
            bytes::Bytes::from_static(b"original bytes"),
            held,
        )
        .await;
    assert_eq!(result, Err(catcoms_rt::PublishOnceError::Revoked));
    store.retire_chat_admission(&binding, &mut rng).unwrap();
    assert!(matches!(
        store
            .resolve_chat_admission(&identity, binding.nonce())
            .unwrap(),
        ChatAdmissionResolution::Retired
    ));
}
