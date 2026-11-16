//! Actual SDK devices/providers; all tests authored UNRUN during the compilation pause.
use super::*;
use crate::{
    persist::{restore_join_device_provider_bounded, snapshot_join_device_bounded},
    ServerGroup,
};

fn counts() -> (usize, usize, usize, usize) {
    (
        SCANNED_ROWS.with(std::cell::Cell::get),
        SIZED_SIGNERS.with(std::cell::Cell::get),
        ENCODED_SIGNERS.with(std::cell::Cell::get),
        PROBE_ENTRIES.with(std::cell::Cell::get),
    )
}

#[test]
fn actual_fresh_duplicate_and_signature_only_restore_pass_without_fresh_history_claim() {
    let device = MlsDevice::generate().unwrap();
    let original = snapshot_join_device_bounded(&device, 64 * 1024, 64 * 1024).unwrap();
    device.preflight_fresh_founder_input().unwrap();
    device
        .duplicate()
        .unwrap()
        .preflight_fresh_founder_input()
        .unwrap();
    let restored = restore_join_device_provider_bounded(&original, 64 * 1024, 64 * 1024).unwrap();
    restored.preflight_fresh_founder_input().unwrap();
    assert_eq!(restored.public_key_bytes(), device.public_key_bytes());
    assert_eq!(
        original.as_slice(),
        snapshot_join_device_bounded(&device, 64 * 1024, 64 * 1024)
            .unwrap()
            .as_slice()
    );
    assert_eq!(MlsDevice::FOUNDER_INPUT_PRIVATE_INVOICE, 32 * 1024);
}

#[test]
fn real_created_group_and_real_key_package_rows_refuse_before_probe_or_encoding() {
    let device = MlsDevice::generate().unwrap();
    let _group = ServerGroup::create(&device).unwrap();
    let before = counts();
    assert!(device.preflight_fresh_founder_input().is_err());
    let after = counts();
    assert_eq!(before.0, after.0);
    assert_eq!(before.2, after.2);
    assert_eq!(before.3, after.3);
    let member = MlsDevice::generate().unwrap();
    let _package = member.key_package().unwrap();
    let before = counts();
    assert!(member.preflight_fresh_founder_input().is_err());
    let after = counts();
    assert_eq!(before.0, after.0);
    assert_eq!(before.2, after.2);
    assert_eq!(before.3, after.3);
}

#[test]
fn sparse_real_map_and_deterministic_tombstone_allocation_refuse_before_scan() {
    let device = MlsDevice::generate().unwrap();
    let allocation = {
        let mut map = device.provider.storage().values.write().unwrap();
        map.reserve(4096);
        map.allocation_size()
    };
    assert!(allocation > MAX_BACKING);
    let before = counts();
    assert!(device.preflight_fresh_founder_input().is_err());
    let after = counts();
    assert_eq!(before.0, after.0);
    assert_eq!(before.2, after.2);
    assert_eq!(before.3, after.3);
    assert_eq!(
        device
            .provider
            .storage()
            .values
            .read()
            .unwrap()
            .allocation_size(),
        allocation
    );

    #[derive(Default)]
    struct Collision;
    impl std::hash::Hasher for Collision {
        fn finish(&self) -> u64 {
            0
        }
        fn write(&mut self, _: &[u8]) {}
    }
    let mut map: StorageMap<std::hash::BuildHasherDefault<Collision>> = StorageMap::default();
    for n in 0..4096u64 {
        map.insert(n.to_be_bytes().to_vec(), Vec::new());
    }
    let allocated = map.allocation_size();
    let reported = map.capacity();
    for n in 0..4095u64 {
        map.remove(n.to_be_bytes().as_slice()).unwrap();
    }
    assert_eq!(map.len(), 1);
    assert_eq!(map.allocation_size(), allocated);
    assert!(map.capacity() < reported);
    let before = counts();
    assert!(preflight_map(&map).is_err());
    assert_eq!(before.0, counts().0);
}

#[test]
fn huge_actual_vec_spare_capacity_refuses_without_encoding_or_shrinking() {
    let device = MlsDevice::generate().unwrap();
    let capacity = {
        let mut map = device.provider.storage().values.write().unwrap();
        let row = map.values_mut().next().unwrap();
        let original = Zeroizing::new(std::mem::take(row));
        let mut large = Vec::with_capacity(MAX_PAYLOAD * 4);
        large.extend_from_slice(&original);
        let capacity = large.capacity();
        *row = large;
        capacity
    };
    let before = counts();
    assert!(device.preflight_fresh_founder_input().is_err());
    let after = counts();
    assert_eq!(before.2, after.2);
    assert_eq!(before.3, after.3);
    assert_eq!(
        device
            .provider
            .storage()
            .values
            .read()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .capacity(),
        capacity
    );
}

#[test]
fn malformed_public_scheme_private_length_and_mismatched_keypair_refuse_at_correct_boundary() {
    let mut device = MlsDevice::generate().unwrap();
    device.signer =
        SignatureKeyPair::from_raw(SignatureScheme::ED25519, vec![1; 32], vec![2; 100_000]);
    let before = counts();
    assert!(device.preflight_fresh_founder_input().is_err());
    assert_eq!(before, counts());
    let mut device = MlsDevice::generate().unwrap();
    let public = device.public_key_bytes();
    device.signer =
        SignatureKeyPair::from_raw(SignatureScheme::ECDSA_SECP256R1_SHA256, vec![1; 31], public);
    let before = counts();
    assert!(device.preflight_fresh_founder_input().is_err());
    assert_eq!(before, counts());
    let mut device = MlsDevice::generate().unwrap();
    let public = device.public_key_bytes();
    device.signer = SignatureKeyPair::from_raw(SignatureScheme::ED25519, vec![1; 100_000], public);
    let before = counts();
    assert!(device.preflight_fresh_founder_input().is_err());
    let after = counts();
    assert_eq!(before.2, after.2);
    assert_eq!(before.3, after.3);
    let mut device = MlsDevice::generate().unwrap();
    let public = device.public_key_bytes();
    device.signer = SignatureKeyPair::from_raw(SignatureScheme::ED25519, vec![1; 31], public);
    let before = counts();
    assert!(device.preflight_fresh_founder_input().is_err());
    assert_eq!(before.3, counts().3);
    let mut device = MlsDevice::generate().unwrap();
    let public = device.public_key_bytes();
    device.signer = SignatureKeyPair::from_raw(SignatureScheme::ED25519, vec![1; 32], public);
    // TLS shape is valid; deterministic challenge verification refuses wrong private/public.
    let before = counts();
    assert!(device.preflight_fresh_founder_input().is_err());
    let after = counts();
    assert_eq!(after.2, before.2 + 1);
    assert_eq!(after.3, before.3);
}

#[test]
fn canonical_row_mismatch_and_noncanonical_json_refuse_without_decoding_and_wipe_probe() {
    let device = MlsDevice::generate().unwrap();
    {
        let mut map = device.provider.storage().values.write().unwrap();
        let row = map.values_mut().next().unwrap();
        row.insert(0, b' ');
    }
    let before = WIPED_ROWS.with(std::cell::Cell::get);
    assert!(device.preflight_fresh_founder_input().is_err());
    assert_eq!(WIPED_ROWS.with(std::cell::Cell::get), before + 1);
    // This is a deliberate strict local input policy, not a generic compatibility decoder.
    let device = MlsDevice::generate().unwrap();
    {
        let mut map = device.provider.storage().values.write().unwrap();
        let (mut key, value) = map.drain().next().unwrap();
        key[0] ^= 1;
        map.insert(key, value);
    }
    let before = WIPED_ROWS.with(std::cell::Cell::get);
    assert!(device.preflight_fresh_founder_input().is_err());
    assert_eq!(WIPED_ROWS.with(std::cell::Cell::get), before + 1);
}

#[test]
fn secret_probe_drop_handles_poison_without_panic_and_wipes_owned_row() {
    let probe = SecretProbe(MemoryStorage::default());
    probe.0.values.write().unwrap().insert(vec![1], vec![2; 32]);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = probe.0.values.write().unwrap();
        panic!("test poison private probe");
    }));
    assert!(result.is_err());
    let before = WIPED_ROWS.with(std::cell::Cell::get);
    drop(probe);
    assert_eq!(WIPED_ROWS.with(std::cell::Cell::get), before + 1);
}
