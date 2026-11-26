use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn governed_add_welcome_intent_requires_exact_governing_ciphertext() {
    let spec = WelcomeBindingSpecData {
        invite: [1; 32],
        anchor: [2; 32],
        request_binding: [3; 32],
        key_package_wire: [4; 32],
        key_package_reference: [5; 32],
        admitter: [6; 32],
        add_header: [7; 32],
        admission: [8; 32],
        welcome: [9; 32],
        policy: [10; 32],
        public_routing_plan: [11; 32],
        sealed_routing_transfer: [12; 32],
    };
    let key = [25; 32];
    let mut rng = ChaCha20Rng::seed_from_u64(139);
    let governing = [26; governing_record::MIN_BYTES];
    let preimage = spec.signing_bytes_data().unwrap();
    let raw = seal_data(&key, &governing, &preimage, &mut rng).unwrap();
    let read = open_data(&key, &governing, &raw).unwrap();
    assert_eq!(read.signing_bytes_data().unwrap(), preimage);
    let mut substitute = governing;
    substitute[0] ^= 1;
    assert!(open_data(&key, &substitute, &raw).is_err());
    assert!(open_data(&key, &governing, &raw[..raw.len() - 1]).is_err());
    let mut malformed = preimage;
    malformed.push(0);
    assert!(seal_data(&key, &governing, &malformed, &mut rng).is_err());
}
