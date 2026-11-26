use super::*;
use catcoms_crypto::identity::DeviceKeypair;
use catcoms_replication::chat::join_bindings::JoinPayloadKindData;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn governed_add_response_requires_original_raw_binding_and_payloads() {
    let signer = DeviceKeypair::from_seed(&[78; 32]);
    let welcome = b"original welcome";
    let routing = b"original sealed route";
    let spec = WelcomeBindingSpecData {
        invite: [1; 32],
        anchor: [2; 32],
        request_binding: [3; 32],
        key_package_wire: [4; 32],
        key_package_reference: [5; 32],
        admitter: signer.verifying_key().to_bytes(),
        add_header: [7; 32],
        admission: [8; 32],
        welcome: JoinPayloadKindData::Welcome
            .commitment_data(welcome)
            .unwrap(),
        policy: [10; 32],
        public_routing_plan: [11; 32],
        sealed_routing_transfer: JoinPayloadKindData::RoutingTransfer
            .commitment_data(routing)
            .unwrap(),
    };
    let preimage = spec.signing_bytes_data().unwrap();
    let signature = signer.sign(&preimage);
    let binding = SignedWelcomeBindingData::sign_using(spec.clone(), |_| Ok(signature)).unwrap();
    let actual = V2JoinResponseData {
        binding: &binding,
        welcome,
        sealed_routing: routing,
    };
    let packet = actual.encode_packet_data().unwrap();
    let key = [31; 32];
    let raw_welcome = [32; WELCOME_RAW_BYTES];
    let mut rng = ChaCha20Rng::seed_from_u64(146);
    let raw = seal_data(&key, &raw_welcome, &packet, &mut rng).unwrap();
    let original = || OriginalResponse {
        preimage: &preimage,
        signature: &signature,
        payloads: [welcome, routing],
    };
    assert_eq!(
        open_data(&key, &raw_welcome, &raw, original())
            .unwrap()
            .as_slice(),
        packet
    );
    assert!(open_data(&key, &[33; WELCOME_RAW_BYTES], &raw, original()).is_err());
    assert!(open_data(&key, &raw_welcome, &raw[..raw.len() - 1], original()).is_err());

    let alternate_route = b"different sealed route";
    let spec = WelcomeBindingSpecData {
        sealed_routing_transfer: JoinPayloadKindData::RoutingTransfer
            .commitment_data(alternate_route)
            .unwrap(),
        ..spec
    };
    let alternate_binding =
        SignedWelcomeBindingData::sign_using(spec, |bytes| Ok(signer.sign(bytes))).unwrap();
    let alternate = V2JoinResponseData {
        binding: &alternate_binding,
        sealed_routing: alternate_route,
        ..actual
    };
    let alternate_raw = seal_data(
        &key,
        &raw_welcome,
        &alternate.encode_packet_data().unwrap(),
        &mut rng,
    )
    .unwrap();
    assert!(open_data(&key, &raw_welcome, &alternate_raw, original()).is_err());
}
