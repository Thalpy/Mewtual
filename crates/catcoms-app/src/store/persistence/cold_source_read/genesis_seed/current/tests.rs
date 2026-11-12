//! Genuine saved source -> physical namespace -> scope/head -> same restored node.
//! No decoded receipt, watch or fixture flag substitutes for the actual storage barriers.
use super::*;
use crate::store::{
    ArchiveBootstrapCatalogPolicy, ArchiveFounderBegin, ArchiveFounderInputs, ArchiveFounderPolicy,
    CanonicalFilenameInventoryPolicy, PreparedArchiveBootstrapStore, PreparedV2CurrentLayoutStore,
    V2CurrentLayoutPolicy,
};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_sync::owner_v2::FounderSnapshotLimits;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};

const SECRET: &[u8] = b"actual V2 Current owning caller";
const CAPACITY: usize = 1024 * 1024 * 1024;
type Preparation = PreparedV2GenesisCurrent<MemNetwork, ChaCha20Rng>;

struct Fixture {
    preparation: Preparation,
    layout: V2CurrentLayoutStore,
    pool: critical_namespaces::read_pool::CriticalCatalogReadPool,
    rng: ChaCha20Rng,
    _directory: tempfile::TempDir,
}
fn archive(
    pool: &critical_namespaces::read_pool::CriticalCatalogReadPool,
) -> ArchiveBootstrapCatalogPolicy {
    ArchiveBootstrapCatalogPolicy::new(
        pool.clone(),
        4 * critical_namespaces::archive_bootstrap::MAX_SEED_FRAMED,
    )
    .unwrap()
    .with_bootstrap_source_data_read()
}
fn fixture(seed: u64) -> Fixture {
    fixture_with(seed, false)
}
/// The same actual fixture; `member_outbox` additionally configures the concrete namespace5
/// handler (on its own independent pool), so the Current3 opening declares and mounts the
/// member-outbox parent. No member source, save or authority is created by the declaration.
fn fixture_with(seed: u64, member_outbox: bool) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    drop(
        ServerStore::open(directory.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    let pool = critical_namespaces::read_pool::CriticalCatalogReadPool::new(CAPACITY).unwrap();
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
        .prepare_archive_founder([0x52; 32], "actual Current source", policy, inputs)
        .unwrap()
    {
        ArchiveFounderBegin::Original(creator) => creator,
        _ => panic!("fixture must perform actual original creation"),
    };
    creator.advance(&old_store, &mut rng).unwrap();
    let group = creator.group_data().unwrap().to_vec();
    let mut plan = creator.into_ram_plan(&old_store).unwrap();
    plan.save_founding_intent(&old_store, &mut rng).unwrap();
    let mut source = plan.into_source_preparation(&old_store).unwrap();
    source.save_source_terminal(&old_store, &mut rng).unwrap();
    drop(source);
    drop(old_store);

    // Independently open the physical declaration, read SourceSaved and restore the SDK.
    // No warm source write arm or owner crosses this mount boundary.
    let mut policy = V2CurrentLayoutPolicy::new(
        archive(&pool),
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * critical_namespaces::archive_current_namespace_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap();
    if member_outbox {
        let member_pool =
            critical_namespaces::read_pool::CriticalCatalogReadPool::new(64 * 1024 * 1024).unwrap();
        policy = policy
            .with_member_outbox(
                crate::store::V2MemberOutboxPolicy::new(
                    member_pool,
                    256,
                    256 * 1024 * 1024,
                    1 << 20,
                    1,
                    critical_namespaces::member_outbox_layout::codec::PERSISTENT_MINIMUM,
                )
                .unwrap(),
            )
            .unwrap();
    }
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
    let restored = store.prepare_genesis_seed(read, budget).unwrap();
    let preparation = Preparation::new(
        restored,
        Arc::new(Hub::new().join(PeerId::from_u64(seed))),
        ChaCha20Rng::seed_from_u64(seed),
        Arc::new(ManualClock::new(20)),
    );
    Fixture {
        preparation,
        layout,
        pool,
        rng,
        _directory: directory,
    }
}

struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("RNG before actual payment")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("RNG before actual payment")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("RNG before actual payment")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("RNG before actual payment")
    }
}
impl CryptoRng for NeverRng {}

#[test]
fn actual_current_keeps_same_source_and_node_across_retries_and_extraction() {
    let mut f = fixture(0xee01);
    let original = f
        .preparation
        .graph
        .source()
        .unwrap()
        .source()
        .observe_original();
    let original_id = f.preparation.graph.source().unwrap().data().source;
    let original_sdk = f
        .preparation
        .graph
        .source()
        .unwrap()
        .seed()
        .observe_original_seed_data();
    assert!(f.preparation.take_current(&f.layout).unwrap().is_none());
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let task = f.preparation.task.as_ref().unwrap().observe_original();
    let writer = f.preparation.writer.as_ref().unwrap().observe_original();
    let charged = f.pool.charged_for_test();
    // A saved Current retry consumes no RNG and cannot reconstruct its node or artifacts.
    f.preparation.retry(&f.layout, &mut NeverRng).unwrap();
    let Graph::Node { node, .. } = &f.preparation.graph else {
        panic!("same node required")
    };
    original_sdk
        .require_same_seed_data(node.borrow_genesis_seed_data())
        .unwrap();
    original
        .require_same_original(f.preparation.graph.source().unwrap().source())
        .unwrap();
    assert_eq!(f.pool.charged_for_test(), charged);
    let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    assert_eq!(current.data().source, original_id);
    original_sdk
        .require_same_seed_data(current.node.borrow_genesis_seed_data())
        .unwrap();
    current.validate_original(&f.layout).unwrap();
    task.require_same_original(&current.task).unwrap();
    assert!(f.preparation.take_current(&f.layout).unwrap().is_none());
    drop(f.preparation);
    current.validate_original(&f.layout).unwrap();
    drop(current);
    assert!(writer.cancellation().is_cancelled());
}

#[test]
fn one_byte_short_wrapper_payment_preserves_original_inputs_without_rng_or_files() {
    let mut f = fixture(0xee02);
    let original = f
        .preparation
        .graph
        .source()
        .unwrap()
        .source()
        .observe_original();
    let baseline = f.pool.charged_for_test();
    let blocker = f
        .pool
        .try_reserve(CAPACITY - baseline - WRAPPER_FEE + 1)
        .unwrap();
    assert!(f.preparation.retry(&f.layout, &mut NeverRng).is_err());
    assert!(f.preparation.inputs.is_some());
    assert!(f.preparation.fee.is_none());
    assert!(f.preparation.activation.is_none());
    assert!(f.preparation.task.is_none());
    original
        .require_same_original(f.preparation.graph.source().unwrap().source())
        .unwrap();
    drop(blocker);
    assert_eq!(f.pool.charged_for_test(), baseline);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    f.preparation
        .take_current(&f.layout)
        .unwrap()
        .unwrap()
        .validate_original(&f.layout)
        .unwrap();
}

#[test]
fn cancellation_after_parts_preserves_exact_original_graph_and_runtime_parts() {
    let mut f = fixture(0xee03);
    let original = f
        .preparation
        .graph
        .source()
        .unwrap()
        .source()
        .observe_original();
    let revoked = f.layout.checked_store().unwrap().chat_mount_revoker();
    f.preparation.after_parts = Some(Box::new(move || revoked.revoke()));
    assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
    assert!(matches!(f.preparation.graph, Graph::Seed(_)));
    assert!(f.preparation.parts.is_some());
    assert!(f.preparation.inputs.is_none());
    original
        .require_same_original(f.preparation.graph.source().unwrap().source())
        .unwrap();
    assert!(f.preparation.take_current(&f.layout).unwrap().is_none());
    assert!(f.preparation.retry(&f.layout, &mut NeverRng).is_err());
}

#[test]
fn cancellation_after_pair_retains_node_but_never_returns_a_current() {
    let mut f = fixture(0xee04);
    let revoked = f.layout.checked_store().unwrap().chat_mount_revoker();
    f.preparation.after_pair = Some(Box::new(move || revoked.revoke()));
    assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
    assert!(matches!(f.preparation.graph, Graph::Node { .. }));
    assert!(f.preparation.parts.is_none());
    assert!(f.preparation.inputs.is_none());
    assert!(f.preparation.take_current(&f.layout).is_err());
}

#[test]
fn cancellation_after_temporary_refund_is_rechecked_before_current_extraction() {
    let mut f = fixture(0xee05);
    let revoked = f.layout.checked_store().unwrap().chat_mount_revoker();
    f.preparation.after_temporary_refund = Some(Box::new(move || revoked.revoke()));
    assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
    assert!(matches!(f.preparation.graph, Graph::Node { .. }));
    assert!(f.preparation.take_current(&f.layout).is_err());
}

#[test]
fn saved_success_cannot_be_extracted_after_its_mount_is_revoked() {
    let mut f = fixture(0xee06);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    f.layout
        .checked_store()
        .unwrap()
        .chat_mount_revoker()
        .revoke();
    assert!(f.preparation.take_current(&f.layout).is_err());
    assert!(matches!(f.preparation.graph, Graph::Node { .. }));
    assert!(f.preparation.scope.is_some() && f.preparation.writer.is_some());
}

#[test]
fn failed_initializing_flush_retries_the_same_scope_owner_and_saved_seed() {
    use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
    for after_rename in [false, true] {
        let mut f = fixture(0xee10 + u64::from(after_rename));
        let store = f.layout.checked_store().unwrap();
        if after_rename {
            store
                .chat_parent_sync_failure
                .store(true, std::sync::atomic::Ordering::SeqCst);
        } else {
            store
                .chat_fence_pre_rename_failure
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
        assert!(matches!(f.preparation.graph, Graph::Seed(_)));
        let scope = f.preparation.scope.as_ref().unwrap();
        let nonce = scope.nonce();
        let owner = scope.owner_identity();
        let seed = scope.activation().clone();
        assert!(f.preparation.writer.is_none());
        f.preparation.retry(&f.layout, &mut f.rng).unwrap();
        let scope = f.preparation.scope.as_ref().unwrap();
        assert_eq!(nonce, scope.nonce());
        assert!(Arc::ptr_eq(&owner, &scope.owner_identity()));
        assert!(Arc::ptr_eq(&seed, scope.activation()));
        assert_eq!(
            scope.current_binding().unwrap().planned_revision_data(),
            Some(1)
        );
        f.preparation
            .take_current(&f.layout)
            .unwrap()
            .unwrap()
            .validate_original(&f.layout)
            .unwrap();
    }
}

#[test]
fn failed_current_flush_retains_exact_saved_head_and_never_replays_its_callback() {
    use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
    use std::sync::atomic::Ordering;
    for after_rename in [false, true] {
        let mut f = fixture(0xee30 + u64::from(after_rename));
        f.preparation.after_head = Some(Box::new(move |store| {
            if after_rename {
                store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
            } else {
                store
                    .chat_fence_pre_rename_failure
                    .store(true, Ordering::SeqCst);
            }
        }));
        assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
        assert!(matches!(f.preparation.graph, Graph::Seed(_)));
        let scope = f.preparation.scope.as_ref().unwrap();
        let nonce = scope.nonce();
        let owner = scope.owner_identity();
        let head = scope.saved_head().unwrap().clone();
        let writer = f.preparation.writer.as_ref().unwrap().observe_original();
        // An erroneous second callback is observable, even if it only validates old files.
        f.preparation.after_head = Some(Box::new(|_| panic!("saved head callback replayed")));
        f.preparation.retry(&f.layout, &mut f.rng).unwrap();
        let scope = f.preparation.scope.as_ref().unwrap();
        assert_eq!(nonce, scope.nonce());
        assert!(Arc::ptr_eq(&owner, &scope.owner_identity()));
        assert!(Arc::ptr_eq(&head, scope.saved_head().unwrap()));
        f.preparation
            .writer
            .as_ref()
            .unwrap()
            .require_same_observation(&writer)
            .unwrap();
        f.preparation
            .take_current(&f.layout)
            .unwrap()
            .unwrap()
            .validate_original(&f.layout)
            .unwrap();
    }
}

#[test]
fn unrelated_failed_write_blocks_extraction_and_current_resync_preserves_its_row() {
    use crate::store::ChatAdmissionKey;
    use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
    use std::sync::atomic::Ordering;
    let mut f = fixture(0xee40);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let store = f.layout.checked_store().unwrap();
    let capture = store
        .capture_chat_admission(
            ChatAdmissionKey::new(b"actual unrelated pending scope", [0x61; 32]).unwrap(),
        )
        .unwrap();
    store
        .chat_fence_pre_rename_failure
        .store(true, Ordering::SeqCst);
    assert!(store
        .prepare_legacy_chat_admission(&capture, b"same other initialization", &mut f.rng)
        .is_err());
    // A previous successful return does not make a now-dirty latest global table current.
    assert!(f.preparation.take_current(&f.layout).is_err());
    assert!(matches!(f.preparation.graph, Graph::Node { .. }));
    let head = f
        .preparation
        .scope
        .as_ref()
        .unwrap()
        .saved_head()
        .unwrap()
        .clone();
    f.preparation.after_head = Some(Box::new(|_| panic!("Current resync reran head")));
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    assert!(Arc::ptr_eq(
        &head,
        f.preparation.scope.as_ref().unwrap().saved_head().unwrap()
    ));
    // The actual cached other-group initializer succeeds WITHOUT RNG only if resync kept
    // that exact issued row. Replacing the global table with our old view would fail here.
    let other = store
        .prepare_legacy_chat_admission(&capture, b"same other initialization", &mut NeverRng)
        .unwrap();
    assert!(!other.reservation_observer().is_cancelled());
    f.preparation
        .take_current(&f.layout)
        .unwrap()
        .unwrap()
        .validate_original(&f.layout)
        .unwrap();
}

type CloseArm = Arc<
    std::sync::Mutex<
        Option<(
            catcoms_rt::RequestCancellation,
            catcoms_rt::RequestCancellation,
            Arc<std::sync::atomic::AtomicBool>,
        )>,
    >,
>;

/// Host payload whose destruction observes the ACTUAL installed cancellation objects. This
/// is an R input, not a fabricated admission or writer; arm it only after real Current2.
struct ObservedRng {
    rng: ChaCha20Rng,
    arm: CloseArm,
    notice: Arc<std::sync::atomic::AtomicU8>,
}
impl RngCore for ObservedRng {
    fn next_u32(&mut self) -> u32 {
        self.rng.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        self.rng.fill_bytes(bytes);
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), rand_core::Error> {
        self.rng.try_fill_bytes(bytes)
    }
}
impl CryptoRng for ObservedRng {}
impl Drop for ObservedRng {
    fn drop(&mut self) {
        if let Some((writer, scope, wrapper_dropped)) = self.arm.lock().unwrap().take() {
            let correct = writer.is_cancelled()
                && scope.is_cancelled()
                && !wrapper_dropped.load(std::sync::atomic::Ordering::SeqCst);
            self.notice.store(
                if correct { 2 } else { 1 },
                std::sync::atomic::Ordering::SeqCst,
            );
        }
    }
}

#[test]
fn both_owning_drop_paths_close_actual_authority_before_runtime_payload_and_fee() {
    use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    for extract in [false, true] {
        let Fixture {
            mut preparation,
            layout,
            pool: _,
            mut rng,
            _directory,
        } = fixture(0xee20 + u64::from(extract));
        // Reuse the exact original seed and preexisting runtime inputs, changing only the
        // host-provided R type before any activation effect. No fresh graph/source capture.
        let Graph::Seed(seed) = std::mem::replace(&mut preparation.graph, Graph::Moving) else {
            panic!("new inert preparation owns seed")
        };
        let inputs = preparation.inputs.take().unwrap();
        drop(preparation);
        let arm = Arc::new(std::sync::Mutex::new(None));
        let notice = Arc::new(AtomicU8::new(0));
        let mut preparation = PreparedV2GenesisCurrent::new(
            seed,
            inputs.transport,
            ObservedRng {
                rng: inputs.rng,
                arm: arm.clone(),
                notice: notice.clone(),
            },
            inputs.clock,
        );
        preparation.retry(&layout, &mut rng).unwrap();
        let wrapper_dropped = Arc::new(AtomicBool::new(false));
        preparation
            .fee
            .as_mut()
            .unwrap()
            .observe_drop_for_test(wrapper_dropped.clone());
        *arm.lock().unwrap() = Some((
            preparation
                .writer
                .as_ref()
                .unwrap()
                .observe_original()
                .cancellation(),
            preparation.scope.as_ref().unwrap().reservation_observer(),
            wrapper_dropped.clone(),
        ));
        if extract {
            let current = preparation.take_current(&layout).unwrap().unwrap();
            drop(preparation);
            assert_eq!(
                notice.load(Ordering::SeqCst),
                0,
                "empty pending must not drop node"
            );
            drop(current);
        } else {
            drop(preparation);
        }
        assert_eq!(
            notice.load(Ordering::SeqCst),
            2,
            "writer/scope cancellation and wrapper payment must precede R cleanup"
        );
        assert!(
            wrapper_dropped.load(Ordering::SeqCst),
            "actual wrapper reservation must finally drop"
        );
    }
}

#[cfg(windows)]
mod seed_settlement;
mod settlement_scope;

#[cfg(windows)]
mod text_group_guard;
#[cfg(windows)]
mod text_lane_handoff;
#[cfg(windows)]
mod text_owner;
