use super::*;
use crate::{restore_server, serialize_key_package, snapshot_server, ServerGroup};

#[test]
fn retained_fixed_package_reference_uses_public_production_tls_shape() {
    for digest in [[0; 32], [1; 32], [255; 32]] {
        let typed = retained_package_ref(&digest).unwrap();
        assert_eq!(typed.as_slice(), &digest);
        assert_eq!(
            typed.tls_serialize_detached().unwrap(),
            tls_codec::VLByteSlice(&digest)
                .tls_serialize_detached()
                .unwrap()
        );
    }
}

#[test]
fn retained_private_bundle_check_binds_exact_package_device_group_and_nonce_without_mutation() {
    let device = MlsDevice::generate().unwrap();
    let group = b"retained-source-group";
    let nonce = [0x72; 16];
    let first = device.key_package_for_invite(group, nonce).unwrap();
    let first_ref = device.key_package_reference(&first).unwrap();
    let second = device.key_package_for_invite(group, nonce).unwrap();
    let second_ref = device.key_package_reference(&second).unwrap();
    let first_wire = serialize_key_package(&first).unwrap();
    let second_wire = serialize_key_package(&second).unwrap();
    for _ in 0..2 {
        assert!(device
            .validate_retained_join_package(&first_wire, &first_ref, group, &nonce)
            .is_ok());
    }
    assert!(device
        .validate_retained_join_package(&second_wire, &first_ref, group, &nonce)
        .is_err());
    assert!(device
        .validate_retained_join_package(&first_wire, &second_ref, group, &nonce)
        .is_err());
    assert!(device
        .validate_retained_join_package(&first_wire, &first_ref, b"other", &nonce)
        .is_err());
    assert!(device
        .validate_retained_join_package(&first_wire, &first_ref, group, &[0x73; 16])
        .is_err());
    assert!(MlsDevice::generate()
        .unwrap()
        .validate_retained_join_package(&first_wire, &first_ref, group, &nonce)
        .is_err());
    let mut trailing = first_wire.clone();
    trailing.push(0);
    assert!(device
        .validate_retained_join_package(&trailing, &first_ref, group, &nonce)
        .is_err());
    assert!(device
        .validate_retained_join_package(&first_wire, &first_ref, group, &nonce)
        .is_ok());
}

#[test]
fn retained_private_expired_bundle_is_consistent_data_but_not_new_admission() {
    let device = MlsDevice::generate().unwrap();
    let group = b"expired-retained-source";
    let nonce = [0x74; 16];
    let credential = crate::MembershipCredential {
        device_id: device.device_id(),
        group_id: group.to_vec(),
        invite_nonce: nonce,
    };
    let bundle = KeyPackage::builder()
        .key_package_lifetime(Lifetime::init(1, 2))
        .leaf_node_capabilities(capabilities())
        .build(
            CIPHERSUITE,
            device.provider(),
            device.signer(),
            CredentialWithKey {
                credential: BasicCredential::new(credential.encode()).into(),
                signature_key: device.public_key_ref().into(),
            },
        )
        .unwrap();
    let package = bundle.key_package();
    let reference = device.key_package_reference(package).unwrap();
    let wire = serialize_key_package(package).unwrap();
    assert!(device.parse_admitted_key_package(&wire, 0).is_err());
    assert!(device
        .validate_retained_join_package(&wire, &reference, group, &nonce)
        .is_ok());
}

#[test]
fn retained_key_package_labels_match_actual_add_selected_member_and_restore() {
    let owner = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    let member = MlsDevice::generate().unwrap();
    let package = member
        .key_package_for_invite(&group.group_id(), [1; 16])
        .unwrap();
    let wire = serialize_key_package(&package).unwrap();
    let identity = owner.key_package_identity(&package).unwrap();
    assert_eq!(
        identity,
        owner.retained_key_package_identity(&wire).unwrap(),
        "canonical KP body RefHash must equal the SDK's validated-object reference"
    );
    let (outcome, actual) = group.add_member_observed(&owner, package).unwrap();
    assert_eq!(identity.subject(), actual.subject());
    assert_eq!(
        Some(*identity.key_package_reference()),
        actual.key_package_reference()
    );
    assert_eq!(
        Some(*identity.credential_digest()),
        actual.credential_digest()
    );
    let joined = ServerGroup::join_for_key_package(
        &member,
        &outcome.welcome,
        identity.key_package_reference(),
    )
    .unwrap();
    assert_eq!(
        *identity.credential_digest(),
        joined.own_credential_digest(&member).unwrap()
    );
    let saved = snapshot_server(&member, &joined).unwrap();
    let (restored, joined) = restore_server(&saved).unwrap();
    assert_eq!(
        *identity.credential_digest(),
        joined.own_credential_digest(&restored).unwrap()
    );
    assert_eq!(
        identity,
        restored.retained_key_package_identity(&wire).unwrap(),
        "historical metadata does not depend on today's own leaf or provider identity"
    );
}

#[test]
fn retained_expired_signed_package_is_data_while_new_admission_still_refuses() {
    let device = MlsDevice::generate().unwrap();
    // Already expired in 1970; this does not pretend ManualClock controls OpenMLS SystemTime.
    // It verifies DATA extraction only, not a historical owner-accepted outcome. The actual
    // saved-result expired-after-success fixture belongs to the covering consumer acceptance.
    let package = KeyPackage::builder()
        .key_package_lifetime(Lifetime::init(1, 2))
        .leaf_node_capabilities(capabilities())
        .build(
            CIPHERSUITE,
            device.provider(),
            device.signer(),
            device.credential(),
        )
        .unwrap()
        .key_package()
        .clone();
    let wire = serialize_key_package(&package).unwrap();
    let expected = device.key_package_identity(&package).unwrap();
    assert!(device.parse_key_package(&wire).is_err());
    assert!(device.parse_admitted_key_package(&wire, 0).is_err());
    assert_eq!(
        device.retained_key_package_identity(&wire).unwrap(),
        expected
    );
    assert_eq!(
        device.retained_key_package_identity(&wire).unwrap(),
        expected
    );
}

#[test]
fn retained_package_signature_mutation_changes_reference_without_claiming_authentication() {
    let device = MlsDevice::generate().unwrap();
    let package = device.key_package().unwrap();
    let mut wire = serialize_key_package(&package).unwrap();
    let original = device.retained_key_package_identity(&wire).unwrap();
    let last = wire.len() - 1;
    wire[last] ^= 1; // canonical KP's final signature byte
    assert!(device.parse_key_package(&wire).is_err());
    let changed = device.retained_key_package_identity(&wire).unwrap();
    assert_ne!(
        changed.key_package_reference(),
        original.key_package_reference()
    );
    assert_eq!(changed.subject(), original.subject());
    assert_eq!(changed.credential_digest(), original.credential_digest());
    // A real consumer MUST compare the original independently owner-signed reference. Returning
    // this changed plain-data label is not signature validation or permission to admit it.
}

#[test]
fn retained_package_wrong_headers_and_oversize_refuse_before_sdk_decode() {
    let device = MlsDevice::generate().unwrap();
    let wire = serialize_key_package(&device.key_package().unwrap()).unwrap();
    let before = RETAINED_SDK_DECODES.with(std::cell::Cell::get);
    for at in 0..8 {
        let mut bad = wire.clone();
        bad[at] ^= 1;
        assert!(device.retained_key_package_identity(&bad).is_err());
    }
    assert!(device.retained_key_package_identity(&wire[..7]).is_err());
    let mut huge = vec![0; 32 * 1024 + 1];
    huge[..8].copy_from_slice(&wire[..8]);
    assert!(device.retained_key_package_identity(&huge).is_err());
    assert_eq!(RETAINED_SDK_DECODES.with(std::cell::Cell::get), before);
    let mut trailing = wire;
    trailing.push(0);
    assert!(device.retained_key_package_identity(&trailing).is_err());
}

#[test]
fn owner_credential_content_cap_refuses_before_tls_encoding_and_preserves_typed_domain() {
    let calls = std::cell::Cell::new(0);
    let huge = Credential::new(
        CredentialType::Basic,
        vec![1; MAX_MEMBER_CATALOG_CREDENTIAL_BYTES + 1],
    );
    assert!(credential_digest_with(&huge, || calls.set(calls.get() + 1)).is_err());
    assert_eq!(calls.get(), 0);
    let exact = Credential::new(
        CredentialType::Basic,
        vec![2; MAX_MEMBER_CATALOG_CREDENTIAL_BYTES],
    );
    let encoded = exact.tls_serialize_detached().unwrap();
    assert_eq!(
        credential_digest_with(&exact, || calls.set(calls.get() + 1)).unwrap(),
        credential_digest(&encoded)
    );
    assert_eq!(calls.get(), 1);
}
