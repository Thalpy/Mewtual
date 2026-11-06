//! Actual-source pre-seed scope and retained-custody regressions. No test
//! token, watch, decoded Saved proof or fabricated V2 Current stands in for original custody.
use super::*;
use crate::store::{
    ArchiveBootstrapCatalogPolicy, ArchiveFounderBegin, ArchiveFounderInputs, ArchiveFounderPolicy,
    CanonicalFilenameInventoryPolicy, GenesisSeedBudget, PreparedArchiveBootstrapStore,
    PreparedV2CurrentLayoutStore, RetainedBootstrapGenesisSeed, V2CurrentLayoutPolicy,
    V2CurrentLayoutStore,
};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use catcoms_sync::owner_v2::FounderSnapshotLimits;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::cell::Cell;

const SECRET: &[u8] = b"actual V2 scope source fixture";
const CAPACITY: usize = 1024 * 1024 * 1024;

struct Fixture {
    // Actual graph/source/read payments drain before the physical Store and its final fees.
    source: RetainedBootstrapGenesisSeed,
    layout: V2CurrentLayoutStore,
    pool: CriticalCatalogReadPool,
    rng: ChaCha20Rng,
    _directory: tempfile::TempDir,
}
impl Fixture {
    fn store(&self) -> &ServerStore {
        self.layout.checked_store().unwrap()
    }
    fn source_hash(&self) -> blake3::Hash {
        blake3::hash(self.source.source().data().view_data().snapshot)
    }
    fn key(&self, signer: [u8; 32]) -> ChatAdmissionKey {
        ChatAdmissionKey::new(self.source.data().context.spec_data().group, signer).unwrap()
    }
}

fn archive(pool: &CriticalCatalogReadPool) -> ArchiveBootstrapCatalogPolicy {
    ArchiveBootstrapCatalogPolicy::new(
        pool.clone(),
        4 * critical_namespaces::archive_bootstrap::MAX_SEED_FRAMED,
    )
    .unwrap()
    .with_bootstrap_source_data_read()
}
fn fixture(seed: u64) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    drop(
        ServerStore::open(directory.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    let pool = CriticalCatalogReadPool::new(CAPACITY).unwrap();
    let mut first =
        PreparedArchiveBootstrapStore::open(directory.path(), SECRET, &mut rng, archive(&pool))
            .unwrap();
    first.retry(&mut rng).unwrap();
    let old_store = first.into_store().unwrap();
    let device = MlsDevice::generate().unwrap();
    let signer: [u8; 32] = device.public_key_bytes().try_into().unwrap();
    let limits = FounderSnapshotLimits::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
    )
    .unwrap();
    let policy = ArchiveFounderPolicy::new(
        limits,
        3,
        critical_namespaces::archive_bootstrap::founder_codec::PERSISTENT_CONTENT_BYTES,
    )
    .unwrap();
    let inputs = ArchiveFounderInputs::new(
        Hub::new().join(PeerId::from_u64(seed)),
        device,
        ChaCha20Rng::seed_from_u64(seed),
        Box::new(ManualClock::new(10)),
    );
    let mut creator = match old_store
        .prepare_archive_founder([0x51; 32], "actual scope source", policy, inputs)
        .unwrap()
    {
        ArchiveFounderBegin::Original(creator) => creator,
        _ => panic!("fixture must perform the actual original creation debit"),
    };
    creator.advance(&old_store, &mut rng).unwrap();
    let group = creator.group_data().unwrap().to_vec();
    let mut plan = creator.into_ram_plan(&old_store).unwrap();
    plan.save_founding_intent(&old_store, &mut rng).unwrap();
    let mut original_source = plan.into_source_preparation(&old_store).unwrap();
    original_source
        .save_source_terminal(&old_store, &mut rng)
        .unwrap();
    drop(original_source);
    drop(old_store);

    // A NEW physical declaration/read/SDK attempt. No old warm owner or write arm crosses
    // the mount; the source is real SourceSaved, not a decoded receipt supplied by the test.
    let policy = V2CurrentLayoutPolicy::new(
        archive(&pool),
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * critical_namespaces::archive_current_namespace_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap();
    let mut opening =
        PreparedV2CurrentLayoutStore::open(directory.path(), SECRET, &mut rng, policy).unwrap();
    opening.retry(&mut rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    let store = layout.checked_store().unwrap();
    let read = store
        .read_saved_bootstrap_source_data(&group, signer)
        .unwrap();
    let budget = GenesisSeedBudget::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap();
    let source = store.prepare_genesis_seed(read, budget).unwrap();
    Fixture {
        source,
        layout,
        pool,
        rng,
        _directory: directory,
    }
}

struct State {
    table: BTreeMap<ChatAdmissionKey, Record>,
    dirty: bool,
    owners: Vec<(ChatAdmissionKey, [u8; 32], u64, Arc<()>, bool)>,
}
fn state(store: &ServerStore) -> State {
    // Test-only observation, including AFTER the real mount is revoked. This directly reads
    // the inert metadata mutex; it neither issues nor bypasses an admission permit.
    let runtime = store.chat_mount.runtime.lock().unwrap();
    State {
        table: runtime.table.as_ref().unwrap().clone(),
        dirty: runtime.dirty,
        owners: runtime
            .owners
            .iter()
            .map(|(key, owner)| {
                (
                    key.clone(),
                    owner.nonce,
                    owner.revision,
                    owner.life.identity(),
                    owner.life.live(),
                )
            })
            .collect(),
    }
}
fn require_same_state(before: &State, store: &ServerStore) {
    let after = state(store);
    // Never Debug-print original private group/nonce/history bytes on an assertion failure.
    assert!(before.table == after.table, "scope table changed");
    assert_eq!(before.dirty, after.dirty, "dirty state changed");
    assert_eq!(
        before.owners.len(),
        after.owners.len(),
        "owner count changed"
    );
    for (old, next) in before.owners.iter().zip(&after.owners) {
        assert!(old.0 == next.0 && old.1 == next.1 && old.2 == next.2);
        assert!(Arc::ptr_eq(&old.3, &next.3), "owner identity replaced");
        assert_eq!(old.4, next.4, "owner negative changed");
    }
}
fn attempt(f: &Fixture, calls: &Cell<usize>) -> Result<(), AppError> {
    f.store()
        .with_vacant_v2_genesis_admission(&f.source.borrow_source_data(), |admission| {
            calls.set(calls.get() + 1);
            admission.validate_source_only()
        })
}

#[test]
fn genuine_paid_source_preseed_admission_is_read_only_and_releases_its_temporary_fee() {
    let f = fixture(0xe101);
    let before = state(f.store());
    assert!(before.table.is_empty() && before.owners.is_empty() && !before.dirty);
    let bytes = f.source_hash();
    let charged = f.pool.charged_for_test();
    let calls = Cell::new(0);
    attempt(&f, &calls).unwrap();
    assert_eq!(calls.get(), 1);
    require_same_state(&before, f.store());
    assert!(f.source_hash() == bytes);
    f.source.validate_original(f.store()).unwrap();
    assert_eq!(f.pool.charged_for_test(), charged);
}

#[test]
fn every_same_group_row_phase_and_signer_refuses_without_pin_or_head() {
    let mut f = fixture(0xe102);
    let original = f.source.data().context.spec_data().signer;
    let mut other = original;
    other[0] ^= 1;
    let calls = Cell::new(0);
    let bytes = f.source_hash();
    for signer in [original, other] {
        for phase in [Phase::Initializing, Phase::Current, Phase::Retired] {
            let key = f.key(signer);
            {
                // NEGATIVE inventory fixtures only. These rows issue no binding, watch,
                // Fresh/Saved/Current proof or SDK permission. Persist through the REAL
                // scope writer so the test does not depend on an artificially dirty table.
                let store = f.layout.checked_store().unwrap();
                let mut runtime = store.chat_mount.locked().unwrap();
                runtime.table.as_mut().unwrap().clear();
                runtime.table.as_mut().unwrap().insert(
                    key,
                    Record {
                        nonce: [0x22; 32],
                        revision: 9,
                        phase,
                        origin: Origin::Legacy,
                        provenance: [0x23; 32],
                    },
                );
                store
                    .flush_chat_admissions(&mut runtime, &mut f.rng)
                    .unwrap();
            }
            let before = state(f.store());
            let charged = f.pool.charged_for_test();
            assert!(attempt(&f, &calls).is_err());
            require_same_state(&before, f.store());
            assert!(f.source_hash() == bytes);
            assert_eq!(f.pool.charged_for_test(), charged);
        }
    }
    assert_eq!(calls.get(), 0, "row conflict must precede the callback");
}

#[test]
fn unrelated_group_rows_are_preserved_and_do_not_manufacture_a_same_group_conflict() {
    let mut f = fixture(0xe103);
    let mut group = f.source.data().context.spec_data().group.to_vec();
    group[0] ^= 1;
    let key = ChatAdmissionKey::new(&group, [0x24; 32]).unwrap();
    {
        let store = f.layout.checked_store().unwrap();
        let mut runtime = store.chat_mount.locked().unwrap();
        runtime.table.as_mut().unwrap().insert(
            key,
            Record {
                nonce: [0x25; 32],
                revision: 8,
                phase: Phase::Retired,
                origin: Origin::Legacy,
                provenance: [0x26; 32],
            },
        );
        store
            .flush_chat_admissions(&mut runtime, &mut f.rng)
            .unwrap();
    }
    let before = state(f.store());
    let calls = Cell::new(0);
    attempt(&f, &calls).unwrap();
    assert_eq!(calls.get(), 1);
    require_same_state(&before, f.store());
}

#[test]
fn actual_other_signer_initializing_owner_is_neither_renewed_nor_evicted() {
    let mut f = fixture(0xe104);
    let mut signer = f.source.data().context.spec_data().signer;
    signer[0] ^= 1;
    let key = f.key(signer);
    // Use the REAL generic legacy initializer to retain an actual unexposed owner. No
    // covering-save callback or Current result is fabricated, and no watch is built here.
    let store = f.layout.checked_store().unwrap();
    let capture = store.capture_chat_admission(key).unwrap();
    let original = store
        .prepare_legacy_chat_admission(&capture, b"retained legacy initialization", &mut f.rng)
        .unwrap();
    assert!(!original.reservation_observer().is_cancelled());
    let before = state(f.store());
    assert_eq!(before.owners.len(), 1);
    let calls = Cell::new(0);
    assert!(attempt(&f, &calls).is_err());
    assert_eq!(calls.get(), 0);
    require_same_state(&before, f.store());
    assert!(!original.reservation_observer().is_cancelled());
}

#[test]
fn one_byte_short_temporary_metadata_credit_preserves_source_and_scope() {
    let f = fixture(0xe105);
    let required = GROUP_BOUNDS_ALLOCATION_BYTES
        + 256
        + std::mem::size_of::<ChatAdmissionKey>()
        + std::mem::size_of::<VacantV2GenesisAdmission<'_>>()
        + 64;
    let baseline = f.pool.charged_for_test();
    let blocker = f
        .pool
        .try_reserve(CAPACITY - baseline - required + 1)
        .unwrap();
    let before = state(f.store());
    let bytes = f.source_hash();
    let calls = Cell::new(0);
    assert!(attempt(&f, &calls).is_err());
    assert_eq!(calls.get(), 0);
    require_same_state(&before, f.store());
    assert!(f.source_hash() == bytes);
    drop(blocker);
    assert_eq!(f.pool.charged_for_test(), baseline);
    attempt(&f, &calls).unwrap();
    assert_eq!(calls.get(), 1, "same original source remains usable");
}

#[test]
fn actual_mount_revocation_refuses_without_scope_or_source_effect() {
    let f = fixture(0xe106);
    // Retain the actual already-borrowed Store before cancellation. The layout wrapper's
    // normal accessor may correctly refuse afterward; that must not turn our assertion into
    // an unwrap panic before exercising the original scope entry itself.
    let store = f.store();
    let before = state(store);
    let bytes = f.source_hash();
    let charged = f.pool.charged_for_test();
    store.chat_mount.revoke();
    let calls = Cell::new(0);
    assert!(store
        .with_vacant_v2_genesis_admission(&f.source.borrow_source_data(), |admission| {
            calls.set(calls.get() + 1);
            admission.validate_source_only()
        })
        .is_err());
    assert_eq!(calls.get(), 0);
    require_same_state(&before, store);
    assert!(f.source_hash() == bytes);
    assert_eq!(f.pool.charged_for_test(), charged);
}

#[test]
fn actual_archive_pre_row_task_reserves_all_its_signers_but_allows_another_group() {
    let mut f = fixture(0xe107);
    let device = MlsDevice::generate().unwrap();
    let signer: [u8; 32] = device.public_key_bytes().try_into().unwrap();
    let limits = FounderSnapshotLimits::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
    )
    .unwrap();
    let policy = ArchiveFounderPolicy::new(
        limits,
        3,
        critical_namespaces::archive_bootstrap::founder_codec::PERSISTENT_CONTENT_BYTES,
    )
    .unwrap();
    let input = ArchiveFounderInputs::new(
        Hub::new().join(PeerId::from_u64(0xe108)),
        device,
        ChaCha20Rng::seed_from_u64(0xe108),
        Box::new(ManualClock::new(10)),
    );
    let store = f.layout.checked_store().unwrap();
    let mut creator = match store
        .prepare_archive_founder([0x52; 32], "actual other pre-row source", policy, input)
        .unwrap()
    {
        ArchiveFounderBegin::Original(original) => original,
        _ => panic!("actual separate original creation required"),
    };
    creator.advance(store, &mut f.rng).unwrap();
    let group = creator.group_data().unwrap().to_vec();
    assert!(group != f.source.data().context.spec_data().group);
    let mut plan = creator.into_ram_plan(store).unwrap();
    plan.prepare_ram_plan(store, &mut f.rng).unwrap();
    assert!(!plan.observer().is_cancelled());
    let mut other_signer = signer;
    other_signer[0] ^= 1;
    let queried = ChatAdmissionKey::new(&group, other_signer).unwrap();
    {
        let runtime = store.chat_mount.locked().unwrap();
        assert!(!runtime.predecessors.excludes_generic(&queried));
        assert!(runtime.predecessors.excludes_group(queried.group()));
        assert!(!runtime
            .predecessors
            .excludes_group(f.source.data().context.spec_data().group));
    }
    // This is an ACTUAL Archive registration, not a forged group collision or changed Fresh
    // witness. Query its real group with another key; exercise genuine source A separately.
    let before = state(f.store());
    let charged = f.pool.charged_for_test();
    let calls = Cell::new(0);
    attempt(&f, &calls).unwrap();
    require_same_state(&before, f.store());
    assert_eq!(calls.get(), 1);
    assert_eq!(f.pool.charged_for_test(), charged);
    assert!(!plan.observer().is_cancelled());
    plan.close();
    let runtime = f.store().chat_mount.locked().unwrap();
    assert!(runtime.predecessors.excludes_group(queried.group()));
}

#[test]
fn genuine_saved_seed_metadata_payment_follows_runtime_owner_after_all_callers_drop() {
    use crate::store::persistence::v2_genesis_activation::PreparedV2GenesisActivation;
    let mut f = fixture(0xe109);
    let source = f.source.borrow_source_data();
    let store = f.layout.checked_store().unwrap();
    let mut task = f.layout.prepare_genesis_group_namespace(&source).unwrap();
    task.retry_directory(store, &source).unwrap();
    let baseline = f.pool.charged_for_test();
    let mut activation = PreparedV2GenesisActivation::prepare(store, &source, &mut f.rng).unwrap();
    let seed = store
        .with_vacant_v2_genesis_admission(&source, |admission| {
            let guard = task.lock_vacant(admission)?;
            activation.save_seed(&guard)
        })
        .unwrap();
    let weak = Arc::downgrade(&seed);
    let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
    let mut slot = None;
    store
        .reserve_v2_genesis_initializing(
            &source,
            &task,
            seed.clone(),
            &mut slot,
            &buffers,
            &mut f.rng,
        )
        .unwrap();
    assert!(slot.as_ref().unwrap().inner.record.phase == Phase::Initializing);
    drop(slot);
    drop(seed);
    drop(activation);
    drop(buffers);
    assert_eq!(
        weak.strong_count(),
        1,
        "ONLY actual Runtime owner custody remains"
    );
    assert_eq!(
        f.pool.charged_for_test(),
        baseline + critical_namespaces::archive_current_codec::ACTIVATION_METADATA_FEE
    );
    {
        let runtime = store.chat_mount.locked().unwrap();
        let owner = runtime.owners.values().next().unwrap();
        assert!(owner.custody.is_some());
        assert!(owner.life.live());
    }
    // No Root resource owns a Store/sender/graph. A graph/read may remain after the real
    // physical Store drops, while its mount is negative and all saved-seed custody drains.
    drop(task);
    drop(f.layout);
    assert_eq!(weak.strong_count(), 0);
    drop(f.source);
    assert_eq!(f.pool.charged_for_test(), 0);
}
