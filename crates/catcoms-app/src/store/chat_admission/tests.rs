use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &[u8] = b"admission fixture secret";
fn mounted() -> (tempfile::TempDir, ServerStore, ChaCha20Rng) {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x00ad_1131);
    let store = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    store
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    (dir, store, rng)
}
fn key(n: u8) -> ChatAdmissionKey {
    ChatAdmissionKey::new(&[n], [n; 32]).unwrap()
}
// These storage-unit fixtures supply the already-trusted host save callback. The full Actor
// integration must separately prove that it actually saves the scope/MLS/history union.
fn issue(
    store: &ServerStore,
    key: ChatAdmissionKey,
    rng: &mut ChaCha20Rng,
) -> ChatAdmissionBinding {
    let capture = store.capture_chat_admission(key).unwrap();
    let prepared = store
        .prepare_legacy_chat_admission(&capture, b"exact legacy fixture", rng)
        .unwrap();
    store
        .finish_chat_admission(&prepared, rng, |_, _| Ok(()))
        .unwrap()
        .into_binding()
}

#[test]
fn captured_active_and_legacy_inputs_cannot_reopen_retired_admission() {
    let (dir, store, mut rng) = mounted();
    let identity = key(1);
    let active = issue(&store, identity.clone(), &mut rng);
    let nonce = active.nonce();
    let delayed_join = store.capture_chat_admission(identity.clone()).unwrap();
    store.retire_chat_admission(&active, &mut rng).unwrap();
    assert!(!active.is_live());
    assert!(store
        .with_chat_admission_storage(&active, |_| Ok(()))
        .is_err());
    assert!(matches!(
        store.resolve_chat_admission(&identity, nonce).unwrap(),
        ChatAdmissionResolution::Retired
    ));
    let after_retirement = store.capture_chat_admission(identity.clone()).unwrap();
    assert!(store
        .prepare_legacy_chat_admission(&after_retirement, b"exact legacy fixture", &mut rng)
        .is_err());
    assert!(
        store
            .prepare_fresh_chat_admission(&delayed_join, [11; 32], &mut rng)
            .is_err(),
        "a delayed pre-Leave completion cannot borrow the new lineage"
    );
    let next = store
        .prepare_fresh_chat_admission(&after_retirement, [12; 32], &mut rng)
        .unwrap();
    let rejoined = store
        .finish_chat_admission(&next, &mut rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    assert_ne!(rejoined.nonce(), nonce);
    assert!(matches!(
        store.resolve_chat_admission(&identity, nonce).unwrap(),
        ChatAdmissionResolution::Retired
    ));
    drop(store);
    assert!(active.publication_authority().is_cancelled());
    assert!(rejoined.publication_authority().is_cancelled());
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    reopened
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    assert!(matches!(
        reopened.resolve_chat_admission(&identity, nonce).unwrap(),
        ChatAdmissionResolution::Retired
    ));
    assert!(matches!(
        reopened
            .resolve_chat_admission(&identity, rejoined.nonce())
            .unwrap(),
        ChatAdmissionResolution::Current(_)
    ));
    let legacy = reopened.capture_chat_admission(identity).unwrap();
    assert!(reopened
        .prepare_legacy_chat_admission(&legacy, b"older tail-less capture", &mut rng)
        .is_err());
}

#[test]
fn original_mount_revocation_blocks_queued_publication_and_cross_store_negative_save() {
    let (dir, store, mut rng) = mounted();
    let binding = issue(&store, key(2), &mut rng);
    let queued = binding.publication_authority();
    store.revoke_chat_mount();
    assert!(queued.is_cancelled());
    assert!(store
        .with_original_chat_store::<()>(&binding, |_| panic!("revoked storage admission"))
        .is_err());
    drop(store);
    let replacement = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    replacement
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    assert!(replacement
        .with_original_chat_store::<()>(&binding, |_| panic!(
            "never write through a replacement mount"
        ))
        .is_err());
    assert!(
        queued.is_cancelled(),
        "same DEK reopen cannot revive an old observer"
    );
}

#[test]
fn repeated_nonce_rng_cannot_make_a_fresh_admission_equal_the_retired_one() {
    let (_dir, store, mut maintenance_rng) = mounted();
    let identity = key(9);
    let capture = store.capture_chat_admission(identity.clone()).unwrap();
    let mut original_nonce_rng = ChaCha20Rng::seed_from_u64(0x009e_0ea7);
    let mut repeated_nonce_rng = original_nonce_rng.clone();
    let first = store
        .prepare_legacy_chat_admission(&capture, b"first", &mut original_nonce_rng)
        .unwrap();
    let old = store
        .finish_chat_admission(&first, &mut maintenance_rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    store
        .retire_chat_admission(&old, &mut maintenance_rng)
        .unwrap();
    let capture = store.capture_chat_admission(identity.clone()).unwrap();
    // Both issuance calls read exactly the same first random bytes. Without the persisted
    // revision in identity derivation, this would repeat N1 and resolve its old capture as N2.
    let next = store
        .prepare_fresh_chat_admission(&capture, [16; 32], &mut repeated_nonce_rng)
        .unwrap();
    let fresh = store
        .finish_chat_admission(&next, &mut maintenance_rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    assert_ne!(old.nonce(), fresh.nonce());
    assert!(!old.is_live());
    assert!(matches!(
        store
            .resolve_chat_admission(&identity, old.nonce())
            .unwrap(),
        ChatAdmissionResolution::Retired
    ));
}

#[test]
fn retirement_keeps_exact_negative_candidate_before_and_after_actual_rename() {
    for after_rename in [false, true] {
        let (dir, store, mut rng) = mounted();
        let identity = key(3);
        let binding = issue(&store, identity.clone(), &mut rng);
        if after_rename {
            store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
        } else {
            store
                .chat_fence_pre_rename_failure
                .store(true, Ordering::SeqCst);
        }
        let failed = store.retire_chat_admission(&binding, &mut rng).unwrap_err();
        assert_eq!(
            matches!(failed, AppError::CommittedButNotDurable(_)),
            after_rename
        );
        assert!(!binding.is_live());
        assert!(store
            .with_chat_admission_storage::<()>(&binding, |_| panic!(
                "failed retirement is irreversible in process"
            ))
            .is_err());
        assert!(
            store
                .resolve_chat_admission(&identity, binding.nonce())
                .is_err(),
            "hidden dirty debt cannot mint a binding"
        );
        store.repair_chat_admission_inventory(&mut rng).unwrap();
        assert!(matches!(
            store
                .resolve_chat_admission(&identity, binding.nonce())
                .unwrap(),
            ChatAdmissionResolution::Retired
        ));
        drop(store);
        let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
        reopened
            .enable_chat_scope_management(SECRET, &mut rng)
            .unwrap();
        assert!(matches!(
            reopened
                .resolve_chat_admission(&identity, binding.nonce())
                .unwrap(),
            ChatAdmissionResolution::Retired
        ));
    }
}

#[test]
fn initializing_scope_waits_for_real_covering_server_save_and_reuses_its_nonce() {
    let (_dir, store, mut rng) = mounted();
    let identity = key(4);
    let capture = store.capture_chat_admission(identity.clone()).unwrap();
    let prepared = store
        .prepare_legacy_chat_admission(&capture, b"initial source", &mut rng)
        .unwrap();
    let nonce = prepared.nonce();
    store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
    let failed = store.finish_chat_admission(&prepared, &mut rng, |store, rng| {
        store.save_server(77, b"exact possible scope candidate", rng)
    });
    assert!(matches!(failed, Err(AppError::CommittedButNotDurable(_))));
    assert_eq!(
        store.load_server(77).unwrap().as_slice(),
        b"exact possible scope candidate"
    );
    assert!(matches!(
        store.resolve_chat_admission(&identity, nonce).unwrap(),
        ChatAdmissionResolution::Initializing(_)
    ));
    let replay = store
        .prepare_legacy_chat_admission(&capture, b"initial source", &mut rng)
        .unwrap();
    assert_eq!(replay.nonce(), nonce);
    let bound = store
        .finish_chat_admission(&replay, &mut rng, |store, rng| {
            store.save_server(77, b"exact possible scope candidate", rng)
        })
        .unwrap();
    let bound = bound.into_binding();
    assert_eq!(bound.nonce(), nonce);
    assert!(bound.is_live());
}

#[test]
fn full_inventory_refuses_new_identity_but_reserved_retirement_still_fits() {
    let (_dir, store, mut rng) = mounted();
    let bound = issue(&store, key(5), &mut rng);
    {
        let mut runtime = store.chat_mount.locked().unwrap();
        let table = runtime.table.as_mut().unwrap();
        for n in 1..MAX_CHAT_ADMISSION_IDENTITIES {
            table.insert(
                ChatAdmissionKey::new(&(n as u64).to_be_bytes(), [9; 32]).unwrap(),
                Record {
                    nonce: [9; 32],
                    revision: 1,
                    phase: Phase::Retired,
                    origin: Origin::Legacy,
                    provenance: [8; 32],
                },
            );
        }
        assert_eq!(table.len(), MAX_CHAT_ADMISSION_IDENTITIES);
        store.flush_chat_admissions(&mut runtime, &mut rng).unwrap();
    }
    let before = fs::metadata(store.dir.join(FILE)).unwrap().len();
    let fresh = store.capture_chat_admission(key(6)).unwrap();
    assert!(store
        .prepare_legacy_chat_admission(&fresh, b"new", &mut rng)
        .is_err());
    store.retire_chat_admission(&bound, &mut rng).unwrap();
    assert_eq!(
        fs::metadata(store.dir.join(FILE)).unwrap().len(),
        before,
        "retirement consumes its pre-reserved fixed-size state"
    );
}

#[test]
fn managed_missing_inventory_and_nonempty_legacy_staging_refuse_activation() {
    let (dir, store, mut rng) = mounted();
    drop(store);
    fs::remove_file(dir.path().join(FILE)).unwrap();
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    assert!(reopened
        .enable_chat_scope_management(SECRET, &mut rng)
        .is_err());

    let dir = tempfile::tempdir().unwrap();
    let store = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    let old_wrapper = fs::read(dir.path().join("vault.bin")).unwrap();
    store
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    let _binding = issue(&store, key(7), &mut rng);
    drop(store);
    fs::write(dir.path().join("vault.bin"), old_wrapper).unwrap();
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    assert!(
        reopened
            .enable_chat_scope_management(SECRET, &mut rng)
            .is_err(),
        "legacy activation cannot clear previously managed nonempty lineage"
    );
}

#[tokio::test]
async fn duplicate_current_completion_preserves_newer_real_messages_and_saved_stop() {
    use crate::durable_chat::{DurableChatRequest, DurableSendRequest};
    use catcoms_rt::{Hub, ManualClock, PeerId};
    use catcoms_sync::durable_chat::ChatSaveError;
    let (dir, store, mut rng) = mounted();
    let mut server = crate::Server::found(
        Hub::new().join(PeerId::from_u64(807)),
        catcoms_mls::MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(807),
        Box::new(ManualClock::new(1_000)),
        "scope fixture",
    )
    .unwrap();
    server.open_channel(7).await.unwrap();
    let initial = server.snapshot().unwrap();
    let identity = ChatAdmissionKey::new(
        &server.group_id(),
        server.sync.my_public_key().try_into().unwrap(),
    )
    .unwrap();
    let capture = store.capture_chat_admission(identity).unwrap();
    let prepared = store
        .prepare_fresh_chat_admission(&capture, [13; 32], &mut rng)
        .unwrap();
    let bound = store
        .finish_chat_admission(&prepared, &mut rng, |store, rng| {
            store.save_server(77, &initial, rng)
        })
        .unwrap()
        .into_binding();
    server
        .send_reply(7, "newer retained message", "")
        .await
        .unwrap();
    let stopped = DurableChatRequest::Send(DurableSendRequest {
        token: [14; 16],
        expected_context: server.sync.durable_send_context().unwrap(),
        channel: 7,
        text: "never authored".into(),
        reply_to: String::new(),
    })
    .retired_identity();
    server
        .sync
        .stop_durable_chat(
            stopped.token,
            stopped.binding,
            stopped.context,
            stopped.channel,
            stopped.message_id.clone(),
            stopped.action,
            stopped.target,
            |bytes, rng| {
                store
                    .save_server(77, bytes, rng)
                    .map_err(|error| match error {
                        AppError::CommittedButNotDurable(reason) => {
                            ChatSaveError::ReplacedNotDurable(reason)
                        }
                        other => ChatSaveError::NotReplaced(other.to_string()),
                    })
            },
        )
        .unwrap();
    let latest = store.load_server(77).unwrap();
    assert!(matches!(
        store
            .finish_chat_admission(&prepared, &mut rng, |_, _| panic!(
                "duplicate completion must never overwrite the newer graph/Stop"
            ))
            .unwrap(),
        ChatAdmissionFinish::AlreadyCurrent(_)
    ));
    let replay = store
        .prepare_fresh_chat_admission(&capture, [13; 32], &mut rng)
        .unwrap();
    assert_eq!(replay.nonce(), bound.nonce());
    assert!(
        bound.is_live(),
        "same-provenance replay must not replace the current owner"
    );
    assert_eq!(store.load_server(77).unwrap().as_slice(), latest.as_slice());
    drop(store);
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    reopened
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    let restored = crate::Server::restore(
        &reopened.load_server(77).unwrap(),
        Hub::new().join(PeerId::from_u64(808)),
        ChaCha20Rng::seed_from_u64(808),
        Box::new(ManualClock::new(2_000)),
        "scope fixture",
    )
    .unwrap();
    assert_eq!(restored.messages(7).len(), 1);
    let terminal = restored
        .sync
        .durable_chat_lookup(
            stopped.token,
            stopped.binding,
            stopped.context,
            stopped.channel,
        )
        .unwrap()
        .unwrap();
    assert!(terminal.durable);
    assert_eq!(
        terminal.disposition,
        catcoms_sync::durable_chat::ChatDisposition::StoppedBeforeCommit
    );
}

#[test]
fn old_nonce_negative_repair_cannot_replace_a_new_admissions_saved_server() {
    let (dir, store, mut rng) = mounted();
    let identity = key(8);
    let old = issue(&store, identity.clone(), &mut rng);
    store.retire_chat_admission(&old, &mut rng).unwrap();
    assert!(
        store.with_original_chat_store(&old, |_| Ok(())).is_ok(),
        "same-nonce retired repair remains allowed"
    );
    let capture = store.capture_chat_admission(identity).unwrap();
    let next = store
        .prepare_fresh_chat_admission(&capture, [15; 32], &mut rng)
        .unwrap();
    let current = store
        .finish_chat_admission(&next, &mut rng, |store, rng| {
            store.save_server(88, b"new admission's exact covering server bytes", rng)
        })
        .unwrap()
        .into_binding();
    assert!(store
        .with_original_chat_store::<()>(&old, |_| panic!(
            "old admission cannot replace the new slot"
        ))
        .is_err());
    assert!(current.is_live());
    drop(store);
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    reopened
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    assert_eq!(
        reopened.load_server(88).unwrap().as_slice(),
        b"new admission's exact covering server bytes"
    );
}
