//! Genuine Store + existing direct transport + selected-KP member vertical. No fabricated
//! saved flag, fresh source or signed owner certificate is accepted by these fixtures.
use super::*;
use bytes::Bytes;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_rt::{MeshTransport, ProtocolId, TransportError};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::atomic::Ordering;

const SECRET: &[u8] = b"actual first covering owner Add fixture";
type Owner = OwnedOwnerServer<MemNetwork, ChaCha20Rng>;
fn budget() -> OwnerEvidenceBudget {
    OwnerEvidenceBudget::checkpoint(
        1,
        128 * 1024 * 1024,
        128 * 1024 * 1024,
        24 * 1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap()
}
fn open(path: &std::path::Path, rng: &mut ChaCha20Rng) -> ServerStore {
    let store = ServerStore::open(path, SECRET, rng)
        .unwrap()
        .into_owner_operations_store(SECRET, rng)
        .unwrap();
    store.configure_owner_evidence_budget(budget()).unwrap();
    store
}
fn founder(store: &ServerStore, hub: &Arc<Hub>, seed: u64, rng: &mut ChaCha20Rng) -> Owner {
    let mut initial = store
        .prepare_owner_evidence_founder(
            hub.join(PeerId::from_u64(seed)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(seed),
            Box::new(ManualClock::new(1_000)),
            "covering owner",
            9,
        )
        .unwrap();
    initial.retry(store, rng).unwrap()
}

// Keep the two physical Stores/RNGs and original source/destination peers explicit in this
// adversarial integration helper; none of these arguments is an admission/save-success flag.
#[allow(clippy::too_many_arguments)]
async fn complete_actual_join(
    owner: &mut Owner,
    source: &ServerStore,
    destination: &ServerStore,
    hub: &Arc<Hub>,
    source_peer: u64,
    member_peer: u64,
    source_rng: &mut ChaCha20Rng,
    member_rng: &mut ChaCha20Rng,
    fault: Option<u8>,
) -> Owner {
    let invite = owner
        .mint_owner_checkpoint_invite(source, [7; 16], 100_000, vec![])
        .unwrap();
    let attempt = destination
        .prepare_owner_evidence_checkpoint_join(
            source,
            owner,
            MlsDevice::generate().unwrap(),
            invite,
        )
        .unwrap();
    let transport = hub.join(PeerId::from_u64(member_peer));
    let join = attempt.finish_direct(
        transport,
        ChaCha20Rng::seed_from_u64(member_peer),
        Box::new(ManualClock::new(1_000)),
        "actual selected member",
        PeerId::from_u64(source_peer),
    );
    let drive = async {
        if let Some(fault) = fault {
            source
                .owner_evidence
                .canonical_fault
                .store(fault, Ordering::SeqCst);
        }
        for _ in 0..32 {
            match owner.run_owner_checkpoint_once(source, source_rng).await {
                Ok(_) => {}
                Err(error) => {
                    assert!(fault.is_some(), "unexpected actual save refusal: {error}");
                    assert_eq!(
                        matches!(error, AppError::CommittedButNotDurable(_)),
                        fault == Some(2)
                    );
                    let pending = owner
                        .pending_add
                        .as_ref()
                        .expect("whole operation survives failure");
                    let pointer = pending.frozen.as_ptr();
                    let digest = *blake3::hash(&pending.frozen).as_bytes();
                    let id = pending.request_id;
                    assert_eq!(owner.server.sync.epoch(), 0, "live source MLS is unchanged");
                    assert_eq!(owner.server.sync.member_count(), 1);
                    assert!(owner.server.sync.owner_operation_request_id().is_none());
                    assert_eq!(owner.current_head.generation, 1);
                    assert!(source.owner_evidence.canonical.lock().unwrap().hot_bytes > 0);
                    // A second real refusal proves retry does not regenerate/reseal routing,
                    // repeat the Add or re-encode its randomized provider map.
                    if fault == Some(2) {
                        // A renamed exact candidate is now re-synced, never rewritten. Pin
                        // that stronger recovery contract with its actual barrier fault.
                        source
                            .owner_evidence
                            .restore_sync_failure
                            .store(true, Ordering::SeqCst);
                    } else {
                        source
                            .owner_evidence
                            .canonical_fault
                            .store(1, Ordering::SeqCst);
                    }
                    assert!(owner
                        .run_owner_checkpoint_once(source, source_rng)
                        .await
                        .is_err());
                    source
                        .owner_evidence
                        .restore_sync_failure
                        .store(false, Ordering::SeqCst);
                    let pending = owner.pending_add.as_ref().unwrap();
                    assert_eq!(pending.frozen.as_ptr(), pointer);
                    assert_eq!(*blake3::hash(&pending.frozen).as_bytes(), digest);
                    assert_eq!(pending.request_id, id);
                    owner
                        .run_owner_checkpoint_once(source, source_rng)
                        .await
                        .unwrap();
                }
            }
            if owner.current_head.generation == 2 {
                return;
            }
        }
        panic!("bounded actual direct-request ticks did not produce the covering Add");
    };
    // Wall-clock bound is a harness deadlock guard only; no protocol deadline or expiry
    // correctness is inferred from it. Protocol time stays on ManualClock.
    let guard = catcoms_rt::SystemClock;
    let (joined, ()) = tokio::select! {
        result=async {tokio::join!(join,drive)}=>result,
        _=catcoms_rt::Clock::sleep(&guard,std::time::Duration::from_secs(20))=>{
            panic!("actual request/member fixture deadlocked");
        },
    };
    let mut joined = joined.unwrap();
    assert!(owner.pending_add.is_none());
    assert_eq!(owner.server.sync.epoch(), 1);
    assert_eq!(owner.server.sync.member_count(), 2);
    joined
        .supply_current_owner_checkpoint(destination, source, owner, 9)
        .unwrap();
    let member = joined.retry(destination, member_rng).unwrap();
    assert!(member.is_live());
    assert_eq!(member.group(), owner.group());
    assert_eq!(member.profile(), owner.profile());
    assert_eq!(member.server.sync.epoch(), 1);
    assert_eq!(member.server.sync.member_count(), 2);
    assert_eq!(
        source.owner_evidence.canonical.lock().unwrap().hot_bytes,
        0,
        "lower response bytes have retired after actual selected-KP completion"
    );
    member
}

#[tokio::test]
async fn owner_actual_first_add_saves_before_original_reply_and_member_canonical_ready() {
    let source_dir = tempfile::tempdir().unwrap();
    let member_dir = tempfile::tempdir().unwrap();
    let mut source_rng = ChaCha20Rng::seed_from_u64(0xada1);
    let mut member_rng = ChaCha20Rng::seed_from_u64(0xada2);
    let source = open(source_dir.path(), &mut source_rng);
    let destination = open(member_dir.path(), &mut member_rng);
    let hub = Hub::new();
    let mut owner = founder(&source, &hub, 0xada1, &mut source_rng);
    let member = complete_actual_join(
        &mut owner,
        &source,
        &destination,
        &hub,
        0xada1,
        0xada2,
        &mut source_rng,
        &mut member_rng,
        None,
    )
    .await;
    assert!(owner.is_live());
    assert!(member.is_live());
    assert_eq!(owner.current_head.generation, 2);
    assert_eq!(
        member.current_head.generation, 1,
        "member still uses its existing initial grammar"
    );
}

#[tokio::test]
async fn owner_actual_first_add_pre_and_post_rename_failures_reuse_whole_original_candidate() {
    for fault in [1, 2] {
        let source_dir = tempfile::tempdir().unwrap();
        let member_dir = tempfile::tempdir().unwrap();
        let mut source_rng = ChaCha20Rng::seed_from_u64(0xadb1 + fault as u64);
        let mut member_rng = ChaCha20Rng::seed_from_u64(0xadc1 + fault as u64);
        let source = open(source_dir.path(), &mut source_rng);
        let destination = open(member_dir.path(), &mut member_rng);
        let hub = Hub::new();
        let mut owner = founder(&source, &hub, 0xadb1 + fault as u64, &mut source_rng);
        complete_actual_join(
            &mut owner,
            &source,
            &destination,
            &hub,
            0xadb1 + fault as u64,
            0xadc1 + fault as u64,
            &mut source_rng,
            &mut member_rng,
            Some(fault),
        )
        .await;
        assert_eq!(owner.current_head.generation, 2);
    }
}

#[tokio::test]
async fn owner_actual_successor_cold_reopen_requires_original_v5_parent_and_full_result_chain() {
    let source_dir = tempfile::tempdir().unwrap();
    let member_dir = tempfile::tempdir().unwrap();
    let mut source_rng = ChaCha20Rng::seed_from_u64(0xadd1);
    let mut member_rng = ChaCha20Rng::seed_from_u64(0xadd2);
    let source = open(source_dir.path(), &mut source_rng);
    let destination = open(member_dir.path(), &mut member_rng);
    let hub = Hub::new();
    let mut owner = founder(&source, &hub, 0xadd1, &mut source_rng);
    let member = complete_actual_join(
        &mut owner,
        &source,
        &destination,
        &hub,
        0xadd1,
        0xadd2,
        &mut source_rng,
        &mut member_rng,
        None,
    )
    .await;
    let group = owner.group();
    let signer = *owner.founder();
    let head = owner.current_head.clone();
    let request = owner.server.sync.owner_operation_request_id().unwrap();
    drop(owner);
    drop(source);
    let source = open(source_dir.path(), &mut source_rng);
    let OwnerRestoreClassification::Current(prepared) = source
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("exact actual successor must remain Current");
    };
    let mut restored = prepared
        .restore_private(
            &source,
            Hub::new().join(PeerId::from_u64(0xadd3)),
            ChaCha20Rng::seed_from_u64(0xadd3),
            Box::new(ManualClock::new(1_000_000)),
            "restored owner",
        )
        .unwrap();
    let owner = restored.retry(&source, &mut source_rng).unwrap();
    assert!(owner.is_live());
    assert!(member.is_live());
    assert_eq!(owner.current_head, head);
    assert_eq!(
        owner.server.sync.owner_operation_request_id(),
        Some(request)
    );
    assert_eq!(owner.server.sync.epoch(), 1);
    assert_eq!(owner.server.sync.member_count(), 2);
}

fn raw_join(invite: &InviteToken, package: &[u8]) -> Bytes {
    // Existing direct JOIN kind/protocol, deliberately unchanged by the closed host.
    let mut encoder = Encoder::new();
    encoder.put_u8(1);
    encoder.put_bytes(&invite.encode()).unwrap();
    encoder.put_bytes(package).unwrap();
    Bytes::from(encoder.finish())
}
async fn exchange(
    owner: &mut Owner,
    store: &ServerStore,
    rng: &mut ChaCha20Rng,
    client: &MemNetwork,
    peer: PeerId,
    body: Bytes,
) -> Result<Bytes, TransportError> {
    let mut response = Box::pin(client.request(peer, ProtocolId("/catcoms/rr/1"), body));
    assert!(futures::poll!(&mut response).is_pending()); // real request is now enqueued
    for _ in 0..32 {
        owner.run_owner_checkpoint_once(store, rng).await.unwrap();
        if let std::task::Poll::Ready(result) = futures::poll!(&mut response) {
            return result;
        }
    }
    panic!("bounded actual response ticks exhausted");
}

#[tokio::test]
async fn owner_actual_lost_wait_retries_exact_bytes_before_expired_invite_policy_and_after_reopen()
{
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xade1);
    let store = open(directory.path(), &mut rng);
    let hub = Hub::new();
    let peer = PeerId::from_u64(0xade1);
    let client = hub.join(PeerId::from_u64(0xade2));
    let mut owner = founder(&store, &hub, 0xade1, &mut rng);
    let invite = owner
        .mint_owner_checkpoint_invite(&store, [8; 16], 2_000, vec![])
        .unwrap();
    // This requester deliberately RETAINS its actual original device/KP across wait loss.
    // App join-attempt secret persistence is separate and is not inferred from this source test.
    let member = MlsDevice::generate().unwrap();
    let package = member
        .key_package_for_invite(&owner.group(), invite.invite_nonce)
        .unwrap();
    let reference = member.key_package_reference(&package).unwrap();
    let body = raw_join(
        &invite,
        &catcoms_mls::serialize_key_package(&package).unwrap(),
    );
    let mut first = Box::pin(client.request(peer, ProtocolId("/catcoms/rr/1"), body.clone()));
    assert!(futures::poll!(&mut first).is_pending());
    store
        .owner_evidence
        .canonical_fault
        .store(1, Ordering::SeqCst);
    for _ in 0..32 {
        if owner
            .run_owner_checkpoint_once(&store, &mut rng)
            .await
            .is_err()
        {
            break;
        }
    }
    assert!(owner.pending_add.is_some());
    assert!(
        futures::poll!(&mut first).is_pending(),
        "no response before covering save"
    );
    drop(first); // the actual original Responder's receiver is now synchronously revoked
    assert!(!owner
        .pending_add
        .as_ref()
        .unwrap()
        .request
        .as_ref()
        .unwrap()
        .is_current());
    owner
        .run_owner_checkpoint_once(&store, &mut rng)
        .await
        .unwrap();
    assert_eq!(owner.server.sync.epoch(), 1);
    assert_eq!(owner.server.sync.member_count(), 2);
    let exact = exchange(&mut owner, &store, &mut rng, &client, peer, body.clone())
        .await
        .unwrap();
    let mut decoded = Decoder::new(&exact);
    assert_eq!(decoded.get_u8().unwrap(), 1);
    let welcome = decoded.get_bytes().unwrap();
    assert_eq!(decoded.get_bytes().unwrap().len(), 64);
    decoded.get_bytes().unwrap();
    decoded.finish().unwrap();
    let joined =
        catcoms_mls::ServerGroup::join_for_key_package(&member, welcome, &reference).unwrap();
    assert_eq!(joined.group_id(), owner.group());
    assert_eq!(joined.epoch(), 1);
    let expected = exact.to_vec();
    drop(exact);
    let group = owner.group();
    let signer = *owner.founder();
    let request = owner.server.sync.owner_operation_request_id();
    drop(owner);
    drop(store);
    let store = open(directory.path(), &mut rng);
    let hub = Hub::new();
    let peer = PeerId::from_u64(0xade3);
    let client = hub.join(PeerId::from_u64(0xade4));
    let OwnerRestoreClassification::Current(prepared) = store
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("accepted result must remain Current");
    };
    let mut restored = prepared
        .restore_private(
            &store,
            hub.join(peer),
            ChaCha20Rng::seed_from_u64(0xade3),
            Box::new(ManualClock::new(1_000_000)),
            "expired-invite result retry",
        )
        .unwrap();
    let mut owner = restored.retry(&store, &mut rng).unwrap();
    let reply = exchange(&mut owner, &store, &mut rng, &client, peer, body)
        .await
        .unwrap();
    assert_eq!(reply.as_ref(), expected.as_slice());
    assert_eq!(owner.server.sync.owner_operation_request_id(), request);
    let other = MlsDevice::generate().unwrap();
    let different = other
        .key_package_for_invite(&group, invite.invite_nonce)
        .unwrap();
    assert!(matches!(
        exchange(
            &mut owner,
            &store,
            &mut rng,
            &client,
            peer,
            raw_join(
                &invite,
                &catcoms_mls::serialize_key_package(&different).unwrap()
            )
        )
        .await,
        Err(TransportError::NoResponse)
    ));
    assert_eq!(owner.server.sync.epoch(), 1);
    assert_eq!(owner.server.sync.member_count(), 2);
}

#[tokio::test]
async fn owner_actual_temp_synced_successor_recovers_original_add_same_process_and_after_cold_reopen(
) {
    // Duplicate means a prior exact retry renamed the child but process loss also retained an
    // identical stage (e.g. a replayed fixed-name copy). Only complete equality permits unlink.
    for (cold, duplicate, corrupt) in [
        (false, false, 0),
        (true, false, 0),
        (true, true, 0),
        (true, false, 1),
        (true, false, 2),
        (true, false, 3),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xadf1);
        let store = open(directory.path(), &mut rng);
        let hub = Hub::new();
        let peer = PeerId::from_u64(0xadf1);
        let client = hub.join(PeerId::from_u64(0xadf2));
        let mut owner = founder(&store, &hub, 0xadf1, &mut rng);
        let group = owner.group();
        let signer = *owner.founder();
        let name = hex::encode(key(&group, &signer));
        let canonical = directory
            .path()
            .join("owner-state")
            .join(format!("{name}.bin"));
        let stage = directory
            .path()
            .join("owner-state")
            .join(format!("{name}.replace.tmp"));
        let original = std::fs::read(&canonical).unwrap();
        let invite = owner
            .mint_owner_checkpoint_invite(&store, [9; 16], 100_000, vec![])
            .unwrap();
        let member = MlsDevice::generate().unwrap();
        let package = member
            .key_package_for_invite(&group, invite.invite_nonce)
            .unwrap();
        let reference = member.key_package_reference(&package).unwrap();
        let body = raw_join(
            &invite,
            &catcoms_mls::serialize_key_package(&package).unwrap(),
        );
        let mut wait = Box::pin(client.request(peer, ProtocolId("/catcoms/rr/1"), body.clone()));
        assert!(futures::poll!(&mut wait).is_pending());
        store
            .owner_evidence
            .canonical_fault
            .store(3, Ordering::SeqCst);
        for _ in 0..32 {
            if owner
                .run_owner_checkpoint_once(&store, &mut rng)
                .await
                .is_err()
            {
                break;
            }
        }
        let pending = owner
            .pending_add
            .as_ref()
            .expect("retained actual operation");
        let request = pending.request_id;
        let candidate = std::fs::read(&stage).expect("actual synced complete stage was preserved");
        assert_eq!(std::fs::read(&canonical).unwrap(), original);
        assert!(futures::poll!(&mut wait).is_pending());
        let reply = if cold {
            // A live original writer precludes a competing cold source. Process loss then leaves
            // an actual authenticated parent+child, not a fabricated Saved/Current acknowledgement.
            assert!(store
                .prepare_owner_evidence_restore(&group, signer)
                .is_err());
            drop(wait);
            drop(owner);
            drop(store);
            if duplicate {
                std::fs::write(&canonical, &candidate).unwrap();
            }
            if corrupt != 0 {
                let mut changed = candidate.clone();
                if corrupt == 1 {
                    *changed.last_mut().unwrap() ^= 1;
                } else {
                    // Authenticated local corruption fixture: preserve the genuine Add/response
                    // and signatures but change either its original parent or an unchanged key.
                    // Full owner-signature/child-state validation alone misses the key change.
                    let mut plain = Zeroizing::new(
                        unseal(
                            &store_key(directory.path(), &mut rng),
                            &crate::store::unframe(&changed).unwrap(),
                        )
                        .unwrap(),
                    );
                    let offset = {
                        let mut d = Decoder::new(&plain);
                        d.get_str().unwrap();
                        for _ in 0..5 {
                            d.get_bytes().unwrap();
                        }
                        d.get_u64().unwrap();
                        d.get_u64().unwrap();
                        let parent = d.get_bytes().unwrap();
                        let parent_offset = parent.as_ptr() as usize - plain.as_ptr() as usize;
                        d.get_u64().unwrap();
                        d.get_bytes().unwrap();
                        d.get_bytes().unwrap();
                        let snapshot = d.get_bytes().unwrap();
                        if corrupt == 3 {
                            parent_offset
                        } else {
                            let mut s = Decoder::new(snapshot);
                            for _ in 0..3 {
                                s.get_bytes().unwrap();
                            }
                            assert_eq!(s.get_u32().unwrap(), 0);
                            assert_eq!(s.get_u32().unwrap(), 1);
                            s.get_bytes().unwrap();
                            assert_eq!(s.get_u32().unwrap(), 0);
                            let wrap = s.get_bytes().unwrap();
                            wrap.as_ptr() as usize - plain.as_ptr() as usize
                        }
                    };
                    plain[offset] ^= 1;
                    changed = crate::store::frame(
                        &seal(&store_key(directory.path(), &mut rng), &plain, &mut rng).unwrap(),
                    );
                }
                std::fs::write(&stage, &changed).unwrap();
                let store = open(directory.path(), &mut rng);
                if corrupt == 2 {
                    let OwnerRestoreClassification::Current(prepared) = store
                        .prepare_owner_evidence_restore(&group, signer)
                        .unwrap()
                    else {
                        panic!("actual unchanged Current row required");
                    };
                    assert!(prepared
                        .restore_private(
                            &store,
                            Hub::new().join(PeerId::from_u64(0xadf5)),
                            ChaCha20Rng::seed_from_u64(0xadf5),
                            Box::new(ManualClock::new(1_000)),
                            "changed stable key"
                        )
                        .is_err());
                } else {
                    assert!(store
                        .prepare_owner_evidence_restore(&group, signer)
                        .is_err());
                }
                assert_eq!(std::fs::read(&canonical).unwrap(), original);
                assert_eq!(std::fs::read(&stage).unwrap(), changed);
                assert_eq!(store.owner_evidence.canonical.lock().unwrap().hot_bytes, 0);
                continue;
            }
            let store = open(directory.path(), &mut rng);
            let hub = Hub::new();
            let client = hub.join(PeerId::from_u64(0xadf4));
            let peer = PeerId::from_u64(0xadf3);
            let OwnerRestoreClassification::Current(prepared) = store
                .prepare_owner_evidence_restore(&group, signer)
                .unwrap()
            else {
                panic!("exact retained child must remain Current");
            };
            let mut restored = prepared
                .restore_private(
                    &store,
                    hub.join(peer),
                    ChaCha20Rng::seed_from_u64(0xadf3),
                    Box::new(ManualClock::new(1_000_000)),
                    "retained first Add",
                )
                .unwrap();
            store
                .owner_evidence
                .stage_sync_failure
                .store(true, Ordering::SeqCst);
            assert!(restored.retry(&store, &mut rng).is_err());
            assert_eq!(
                std::fs::read(&canonical).unwrap(),
                if duplicate { &candidate } else { &original }.as_slice()
            );
            assert_eq!(std::fs::read(&stage).unwrap(), candidate);
            store
                .owner_evidence
                .stage_sync_failure
                .store(false, Ordering::SeqCst);
            let mut owner = restored.retry(&store, &mut rng).unwrap();
            assert_eq!(
                owner.server.sync.owner_operation_request_id(),
                Some(request)
            );
            assert_eq!(owner.server.sync.epoch(), 1);
            assert!(!stage.exists());
            let reply = exchange(&mut owner, &store, &mut rng, &client, peer, body)
                .await
                .unwrap();
            assert_eq!(owner.server.sync.member_count(), 2);
            reply
        } else {
            let pointer = owner.pending_add.as_ref().unwrap().frozen.as_ptr();
            store
                .owner_evidence
                .stage_sync_failure
                .store(true, Ordering::SeqCst);
            assert!(owner
                .run_owner_checkpoint_once(&store, &mut rng)
                .await
                .is_err());
            assert_eq!(owner.pending_add.as_ref().unwrap().frozen.as_ptr(), pointer);
            assert_eq!(owner.pending_add.as_ref().unwrap().request_id, request);
            assert_eq!(std::fs::read(&stage).unwrap(), candidate);
            store
                .owner_evidence
                .stage_sync_failure
                .store(false, Ordering::SeqCst);
            owner
                .run_owner_checkpoint_once(&store, &mut rng)
                .await
                .unwrap();
            assert!(owner.pending_add.is_none());
            assert_eq!(owner.server.sync.epoch(), 1);
            assert!(!stage.exists());
            wait.await.unwrap()
        };
        let mut decoded = Decoder::new(&reply);
        assert_eq!(decoded.get_u8().unwrap(), 1);
        let welcome = decoded.get_bytes().unwrap();
        let joined =
            catcoms_mls::ServerGroup::join_for_key_package(&member, welcome, &reference).unwrap();
        assert_eq!(joined.epoch(), 1);
        assert_eq!(joined.member_count(), 2);
    }
}

// Reopen only after the fixture's original Store dropped. No key generation/source authority
// is inferred: this reads the actual same vault DEK for deliberate authenticated-disk fixtures.
fn store_key(path: &std::path::Path, rng: &mut ChaCha20Rng) -> [u8; 32] {
    let store = open(path, rng);
    store.keys.db_key().unwrap()
}

#[tokio::test]
async fn owner_actual_writer_drop_settles_only_already_renamed_candidate_without_a_stage() {
    for fault in [1, 2] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xad81 + fault);
        let store = open(directory.path(), &mut rng);
        let hub = Hub::new();
        let peer = PeerId::from_u64(0xad81);
        let client = hub.join(PeerId::from_u64(0xad82));
        let mut owner = founder(&store, &hub, 0xad81, &mut rng);
        let group = owner.group();
        let signer = *owner.founder();
        let invite = owner
            .mint_owner_checkpoint_invite(&store, [81; 16], 100_000, vec![])
            .unwrap();
        let member = MlsDevice::generate().unwrap();
        let package = member
            .key_package_for_invite(&group, invite.invite_nonce)
            .unwrap();
        let body = raw_join(
            &invite,
            &catcoms_mls::serialize_key_package(&package).unwrap(),
        );
        let mut wait = Box::pin(client.request(peer, ProtocolId("/catcoms/rr/1"), body.clone()));
        assert!(futures::poll!(&mut wait).is_pending());
        store
            .owner_evidence
            .canonical_fault
            .store(fault as u8, Ordering::SeqCst);
        for _ in 0..32 {
            if owner
                .run_owner_checkpoint_once(&store, &mut rng)
                .await
                .is_err()
            {
                break;
            }
        }
        let request = owner.pending_add.as_ref().unwrap().request_id;
        let identifier = key(&group, &signer);
        assert!(!store.owner_canonical_stage_exists(&identifier).unwrap());
        let path = directory
            .path()
            .join("owner-state")
            .join(format!("{}.bin", hex::encode(identifier)));
        let actual = std::fs::read(&path).unwrap();
        drop(wait);
        drop(owner); // SAME Store keeps original current1/pending2 debt; only writer is gone.
        if fault == 1 {
            assert!(
                store
                    .prepare_owner_evidence_restore(&group, signer)
                    .is_err(),
                "old parent with no retained candidate cannot manufacture a second Add"
            );
        } else {
            let OwnerRestoreClassification::Current(prepared) = store
                .prepare_owner_evidence_restore(&group, signer)
                .unwrap()
            else {
                panic!("actual renamed child retains Current incarnation");
            };
            let mut restored = prepared
                .restore_private(
                    &store,
                    hub.join(peer),
                    ChaCha20Rng::seed_from_u64(0xad83),
                    Box::new(ManualClock::new(1_000)),
                    "already renamed child",
                )
                .unwrap();
            store
                .owner_evidence
                .restore_sync_failure
                .store(true, Ordering::SeqCst);
            assert!(restored.retry(&store, &mut rng).is_err());
            store
                .owner_evidence
                .restore_sync_failure
                .store(false, Ordering::SeqCst);
            let mut owner = restored.retry(&store, &mut rng).unwrap();
            assert_eq!(
                owner.server.sync.owner_operation_request_id(),
                Some(request)
            );
            assert_eq!(owner.server.sync.epoch(), 1);
            let reply = exchange(&mut owner, &store, &mut rng, &client, peer, body)
                .await
                .unwrap();
            assert!(!reply.is_empty());
            assert_eq!(owner.server.sync.member_count(), 2);
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            actual,
            "repair must not rewrite/reseal exact visible bytes"
        );
    }
}

#[tokio::test]
async fn owner_actual_retirement_preempts_pending_install_and_never_revives_on_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xad01);
    let store = open(directory.path(), &mut rng);
    let hub = Hub::new();
    let peer = PeerId::from_u64(0xad01);
    let client = hub.join(PeerId::from_u64(0xad02));
    let mut owner = founder(&store, &hub, 0xad01, &mut rng);
    let group = owner.group();
    let signer = *owner.founder();
    let invite = owner
        .mint_owner_checkpoint_invite(&store, [10; 16], 100_000, vec![])
        .unwrap();
    let member = MlsDevice::generate().unwrap();
    let package = member
        .key_package_for_invite(&group, invite.invite_nonce)
        .unwrap();
    let mut wait = Box::pin(client.request(
        peer,
        ProtocolId("/catcoms/rr/1"),
        raw_join(
            &invite,
            &catcoms_mls::serialize_key_package(&package).unwrap(),
        ),
    ));
    assert!(futures::poll!(&mut wait).is_pending());
    store
        .owner_evidence
        .canonical_fault
        .store(1, Ordering::SeqCst);
    for _ in 0..32 {
        if owner
            .run_owner_checkpoint_once(&store, &mut rng)
            .await
            .is_err()
        {
            break;
        }
    }
    assert!(owner.pending_add.is_some());
    owner.retire_local_owner(&store, &mut rng).unwrap();
    assert!(!owner.is_live());
    assert!(owner
        .run_owner_checkpoint_once(&store, &mut rng)
        .await
        .is_err());
    assert_eq!(owner.server.sync.epoch(), 0);
    assert!(owner.server.sync.owner_operation_request_id().is_none());
    drop(owner);
    assert!(matches!(wait.await, Err(TransportError::NoResponse)));
    drop(store);
    let store = open(directory.path(), &mut rng);
    assert!(matches!(
        store
            .prepare_owner_evidence_restore(&group, signer)
            .unwrap(),
        OwnerRestoreClassification::Retired(_)
    ));
}
