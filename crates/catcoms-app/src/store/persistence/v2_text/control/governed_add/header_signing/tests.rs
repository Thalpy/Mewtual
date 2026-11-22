use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn governed_add_header_signing_requires_exact_outcome_and_attempt() {
    let key = [21; 32];
    let mut rng = ChaCha20Rng::seed_from_u64(120);
    let original = [22; MIN_FRAME];
    let parent = HeaderOutcomeCommitment::from_outcome(&original).unwrap();
    let attempt = parent.seal_attempt(&key, &mut rng).unwrap();
    let signature = [23; 64];
    let raw = parent
        .seal_raw(&key, &attempt, &signature, &mut rng)
        .unwrap();
    assert_eq!(parent.open_raw(&key, &attempt, &raw).unwrap(), signature);

    let replaced_attempt = parent.seal_attempt(&key, &mut rng).unwrap();
    parent.require_attempt(&key, &replaced_attempt).unwrap();
    assert!(parent.open_raw(&key, &replaced_attempt, &raw).is_err());
    let other = HeaderOutcomeCommitment::from_outcome(&[24; MIN_FRAME]).unwrap();
    assert!(other.open_raw(&key, &attempt, &raw).is_err());
    assert!(other
        .seal_raw(&key, &attempt, &signature, &mut rng)
        .is_err());
    assert!(parent.require_attempt(&key, &raw).is_err());
    assert!(parent.open_raw(&key, &raw, &attempt).is_err());
    assert!(parent
        .open_raw(&key, &attempt, &raw[..raw.len() - 1])
        .is_err());
    assert!(HeaderOutcomeCommitment::from_outcome(&original[..MIN_FRAME - 1]).is_err());
}
