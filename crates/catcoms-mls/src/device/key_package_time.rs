//! Genuine signed-package coverage for the narrow vendored validation clock seam.
//! This does not admit an Add or replace the application's bounded input/range policy.

use super::*;
use openmls::key_packages::KeyPackageIn;
use openmls::treesync::errors::LifetimeError;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const NOT_BEFORE: u64 = 10_000;
const NOT_AFTER: u64 = 20_000;

fn time(seconds: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .unwrap()
}

fn fixed_package(device: &MlsDevice, builder: KeyPackageBuilder) -> KeyPackage {
    builder
        // Explicit lifetime avoids the builder's default ambient clock entirely.
        .key_package_lifetime(Lifetime::init(NOT_BEFORE, NOT_AFTER))
        .build(
            CIPHERSUITE,
            device.provider(),
            device.signer(),
            device.credential(),
        )
        .unwrap()
        .key_package()
        .clone()
}

fn incoming(wire: &[u8]) -> KeyPackageIn {
    let mut remaining = wire;
    let message = MlsMessageIn::tls_deserialize(&mut remaining).unwrap();
    assert!(
        remaining.is_empty(),
        "fixture must consume its complete TLS input"
    );
    match message.extract() {
        MlsMessageBodyIn::KeyPackage(package) => package,
        _ => panic!("fixture must be a genuine KeyPackage message"),
    }
}

fn unique_offset(haystack: &[u8], needle: &[u8]) -> usize {
    let offsets: Vec<_> = haystack
        .windows(needle.len())
        .enumerate()
        .filter_map(|(offset, bytes)| (bytes == needle).then_some(offset))
        .collect();
    assert_eq!(
        offsets.len(),
        1,
        "mutate the exact original public TLS field"
    );
    offsets[0]
}

#[test]
fn explicit_time_checks_complete_signed_package_at_exact_lifetime_boundaries() {
    let device = MlsDevice::generate().unwrap();
    let package = fixed_package(
        &device,
        KeyPackage::builder().leaf_node_capabilities(capabilities()),
    );
    let wire = serialize_key_package(&package).unwrap();
    let crypto = device.provider().crypto();
    assert_eq!(
        incoming(&wire).validate_with_time(crypto, ProtocolVersion::Mls10, time(NOT_BEFORE - 1)),
        Err(KeyPackageVerifyError::LifetimeError(
            LifetimeError::NotValidYet {
                not_before: NOT_BEFORE,
                now: NOT_BEFORE - 1,
            }
        ))
    );
    // Windows SystemTime uses 100ns ticks: subtracting 1ns can round to the
    // unchanged expiry instant. A millisecond is representable on native targets
    // and still verifies the existing whole-second floor immediately before expiry.
    for now in [time(NOT_BEFORE), time(NOT_AFTER) - Duration::from_millis(1)] {
        let verified = incoming(&wire)
            .validate_with_time(crypto, ProtocolVersion::Mls10, now)
            .unwrap();
        assert_eq!(serialize_key_package(&verified).unwrap(), wire);
        assert_eq!(
            verified.hash_ref(crypto).unwrap(),
            package.hash_ref(crypto).unwrap()
        );
    }
    for now in [NOT_AFTER, NOT_AFTER + 1] {
        assert_eq!(
            incoming(&wire).validate_with_time(crypto, ProtocolVersion::Mls10, time(now)),
            Err(KeyPackageVerifyError::LifetimeError(
                LifetimeError::Expired {
                    not_after: NOT_AFTER,
                    now,
                }
            ))
        );
    }
}

#[test]
fn explicit_pre_unix_time_reaches_the_original_lifetime_error() {
    let device = MlsDevice::generate().unwrap();
    let package = fixed_package(&device, KeyPackage::builder());
    let before_unix = UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap();
    assert_eq!(
        incoming(&serialize_key_package(&package).unwrap()).validate_with_time(
            device.provider().crypto(),
            ProtocolVersion::Mls10,
            before_unix,
        ),
        Err(KeyPackageVerifyError::LifetimeError(
            LifetimeError::SystemTimeBeforeUnixEpoch
        ))
    );
}

#[test]
fn explicit_time_preserves_both_actual_signature_checks_before_lifetime() {
    let device = MlsDevice::generate().unwrap();
    let package = fixed_package(&device, KeyPackage::builder());
    let wire = serialize_key_package(&package).unwrap();
    let crypto = device.provider().crypto();
    let mut bad_package_signature = wire.clone();
    *bad_package_signature.last_mut().unwrap() ^= 1;
    assert_eq!(
        incoming(&bad_package_signature).validate_with_time(
            crypto,
            ProtocolVersion::Mls10,
            time(0)
        ),
        Err(KeyPackageVerifyError::InvalidSignature)
    );
    let leaf = package.leaf_node().tls_serialize_detached().unwrap();
    let leaf_at = unique_offset(&wire, &leaf);
    let mut bad_leaf_signature = wire;
    // The canonical LeafNode ends with its real Ed25519 signature; no test verifier is used.
    bad_leaf_signature[leaf_at + leaf.len() - 1] ^= 1;
    assert_eq!(
        incoming(&bad_leaf_signature).validate_with_time(crypto, ProtocolVersion::Mls10, time(0)),
        Err(KeyPackageVerifyError::InvalidLeafNodeSignature)
    );
}

#[test]
fn explicit_time_preserves_protocol_and_distinct_init_key_checks() {
    let device = MlsDevice::generate().unwrap();
    let package = fixed_package(&device, KeyPackage::builder());
    let mut wire = serialize_key_package(&package).unwrap();
    let crypto = device.provider().crypto();
    // The original signatures remain valid: only the expected protocol differs.
    assert_eq!(
        incoming(&wire).validate_with_time(
            crypto,
            ProtocolVersion::Other(0x1234),
            time(NOT_BEFORE)
        ),
        Err(KeyPackageVerifyError::InvalidProtocolVersion)
    );
    let init = package.hpke_init_key().tls_serialize_detached().unwrap();
    let init_at = unique_offset(&wire, &init);
    // InitKey is the first HPKE vector inside KeyPackageTBS. Copy the genuine leaf's
    // public encryption key vector; this deliberately leaves the outer signature stale.
    let leaf = package.leaf_node().tls_serialize_detached().unwrap();
    assert_eq!(
        init.len(),
        33,
        "pinned X25519 vector has one-byte length plus 32 bytes"
    );
    wire[init_at..init_at + init.len()].copy_from_slice(&leaf[..init.len()]);
    assert_eq!(
        incoming(&wire).validate_with_time(crypto, ProtocolVersion::Mls10, time(NOT_BEFORE)),
        Err(KeyPackageVerifyError::InitKeyEqualsEncryptionKey)
    );
}

#[test]
fn explicit_time_preserves_extension_support_after_both_real_signatures() {
    let device = MlsDevice::generate().unwrap();
    let extension = Extension::Unknown(0xff00, UnknownExtension(vec![1, 0x55]));
    let unsupported = fixed_package(
        &device,
        KeyPackage::builder()
            .leaf_node_capabilities(capabilities())
            .key_package_extensions(Extensions::single(extension.clone()).unwrap()),
    );
    assert_eq!(
        incoming(&serialize_key_package(&unsupported).unwrap()).validate_with_time(
            device.provider().crypto(),
            ProtocolVersion::Mls10,
            time(NOT_BEFORE),
        ),
        Err(KeyPackageVerifyError::UnsupportedExtension)
    );
    let supported = fixed_package(
        &device,
        KeyPackage::builder()
            .leaf_node_capabilities(Capabilities::new(
                None,
                Some(&[CIPHERSUITE]),
                Some(&[ExtensionType::Unknown(0xff00)]),
                None,
                Some(&[CredentialType::Basic]),
            ))
            .key_package_extensions(Extensions::single(extension).unwrap()),
    );
    let wire = serialize_key_package(&supported).unwrap();
    assert!(incoming(&wire)
        .validate_with_time(
            device.provider().crypto(),
            ProtocolVersion::Mls10,
            time(NOT_BEFORE),
        )
        .is_ok());
    let mut wrong_context = wire;
    let encoded_extension = supported.extensions().tls_serialize_detached().unwrap();
    let at = unique_offset(&wrong_context, &encoded_extension);
    // The extension vector has one-byte length, then the two-byte extension type.
    // ApplicationId's payload is well-formed here, but its context is leaf-only.
    assert_eq!(&encoded_extension[..3], &[5, 0xff, 0]);
    wrong_context[at + 1..at + 3].copy_from_slice(&[0, 1]);
    assert!(matches!(
        incoming(&wrong_context).validate_with_time(
            device.provider().crypto(),
            ProtocolVersion::Mls10,
            time(NOT_BEFORE),
        ),
        Err(KeyPackageVerifyError::ExtensionTypeNotValidInKeyPackage(_))
    ));
}

#[test]
fn legacy_validator_still_uses_wall_clock_and_does_not_reuse_explicit_time() {
    let device = MlsDevice::generate().unwrap();
    let package = fixed_package(&device, KeyPackage::builder());
    let wire = serialize_key_package(&package).unwrap();
    let crypto = device.provider().crypto();
    assert!(incoming(&wire)
        .validate_with_time(crypto, ProtocolVersion::Mls10, time(NOT_BEFORE))
        .is_ok());
    // Fixed expiry is in 1970; deliberately exercise the legacy ambient path without
    // sampling a test clock or sleeping. The explicit path must not affect this call.
    assert!(matches!(
        incoming(&wire).validate(crypto, ProtocolVersion::Mls10),
        Err(KeyPackageVerifyError::LifetimeError(
            LifetimeError::Expired {
                not_after: NOT_AFTER,
                ..
            }
        ))
    ));
    assert!(incoming(&wire)
        .validate_with_time(crypto, ProtocolVersion::Mls10, time(NOT_BEFORE))
        .is_ok());
}
