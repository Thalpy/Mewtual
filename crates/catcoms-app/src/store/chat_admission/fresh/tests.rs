//! Factory/capture regressions, alongside the separately required actual canonical owner-save
//! vertical. Storage-only records below make no historical membership/owner-authority claim.
use super::*;
use catcoms_rt::{Hub, ManualClock};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &[u8] = b"private fresh factory fixture";

fn managed() -> (tempfile::TempDir, ServerStore, ChaCha20Rng) {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xf2e5_0001);
    let store = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    store
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    (dir, store, rng)
}

fn founder(
    store: &ServerStore,
    peer: u64,
) -> PrivateManagedServer<catcoms_rt::MemNetwork, ChaCha20Rng> {
    store
        .private_managed_founder(
            Hub::new().join(PeerId::from_u64(peer)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(peer),
            Box::new(ManualClock::new(1_000)),
            "private owner initialization",
        )
        .unwrap()
}

#[test]
fn private_factory_refuses_unmanaged_store_and_restored_candidate_substitution() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xf2e5_0002);
    let store = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    let result = store.private_managed_founder(
        Hub::new().join(PeerId::from_u64(802)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(802),
        Box::new(ManualClock::new(1_000)),
        "unmanaged",
    );
    assert!(matches!(result, Err(AppError::Invalid(_))));
    store
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    let mut private = founder(&store, 803);
    let original = private
        .server_mut_for_owner_initialization()
        .unwrap()
        .snapshot()
        .unwrap();
    let restored = Server::restore(
        &original,
        Hub::new().join(PeerId::from_u64(804)),
        ChaCha20Rng::seed_from_u64(804),
        Box::new(ManualClock::new(1_000)),
        "same bytes without actual source",
    )
    .unwrap();
    assert_eq!(restored.group_id(), private.server().unwrap().group_id());
    *private.server_mut_for_owner_initialization().unwrap() = restored;
    assert!(private.reserve_scope(&store, &mut rng).is_err());
    assert!(store
        .chat_mount
        .locked()
        .unwrap()
        .table
        .as_ref()
        .unwrap()
        .get(&private.capture.key)
        .is_none());
}

#[test]
fn private_fresh_candidate_preserves_actual_source_and_exact_issued_retry_across_storage_faults() {
    for after_rename in [false, true] {
        let (dir, store, mut rng) = managed();
        let mut private = founder(&store, 805);
        let group = private.server().unwrap().group_id();
        let attempt = private.attempt;
        if after_rename {
            store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
        } else {
            store
                .chat_fence_pre_rename_failure
                .store(true, Ordering::SeqCst);
        }
        let failed = private.reserve_scope(&store, &mut rng).unwrap_err();
        assert_eq!(
            matches!(failed, AppError::CommittedButNotDurable(_)),
            after_rename,
            "only the actual post-rename failure is ambiguous"
        );
        let issued = private
            .capture
            .issued
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        let resumed = private.reserve_scope(&store, &mut rng).unwrap();
        assert_eq!(resumed.nonce(), issued.nonce());
        assert!(Arc::ptr_eq(&resumed.owner_identity, &issued.owner_identity));
        assert_eq!(private.server().unwrap().group_id(), group);
        assert_eq!(
            private
                .server()
                .unwrap()
                .sync
                .fresh_chat_admission()
                .unwrap()
                .attempt_id(),
            attempt
        );
        assert!(private
            .server()
            .unwrap()
            .sync
            .chat_scope_label()
            .unwrap()
            .initializing());
        assert!(private
            .server()
            .unwrap()
            .sync
            .durable_send_context()
            .is_err());
        // The retry crossed the real inventory write and parent barrier. A new mounted Store
        // resolves the SAME Initializing nonce; it cannot bootstrap a replacement admission.
        let identity = private.capture.key.clone();
        drop(private);
        drop(store);
        let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
        let ChatAdmissionResolution::Initializing(restored) = reopened
            .resolve_chat_admission(&identity, resumed.nonce())
            .unwrap()
        else {
            panic!("only the retained reservation was persisted");
        };
        assert_eq!(restored.nonce(), resumed.nonce());
    }
}

#[tokio::test]
async fn actual_direct_join_keeps_exact_fresh_completion_private_until_the_covering_save() {
    let (_dir, store, mut rng) = managed();
    let hub = Hub::new();
    let owner_peer = PeerId::from_u64(809);
    let mut owner = Server::found(
        hub.join(owner_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(809),
        Box::new(ManualClock::new(1_000)),
        "actual join authority fixture",
    )
    .unwrap();
    owner.subscribe_control().await.unwrap();
    let profile = owner.sync.prepare_owner_evidence_checkpoint(1).unwrap();
    let expected_profile = store.capture_trusted_owner_profile(&profile).unwrap();
    let invite = owner.mint_invite([9; 16], 10_000, vec![]).unwrap();
    let device = MlsDevice::generate().unwrap();
    let full_signer = device.public_key_bytes();
    let attempt = store
        .private_managed_join_attempt(device, invite, expected_profile)
        .unwrap();
    let (joined, tick) = tokio::join!(
        attempt.finish_direct(
            hub.join(PeerId::from_u64(810)),
            ChaCha20Rng::seed_from_u64(810),
            Box::new(ManualClock::new(1_000)),
            "never registered joined member",
            owner_peer,
        ),
        owner.sync_once(),
    );
    tick.unwrap();
    let mut joined = joined.unwrap();
    assert_eq!(
        joined.captured_profile().unwrap().profile(),
        profile.profile_id(),
        "the root was captured before the actual Welcome, never selected from it"
    );
    let source = joined
        .server()
        .unwrap()
        .sync
        .fresh_chat_admission()
        .unwrap();
    assert_eq!(source.group(), owner.group_id());
    assert_eq!(source.signer().as_slice(), full_signer);
    assert!(source.key_package_reference().is_some());
    assert!(source.authenticated_welcome_hash().is_some());
    assert!(joined
        .server()
        .unwrap()
        .sync
        .durable_send_context()
        .is_err());
    let preparation = joined.reserve_scope(&store, &mut rng).unwrap();
    assert!(joined
        .server()
        .unwrap()
        .sync
        .chat_scope_label()
        .unwrap()
        .initializing());
    assert!(joined
        .server()
        .unwrap()
        .sync
        .durable_send_context()
        .is_err());

    // The local reservation is negative custody, not an owner-certified historical Add.
    // Retiring it fences both this completed join and any earlier pre-await capture. No
    // covering callback is run and no runnable Server has ever escaped this wrapper.
    store
        .retire_chat_preparation(&preparation, &mut rng)
        .unwrap();
    assert!(joined.reserve_scope(&store, &mut rng).is_err());
    assert!(joined
        .server()
        .unwrap()
        .sync
        .durable_send_context()
        .is_err());
}

#[tokio::test]
async fn actual_same_key_remove_and_fresh_welcome_cannot_reuse_the_old_private_scope() {
    let (_dir, store, mut rng) = managed();
    let hub = Hub::new();
    let owner_peer = PeerId::from_u64(811);
    let mut owner = Server::found(
        hub.join(owner_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(811),
        Box::new(ManualClock::new(1_000)),
        "actual remove/rejoin authority fixture",
    )
    .unwrap();
    owner.subscribe_control().await.unwrap();
    let profile = owner.sync.prepare_owner_evidence_checkpoint(1).unwrap();
    let first_device = MlsDevice::generate().unwrap();
    // Capture genuine fresh provider custody before A1 stores this GroupId. A2 retains the
    // same full signing key but gets its OWN freshly minted KP and authenticated Welcome.
    let rejoining_device = first_device.duplicate().unwrap();
    let full_signer = first_device.public_key_bytes();
    let invite = owner.mint_invite([11; 16], 10_000, vec![]).unwrap();
    let first_attempt = store
        .private_managed_join_attempt(
            first_device,
            invite,
            store.capture_trusted_owner_profile(&profile).unwrap(),
        )
        .unwrap();
    let (first, tick) = tokio::join!(
        first_attempt.finish_direct(
            hub.join(PeerId::from_u64(812)),
            ChaCha20Rng::seed_from_u64(812),
            Box::new(ManualClock::new(1_000)),
            "A1 remains private",
            owner_peer,
        ),
        owner.sync_once(),
    );
    tick.unwrap();
    let mut first = first.unwrap();
    let first_kp = first
        .server()
        .unwrap()
        .sync
        .fresh_chat_admission()
        .unwrap()
        .key_package_reference()
        .unwrap();
    let old_no_tail = first
        .server_mut_for_owner_initialization()
        .unwrap()
        .snapshot()
        .unwrap();
    assert!(first.server().unwrap().sync.chat_scope_label().is_none());
    let old_preparation = first.reserve_scope(&store, &mut rng).unwrap();
    let old_attempt = first.attempt;
    store
        .retire_chat_preparation(&old_preparation, &mut rng)
        .unwrap();
    let first_fingerprint = first.server().unwrap().my_fingerprint();
    owner.remove_member(&first_fingerprint).await.unwrap();
    assert_eq!(owner.member_count(), 1, "a real MLS Remove completed");

    let invite = owner.mint_invite([12; 16], 10_000, vec![]).unwrap();
    let second_attempt = store
        .private_managed_join_attempt(
            rejoining_device,
            invite,
            store.capture_trusted_owner_profile(&profile).unwrap(),
        )
        .unwrap();
    let (second, tick) = tokio::join!(
        second_attempt.finish_direct(
            hub.join(PeerId::from_u64(813)),
            ChaCha20Rng::seed_from_u64(813),
            Box::new(ManualClock::new(1_000)),
            "A2 remains private",
            owner_peer,
        ),
        owner.sync_once(),
    );
    tick.unwrap();
    let mut second = second.unwrap();
    assert_eq!(second.server().unwrap().sync.my_public_key(), full_signer);
    assert_ne!(second.attempt, old_attempt);
    assert_ne!(
        second
            .server()
            .unwrap()
            .sync
            .fresh_chat_admission()
            .unwrap()
            .key_package_reference(),
        Some(first_kp)
    );
    let second_preparation = second.reserve_scope(&store, &mut rng).unwrap();
    assert_ne!(second_preparation.nonce(), old_preparation.nonce());
    old_preparation.revoker().revoke();
    assert!(!second_preparation.admission.is_cancelled());
    assert!(first.reserve_scope(&store, &mut rng).is_err());
    let old_snapshot_capture = store
        .capture_chat_admission(second.capture.key.clone())
        .unwrap();
    assert!(
        store
            .prepare_legacy_chat_admission(&old_snapshot_capture, &old_no_tail, &mut rng)
            .is_err(),
        "captured legacy bytes cannot mint a third scope"
    );
    assert!(second
        .server()
        .unwrap()
        .sync
        .durable_send_context()
        .is_err());
}

#[tokio::test]
async fn original_join_attempt_refuses_revocation_and_mount_drop_before_any_handshake() {
    let (dir, store, mut rng) = managed();
    let mut owner = Server::found(
        Hub::new().join(PeerId::from_u64(806)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(806),
        Box::new(ManualClock::new(1_000)),
        "join attempt source",
    )
    .unwrap();
    let profile = owner.sync.prepare_owner_evidence_checkpoint(1).unwrap();
    let invite = owner.sync.mint_invite([6; 16], 10_000, vec![]).unwrap();
    let member = MlsDevice::generate().unwrap();
    let key = ChatAdmissionKey::new(
        &invite.group_id,
        member.public_key_bytes().try_into().unwrap(),
    )
    .unwrap();
    // This exact local row is a storage fixture, not an owner-certified admission. The test
    // proves the pre-await negative fence is consulted before the fixed real join can even run.
    let captured = store.capture_chat_admission(key).unwrap();
    let prepared = store
        .prepare_legacy_chat_admission(&captured, b"old local scope fixture", &mut rng)
        .unwrap();
    let binding = store
        .finish_chat_admission(&prepared, &mut rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    let attempt = store
        .private_managed_join_attempt(
            member,
            invite.clone(),
            store.capture_trusted_owner_profile(&profile).unwrap(),
        )
        .unwrap();
    // This separate still-unissued identity observes only the physical mount lifetime.
    let mount_attempt = store
        .private_managed_join_attempt(
            MlsDevice::generate().unwrap(),
            invite,
            store.capture_trusted_owner_profile(&profile).unwrap(),
        )
        .unwrap();
    binding.revoker().revoke();
    let result = attempt
        .finish_direct(
            Hub::new().join(PeerId::from_u64(807)),
            ChaCha20Rng::seed_from_u64(807),
            Box::new(ManualClock::new(1_000)),
            "revoked attempt",
            PeerId::from_u64(806),
        )
        .await;
    assert!(matches!(result, Err(AppError::Invalid(_))));
    store.retire_chat_admission(&binding, &mut rng).unwrap();
    drop(store);
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    let result = mount_attempt
        .finish_direct(
            Hub::new().join(PeerId::from_u64(808)),
            ChaCha20Rng::seed_from_u64(808),
            Box::new(ManualClock::new(1_000)),
            "original mount is gone",
            PeerId::from_u64(806),
        )
        .await;
    assert!(matches!(result, Err(AppError::Invalid(_))));
    assert!(reopened.chat_vault_format().is_ok());
}

#[test]
fn actual_removed_own_leaf_snapshot_remains_refused_by_positive_mls_restore() {
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut owner_group = catcoms_mls::ServerGroup::create(&owner).unwrap();
    let add = owner_group
        .add_member(&owner, member.key_package().unwrap())
        .unwrap();
    let mut member_group = catcoms_mls::ServerGroup::join(&member, &add.welcome).unwrap();
    let removal = owner_group
        .stage_remove(&owner, &member.device_id())
        .unwrap();
    owner_group.merge_staged_self(&owner).unwrap();
    assert!(matches!(
        member_group
            .process_incoming(&member, &removal.commit)
            .unwrap(),
        catcoms_mls::Incoming::CommitApplied { removed: true }
    ));
    assert!(!member_group.is_active());
    assert!(!member_group.has_local_signer(&member.public_key_bytes()));
    let retired = catcoms_mls::snapshot_server(&member, &member_group).unwrap();
    assert!(catcoms_mls::restore_server(&retired).is_err());
    // This strict positive check is intentionally retained. The canonical factory returns
    // OfflineRetired with its actual validated source/archive instead of weakening this check
    // or exposing an inactive Server to ordinary subscription/discovery/Actor entry points.
    assert!(!retired.is_empty());
}
