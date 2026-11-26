use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn governed_add_header_signing_requires_exact_outcome_and_attempt() {
    let key = [21; 32];
    let mut rng = ChaCha20Rng::seed_from_u64(120);
    let original = [22; MIN_FRAME];
    let parent = AddSigningParent::from_outcome(&original).unwrap();
    let attempt = parent.seal_attempt(&key, &mut rng).unwrap();
    let signature = [23; 64];
    let raw = parent
        .seal_raw(&key, &attempt, &signature, &mut rng)
        .unwrap();
    assert_eq!(parent.open_raw(&key, &attempt, &raw).unwrap(), signature);

    let replaced_attempt = parent.seal_attempt(&key, &mut rng).unwrap();
    parent.require_attempt(&key, &replaced_attempt).unwrap();
    assert!(parent.open_raw(&key, &replaced_attempt, &raw).is_err());
    let other = AddSigningParent::from_outcome(&[24; MIN_FRAME]).unwrap();
    assert!(other.open_raw(&key, &attempt, &raw).is_err());
    assert!(other
        .seal_raw(&key, &attempt, &signature, &mut rng)
        .is_err());
    assert!(parent.require_attempt(&key, &raw).is_err());
    assert!(parent.open_raw(&key, &raw, &attempt).is_err());
    assert!(parent
        .open_raw(&key, &attempt, &raw[..raw.len() - 1])
        .is_err());
    assert!(AddSigningParent::from_outcome(&original[..MIN_FRAME - 1]).is_err());
}

#[test]
fn governed_add_welcome_signing_requires_its_own_role_and_attempt() {
    let key = [31; 32];
    let mut rng = ChaCha20Rng::seed_from_u64(142);
    let parent = AddSigningParent::from_welcome_intent(&[32; welcome_intent::MIN_BYTES]).unwrap();
    let header = AddSigningParent::from_outcome(&[32; MIN_FRAME]).unwrap();
    let attempt = parent.seal_attempt(&key, &mut rng).unwrap();
    let signature = [33; 64];
    let raw = parent
        .seal_raw(&key, &attempt, &signature, &mut rng)
        .unwrap();
    assert_eq!(attempt.len(), WELCOME_ATTEMPT_BYTES);
    assert_eq!(raw.len(), WELCOME_RAW_BYTES);
    assert_eq!(parent.open_raw(&key, &attempt, &raw).unwrap(), signature);
    assert!(header.open_raw(&key, &attempt, &raw).is_err());
    let other = parent.seal_attempt(&key, &mut rng).unwrap();
    assert!(parent.open_raw(&key, &other, &raw).is_err());
}
