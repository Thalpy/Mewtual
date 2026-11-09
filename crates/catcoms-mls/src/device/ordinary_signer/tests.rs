//! Actual generated and restored devices; no fixture key or test-only signer is used.
use super::*;
use crate::persist::{restore_join_device_provider_bounded, snapshot_join_device_bounded};

fn key(device: &MlsDevice) -> [u8; 32] {
    device.public_key_bytes().try_into().unwrap()
}

#[test]
fn exact_retained_key_signs_a_verifiable_bounded_payload() {
    let device = MlsDevice::generate().unwrap();
    let expected = key(&device);
    let payload = b"catcoms ordinary member action bytes";
    let raw = device
        .sign_retained_ed25519_bounded(&expected, payload, payload.len())
        .unwrap();
    assert!(catcoms_crypto::verify_with_public_bytes(
        &expected, payload, &raw
    ));
    // Ed25519 is deterministic: the narrow path signs with the SAME retained key as the
    // generic one, not a derived or duplicated key.
    assert_eq!(raw, device.sign(payload).unwrap());
}

#[test]
fn another_devices_key_is_refused_before_signing() {
    let device = MlsDevice::generate().unwrap();
    let other = MlsDevice::generate().unwrap();
    assert!(device.retained_ed25519_key_matches(&key(&device)));
    assert!(!device.retained_ed25519_key_matches(&key(&other)));
    let payload = b"bytes admitted for the other device";
    assert!(device
        .sign_retained_ed25519_bounded(&key(&other), payload, 1024)
        .is_err());
    // A single flipped bit of the full key is also a different signer.
    let mut near = key(&device);
    near[31] ^= 1;
    assert!(device
        .sign_retained_ed25519_bounded(&near, payload, 1024)
        .is_err());
}

#[test]
fn empty_and_oversized_payloads_are_refused_at_the_exact_bound() {
    let device = MlsDevice::generate().unwrap();
    let expected = key(&device);
    assert!(device
        .sign_retained_ed25519_bounded(&expected, &[], 16)
        .is_err());
    let exact = vec![7u8; 16];
    let raw = device
        .sign_retained_ed25519_bounded(&expected, &exact, 16)
        .unwrap();
    assert!(catcoms_crypto::verify_with_public_bytes(
        &expected, &exact, &raw
    ));
    assert!(device
        .sign_retained_ed25519_bounded(&expected, &[7u8; 17], 16)
        .is_err());
}

#[test]
fn restored_same_device_keeps_its_key_while_a_fresh_device_cannot_answer_for_it() {
    let device = MlsDevice::generate().unwrap();
    let expected = key(&device);
    let snapshot = snapshot_join_device_bounded(&device, 64 * 1024, 64 * 1024).unwrap();
    let restored = restore_join_device_provider_bounded(&snapshot, 64 * 1024, 64 * 1024).unwrap();
    let payload = b"after a signature-only restore";
    let raw = restored
        .sign_retained_ed25519_bounded(&expected, payload, 1024)
        .unwrap();
    assert!(catcoms_crypto::verify_with_public_bytes(
        &expected, payload, &raw
    ));
    let replacement = MlsDevice::generate().unwrap();
    assert!(replacement
        .sign_retained_ed25519_bounded(&expected, payload, 1024)
        .is_err());
}
