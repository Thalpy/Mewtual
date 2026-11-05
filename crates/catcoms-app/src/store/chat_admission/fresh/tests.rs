//! Factory/capture regressions, alongside the separately required actual canonical owner-save
//! vertical. Storage-only records below make no historical membership/owner-authority claim.
use super::*;
use bytes::Bytes;
use catcoms_rt::{Hub, ManualClock};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &[u8] = b"private fresh factory fixture";

fn managed() -> (tempfile::TempDir, ServerStore, ChaCha20Rng) {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xf2e5_0001);
    let store = ServerStore::open(dir.path(), SECRET, &mut rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, &mut rng)
        .unwrap();
    store
        .configure_owner_evidence_budget(
            crate::store::OwnerEvidenceBudget::checkpoint(
                4,
                32 * 1024 * 1024,
                32 * 1024 * 1024,
                1024 * 1024,
                512 * 1024,
                512 * 1024,
            )
            .unwrap(),
        )
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
    let mut attempt = store
        .private_managed_join_attempt(
            device,
            invite,
            expected_profile,
            store.reserve_private_join_creation_for_test().unwrap(),
        )
        .unwrap()
        .bind_direct(hub.join(PeerId::from_u64(810)), owner_peer);
    let (joined, tick) = tokio::join!(
        attempt.finish_direct(
            ChaCha20Rng::seed_from_u64(810),
            Box::new(ManualClock::new(1_000)),
            "never registered joined member",
        ),
        owner.sync_once(),
    );
    tick.unwrap();
    let (mut joined, _creation) = joined.unwrap();
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
    let mut first_attempt = store
        .private_managed_join_attempt(
            first_device,
            invite,
            store.capture_trusted_owner_profile(&profile).unwrap(),
            store.reserve_private_join_creation_for_test().unwrap(),
        )
        .unwrap()
        .bind_direct(hub.join(PeerId::from_u64(812)), owner_peer);
    let (first, tick) = tokio::join!(
        first_attempt.finish_direct(
            ChaCha20Rng::seed_from_u64(812),
            Box::new(ManualClock::new(1_000)),
            "A1 remains private",
        ),
        owner.sync_once(),
    );
    tick.unwrap();
    let (mut first, _first_creation) = first.unwrap();
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
    let mut second_attempt = store
        .private_managed_join_attempt(
            rejoining_device,
            invite,
            store.capture_trusted_owner_profile(&profile).unwrap(),
            store.reserve_private_join_creation_for_test().unwrap(),
        )
        .unwrap()
        .bind_direct(hub.join(PeerId::from_u64(813)), owner_peer);
    let (second, tick) = tokio::join!(
        second_attempt.finish_direct(
            ChaCha20Rng::seed_from_u64(813),
            Box::new(ManualClock::new(1_000)),
            "A2 remains private",
        ),
        owner.sync_once(),
    );
    tick.unwrap();
    let (mut second, _second_creation) = second.unwrap();
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
    let mut attempt = store
        .private_managed_join_attempt(
            member,
            invite.clone(),
            store.capture_trusted_owner_profile(&profile).unwrap(),
            store.reserve_private_join_creation_for_test().unwrap(),
        )
        .unwrap()
        .bind_direct(
            Hub::new().join(PeerId::from_u64(807)),
            PeerId::from_u64(806),
        );
    // This separate still-unissued identity observes only the physical mount lifetime.
    let mut mount_attempt = store
        .private_managed_join_attempt(
            MlsDevice::generate().unwrap(),
            invite,
            store.capture_trusted_owner_profile(&profile).unwrap(),
            store.reserve_private_join_creation_for_test().unwrap(),
        )
        .unwrap()
        .bind_direct(
            Hub::new().join(PeerId::from_u64(808)),
            PeerId::from_u64(806),
        );
    binding.revoker().revoke();
    let result = attempt
        .finish_direct(
            ChaCha20Rng::seed_from_u64(807),
            Box::new(ManualClock::new(1_000)),
            "revoked attempt",
        )
        .await;
    assert!(matches!(result, Err(AppError::Invalid(_))));
    store.retire_chat_admission(&binding, &mut rng).unwrap();
    drop(store);
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    let result = mount_attempt
        .finish_direct(
            ChaCha20Rng::seed_from_u64(808),
            Box::new(ManualClock::new(1_000)),
            "original mount is gone",
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

#[test]
fn private_join_cannot_use_another_mounted_stores_creation_fee() {
    let (_dir, store, _rng) = managed();
    let (_other_dir, other, _other_rng) = managed();
    let mut owner = founder(&store, 0x5b01);
    let profile = owner
        .server_mut_for_owner_initialization()
        .unwrap()
        .sync
        .prepare_owner_evidence_checkpoint(1)
        .unwrap();
    let invite = owner
        .server_mut_for_owner_initialization()
        .unwrap()
        .mint_invite([0x5b; 16], 10_000, vec![])
        .unwrap();
    let pin = store.capture_trusted_owner_profile(&profile).unwrap();
    let foreign_fee = other.reserve_private_join_creation_for_test().unwrap();
    assert!(store
        .private_managed_join_attempt(MlsDevice::generate().unwrap(), invite, pin, foreign_fee)
        .is_err());
}

#[tokio::test]
async fn cancelled_join_keeps_original_paid_source_and_cannot_adopt_reopened_mount() {
    let (dir, store, mut rng) = managed();
    let hub = Hub::new();
    let owner_peer = PeerId::from_u64(0x5b02);
    let mut owner = Server::found(
        hub.join(owner_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0x5b02),
        Box::new(ManualClock::new(1_000)),
        "original join authority",
    )
    .unwrap();
    let profile = owner.sync.prepare_owner_evidence_checkpoint(1).unwrap();
    let invite = owner.mint_invite([0x5b; 16], 10_000, vec![]).unwrap();
    let pin = store.capture_trusted_owner_profile(&profile).unwrap();
    let mut attempt = store
        .private_managed_join_attempt(
            MlsDevice::generate().unwrap(),
            invite,
            pin,
            store.reserve_private_join_creation_for_test().unwrap(),
        )
        .unwrap()
        .bind_direct(hub.join(PeerId::from_u64(0x5b03)), owner_peer);
    let signer = *attempt.attempt.source.as_ref().unwrap().signer();
    {
        let mut future = Box::pin(attempt.finish_direct(
            ChaCha20Rng::seed_from_u64(0x5b03),
            Box::new(ManualClock::new(1_000)),
            "cancel before response",
        ));
        assert!(futures::poll!(&mut future).is_pending());
        drop(future);
    }
    assert_eq!(attempt.attempt.source.as_ref().unwrap().signer(), &signer);
    assert!(attempt.attempt.creation.is_some());
    assert!(attempt.attempt.capture.is_some());
    drop(store);
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    let result = attempt
        .finish_direct(
            ChaCha20Rng::seed_from_u64(0x5b04),
            Box::new(ManualClock::new(1_000)),
            "old task cannot adopt reopened store",
        )
        .await;
    assert!(matches!(result, Err(AppError::Invalid(_))));
    assert_eq!(attempt.attempt.source.as_ref().unwrap().signer(), &signer);
    assert!(reopened.chat_vault_format().is_ok());
}

/// Real MemNetwork requests, with a deterministic lifecycle prefix and one deliberately lost
/// inline result converted into JOIN_PENDING. The SAME actual wrapper is owned across retries.
struct PrefixJoinTransport {
    inner: catcoms_rt::MemNetwork,
    prefix: std::sync::Mutex<std::collections::VecDeque<catcoms_rt::TransportEvent>>,
    first_pending: std::sync::atomic::AtomicBool,
    requests: Arc<std::sync::Mutex<Vec<Bytes>>>,
}
#[async_trait::async_trait]
impl MeshTransport for PrefixJoinTransport {
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }
    fn connection_snapshot(&self) -> Vec<catcoms_rt::PeerConnectionSnapshot> {
        self.inner.connection_snapshot()
    }
    fn connection_lease(&self, peer: PeerId) -> Option<catcoms_rt::ConnectionLease> {
        self.inner.connection_lease(peer)
    }
    fn supports_connection_leases(&self) -> bool {
        true
    }
    async fn subscribe(&self, topic: catcoms_rt::Topic) -> Result<(), catcoms_rt::TransportError> {
        self.inner.subscribe(topic).await
    }
    async fn unsubscribe(
        &self,
        topic: catcoms_rt::Topic,
    ) -> Result<(), catcoms_rt::TransportError> {
        self.inner.unsubscribe(topic).await
    }
    async fn publish(
        &self,
        topic: catcoms_rt::Topic,
        data: Bytes,
    ) -> Result<(), catcoms_rt::TransportError> {
        self.inner.publish(topic, data).await
    }
    async fn request(
        &self,
        peer: PeerId,
        protocol: catcoms_rt::ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, catcoms_rt::TransportError> {
        self.requests.lock().unwrap().push(data.clone());
        let response = self.inner.request(peer, protocol, data).await?;
        if self.first_pending.swap(false, Ordering::SeqCst) {
            assert_eq!(
                response.first(),
                Some(&1),
                "actual inviter returned existing JOIN_READY"
            );
            return Ok(Bytes::from_static(&[0])); // existing JOIN_PENDING wire tag
        }
        Ok(response)
    }
    async fn request_cancellable(
        &self,
        peer: PeerId,
        protocol: catcoms_rt::ProtocolId,
        data: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<Bytes, catcoms_rt::TransportError> {
        self.inner
            .request_cancellable(peer, protocol, data, cancellation)
            .await
    }
    async fn next_event(&self) -> Option<catcoms_rt::TransportEvent> {
        let prefix = self.prefix.lock().unwrap().pop_front();
        if prefix.is_some() {
            return prefix;
        }
        self.inner.next_event().await
    }
}

#[tokio::test]
async fn actual_app_retry_preserves_same_transport_lifecycle_prefix_and_never_adopts_it_into_new_transport(
) {
    let (_dir, store, _rng) = managed();
    let hub = Hub::new();
    let owner_peer = PeerId::from_u64(0x5b05);
    let member_peer = PeerId::from_u64(0x5b06);
    let mut owner = Server::found(
        hub.join(owner_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0x5b05),
        Box::new(ManualClock::new(1_000)),
        "lifecycle owner",
    )
    .unwrap();
    owner.subscribe_control().await.unwrap();
    let profile = owner.sync.prepare_owner_evidence_checkpoint(1).unwrap();
    let invite = owner.mint_invite([0x5c; 16], 10_000, vec![]).unwrap();
    let network = hub.join(member_peer);
    let path = catcoms_rt::ConnectionPath {
        family: catcoms_rt::ConnectionFamily::Ipv4,
        transport: catcoms_rt::ConnectionTransport::QuicV1,
        direction: catcoms_rt::ConnectionDirection::Dialer,
    };
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let transport = PrefixJoinTransport {
        inner: network.clone(),
        first_pending: std::sync::atomic::AtomicBool::new(true),
        requests: requests.clone(),
        prefix: std::sync::Mutex::new(std::collections::VecDeque::from([
            catcoms_rt::TransportEvent::PeerConnected(owner_peer),
            catcoms_rt::TransportEvent::PeerPathsChanged {
                peer: owner_peer,
                active: vec![path],
                newly_established: Some(path),
            },
        ])),
    };
    let mut attempt = store
        .private_managed_join_attempt(
            MlsDevice::generate().unwrap(),
            invite,
            store.capture_trusted_owner_profile(&profile).unwrap(),
            store.reserve_private_join_creation_for_test().unwrap(),
        )
        .unwrap()
        .bind_direct(transport, owner_peer);
    {
        let mut future = Box::pin(attempt.finish_direct(
            ChaCha20Rng::seed_from_u64(0x5b06),
            Box::new(ManualClock::new(1_000)),
            "pending member",
        ));
        assert!(futures::poll!(&mut future).is_pending());
        owner.sync_once().await.unwrap();
        assert!(futures::poll!(&mut future).is_pending()); // actual prefix consumed before cancel
        drop(future);
    }
    assert!(format!("{:?}", attempt.handoff).contains("QuicV1"));
    let (joined, tick) = tokio::join!(
        attempt.finish_direct(
            ChaCha20Rng::seed_from_u64(0x5b06),
            Box::new(ManualClock::new(1_000)),
            "same stream retry"
        ),
        owner.sync_once()
    );
    tick.unwrap();
    let (joined, _creation) = joined.unwrap();
    assert!(joined.server().unwrap().sync.has_connected_peer());
    {
        let recorded = requests.lock().unwrap();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0], recorded[1]);
    }
    assert!(
        attempt.transport.is_none(),
        "only success moved the same original T"
    );

    // A new registration under the SAME local peer has its OWN typed bound attempt and no
    // inherited prefix. There is no API accepting replacement T on the earlier bound finish.
    let invite = owner.mint_invite([0x5d; 16], 10_000, vec![]).unwrap();
    let transport = PrefixJoinTransport {
        inner: network.rejoin(),
        first_pending: std::sync::atomic::AtomicBool::new(false),
        requests: Arc::new(std::sync::Mutex::new(Vec::new())),
        prefix: Default::default(),
    };
    let mut other = store
        .private_managed_join_attempt(
            MlsDevice::generate().unwrap(),
            invite,
            store.capture_trusted_owner_profile(&profile).unwrap(),
            store.reserve_private_join_creation_for_test().unwrap(),
        )
        .unwrap()
        .bind_direct(transport, owner_peer);
    let (other, tick) = tokio::join!(
        other.finish_direct(
            ChaCha20Rng::seed_from_u64(0x5b07),
            Box::new(ManualClock::new(1_000)),
            "new independent stream"
        ),
        owner.sync_once()
    );
    tick.unwrap();
    let (other, _other_creation) = other.unwrap();
    assert!(
        !other.server().unwrap().sync.has_connected_peer(),
        "a new stream must not inherit old peer/path events by local PeerId"
    );
}
