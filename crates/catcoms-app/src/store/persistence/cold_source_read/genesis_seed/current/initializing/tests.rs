//! Real warm SourceSaved to seed to row1/head cut, full drain, NEW paid read/SDK, partial
//! fixed-file fixture, actual repair/task/owner/writer/Current2/node and remount.
#![cfg(windows)]
use super::*;
use crate::store::critical_namespaces::current_layout::cold_read::tests::{
    fixture_at_cut, ColdFixture, ColdFixtureCut,
};
use crate::store::critical_namespaces::{
    archive_bootstrap as archive, archive_current_namespace_codec as layout_codec,
};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::atomic::Ordering;

type Preparation = PreparedColdV2GenesisInitializing<MemNetwork, ChaCha20Rng>;

#[cfg(windows)]
struct PartialFixture {
    preparation: Preparation,
    layout: V2CurrentLayoutStore,
    rng: ChaCha20Rng,
    pool: crate::store::critical_namespaces::read_pool::CriticalCatalogReadPool,
    group: Vec<u8>,
    signer: [u8; 32],
    pin: std::path::PathBuf,
    head: std::path::PathBuf,
    stage: std::path::PathBuf,
    seed: std::path::PathBuf,
    original_pin: Vec<u8>,
    original_head: Vec<u8>,
    original_seed: Vec<u8>,
    directory: tempfile::TempDir,
}
#[cfg(windows)]
fn partial(number: u64) -> PartialFixture {
    let ColdFixture {
        source,
        layout,
        rng,
        directory,
        pool,
        group,
        signer,
    } = fixture_at_cut(number, ColdFixtureCut::ShortStage);
    let store = layout.checked_store().unwrap();
    let source_ref = source.borrow_source_data();
    let capture = PreparedColdV2GenesisRepairCapture::prepare(store, &source_ref).unwrap();
    let resources = capture.read_resources();
    let path = store
        .with_cold_v2_genesis_initializing_read(&source_ref, |a| {
            let guard = store.lock_cold_initializing_genesis_group(a, &resources)?;
            Ok(guard.directory_data().0.to_path_buf())
        })
        .unwrap();
    let pin = path.join("pin.bin");
    let seed = path.join("activation.seed");
    let head = source_ref.source().capture.target.join("head.bin");
    let stage = source_ref.source().capture.target.join("head.stage");
    let original_pin = std::fs::read(&pin).unwrap();
    let original_head = std::fs::read(&stage).unwrap();
    let original_seed = std::fs::read(&seed).unwrap();
    drop(capture);
    drop(resources);
    // The actual producer fault cut already left these partial files. No test rewrite,
    // fabricated selected handle or warm receipt is used to obtain the cold repair role.
    let preparation = Preparation::new(
        source,
        Arc::new(Hub::new().join(PeerId::from_u64(number))),
        ChaCha20Rng::seed_from_u64(number),
        Arc::new(ManualClock::new(20)),
    );
    PartialFixture {
        preparation,
        layout,
        rng,
        pool,
        group,
        signer,
        pin,
        head,
        stage,
        seed,
        original_pin,
        original_head,
        original_seed,
        directory,
    }
}

#[cfg(windows)]
#[test]
fn actual_partial_init1_repairs_exact_candidates_then_new_mount_restores_complete_current2() {
    let mut f = partial(0xc810);
    let source = f.preparation.graph.source().unwrap().data().source;
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    assert!(std::fs::read(&f.pin).unwrap().starts_with(&f.original_pin));
    assert!(std::fs::read(&f.head)
        .unwrap()
        .starts_with(&f.original_head));
    assert_eq!(std::fs::read(&f.seed).unwrap(), f.original_seed);
    assert!(!f.stage.exists());
    assert_eq!(f.preparation.graph.source().unwrap().data().source, source);
    let scope = f.preparation.scope.as_ref().unwrap();
    assert_eq!(scope.planned_revision_data(), Some(1));
    let nonce = scope.nonce();
    let owner = scope.owner_identity();
    let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    current.validate_original(&f.layout).unwrap();
    assert_eq!(current.scope.nonce(), nonce);
    assert!(Arc::ptr_eq(&owner, &current.scope.owner_identity()));
    drop(owner);
    drop(current);
    drop(f.preparation);
    drop(f.layout);
    // Independently NEW physical mount/read/SDK and Current2 capture, never a repaired
    // Head?ordinary ColdHead converter or reconstructed warm Saved receipt.
    let policy = crate::store::V2CurrentLayoutPolicy::new(
        crate::store::ArchiveBootstrapCatalogPolicy::new(
            f.pool.clone(),
            4 * archive::MAX_SEED_FRAMED,
        )
        .unwrap()
        .with_bootstrap_source_data_read(),
        crate::store::CanonicalFilenameInventoryPolicy::new(32, 144 * 1024 * 1024 + 1024 + 40)
            .unwrap(),
        4 * layout_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap();
    let mut opening = crate::store::PreparedV2CurrentLayoutStore::open(
        f.directory.path(),
        b"genuine task-free cold group reads",
        &mut f.rng,
        policy,
    )
    .unwrap();
    opening.retry(&mut f.rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    let read = layout
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let budget = crate::store::GenesisSeedBudget::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap();
    let seed = layout
        .checked_store()
        .unwrap()
        .prepare_genesis_seed(read, budget)
        .unwrap();
    let mut cold = super::super::cold::PreparedColdV2GenesisCurrent::new(
        seed,
        Arc::new(Hub::new().join(PeerId::from_u64(0xc811))),
        ChaCha20Rng::seed_from_u64(0xc811),
        Arc::new(ManualClock::new(30)),
    );
    cold.retry(&layout, &mut f.rng).unwrap();
    let cold = cold.take_current(&layout).unwrap().unwrap();
    cold.validate_original(&layout).unwrap();
    assert_eq!(cold.data().source, source);
    assert_eq!(std::fs::read(&f.seed).unwrap(), f.original_seed);
}

#[cfg(windows)]
#[test]
fn both_actual_scope1_covering_write_faults_preserve_partial_files_and_same_owner() {
    for parent_fault in [false, true] {
        let mut f = partial(0xc812 + u64::from(parent_fault));
        let store = f.layout.checked_store().unwrap();
        if parent_fault {
            store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
        } else {
            store
                .chat_fence_pre_rename_failure
                .store(true, Ordering::SeqCst);
        }
        assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
        assert_eq!(std::fs::read(&f.pin).unwrap(), f.original_pin);
        assert_eq!(std::fs::read(&f.stage).unwrap(), f.original_head);
        assert!(!f.head.exists());
        assert_eq!(std::fs::read(&f.seed).unwrap(), f.original_seed);
        let scope = f.preparation.scope.as_ref().unwrap();
        assert!(scope.saved_head().is_none());
        let owner = scope.owner_identity();
        let nonce = scope.nonce();
        let writer = f.preparation.writer.as_ref().unwrap().observe_original();
        let source = f.preparation.graph.source().unwrap().data().source;
        f.preparation.retry(&f.layout, &mut f.rng).unwrap();
        assert!(Arc::ptr_eq(
            &owner,
            &f.preparation.scope.as_ref().unwrap().owner_identity()
        ));
        assert_eq!(nonce, f.preparation.scope.as_ref().unwrap().nonce());
        f.preparation
            .writer
            .as_ref()
            .unwrap()
            .require_same_observation(&writer)
            .unwrap();
        assert_eq!(f.preparation.graph.source().unwrap().data().source, source);
        f.preparation
            .take_current(&f.layout)
            .unwrap()
            .unwrap()
            .validate_original(&f.layout)
            .unwrap();
    }
}

#[cfg(windows)]
#[test]
fn repaired_head_is_latched_before_scope2_failure_and_never_reissued_on_retry() {
    let mut f = partial(0xc814);
    f.preparation.after_head = Some(Box::new(|store| {
        store
            .chat_fence_pre_rename_failure
            .store(true, Ordering::SeqCst)
    }));
    assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
    let scope = f.preparation.scope.as_ref().unwrap();
    let head = scope.saved_head().unwrap().clone();
    let owner = scope.owner_identity();
    let nonce = scope.nonce();
    assert!(scope.current_binding().is_err());
    assert!(std::fs::read(&f.head)
        .unwrap()
        .starts_with(&f.original_head));
    f.preparation.after_head = Some(Box::new(|_| panic!("repaired Head callback replay")));
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let scope = f.preparation.scope.as_ref().unwrap();
    assert!(Arc::ptr_eq(&head, scope.saved_head().unwrap()));
    assert!(Arc::ptr_eq(&owner, &scope.owner_identity()));
    assert_eq!(nonce, scope.nonce());
    f.preparation
        .take_current(&f.layout)
        .unwrap()
        .unwrap()
        .validate_original(&f.layout)
        .unwrap();
}

#[test]
fn saved_repaired_head_survives_lost_return_and_reuses_original_after_marker_restore() {
    let mut f = partial(0xc815);
    let marker = f
        .layout
        .checked_store()
        .unwrap()
        .dir
        .join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(crate::store::critical_namespaces::current_layout::CURRENT_MARKER);
    let original = std::fs::read(&marker).unwrap();
    let changed = marker.clone();
    let mut damaged = original.clone();
    damaged[0] ^= 1;
    crate::store::persistence::v2_genesis_activation::initializing_repair::after_receipt_for_test(
        Box::new(move || {
            std::fs::write(changed, damaged).unwrap();
        }),
    );
    assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
    let scope = f.preparation.scope.as_ref().unwrap();
    assert!(scope.saved_head().is_none());
    let owner = scope.owner_identity();
    let nonce = scope.nonce();
    let complete = std::fs::read(&f.head).unwrap();
    // Restore bytes in the SAME original marker object, never adopt an equal foreign ID.
    std::fs::write(&marker, original).unwrap();
    crate::store::persistence::v2_genesis_activation::initializing_repair::after_receipt_for_test(
        Box::new(|| panic!("cached repaired Head was recreated")),
    );
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    crate::store::persistence::v2_genesis_activation::initializing_repair::after_receipt_for_test(
        Box::new(|| {}),
    );
    assert_eq!(std::fs::read(&f.head).unwrap(), complete);
    let scope = f.preparation.scope.as_ref().unwrap();
    assert!(Arc::ptr_eq(&owner, &scope.owner_identity()));
    assert_eq!(nonce, scope.nonce());
    f.preparation
        .take_current(&f.layout)
        .unwrap()
        .unwrap()
        .validate_original(&f.layout)
        .unwrap();
}

mod lifecycle;

mod settlement;
