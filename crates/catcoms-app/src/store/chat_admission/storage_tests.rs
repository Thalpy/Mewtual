//! These fixtures exercise the real sealed Store path. They do not stand in for the host's
//! still-required exact Actor incarnation, canonical owner-head CAS or covering scope snapshot.
use super::*;

#[test]
fn held_initializing_and_current_permits_preserve_original_source_and_latest_sealed_bytes() {
    let (dir, store, mut rng) = mounted();
    let (_other_dir, other, _) = mounted();
    let identity = key(51);
    let capture = store.capture_chat_admission(identity.clone()).unwrap();
    let prepared = store
        .prepare_legacy_chat_admission(&capture, b"original legacy snapshot", &mut rng)
        .unwrap();
    let finished = store
        .finish_chat_admission_with_storage(&prepared, &mut rng, |permit, rng| {
            assert_eq!(format!("{permit:?}"), "ChatAdmissionStoragePermit { .. }");
            assert_eq!(permit.mode(), ChatAdmissionStorageMode::Initializing);
            assert_eq!(permit.domain(), prepared.domain());
            assert_eq!(permit.nonce(), prepared.nonce());
            assert_eq!(permit.revision(), prepared.record.revision);
            assert!(Arc::ptr_eq(
                permit.owner_identity(),
                &prepared.owner_identity
            ));
            assert!(!permit.observer().is_cancelled());
            assert!(store.chat_mount.runtime.try_lock().is_err());
            assert!(permit
                .validate_original(&other, identity.group(), identity.signer())
                .is_err());
            assert!(permit
                .validate_original(&store, b"another group", identity.signer())
                .is_err());
            assert!(permit
                .validate_original(&store, identity.group(), &[52; 32])
                .is_err());
            permit.validate_original(&store, identity.group(), identity.signer())?;
            permit
                .store()
                .save_server(51, b"private initialization snapshot", rng)
        })
        .unwrap();
    let binding = finished.into_binding();
    assert!(Arc::ptr_eq(
        &prepared.owner_identity,
        &binding.owner_identity
    ));
    store
        .with_chat_admission_storage_permit(&binding, |permit| {
            assert_eq!(permit.mode(), ChatAdmissionStorageMode::Current);
            assert_eq!(permit.revision(), binding.revision);
            assert!(Arc::ptr_eq(
                permit.owner_identity(),
                &binding.owner_identity
            ));
            permit.validate_original(&store, identity.group(), identity.signer())?;
            permit
                .store()
                .save_server(51, b"newer current head", &mut rng)
        })
        .unwrap();
    assert!(matches!(
        store
            .finish_chat_admission_with_storage(&prepared, &mut rng, |_, _| panic!(
                "AlreadyCurrent must never rerun the old initialization writer"
            ))
            .unwrap(),
        ChatAdmissionFinish::AlreadyCurrent(_)
    ));
    drop(store);
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    assert_eq!(
        reopened.load_server(51).unwrap().as_slice(),
        b"newer current head"
    );
}

#[test]
fn subordinate_writer_wait_rechecks_original_revocation_before_real_storage_handoff() {
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let (dir, store, mut rng) = mounted();
    let identity = key(52);
    let binding = issue(&store, identity.clone(), &mut rng);
    store
        .save_server(52, b"original accepted head", &mut rng)
        .unwrap();
    let head = Mutex::new(());
    let held_head = head.lock().unwrap();
    let (entered_tx, entered_rx) = sync_channel(1);
    let mut worker_rng = rng.clone();
    let result = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            store.with_chat_admission_storage_permit(&binding, |permit| {
                entered_tx.try_send(()).unwrap();
                let _head = head.lock().unwrap();
                // The original permit was admitted before waiting for the subordinate writer.
                // Only this final read can refuse an intervening negative-only revocation.
                permit.validate_original(&store, identity.group(), identity.signer())?;
                permit
                    .store()
                    .save_server(52, b"invalid delayed replacement", &mut worker_rng)
            })
        });
        let entered = entered_rx.recv_timeout(Duration::from_secs(5));
        binding.revoker().revoke();
        // Always release before joining or asserting: a regressed callback must fail promptly,
        // rather than deadlock this fixture while holding the subordinate writer.
        drop(held_head);
        let result = worker.join();
        assert!(entered.is_ok(), "worker did not enter the held admission");
        result.unwrap()
    });
    assert!(result.is_err());
    assert_eq!(
        store.load_server(52).unwrap().as_slice(),
        b"original accepted head"
    );
    store.retire_chat_admission(&binding, &mut rng).unwrap();
    drop(store);
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    assert_eq!(
        reopened.load_server(52).unwrap().as_slice(),
        b"original accepted head"
    );
}

#[test]
fn revoked_private_initialization_cannot_promote_its_saved_bytes_to_current() {
    let (dir, store, mut rng) = mounted();
    let identity = key(53);
    let capture = store.capture_chat_admission(identity.clone()).unwrap();
    let prepared = store
        .prepare_legacy_chat_admission(&capture, b"original legacy snapshot", &mut rng)
        .unwrap();
    let result = store.finish_chat_admission_with_storage(&prepared, &mut rng, |permit, rng| {
        permit.validate_original(&store, identity.group(), identity.signer())?;
        permit
            .store()
            .save_server(53, b"already admitted private covering bytes", rng)?;
        // The admitted synchronous write can finish, but Current/publication cannot be exposed.
        prepared.revoker().revoke();
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(
        store.load_server(53).unwrap().as_slice(),
        b"already admitted private covering bytes"
    );
    assert!(store
        .resolve_chat_admission(&identity, prepared.nonce())
        .is_err());
    store.retire_chat_preparation(&prepared, &mut rng).unwrap();
    drop(store);
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    reopened
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    assert!(matches!(
        reopened
            .resolve_chat_admission(&identity, prepared.nonce())
            .unwrap(),
        ChatAdmissionResolution::Retired
    ));
}
