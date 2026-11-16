//! The RUSTSEC-2026-0387 exception in deny.toml rests on one claim: no secret ever reaches the
//! openmls backend's P-256/P-384/K-256 (crypto-bigint 0.5) code, because this crate uses one
//! X25519/Ed25519 suite and openmls refuses foreign-suite material before any HPKE operation.
//! These tests pin each part of that claim; if one fails, the exception must be re-reviewed.

use super::*;
use crate::config::{capabilities, CIPHERSUITE};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::signatures::Signer as _;

const P256_SUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMP256_AES128GCM_SHA256_P256;

/// A well-formed, correctly signed KeyPackage in the P-256 suite, as a hostile or misconfigured
/// peer could publish. Test-only: this is the one place P-256 secrets are generated.
fn p256_key_package_bytes() -> Vec<u8> {
    let provider = OpenMlsRustCrypto::default();
    let signer = SignatureKeyPair::new(P256_SUITE.signature_algorithm()).unwrap();
    let credential = CredentialWithKey {
        credential: BasicCredential::new(b"p256 peer".to_vec()).into(),
        signature_key: signer.public().into(),
    };
    let bundle = KeyPackage::builder()
        .build(P256_SUITE, &provider, &signer, credential)
        .unwrap();
    crate::serialize_key_package(bundle.key_package()).unwrap()
}

#[test]
fn the_pinned_suite_is_x25519_ed25519_and_capabilities_advertise_only_it() {
    assert_eq!(
        CIPHERSUITE,
        Ciphersuite::MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519
    );
    let caps = capabilities();
    assert_eq!(caps.ciphersuites(), &[CIPHERSUITE.into()]);
    assert_eq!(caps.credentials(), &[CredentialType::Basic]);
}

#[test]
fn a_device_signs_with_ed25519_only() {
    let device = MlsDevice::generate().unwrap();
    assert_eq!(device.signer().signature_scheme(), SignatureScheme::ED25519);
    assert_eq!(device.public_key_bytes().len(), 32);
}

#[test]
fn a_foreign_suite_key_package_parses_but_is_refused_before_any_commit_or_welcome() {
    let alice = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&alice).unwrap();
    let epoch = group.epoch();
    // Signature verification on the peer's own public key is the only P-256 work a peer can
    // drive, and it involves no secret of ours.
    let foreign = alice.parse_key_package(&p256_key_package_bytes()).unwrap();
    assert_eq!(foreign.ciphersuite(), P256_SUITE);

    // Refused by openmls's ValSem105 suite check, which runs before the path and the Welcome's
    // HPKE seals are computed.
    for refused in [
        group
            .add_member(&alice, foreign.clone())
            .unwrap_err()
            .to_string(),
        group.stage_add(&alice, foreign).unwrap_err().to_string(),
    ] {
        assert!(
            refused.to_lowercase().contains("ciphersuite"),
            "refused by the suite check: {refused}"
        );
    }
    assert_eq!(group.epoch(), epoch, "no commit was built or merged");
    // Merging with nothing pending is a no-op in openmls; if a foreign-suite commit had been
    // staged, this would advance the epoch.
    group.merge_staged_self(&alice).unwrap();
    assert_eq!(
        group.epoch(),
        epoch,
        "the refusal left no pending commit behind"
    );
    // The group is unharmed: an ordinary in-suite Add still works.
    let bob = MlsDevice::generate().unwrap();
    let kp = bob
        .key_package_for_invite(&group.group_id(), [5; 16])
        .unwrap();
    group.add_member(&alice, kp).unwrap();
    assert_eq!(group.epoch(), epoch + 1);
}

#[test]
fn a_welcome_claiming_another_suite_is_refused_before_the_group_secrets_are_opened() {
    let alice = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&alice).unwrap();
    let bob = MlsDevice::generate().unwrap();
    let kp = bob
        .key_package_for_invite(&group.group_id(), [6; 16])
        .unwrap();
    let welcome = group.add_member(&alice, kp).unwrap().welcome;

    // MLSMessage framing: version (2 bytes), wire format (2 bytes, 3 = Welcome), then the
    // Welcome's cipher_suite (2 bytes).
    assert_eq!(&welcome[2..6], &[0, 3, 0, 3]);
    let mut foreign = welcome.clone();
    foreign[4..6].copy_from_slice(&u16::from(P256_SUITE).to_be_bytes());
    // openmls deletes a KeyPackage as soon as a Welcome names it, even if the Welcome is then
    // refused (0.8 and 0.9 alike), so the tampered attempt runs on a copy of bob's storage.
    let bob_copy = bob.duplicate().unwrap();
    let refused = ServerGroup::join(&bob_copy, &foreign)
        .unwrap_err()
        .to_string();
    assert!(
        refused.to_lowercase().contains("ciphersuite"),
        "refused by the suite check, not later: {refused}"
    );
    // Control: the untouched Welcome joins, so the refusal above was the suite check alone.
    let joined = ServerGroup::join(&bob, &welcome).unwrap();
    assert_eq!(joined.epoch(), group.epoch());
}

#[test]
fn the_test_only_p256_signer_is_really_p256() {
    // Guards the fixture above: if the backend ever stopped producing P-256 KeyPackages, the
    // refusal tests would pass for the wrong reason.
    let signer = SignatureKeyPair::new(P256_SUITE.signature_algorithm()).unwrap();
    assert_eq!(
        signer.signature_scheme(),
        SignatureScheme::ECDSA_SECP256R1_SHA256
    );
    assert!(signer.sign(b"x").is_ok());
}
