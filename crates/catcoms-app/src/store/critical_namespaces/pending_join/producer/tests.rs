//! Genuine Saved/Current founder -> signed invite -> actual sealed Pending source. These tests
//! exercise DATA custody only, never pretend a decoded phase is a runnable join. All UNRUN.
use super::*;
use crate::store::{OwnedOwnerServer, OwnerEvidenceBudget};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::atomic::Ordering;

mod activation;
mod archive;
mod execution;

const SECRET: &[u8] = b"actual Pending-only destination source fixture";
type Owner = OwnedOwnerServer<MemNetwork, ChaCha20Rng>;
fn budget() -> OwnerEvidenceBudget {
    OwnerEvidenceBudget::checkpoint(
        8,
        128 * 1024 * 1024,
        128 * 1024 * 1024,
        1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap()
}
fn plain_store(path: &Path, rng: &mut ChaCha20Rng) -> ServerStore {
    let store = ServerStore::open(path, SECRET, rng)
        .unwrap()
        .into_critical_namespace_store(SECRET, rng)
        .unwrap();
    store.configure_owner_evidence_budget(budget()).unwrap();
    store
}
fn configured(path: &Path, rng: &mut ChaCha20Rng, pool: &CriticalCatalogReadPool) -> ServerStore {
    let policy =
        PendingJoinCatalogPolicy::new(pool.clone(), 2 * codec::MAX_CATALOG_FRAMED).unwrap();
    let store = ServerStore::open_with_pending_join_catalog(path, SECRET, rng, policy).unwrap();
    store.configure_owner_evidence_budget(budget()).unwrap();
    store
}
fn founder(store: &ServerStore, hub: &Arc<Hub>, peer: u64, rng: &mut ChaCha20Rng) -> Owner {
    let mut initial = store
        .prepare_owner_evidence_founder(
            hub.join(PeerId::from_u64(peer)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(peer),
            Box::new(ManualClock::new(1_000)),
            "genuine source founder",
            9,
        )
        .unwrap();
    initial.retry(store, rng).unwrap()
}
struct Fixture {
    owner: Owner,
    source: ServerStore,
    member: Option<Arc<ServerStore>>,
    pool: CriticalCatalogReadPool,
    member_rng: ChaCha20Rng,
    hub: Arc<Hub>,
    // Every Store/file owner drops before its temporary directory.
    _source_dir: tempfile::TempDir,
    member_dir: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let source_dir = tempfile::tempdir().unwrap();
        let member_dir = tempfile::tempdir().unwrap();
        let mut source_rng = ChaCha20Rng::seed_from_u64(0x8101);
        let mut member_rng = ChaCha20Rng::seed_from_u64(0x8102);
        let source = plain_store(source_dir.path(), &mut source_rng);
        let hub = Hub::new();
        let owner = founder(&source, &hub, 0x8101, &mut source_rng);
        drop(plain_store(member_dir.path(), &mut member_rng));
        let pool = CriticalCatalogReadPool::new(512 * 1024 * 1024).unwrap();
        let member = Arc::new(configured(member_dir.path(), &mut member_rng, &pool));
        Self {
            owner,
            source,
            member: Some(member),
            pool,
            member_rng,
            hub,
            _source_dir: source_dir,
            member_dir,
        }
    }
    fn member(&self) -> &ServerStore {
        self.member.as_deref().unwrap()
    }
    fn invite(&self, nonce: u8) -> InviteToken {
        self.owner
            .mint_owner_checkpoint_invite(&self.source, [nonce; 16], 100_000, vec![])
            .unwrap()
    }
    fn begin(&mut self, invite: InviteToken) -> JournalledCheckpointJoinPreparation {
        self.member
            .as_deref()
            .unwrap()
            .prepare_journalled_owner_checkpoint_join(
                &self.source,
                &self.owner,
                MlsDevice::generate().unwrap(),
                invite,
                &mut self.member_rng,
            )
            .unwrap()
    }
    fn reopen(&mut self) {
        drop(self.member.take());
        self.member = Some(Arc::new(configured(
            self.member_dir.path(),
            &mut self.member_rng,
            &self.pool,
        )));
    }
    fn current_bytes(&self) -> Vec<u8> {
        fs::read(self.member_dir.path().join(Slot::Current.name())).unwrap()
    }
}

#[test]
fn actual_pending_save_reopen_coalesces_exact_original_request_and_unused_replacement_device() {
    let mut fixture = Fixture::new();
    let invite = fixture.invite(1);
    let mut prepared = fixture.begin(invite.clone());
    let original = prepared.metadata.clone();
    prepared
        .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8201))
        .unwrap();
    let stored = fixture.current_bytes();
    prepared
        .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8202))
        .unwrap();
    assert_eq!(
        fixture.current_bytes(),
        stored,
        "idempotent save must not reseal current DATA"
    );
    drop(prepared);
    fixture.reopen();
    let unused = MlsDevice::generate().unwrap();
    let unused_signer = unused.public_key_bytes();
    let mut recovered = fixture
        .member
        .as_deref()
        .unwrap()
        .prepare_journalled_owner_checkpoint_join(
            &fixture.source,
            &fixture.owner,
            unused,
            invite.clone(),
            &mut fixture.member_rng,
        )
        .unwrap();
    assert!(recovered.metadata == original);
    assert_ne!(
        recovered.metadata.signer.as_slice(),
        unused_signer.as_slice()
    );
    assert!(matches!(recovered.source, Source::Recovered(_)));
    recovered
        .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8203))
        .unwrap();
    assert_eq!(fixture.current_bytes(), stored);
    // The old unjournalled factory is closed on this configured Pending mount.
    assert!(fixture
        .member()
        .prepare_owner_evidence_checkpoint_join(
            &fixture.source,
            &fixture.owner,
            MlsDevice::generate().unwrap(),
            invite
        )
        .is_err());
    assert_eq!(fixture.current_bytes(), stored);
    drop(recovered);
    drop(fixture.member.take());
    assert!(
        ServerStore::open(fixture.member_dir.path(), SECRET, &mut fixture.member_rng).is_err(),
        "ordinary readers must refuse the new critical namespace before older cleanup"
    );
    assert_eq!(
        fs::read(fixture.member_dir.path().join(Slot::Current.name())).unwrap(),
        stored
    );
}

#[test]
fn configured_empty_refuses_old_join_and_same_nonce_changed_valid_invite_cannot_replace_source() {
    let mut fixture = Fixture::new();
    let invite = fixture.invite(2);
    assert!(fixture
        .member()
        .prepare_owner_evidence_checkpoint_join(
            &fixture.source,
            &fixture.owner,
            MlsDevice::generate().unwrap(),
            invite.clone()
        )
        .is_err());
    let mut prepared = fixture.begin(invite.clone());
    prepared
        .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8301))
        .unwrap();
    let stored = fixture.current_bytes();
    let changed = fixture
        .owner
        .mint_owner_checkpoint_invite(&fixture.source, [2; 16], 200_000, vec![])
        .unwrap();
    assert!(changed.verify_self());
    assert_ne!(changed.encode(), invite.encode());
    assert!(fixture
        .member
        .as_deref()
        .unwrap()
        .prepare_journalled_owner_checkpoint_join(
            &fixture.source,
            &fixture.owner,
            MlsDevice::generate().unwrap(),
            changed,
            &mut fixture.member_rng
        )
        .is_err());
    assert_eq!(fixture.current_bytes(), stored);
}

#[test]
fn full_five_sources_can_all_pause_then_resume_another_without_sixth_or_key_replacement() {
    let mut fixture = Fixture::new();
    let mut invites = Vec::new();
    let mut originals = Vec::new();
    for nonce in 10..15 {
        let invite = fixture.invite(nonce);
        let mut prepared = fixture.begin(invite.clone());
        prepared
            .save_pending(
                fixture.member(),
                &mut ChaCha20Rng::seed_from_u64(nonce as u64),
            )
            .unwrap();
        originals.push((
            prepared.metadata.source,
            prepared.metadata.ciphertext,
            prepared.metadata.package,
            prepared.metadata.request,
        ));
        prepared
            .pause_pending(
                fixture.member(),
                &mut ChaCha20Rng::seed_from_u64(0x8400 + nonce as u64),
            )
            .unwrap();
        drop(prepared);
        invites.push(invite);
    }
    let summary = fixture.member().pending_join_summary().unwrap();
    assert_eq!(summary.count, 5);
    assert!(summary
        .rows
        .iter()
        .flatten()
        .all(|row| row.scheduling == codec::Scheduling::Paused));
    let before_sixth = fixture.current_bytes();
    let sixth = fixture.invite(15);
    assert!(fixture
        .member
        .as_deref()
        .unwrap()
        .prepare_journalled_owner_checkpoint_join(
            &fixture.source,
            &fixture.owner,
            MlsDevice::generate().unwrap(),
            sixth,
            &mut fixture.member_rng
        )
        .is_err());
    assert_eq!(fixture.current_bytes(), before_sixth);
    let mut selected = fixture.begin(invites[1].clone());
    selected
        .resume_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8401))
        .unwrap();
    let mut other = fixture.begin(invites[2].clone());
    let before_conflict = fixture.current_bytes();
    assert!(other
        .resume_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8402))
        .is_err());
    assert_eq!(fixture.current_bytes(), before_conflict);
    selected
        .pause_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8403))
        .unwrap();
    other
        .resume_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8404))
        .unwrap();
    let summary = fixture.member().pending_join_summary().unwrap();
    assert_eq!(
        summary
            .rows
            .iter()
            .flatten()
            .filter(|row| row.scheduling == codec::Scheduling::Active)
            .count(),
        1
    );
    for (source, ciphertext, package, request) in originals {
        let row = summary
            .rows
            .iter()
            .flatten()
            .find(|row| row.source == source)
            .unwrap();
        assert_eq!(row.ciphertext, ciphertext);
        assert_eq!(row.package, package);
        assert_eq!(row.request, request);
    }
}

#[cfg(any(unix, windows))]
#[test]
fn store_retains_exact_created_stage_and_candidate_after_caller_drop_for_every_warm_boundary() {
    for fault in [3, 5, 4, 1, 2] {
        let mut fixture = Fixture::new();
        let invite = fixture.invite(20);
        let mut prepared = fixture.begin(invite);
        let original = prepared.metadata.clone();
        fixture
            .member()
            .critical_namespace_fault
            .store(fault, Ordering::SeqCst);
        assert!(prepared
            .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8501))
            .is_err());
        let (expected, identity) = {
            let mounted = fixture.member().pending_join_catalog.lock().unwrap();
            let pending = mounted.as_ref().unwrap().pending.as_ref().unwrap();
            (
                pending.bytes.to_vec(),
                pending
                    .created
                    .as_ref()
                    .map(|created| catcoms_storage::kept::session_id(&created.file).unwrap()),
            )
        };
        let staged = fixture.member_dir.path().join(Slot::Stage.name());
        if fault == 5 {
            assert_eq!(fs::metadata(&staged).unwrap().len(), 0);
        }
        if fault == 4 {
            assert_eq!(fs::read(&staged).unwrap(), expected[..17]);
        }
        drop(prepared); // No plaintext master/caller survives to help the following repair.
        fixture
            .member()
            .critical_namespace_fault
            .store(0, Ordering::SeqCst);
        fixture.member().retry_pending_join_catalog().unwrap();
        assert_eq!(
            fixture.current_bytes(),
            expected,
            "repair cannot reseal/reconstruct this output"
        );
        assert!(!staged.exists());
        if let Some(identity) = identity {
            assert!(matches!(
                catcoms_storage::kept::session_at(
                    &fixture.member_dir.path().join(Slot::Current.name()),
                    identity
                ),
                catcoms_storage::kept::SessionAt::Ours
            ));
        }
        let summary = fixture.member().pending_join_summary().unwrap();
        assert!(summary.rows.iter().flatten().any(|row| row == &original));
        assert_eq!(
            fixture.pool.charged_for_test(),
            METADATA_FEE,
            "retired output/source/original fees must drain after both writers"
        );
    }
}

#[cfg(any(unix, windows))]
#[test]
fn warm_partial_repair_refuses_foreign_name_and_wrong_prefix_on_the_same_created_object() {
    for replace in [false, true] {
        let mut fixture = Fixture::new();
        let invite = fixture.invite(21);
        let mut prepared = fixture.begin(invite);
        let before = fixture.current_bytes();
        fixture
            .member()
            .critical_namespace_fault
            .store(4, Ordering::SeqCst);
        assert!(prepared
            .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8601))
            .is_err());
        let stage = fixture.member_dir.path().join(Slot::Stage.name());
        if replace {
            fs::rename(
                &stage,
                fixture.member_dir.path().join("held-original-for-test"),
            )
            .unwrap();
        }
        fs::write(&stage, b"foreign or modified bytes").unwrap();
        fixture
            .member()
            .critical_namespace_fault
            .store(0, Ordering::SeqCst);
        assert!(fixture.member().retry_pending_join_catalog().is_err());
        assert_eq!(fs::read(&stage).unwrap(), b"foreign or modified bytes");
        assert_eq!(fixture.current_bytes(), before);
        assert!(fixture
            .member()
            .pending_join_catalog
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .pending
            .is_some());
    }
}

#[test]
fn cold_complete_stage_settles_original_data_but_cold_partial_stage_preserves_and_refuses() {
    for fault in [1, 2, 4, 5] {
        let mut fixture = Fixture::new();
        let invite = fixture.invite(22);
        let mut prepared = fixture.begin(invite.clone());
        let original = prepared.metadata.clone();
        fixture
            .member()
            .critical_namespace_fault
            .store(fault, Ordering::SeqCst);
        assert!(prepared
            .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8701))
            .is_err());
        let current = fixture.current_bytes();
        let stage_path = fixture.member_dir.path().join(Slot::Stage.name());
        let stage = fs::read(&stage_path).ok();
        drop(prepared);
        drop(fixture.member.take());
        let policy =
            PendingJoinCatalogPolicy::new(fixture.pool.clone(), 2 * codec::MAX_CATALOG_FRAMED)
                .unwrap();
        let reopened = ServerStore::open_with_pending_join_catalog(
            fixture.member_dir.path(),
            SECRET,
            &mut fixture.member_rng,
            policy,
        );
        if fault == 1 || fault == 2 {
            let store = reopened.unwrap();
            assert!(store
                .pending_join_summary()
                .unwrap()
                .rows
                .iter()
                .flatten()
                .any(|row| row == &original));
            assert!(!stage_path.exists());
        } else {
            assert!(
                reopened.is_err(),
                "no cold CreatedStage handle may be manufactured"
            );
            assert_eq!(
                fs::read(fixture.member_dir.path().join(Slot::Current.name())).unwrap(),
                current
            );
            assert_eq!(fs::read(&stage_path).ok(), stage);
        }
    }
}

#[test]
fn configured_quota_refusal_precedes_read_buffers_and_refunds_constructor_fees() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x8801);
    drop(plain_store(directory.path(), &mut rng));
    let current = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let pool = CriticalCatalogReadPool::new(METADATA_FEE + READ_FEE - 1).unwrap();
    let policy =
        PendingJoinCatalogPolicy::new(pool.clone(), 2 * codec::MAX_CATALOG_FRAMED).unwrap();
    assert!(ServerStore::open_with_pending_join_catalog(
        directory.path(),
        SECRET,
        &mut rng,
        policy
    )
    .is_err());
    assert_eq!(pool.read_entries_for_test(), 0);
    assert_eq!(pool.charged_for_test(), 0);
    assert_eq!(
        fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        current
    );
}

#[test]
fn independent_other_group_canonical_control_progresses_while_catalog_output_is_uncertain() {
    let mut fixture = Fixture::new();
    let invite = fixture.invite(23);
    let mut prepared = fixture.begin(invite);
    fixture
        .member()
        .critical_namespace_fault
        .store(1, Ordering::SeqCst);
    assert!(prepared
        .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x8901))
        .is_err());
    let other = founder(
        fixture.member(),
        &fixture.hub,
        0x8902,
        &mut ChaCha20Rng::seed_from_u64(0x8902),
    );
    assert!(other.is_live());
    assert!(fixture
        .member()
        .pending_join_catalog
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .pending
        .is_some());
    assert!(other
        .mint_owner_checkpoint_invite(fixture.member(), [24; 16], 100_000, vec![])
        .is_ok());
    fixture
        .member()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    fixture.member().retry_pending_join_catalog().unwrap();
    assert!(other.is_live());
}

// Negative-boundary probe ONLY: no defaults here are acceptable evidence of delivered traffic.
// A mistakenly reached request increments synchronously and refuses, so the test never hangs.
struct NoRequestTransport(Arc<std::sync::atomic::AtomicUsize>);
#[async_trait::async_trait]
impl catcoms_rt::MeshTransport for NoRequestTransport {
    fn local_peer(&self) -> PeerId {
        PeerId::from_u64(0x9002)
    }
    async fn subscribe(&self, _topic: catcoms_rt::Topic) -> Result<(), catcoms_rt::TransportError> {
        Err(catcoms_rt::TransportError::Closed)
    }
    async fn unsubscribe(
        &self,
        _topic: catcoms_rt::Topic,
    ) -> Result<(), catcoms_rt::TransportError> {
        Err(catcoms_rt::TransportError::Closed)
    }
    async fn publish(
        &self,
        _topic: catcoms_rt::Topic,
        _data: bytes::Bytes,
    ) -> Result<(), catcoms_rt::TransportError> {
        Err(catcoms_rt::TransportError::Closed)
    }
    async fn request(
        &self,
        _peer: PeerId,
        _protocol: catcoms_rt::ProtocolId,
        _data: bytes::Bytes,
    ) -> Result<bytes::Bytes, catcoms_rt::TransportError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(catcoms_rt::TransportError::Closed)
    }
    async fn request_cancellable(
        &self,
        _peer: PeerId,
        _protocol: catcoms_rt::ProtocolId,
        _data: bytes::Bytes,
        _cancellation: catcoms_rt::RequestCancellation,
    ) -> Result<bytes::Bytes, catcoms_rt::TransportError> {
        // This negative-only probe queues no lower work. If any request crosses the original
        // mount fence, count it and refuse immediately, including the required accounted seam.
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(catcoms_rt::TransportError::Closed)
    }
    async fn next_event(&self) -> Option<catcoms_rt::TransportEvent> {
        None
    }
}

#[tokio::test]
async fn old_pre_remount_bound_attempt_cannot_send_on_new_configured_empty_mount() {
    let mut fixture = Fixture::new();
    let invite = fixture.invite(30);
    drop(fixture.member.take());
    let old =
        ServerStore::open(fixture.member_dir.path(), SECRET, &mut fixture.member_rng).unwrap();
    old.configure_owner_evidence_budget(budget()).unwrap();
    let attempt = old
        .prepare_owner_evidence_checkpoint_join(
            &fixture.source,
            &fixture.owner,
            MlsDevice::generate().unwrap(),
            invite,
        )
        .unwrap();
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut direct = attempt.bind_direct_transport(
        NoRequestTransport(requests.clone()),
        PeerId::from_u64(0x8101),
    );
    drop(old);
    fixture.reopen();
    let result = direct
        .finish_direct(
            ChaCha20Rng::seed_from_u64(0x9002),
            Box::new(ManualClock::new(1000)),
            "cancelled prior mount",
        )
        .await;
    assert!(matches!(result, Err(AppError::Invalid(_))));
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.member().pending_join_summary().unwrap().count, 0);
}

#[test]
fn original_negative_winner_after_admission_before_catalog_wait_prevents_pending_install() {
    use std::sync::mpsc;
    use std::time::Duration;
    struct ReleaseOnDrop(Option<mpsc::Sender<()>>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let mut fixture = Fixture::new();
    let invite = fixture.invite(31);
    let mut prepared = fixture.begin(invite);
    let member = fixture.member.as_ref().unwrap().clone();
    let negative = member.chat_mount_revoker();
    let weak = Arc::downgrade(&member);
    let (reached_tx, reached_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (contended_tx, contended_rx) = mpsc::channel();
    let release = ReleaseOnDrop(Some(release_tx));
    *member.pending_join_before_catalog_writer.lock().unwrap() = Some(Box::new(move || {
        reached_tx.send(()).unwrap();
        release_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("bounded harness release");
        let store = weak.upgrade().unwrap();
        let contended = matches!(
            store.pending_join_catalog.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        );
        contended_tx.send(contended).unwrap();
    }));
    let (done_tx, done_rx) = mpsc::channel();
    let worker_store = member.clone();
    let worker = std::thread::spawn(move || {
        let rejected = prepared
            .save_pending(&worker_store, &mut ChaCha20Rng::seed_from_u64(0x9101))
            .is_err();
        drop(prepared);
        drop(worker_store);
        let _ = done_tx.send(rejected);
    });
    reached_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("worker reached actual admission-held seam");
    let catalog = member.pending_join_catalog.lock().unwrap();
    negative.revoke(); // Negative closure never waits for the held admission/catalog writers.
    release.0.as_ref().unwrap().send(()).unwrap();
    assert!(contended_rx.recv_timeout(Duration::from_secs(10)).unwrap());
    drop(catalog);
    assert!(done_rx.recv_timeout(Duration::from_secs(10)).unwrap());
    worker.join().unwrap(); // Join only after explicit bounded completion; no unwind join trap.
    assert!(member
        .pending_join_catalog
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .pending
        .is_none());
    assert!(!fixture.member_dir.path().join(Slot::Stage.name()).exists());
}

#[test]
fn archive_two_unknown_progress_and_wrong_whole_parent_preserve_before_constructor_cleanup() {
    for fault in 0..3 {
        let mut fixture = Fixture::new();
        let invite = fixture.invite(32);
        let mut prepared = fixture.begin(invite);
        prepared
            .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0x9201))
            .unwrap();
        let current = fixture.current_bytes();
        let key = fixture.member().keys.db_key().unwrap();
        let plain = Zeroizing::new(unseal(&key, &unframe(&current).unwrap()).unwrap());
        let mut input = Decoder::new(&plain);
        assert_eq!(input.get_str().unwrap(), codec::TAG_V2);
        let domain = input.get_bytes().unwrap();
        let generation = input.get_u64().unwrap();
        input.get_bytes().unwrap();
        input.get_u32().unwrap();
        input.get_u32().unwrap();
        input.get_u32().unwrap();
        let payload = input.get_bytes().unwrap();
        input.finish().unwrap();
        let mut changed_payload = payload.to_vec();
        if fault == 1 {
            let mut joined = Decoder::new(payload);
            joined.get_str().unwrap();
            joined.get_u32().unwrap();
            let row = joined.get_bytes().unwrap();
            let mut fields = Decoder::new(row);
            fields.get_bytes().unwrap();
            fields.get_u64().unwrap();
            fields.get_u8().unwrap();
            let progress = row.len() - fields.remaining();
            let row_offset = row.as_ptr() as usize - payload.as_ptr() as usize;
            changed_payload[row_offset + progress] = 4; // Unknown Frozen progress is not Pending.
        }
        let mut encoder = Encoder::new();
        encoder.put_str(codec::TAG_V2).unwrap();
        encoder.put_bytes(domain).unwrap();
        encoder.put_u64(generation + 1);
        encoder
            .put_bytes(if fault == 2 {
                &[0x99; 32]
            } else {
                &codec::catalog_digest(&plain)
            })
            .unwrap();
        encoder.put_u32(if fault == 0 { 2 } else { 1 });
        encoder.put_u32(1);
        encoder.put_u32(1);
        encoder.put_bytes(&changed_payload).unwrap();
        if fault == 0 {
            encoder.put_u32(2);
            encoder.put_u32(1);
            encoder
                .put_bytes(b"unsupported real archive handler required")
                .unwrap();
        }
        let future = frame(
            &seal(
                &key,
                &encoder.finish(),
                &mut ChaCha20Rng::seed_from_u64(0x9202),
            )
            .unwrap(),
        );
        let stage_path = fixture.member_dir.path().join(Slot::Stage.name());
        fs::write(&stage_path, &future).unwrap();
        drop(prepared);
        drop(fixture.member.take());
        let policy =
            PendingJoinCatalogPolicy::new(fixture.pool.clone(), 2 * codec::MAX_CATALOG_FRAMED)
                .unwrap();
        assert!(ServerStore::open_with_pending_join_catalog(
            fixture.member_dir.path(),
            SECRET,
            &mut fixture.member_rng,
            policy
        )
        .is_err());
        assert_eq!(fs::read(&stage_path).unwrap(), future);
        assert_eq!(
            fs::read(fixture.member_dir.path().join(Slot::Current.name())).unwrap(),
            current
        );
    }
}
