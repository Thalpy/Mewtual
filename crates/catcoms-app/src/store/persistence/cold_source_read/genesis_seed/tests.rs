//! Actual paid cold source -> actual SDK/metadata capsule, authored UNRUN. Neither a
//! successful restore nor this resource wrapper is a Current/native publication result.
use super::super::tests::{fixture, Fixture};
use super::*;

fn budget() -> GenesisSeedBudget {
    GenesisSeedBudget::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap()
}

#[test]
fn actual_read_sdk_and_metadata_have_independent_same_pool_lifetimes() {
    let f = fixture(0xd001);
    let baseline = f.pool.charged_for_test();
    let read = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let limits = budget();
    let expected =
        baseline + READ_FEE + limits.metadata_bytes().unwrap() + limits.sdk.max_typed_bytes;
    let seed = f.store.prepare_genesis_seed(read, limits).unwrap();
    assert_eq!(f.pool.charged_for_test(), expected);
    assert_eq!(seed.data().source.as_bytes(), &f.source_id);
    assert_eq!(seed.data().context.spec_data().group, f.group.as_slice());
    assert_eq!(seed.data().context.spec_data().signer, f.signer);
    assert_eq!(seed.source().data().source_id_data(), seed.data().source);
    seed.validate_original(&f.store).unwrap();
    let Fixture { store, pool, .. } = f;
    drop(store);
    assert_eq!(pool.charged_for_test(), expected - baseline);
    // The actual SDK pair is still paid, but a DATA view cannot certify a live Current host.
    assert!(!seed.data().state.is_empty());
    drop(seed);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn one_byte_short_combined_payments_refuse_before_sdk_and_return_same_source_for_retry() {
    let f = fixture(0xd002);
    let read = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let identity = read.capture.clone();
    let baseline = f.pool.charged_for_test();
    let limits = budget();
    let required = limits.metadata_bytes().unwrap() + limits.sdk.max_typed_bytes;
    let pressure = f
        .pool
        .try_reserve(1024 * 1024 * 1024 - baseline - required + 1)
        .unwrap();
    SDK_ENTRIES.with(|entries| entries.set(0));
    let returned = f.store.prepare_genesis_seed(read, limits).unwrap_err();
    let (read, reason) = returned.into_parts();
    assert_eq!(reason, GenesisSeedRefusal::CapacityUnavailable);
    assert_eq!(SDK_ENTRIES.with(|entries| entries.get()), 0);
    assert!(Arc::ptr_eq(&read.capture, &identity));
    drop(identity);
    drop(pressure);
    assert_eq!(f.pool.charged_for_test(), baseline);
    // No read_saved_* call: retry the SAME opened source with genuinely new SDK payments.
    let seed = f.store.prepare_genesis_seed(read, limits).unwrap();
    assert_eq!(SDK_ENTRIES.with(|entries| entries.get()), 1);
    assert_eq!(seed.data().source.as_bytes(), &f.source_id);
}

#[test]
fn too_small_sdk_setup_work_releases_provider_payments_and_preserves_opened_source() {
    let f = fixture(0xd003);
    let read = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let identity = read.capture.clone();
    let baseline = f.pool.charged_for_test();
    let mut limits = budget();
    limits.sdk.max_work = 1;
    let (read, reason) = f
        .store
        .prepare_genesis_seed(read, limits)
        .unwrap_err()
        .into_parts();
    assert_eq!(reason, GenesisSeedRefusal::SeedUnavailable);
    assert!(Arc::ptr_eq(&read.capture, &identity));
    assert_eq!(f.pool.charged_for_test(), baseline);
    drop(identity);
    let seed = f.store.prepare_genesis_seed(read, budget()).unwrap();
    assert_eq!(seed.data().source.as_bytes(), &f.source_id);
}

#[test]
fn cancellation_after_fee_wait_prevents_sdk_and_refunds_only_the_unused_new_payments() {
    let f = fixture(0xd004);
    let read = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let baseline = f.pool.charged_for_test();
    let revoker = f.store.chat_mount_revoker();
    *f.store.genesis_before_sdk.lock().unwrap() = Some(Box::new(move || revoker.revoke()));
    SDK_ENTRIES.with(|entries| entries.set(0));
    let (read, reason) = f
        .store
        .prepare_genesis_seed(read, budget())
        .unwrap_err()
        .into_parts();
    assert_eq!(reason, GenesisSeedRefusal::SourceUnavailable);
    assert_eq!(SDK_ENTRIES.with(|entries| entries.get()), 0);
    assert_eq!(f.pool.charged_for_test(), baseline);
    assert!(read.validate_original(&f.store).is_err());
}

#[test]
fn control_replacement_after_real_sdk_decode_refuses_final_source_handoff() {
    let f = fixture(0xd005);
    let read = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let baseline = f.pool.charged_for_test();
    let current = control_path(
        &read.capture.root,
        &read.capture.invocation,
        FounderControlSlot::Intent,
    )
    .unwrap();
    let frame = fs::read(&current).unwrap();
    let replacement = frame.clone();
    let changed = current.clone();
    let displaced = f.path.path().join("held-original-after-genesis-sdk");
    *f.store.genesis_after_sdk.lock().unwrap() = Some(Box::new(move || {
        fs::rename(&changed, displaced).unwrap();
        fs::write(&changed, replacement).unwrap();
    }));
    SDK_ENTRIES.with(|entries| entries.set(0));
    let (read, reason) = f
        .store
        .prepare_genesis_seed(read, budget())
        .unwrap_err()
        .into_parts();
    assert_eq!(reason, GenesisSeedRefusal::SourceUnavailable);
    assert_eq!(SDK_ENTRIES.with(|entries| entries.get()), 1);
    assert_eq!(f.pool.charged_for_test(), baseline);
    assert_eq!(fs::read(current).unwrap(), frame);
    assert!(read.validate_original(&f.store).is_err());
}

#[test]
fn another_physical_store_cannot_use_the_original_sources_payment_or_mount() {
    let a = fixture(0xd006);
    let b = fixture(0xd007);
    let read = a
        .store
        .read_saved_bootstrap_source_data(&a.group, a.signer)
        .unwrap();
    let baseline_a = a.pool.charged_for_test();
    let baseline_b = b.pool.charged_for_test();
    let identity = read.capture.clone();
    SDK_ENTRIES.with(|entries| entries.set(0));
    let (read, reason) = b
        .store
        .prepare_genesis_seed(read, budget())
        .unwrap_err()
        .into_parts();
    assert_eq!(reason, GenesisSeedRefusal::SourceUnavailable);
    assert_eq!(SDK_ENTRIES.with(|entries| entries.get()), 0);
    assert!(Arc::ptr_eq(&read.capture, &identity));
    assert_eq!(a.pool.charged_for_test(), baseline_a);
    assert_eq!(b.pool.charged_for_test(), baseline_b);
    read.validate_original(&a.store).unwrap();
}
