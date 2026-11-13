//! Actual private namespace transaction. Tamper objects are NEGATIVE fixtures, never fake
//! scope/Current/SDK authorities. Primary owns serial compiled validation; these are UNRUN.
use super::super::{archive_current_namespace_codec as codec, pending_join_codec as catalog};
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use std::sync::atomic::Ordering;

const SECRET: &[u8] = b"actual namespace3 private opener";
fn pool() -> CriticalCatalogReadPool {
    CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap()
}
fn archive(pool: &CriticalCatalogReadPool) -> ArchiveBootstrapCatalogPolicy {
    ArchiveBootstrapCatalogPolicy::new(pool.clone(), 4 * archive_bootstrap::MAX_SEED_FRAMED)
        .unwrap()
        .with_bootstrap_source_data_read()
}
fn policy(pool: &CriticalCatalogReadPool) -> V2CurrentLayoutPolicy {
    V2CurrentLayoutPolicy::new(
        archive(pool),
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * codec::MAX_DECLARATION_FRAME,
    )
    .unwrap()
}
fn initialize(path: &Path, rng: &mut ChaCha20Rng) {
    drop(
        ServerStore::open(path, SECRET, rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, rng)
            .unwrap(),
    );
}
fn prepare_two(path: &Path, pool: &CriticalCatalogReadPool, rng: &mut ChaCha20Rng) {
    initialize(path, rng);
    let mut owner = PreparedArchiveBootstrapStore::open(path, SECRET, rng, archive(pool)).unwrap();
    owner.retry(rng).unwrap();
    drop(owner.into_store().unwrap());
}
fn open(
    path: &Path,
    pool: &CriticalCatalogReadPool,
    rng: &mut ChaCha20Rng,
) -> PreparedV2CurrentLayoutStore {
    PreparedV2CurrentLayoutStore::open(path, SECRET, rng, policy(pool)).unwrap()
}
fn private(owner: &PreparedV2CurrentLayoutStore) -> &ServerStore {
    owner.store.as_deref().expect("existing ID2 fixture")
}
fn fault(owner: &PreparedV2CurrentLayoutStore, point: u8) {
    private(owner)
        .critical_namespace_fault
        .store(point, Ordering::SeqCst);
}
struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("retry/refusal reached RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("retry/refusal reached RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("retry/refusal reached RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("retry/refusal reached RNG")
    }
}
impl CryptoRng for NeverRng {}

#[test]
fn current_cached_mount_check_never_constructs_a_revocation_observer() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x33a1);
    initialize(directory.path(), &mut rng);
    let mut prepared = open(directory.path(), &shared, &mut rng);
    prepared.retry(&mut rng).unwrap();
    let ready = prepared.into_current_layout_store().unwrap();
    crate::store::chat_admission::take_revocation_observer_constructions();
    assert!(ready.cached_store().is_ok());
    assert_eq!(
        crate::store::chat_admission::take_revocation_observer_constructions(),
        0,
        "a final cached edge must not allocate Authority or subscribe to watch"
    );
    ready.store.chat_mount.revoke();
    assert!(ready.cached_store().is_err());
    assert_eq!(
        crate::store::chat_admission::take_revocation_observer_constructions(),
        0
    );
}

#[test]
fn complete_private_id2_then_id3_handoff_keeps_same_root_and_is_not_public_store() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3301);
    initialize(directory.path(), &mut rng);
    let mut owner = open(directory.path(), &shared, &mut rng);
    assert!(
        owner.archive.is_some(),
        "ID2 still-private owner must finish first"
    );
    owner = owner.into_current_layout_store().unwrap_err();
    owner.retry(&mut rng).unwrap();
    let original = {
        let store = private(&owner);
        let root_guard = store.archive_bootstrap_root.lock().unwrap();
        let root = root_guard.as_ref().unwrap();
        let layout = store
            .current_layout
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .layout;
        assert_eq!(layout.vault, root.descriptor.vault);
        assert_eq!(layout.root, root.descriptor.root);
        store.registry_mount()
    };
    assert!(!directory.path().join(ARCHIVE_DECLARATION_SEED).exists());
    assert!(!directory.path().join(CURRENT_DECLARATION_SEED).exists());
    owner.retry(&mut NeverRng).unwrap();
    let ready = owner.into_current_layout_store().unwrap();
    assert!(Arc::ptr_eq(
        &original,
        &ready.checked_store().unwrap().registry_mount()
    ));
    assert!(ready
        .read_saved_bootstrap_source_data(b"missing genuine source", [1; 32])
        .is_err());
}

#[test]
fn actual_held_root_validation_keeps_the_original_marker_identity() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3380);
    initialize(directory.path(), &mut rng);
    let mut opening = open(directory.path(), &shared, &mut rng);
    opening.retry(&mut rng).unwrap();
    let ready = opening.into_current_layout_store().unwrap();
    // Setup is outside this bound: it measures only the already-opened Root proof. If a
    // future implementation recursively locks Root, timeout detaches a worker containing
    // ONLY its private fixture instead of hanging the entire suite in join().
    let (done, completed) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let directory = directory;
        let ready = ready; // drain Store/files before its private directory
        let store = ready.checked_store().unwrap();
        let held_root = store.archive_bootstrap_root.lock().unwrap();
        let root = held_root.as_ref().unwrap();
        // Use the actual original Root and mounted namespace with its mutex still held.
        store
            .validate_mounted_current_layout_under_archive_root(root)
            .unwrap();
        let marker = directory
            .path()
            .join(ARCHIVE_BOOTSTRAP_ROOT)
            .join(CURRENT_MARKER);
        let original = fs::read(&marker).unwrap();
        fs::rename(
            &marker,
            directory.path().join("held-original-current-marker"),
        )
        .unwrap();
        fs::write(&marker, &original).unwrap();
        assert!(store
            .validate_mounted_current_layout_under_archive_root(root)
            .is_err());
        assert_eq!(fs::read(marker).unwrap(), original);
        done.send(()).unwrap();
    });
    completed
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("original held-Root proof must finish without recursively locking Root");
    worker.join().unwrap();
}

#[test]
fn declaration_one_byte_short_refuses_before_rng_buffers_or_namespace_effects() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3370);
    prepare_two(directory.path(), &shared, &mut rng);
    let mut owner = open(directory.path(), &shared, &mut rng);
    let current = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let already_charged = shared.charged_for_test();
    // Leave exactly fee-1 available in the SAME real shared account. The actual private
    // declaration constructor must acquire its own debit before RNG/ciphertext/first effect.
    let blocker = shared
        .try_reserve(1024 * 1024 * 1024 - already_charged - codec::DECLARATION_FEE + 1)
        .unwrap();
    let entries = shared.read_entries_for_test();
    assert!(owner.retry(&mut NeverRng).is_err());
    assert!(owner.declaration.is_none());
    assert!(owner.declaration_fee.is_none());
    assert_eq!(shared.read_entries_for_test(), entries);
    assert_eq!(
        fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        current
    );
    assert!(!directory.path().join(CURRENT_DECLARATION_SEED).exists());
    assert!(!directory.path().join(Slot::Stage.name()).exists());
    assert!(!directory
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(CURRENT_MARKER)
        .exists());
    drop(blocker);
    owner.retry(&mut rng).unwrap();
    drop(owner.into_current_layout_store().unwrap());
    assert_eq!(shared.charged_for_test(), 0);
}

#[test]
fn complete_namespace3_default_readers_and_authentic_older_catalog_replay_refuse_before_cleaner() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3360);
    prepare_two(directory.path(), &shared, &mut rng);
    let older = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let mut owner = open(directory.path(), &shared, &mut rng);
    owner.retry(&mut rng).unwrap();
    let raw = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    {
        let store = private(&owner);
        let plain = unseal(&store.keys.db_key().unwrap(), &unframe(&raw).unwrap()).unwrap();
        let domain = store.chat_vault_domain().unwrap();
        assert!(catalog::parse(&plain, &domain).is_err());
        assert!(catalog::parse_bootstrap(&plain, &domain).is_err());
        assert!(catalog::parse_executing(&plain, &domain).is_err());
        assert!(catalog::parse_bootstrap_executing(&plain, &domain).is_err());
    }
    drop(owner.into_current_layout_store().unwrap());
    let old_stage = directory.path().join("chat-owner-pins.replace.tmp");
    fs::write(&old_stage, b"retain this older stage").unwrap();
    assert!(PreparedArchiveBootstrapStore::open(
        directory.path(),
        SECRET,
        &mut rng,
        archive(&shared)
    )
    .is_err());
    assert_eq!(fs::read(&old_stage).unwrap(), b"retain this older stage");
    fs::write(directory.path().join(Slot::Current.name()), &older).unwrap();
    assert!(PreparedArchiveBootstrapStore::open(
        directory.path(),
        SECRET,
        &mut rng,
        archive(&shared)
    )
    .is_err());
    assert!(PreparedV2CurrentLayoutStore::open(
        directory.path(),
        SECRET,
        &mut rng,
        policy(&shared)
    )
    .is_err());
    assert_eq!(fs::read(&old_stage).unwrap(), b"retain this older stage");
    assert_eq!(
        fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        older
    );
}

#[test]
fn namespace3_wrapper_really_reads_preexisting_independently_saved_source_on_new_physical_mount() {
    use crate::store::{ArchiveFounderBegin, ArchiveFounderInputs, ArchiveFounderPolicy};
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{Hub, ManualClock, PeerId};
    use catcoms_sync::owner_v2::FounderSnapshotLimits;
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3361);
    prepare_two(directory.path(), &shared, &mut rng);
    let mut opening =
        PreparedArchiveBootstrapStore::open(directory.path(), SECRET, &mut rng, archive(&shared))
            .unwrap();
    opening.retry(&mut rng).unwrap();
    let store = opening.into_store().unwrap();
    let device = MlsDevice::generate().unwrap();
    let signer = device.public_key_bytes().try_into().unwrap();
    let limits = FounderSnapshotLimits::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
    )
    .unwrap();
    let inputs = ArchiveFounderInputs::new(
        Hub::new().join(PeerId::from_u64(0x3361)),
        device,
        ChaCha20Rng::seed_from_u64(0x3361),
        Box::new(ManualClock::new(10)),
    );
    let mut creator = match store
        .prepare_archive_founder(
            [91; 32],
            "source before layout3",
            ArchiveFounderPolicy::new(
                limits,
                3,
                archive_bootstrap::founder_codec::PERSISTENT_CONTENT_BYTES,
            )
            .unwrap(),
            inputs,
        )
        .unwrap()
    {
        ArchiveFounderBegin::Original(creator) => creator,
        _ => panic!("genuine original required"),
    };
    creator.advance(&store, &mut rng).unwrap();
    let group = creator.group_data().unwrap().to_vec();
    let mut planning = creator.into_ram_plan(&store).unwrap();
    planning.save_founding_intent(&store, &mut rng).unwrap();
    let mut source = planning.into_source_preparation(&store).unwrap();
    source.save_source_terminal(&store, &mut rng).unwrap();
    drop(source);
    drop(store);
    let mut opening = open(directory.path(), &shared, &mut rng);
    opening.retry(&mut rng).unwrap();
    let ready = opening.into_current_layout_store().unwrap();
    let read = ready
        .read_saved_bootstrap_source_data(&group, signer)
        .unwrap();
    assert_eq!(
        read.data().view_data().context.spec_data().group,
        group.as_slice()
    );
    read.validate_original(ready.checked_store().unwrap())
        .unwrap();
    drop(ready);
    let mut next = open(directory.path(), &shared, &mut rng);
    next.retry(&mut NeverRng).unwrap();
    let next = next.into_current_layout_store().unwrap();
    assert!(
        read.validate_original(next.checked_store().unwrap())
            .is_err(),
        "new mount cannot revive the old paid read"
    );
    assert!(read.data().view_data().context.spec_data().signer == signer);
}

#[test]
fn unknown_current_seed_is_orphan_before_new_vault_and_actual_policy_requires_data_reader() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    fs::write(
        directory.path().join(CURRENT_DECLARATION_SEED),
        b"unknown orphan",
    )
    .unwrap();
    assert!(PreparedV2CurrentLayoutStore::open(
        directory.path(),
        SECRET,
        &mut NeverRng,
        policy(&shared)
    )
    .is_err());
    assert!(!directory.path().join("vault.bin").exists());
    let no_reader =
        ArchiveBootstrapCatalogPolicy::new(shared, 4 * archive_bootstrap::MAX_SEED_FRAMED).unwrap();
    assert!(V2CurrentLayoutPolicy::new(
        no_reader,
        CanonicalFilenameInventoryPolicy::new(1, 1).unwrap(),
        4 * codec::MAX_DECLARATION_FRAME
    )
    .is_err());
    assert!(CanonicalFilenameInventoryPolicy::new(0, 1).is_err());
}

#[test]
fn exact_warm_prefix_failures_retry_without_new_rng_ciphertext_or_root() {
    for point in [212, 213, 214, 215, 216, 217, 218] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x3310 + point as u64);
        prepare_two(directory.path(), &shared, &mut rng);
        let mut owner = open(directory.path(), &shared, &mut rng);
        fault(&owner, point);
        assert!(owner.retry(&mut rng).is_err());
        let declaration = owner.declaration.as_ref().unwrap();
        let bytes = declaration.candidate.to_vec();
        let marker = declaration.marker.to_vec();
        let seed = declaration.seed.to_vec();
        fault(&owner, 0);
        owner.retry(&mut NeverRng).unwrap();
        let declaration = owner.declaration.as_ref().unwrap();
        assert_eq!(declaration.candidate.as_slice(), bytes);
        assert_eq!(declaration.marker.as_slice(), marker);
        assert_eq!(declaration.seed.as_slice(), seed);
        assert_eq!(
            fs::read(directory.path().join(Slot::Current.name())).unwrap(),
            bytes
        );
        assert!(!directory.path().join(CURRENT_DECLARATION_SEED).exists());
        drop(owner.into_current_layout_store().unwrap());
    }
}

#[test]
fn complete_independent_seed_repairs_cold_marker_stage_and_postrename_boundaries() {
    for point in [213, 214, 215, 216, 217, 218] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x3320 + point as u64);
        prepare_two(directory.path(), &shared, &mut rng);
        let mut owner = open(directory.path(), &shared, &mut rng);
        fault(&owner, point);
        assert!(owner.retry(&mut rng).is_err());
        let candidate = owner.declaration.as_ref().unwrap().candidate.to_vec();
        drop(owner);
        let mut unchanged = rng.clone();
        let mut reopened = open(directory.path(), &shared, &mut rng);
        reopened.retry(&mut NeverRng).unwrap();
        assert_eq!(rng.next_u64(), unchanged.next_u64());
        assert_eq!(
            fs::read(directory.path().join(Slot::Current.name())).unwrap(),
            candidate
        );
        assert!(!directory.path().join(CURRENT_DECLARATION_SEED).exists());
        drop(reopened.into_current_layout_store().unwrap());
    }
}

#[test]
fn partial_first_seed_and_simultaneous_other_declaration_are_preserved_refused() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3331);
    prepare_two(directory.path(), &shared, &mut rng);
    let mut owner = open(directory.path(), &shared, &mut rng);
    fault(&owner, 212);
    assert!(owner.retry(&mut rng).is_err());
    let old = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let partial = fs::read(directory.path().join(CURRENT_DECLARATION_SEED)).unwrap();
    drop(owner);
    assert!(PreparedV2CurrentLayoutStore::open(
        directory.path(),
        SECRET,
        &mut rng,
        policy(&shared)
    )
    .is_err());
    assert_eq!(
        fs::read(directory.path().join(CURRENT_DECLARATION_SEED)).unwrap(),
        partial
    );
    assert_eq!(
        fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        old
    );
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3332);
    prepare_two(directory.path(), &shared, &mut rng);
    let mut owner = open(directory.path(), &shared, &mut rng);
    fault(&owner, 213);
    assert!(owner.retry(&mut rng).is_err());
    let complete = fs::read(directory.path().join(CURRENT_DECLARATION_SEED)).unwrap();
    fs::write(
        directory.path().join(ARCHIVE_DECLARATION_SEED),
        b"foreign simultaneous declaration",
    )
    .unwrap();
    fault(&owner, 0);
    assert!(owner.retry(&mut NeverRng).is_err());
    drop(owner);
    assert!(PreparedV2CurrentLayoutStore::open(
        directory.path(),
        SECRET,
        &mut rng,
        policy(&shared)
    )
    .is_err());
    assert_eq!(
        fs::read(directory.path().join(CURRENT_DECLARATION_SEED)).unwrap(),
        complete
    );
    assert_eq!(
        fs::read(directory.path().join(ARCHIVE_DECLARATION_SEED)).unwrap(),
        b"foreign simultaneous declaration"
    );
}

#[test]
fn stage_or_marker_tamper_after_large_stage_work_preserves_original_current_and_seed() {
    for changed in 0..4 {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x3340 + changed);
        prepare_two(directory.path(), &shared, &mut rng);
        let mut owner = open(directory.path(), &shared, &mut rng);
        let original = fs::read(directory.path().join(Slot::Current.name())).unwrap();
        let path = if changed == 0 {
            directory.path().join(Slot::Stage.name())
        } else if changed == 3 {
            directory.path().join(CURRENT_DECLARATION_SEED)
        } else {
            directory
                .path()
                .join(ARCHIVE_BOOTSTRAP_ROOT)
                .join(CURRENT_MARKER)
        };
        *private(&owner)
            .owner_evidence
            .current_after_namespace_stage
            .lock()
            .unwrap() = Some(Box::new(move || {
            let mut bytes = fs::read(&path).unwrap();
            if changed >= 2 {
                fs::rename(&path, path.with_extension("retained")).unwrap();
            } else {
                bytes[0] ^= 1;
            }
            fs::write(&path, bytes).unwrap();
        }));
        assert!(owner.retry(&mut rng).is_err());
        assert_eq!(
            fs::read(directory.path().join(Slot::Current.name())).unwrap(),
            original
        );
        assert!(directory.path().join(CURRENT_DECLARATION_SEED).is_file());
        assert!(!owner.complete);
    }
}

#[test]
fn marker_tamper_after_large_seed_verification_cannot_destroy_independent_repair_source() {
    for replace in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x3351);
        prepare_two(directory.path(), &shared, &mut rng);
        let mut owner = open(directory.path(), &shared, &mut rng);
        let marker = directory
            .path()
            .join(ARCHIVE_BOOTSTRAP_ROOT)
            .join(CURRENT_MARKER);
        *private(&owner)
            .owner_evidence
            .current_after_namespace_seed_verification
            .lock()
            .unwrap() = Some(Box::new(move || {
            let mut bytes = fs::read(&marker).unwrap();
            if replace {
                fs::rename(&marker, marker.with_extension("retained")).unwrap();
            } else {
                bytes[0] ^= 1;
            }
            fs::write(marker, bytes).unwrap();
        }));
        assert!(owner.retry(&mut rng).is_err());
        let seed = fs::read(directory.path().join(CURRENT_DECLARATION_SEED)).unwrap();
        assert_eq!(
            seed.as_slice(),
            owner.declaration.as_ref().unwrap().seed.as_slice()
        );
        assert!(!owner.complete);
    }
}

#[test]
fn actual_join_pause_resume_migration_and_first_arm_preserve_complete_namespace3_payload() {
    use crate::store::{CheckpointJoinSubmissionStatus, OwnerEvidenceBudget};
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{Hub, ManualClock, PeerId};
    let source_dir = tempfile::tempdir().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3371);
    let source = ServerStore::open(source_dir.path(), SECRET, &mut rng)
        .unwrap()
        .into_critical_namespace_store(SECRET, &mut rng)
        .unwrap();
    let budget = OwnerEvidenceBudget::checkpoint(
        8,
        256 * 1024 * 1024,
        256 * 1024 * 1024,
        24 * 1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap();
    source.configure_owner_evidence_budget(budget).unwrap();
    let hub = Hub::new();
    let mut founder = source
        .prepare_owner_evidence_founder(
            hub.join(PeerId::from_u64(0x3371)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0x3371),
            Box::new(ManualClock::new(1_000)),
            "actual checkpoint source",
            9,
        )
        .unwrap();
    let owner = founder.retry(&source, &mut rng).unwrap();
    let invite = owner
        .mint_owner_checkpoint_invite(&source, [71; 16], 100_000, vec![])
        .unwrap();
    prepare_two(directory.path(), &shared, &mut rng);
    let execution = V2CurrentLayoutPolicy::new(
        archive(&shared)
            .with_checkpoint_join_execution(2, 10_000)
            .unwrap(),
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * codec::MAX_DECLARATION_FRAME,
    )
    .unwrap();
    let mut opening =
        PreparedV2CurrentLayoutStore::open(directory.path(), SECRET, &mut rng, execution).unwrap();
    opening.retry(&mut rng).unwrap();
    let ready = opening.into_current_layout_store().unwrap();
    let store = ready.checked_store().unwrap();
    store.configure_owner_evidence_budget(budget).unwrap();
    let before = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let key = store.keys.db_key().unwrap();
    let plain = Zeroizing::new(unseal(&key, &unframe(&before).unwrap()).unwrap());
    let payload = store
        .parse_configured_join_catalog(&plain)
        .unwrap()
        .current_payload
        .unwrap()
        .to_vec();
    let mut prepared = store
        .prepare_journalled_owner_checkpoint_join(
            &source,
            &owner,
            MlsDevice::generate().unwrap(),
            invite,
            &mut rng,
        )
        .unwrap();
    prepared.save_pending(store, &mut rng).unwrap();
    prepared.pause_pending(store, &mut rng).unwrap();
    prepared.resume_pending(store, &mut rng).unwrap();
    let activated = prepared
        .activate_original_transport(
            store,
            hub.join(PeerId::from_u64(0x3372)),
            PeerId::from_u64(0x3371),
        )
        .unwrap();
    let mut network = activated.into_network_execution(store).unwrap();
    assert_eq!(
        network
            .ensure_submission_armed(store, &mut rng, Arc::new(ManualClock::new(1_000)))
            .unwrap(),
        CheckpointJoinSubmissionStatus::Armed
    );
    let updated = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let updated_plain = Zeroizing::new(unseal(&key, &unframe(&updated).unwrap()).unwrap());
    let parsed = store.parse_configured_join_catalog(&updated_plain).unwrap();
    assert_eq!(parsed.current_payload.unwrap(), payload);
    assert_eq!(
        parsed.join_version,
        Some(super::super::pending_join::execution_payload::VERSION)
    );
    let row = &parsed.rows.iter().flatten().next().unwrap().metadata;
    assert_eq!(row.execution.unwrap().limit, 2);
    assert_eq!(row.execution.unwrap().spent, 1);
    drop(network);
}

mod fresh_founder;
mod governance_capability;
mod member_outbox;
mod predecessor_fixtures;
