//! Real complete SourceSaved/head/scope remount, actual new claim/owner/registration and
//! actual same restored node. No read receipt, publisher or success watch is fabricated.
use super::*;
use crate::store::critical_namespaces::current_layout::cold_read::tests::{fixture, ColdFixture};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use std::sync::atomic::Ordering;

type Preparation = PreparedColdV2GenesisCurrent<MemNetwork, ChaCha20Rng>;
struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("unexpected RNG on saved replay")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("unexpected RNG on saved replay")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("unexpected RNG on saved replay")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("unexpected RNG on saved replay")
    }
}
impl CryptoRng for NeverRng {}
fn caller(
    number: u64,
) -> (
    Preparation,
    V2CurrentLayoutStore,
    ChaCha20Rng,
    tempfile::TempDir,
) {
    let ColdFixture {
        source,
        layout,
        rng,
        directory,
        ..
    } = fixture(number, false);
    let preparation = Preparation::new(
        source,
        Arc::new(Hub::new().join(PeerId::from_u64(number))),
        ChaCha20Rng::seed_from_u64(number),
        Arc::new(ManualClock::new(20)),
    );
    (preparation, layout, rng, directory)
}

#[test]
fn complete_cold_owner_binds_and_moves_the_same_original_seed_once() {
    let (mut prepared, layout, mut rng, _directory) = caller(0xcf01);
    let original_read = prepared.graph.source().unwrap().source().observe_original();
    let original_sdk = prepared
        .graph
        .source()
        .unwrap()
        .seed()
        .observe_original_seed_data();
    let original_source = prepared.graph.source().unwrap().data().source;
    prepared.retry(&layout, &mut rng).unwrap();
    assert_eq!(
        prepared.scope.as_ref().unwrap().progress_data(),
        ColdV2CurrentProgressData::CurrentBound
    );
    let writer = prepared.writer.as_ref().unwrap().observe_original();
    let saved = prepared
        .receipt
        .as_ref()
        .unwrap()
        .cached_current_for_test()
        .unwrap();
    let owner = prepared.scope.as_ref().unwrap().owner_identity();
    let nonce = prepared.scope.as_ref().unwrap().nonce();
    prepared.retry(&layout, &mut NeverRng).unwrap();
    assert!(Arc::ptr_eq(
        &saved,
        &prepared
            .receipt
            .as_ref()
            .unwrap()
            .cached_current_for_test()
            .unwrap()
    ));
    assert!(Arc::ptr_eq(
        &owner,
        &prepared.scope.as_ref().unwrap().owner_identity()
    ));
    assert_eq!(nonce, prepared.scope.as_ref().unwrap().nonce());
    original_read
        .require_same_original(prepared.graph.source().unwrap().source())
        .unwrap();
    original_sdk
        .require_same_seed_data(prepared.graph.source().unwrap().seed())
        .unwrap();
    let current = prepared.take_current(&layout).unwrap().unwrap();
    assert_eq!(current.data().source, original_source);
    original_sdk
        .require_same_seed_data(current.node.borrow_genesis_seed_data())
        .unwrap();
    current.validate_original(&layout).unwrap();
    drop(prepared);
    current.validate_original(&layout).unwrap();
    drop(current);
    assert!(writer.is_cancelled());
}

#[test]
fn cached_root_receipt_error_then_unrelated_dirty_write_recovers_same_generation() {
    let (mut prepared, layout, mut rng, directory) = caller(0xcf02);
    let store = layout.checked_store().unwrap();
    let marker = store
        .dir
        .join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(crate::store::critical_namespaces::current_layout::CURRENT_MARKER);
    let bytes = std::fs::read(&marker).unwrap();
    let displaced = directory.path().join("held-cold-caller-marker");
    let changed = marker.clone();
    let held = displaced.clone();
    prepared.after_receipt_allocation = Some(Box::new(move || {
        std::fs::rename(&changed, &held).unwrap();
        std::fs::write(&changed, bytes).unwrap();
    }));
    assert!(prepared.retry(&layout, &mut rng).is_err());
    assert_eq!(
        prepared.scope.as_ref().unwrap().progress_data(),
        ColdV2CurrentProgressData::Unbound
    );
    let cached = prepared
        .receipt
        .as_ref()
        .unwrap()
        .cached_current_for_test()
        .unwrap();
    let generation = prepared.writer.as_ref().unwrap().observe_original();
    let owner = prepared.scope.as_ref().unwrap().owner_identity();
    let nonce = prepared.scope.as_ref().unwrap().nonce();
    std::fs::remove_file(&marker).unwrap();
    std::fs::rename(&displaced, &marker).unwrap();

    let other = store
        .capture_chat_admission(
            crate::store::ChatAdmissionKey::new(b"real cold caller other scope", [0x74; 32])
                .unwrap(),
        )
        .unwrap();
    store
        .chat_fence_pre_rename_failure
        .store(true, Ordering::SeqCst);
    assert!(store
        .prepare_legacy_chat_admission(&other, b"same real other scope", &mut rng)
        .is_err());
    // The private Root receipt already exists, but Scope never got it. Do not remake the
    // writer, Head or Arc to recover; general inventory DATA repair must unblock this phase.
    prepared.after_receipt_allocation = Some(Box::new(|| panic!("cached receipt reallocated")));
    prepared.retry(&layout, &mut rng).unwrap();
    assert!(Arc::ptr_eq(
        &cached,
        &prepared
            .receipt
            .as_ref()
            .unwrap()
            .cached_current_for_test()
            .unwrap()
    ));
    prepared
        .writer
        .as_ref()
        .unwrap()
        .require_same_observation(&generation)
        .unwrap();
    assert!(Arc::ptr_eq(
        &owner,
        &prepared.scope.as_ref().unwrap().owner_identity()
    ));
    assert_eq!(nonce, prepared.scope.as_ref().unwrap().nonce());
    // The actual issued other row is still present and already clean: no RNG is allowed.
    store
        .prepare_legacy_chat_admission(&other, b"same real other scope", &mut NeverRng)
        .unwrap();
    prepared
        .take_current(&layout)
        .unwrap()
        .unwrap()
        .validate_original(&layout)
        .unwrap();
}

#[test]
fn cold_node_is_retained_but_never_extracted_after_pair_cancellation() {
    let (mut prepared, layout, mut rng, _directory) = caller(0xcf03);
    prepared.after_pair = Some(Box::new(|store| store.chat_mount_revoker().revoke()));
    assert!(prepared.retry(&layout, &mut rng).is_err());
    assert!(matches!(prepared.graph, Graph::Node { .. }));
    assert!(prepared.parts.is_none() && prepared.inputs.is_none());
    assert!(prepared.take_current(&layout).is_err());
    assert!(prepared.writer.is_some() && prepared.scope.is_some());
}

#[test]
fn cold_final_proof_follows_actual_temporary_credit_refund() {
    let (mut prepared, layout, mut rng, _directory) = caller(0xcf04);
    prepared.after_temporary_refund = Some(Box::new(|store| store.chat_mount_revoker().revoke()));
    assert!(prepared.retry(&layout, &mut rng).is_err());
    assert!(matches!(prepared.graph, Graph::Node { .. }));
    assert!(prepared.take_current(&layout).is_err());
}

#[test]
fn cold_saved_success_cannot_hide_a_replaced_marker_at_extraction() {
    let (mut prepared, layout, mut rng, directory) = caller(0xcf05);
    prepared.retry(&layout, &mut rng).unwrap();
    let store = layout.checked_store().unwrap();
    let marker = store
        .dir
        .join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(crate::store::critical_namespaces::current_layout::CURRENT_MARKER);
    let bytes = std::fs::read(&marker).unwrap();
    let displaced = directory.path().join("held-marker-after-cold-success");
    std::fs::rename(&marker, &displaced).unwrap();
    std::fs::write(&marker, bytes).unwrap();
    assert!(prepared.take_current(&layout).is_err());
    assert!(matches!(prepared.graph, Graph::Node { .. }));
    std::fs::remove_file(&marker).unwrap();
    std::fs::rename(&displaced, &marker).unwrap();
    prepared
        .take_current(&layout)
        .unwrap()
        .unwrap()
        .validate_original(&layout)
        .unwrap();
}
