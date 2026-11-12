use super::*;
const NOW: u64 = 1_800_000_000_000;
const GROUP: [u8; 16] = [17; 16];
const NONCE: [u8; 16] = [29; 16];

fn original() -> (MlsDevice, Vec<u8>, [u8; 32], [u8; 32]) {
    let device = MlsDevice::generate().unwrap();
    let credential = CredentialWithKey {
        credential: BasicCredential::new(
            MembershipCredential {
                device_id: device.device_id(),
                group_id: GROUP.to_vec(),
                invite_nonce: NONCE,
            }
            .encode(),
        )
        .into(),
        signature_key: device.signer.public().into(),
    };
    let bundle = KeyPackage::builder()
        .leaf_node_capabilities(capabilities())
        .key_package_lifetime(Lifetime::init(
            NOW / 1000 - 60,
            NOW / 1000 + 60 * 24 * 60 * 60,
        ))
        .build(CIPHERSUITE, &device.provider, &device.signer, credential)
        .unwrap();
    let reference = device.key_package_reference(bundle.key_package()).unwrap();
    let frame = serialize_key_package(bundle.key_package()).unwrap();
    let subject = device.public_key_ref().try_into().unwrap();
    (device, frame, subject, reference)
}
fn budget() -> PackageBudget {
    PackageBudget::new(1024 * 1024, 16 * 1024 * 1024)
}

#[test]
fn actual_invite_package_has_exact_full_subject_credential_caps_reference_and_explicit_time() {
    let (device, bytes, subject, reference) = original();
    let before = bytes.clone();
    let package = validate(
        &device,
        &bytes,
        &GROUP,
        &subject,
        &NONCE,
        &reference,
        NOW,
        &mut budget(),
    )
    .unwrap();
    assert_eq!(package.identity().subject(), &subject);
    assert_eq!(package.identity().key_package_reference(), &reference);
    assert_eq!(
        serialize_key_package(&package.into_package()).unwrap(),
        before
    );
    assert_eq!(bytes, before);
    // The fixed injected epoch is deliberately independent of the machine's
    // wall clock. Before/not-after and application floor use that SAME value.
    assert!(validate(
        &device,
        &bytes,
        &GROUP,
        &subject,
        &NONCE,
        &reference,
        NOW - 120_000,
        &mut budget()
    )
    .is_err());
    assert!(validate(
        &device,
        &bytes,
        &GROUP,
        &subject,
        &NONCE,
        &reference,
        NOW + 61 * 24 * 60 * 60 * 1000,
        &mut budget()
    )
    .is_err());
}

#[test]
fn malformed_or_cross_bound_package_refuses_before_first_tls_allocation() {
    let (device, bytes, subject, reference) = original();
    let before = TLS_ENTRIES.with(std::cell::Cell::get);
    let mut trailing = bytes.clone();
    trailing.push(0);
    let mut bad_suite = bytes.clone();
    bad_suite[7] = 1;
    let mut bad_source = bytes.clone();
    // A changed exact signed full subject fails the borrowed framing cut, not a
    // later DeviceId-only/crypto mismatch. Locate its unique original VL32 key.
    let at = bytes.windows(32).position(|w| w == subject).unwrap();
    bad_source[at] ^= 1;
    for frame in [
        &trailing[..],
        &bad_suite[..],
        &bad_source[..],
        &bytes[..bytes.len() - 1],
    ] {
        assert!(validate(
            &device,
            frame,
            &GROUP,
            &subject,
            &NONCE,
            &reference,
            NOW,
            &mut budget()
        )
        .is_err());
    }
    assert!(validate(
        &device,
        &bytes,
        &[3; 16],
        &subject,
        &NONCE,
        &reference,
        NOW,
        &mut budget()
    )
    .is_err());
    assert!(validate(
        &device,
        &bytes,
        &GROUP,
        &subject,
        &[4; 16],
        &reference,
        NOW,
        &mut budget()
    )
    .is_err());
    assert_eq!(TLS_ENTRIES.with(std::cell::Cell::get), before);
    let mut wrong_reference = reference;
    wrong_reference[0] ^= 1;
    assert!(validate(
        &device,
        &bytes,
        &GROUP,
        &subject,
        &NONCE,
        &wrong_reference,
        NOW,
        &mut budget()
    )
    .is_err());
    assert_eq!(TLS_ENTRIES.with(std::cell::Cell::get), before + 1);
}

#[test]
fn exact_decoder_fee_and_work_pressure_preserve_original_and_refuse_before_tls() {
    let (device, bytes, subject, reference) = original();
    let facts = preflight::scan(&bytes, &GROUP, &subject, &NONCE).unwrap();
    let required = layout(bytes.len(), facts.credential.len()).unwrap();
    let before = TLS_ENTRIES.with(std::cell::Cell::get);
    assert!(validate(
        &device,
        &bytes,
        &GROUP,
        &subject,
        &NONCE,
        &reference,
        NOW,
        &mut PackageBudget::new(required - 1, 16 * 1024 * 1024)
    )
    .is_err());
    assert!(validate(
        &device,
        &bytes,
        &GROUP,
        &subject,
        &NONCE,
        &reference,
        NOW,
        &mut PackageBudget::new(required, preflight::scan_work(bytes.len()).unwrap() - 1)
    )
    .is_err());
    assert_eq!(TLS_ENTRIES.with(std::cell::Cell::get), before);
    let package = validate(
        &device,
        &bytes,
        &GROUP,
        &subject,
        &NONCE,
        &reference,
        NOW,
        &mut PackageBudget::new(required, 16 * 1024 * 1024),
    )
    .unwrap();
    assert_eq!(
        serialize_key_package(&package.into_package()).unwrap(),
        bytes
    );
}
