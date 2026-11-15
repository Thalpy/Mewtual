//! A narrow seeded RNG for upstream protocol APIs that use rand-core 0.10.
//!
//! The ordinary application seam stays on 0.6. Production supplies this seed
//! through that original seam; no ambient default or OS handle lives here.
//! AutoNAT forks its RNG for child streams, so a zero-state adapter around an
//! OS RNG (or a clone that repeats a stream) cannot implement this contract.
use crate::CryptoRngCore;
use rand_core_0_10::{TryCryptoRng, TryRng};
use std::{convert::Infallible, fmt};
use zeroize::Zeroizing;

// These are the actual upstream protocol bounds, not new application authority.
pub use rand_core_0_10::{Rng, SeedableRng};

/// A uniquely owned, explicitly seeded ChaCha20 stream. Debug omits its state;
/// Clone/Copy/Default are deliberately absent. `SeedableRng::fork` advances the
/// parent to seed a distinct child instead of duplicating its current position.
pub struct SeededProtocolRng {
    inner: rand_chacha::ChaCha20Rng,
}
impl SeededProtocolRng {
    /// Draw exactly one 32-byte seed from the existing injected CSPRNG. The
    /// caller chooses the sanctioned OS seam in production or a test source.
    /// Infallible OS-failure behavior matches that seam's `fill_bytes` contract.
    pub fn seeded_by(source: &mut (impl CryptoRngCore + ?Sized)) -> Self {
        let mut seed = Zeroizing::new([0u8; 32]);
        source.fill_bytes(seed.as_mut());
        Self::from_seed(*seed)
    }
}
impl fmt::Debug for SeededProtocolRng {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SeededProtocolRng").finish_non_exhaustive()
    }
}
impl SeedableRng for SeededProtocolRng {
    type Seed = [u8; 32];
    fn from_seed(seed: Self::Seed) -> Self {
        Self {
            inner: rand_chacha::ChaCha20Rng::from_seed(seed),
        }
    }
}
impl TryRng for SeededProtocolRng {
    type Error = Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        self.inner.try_next_u32()
    }
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        self.inner.try_next_u64()
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
        self.inner.try_fill_bytes(dest)
    }
}
impl TryCryptoRng for SeededProtocolRng {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CryptoRng, RngCore};

    fn sample(rng: &mut SeededProtocolRng) -> [u8; 128] {
        let mut bytes = [0; 128];
        rng.try_fill_bytes(&mut bytes).unwrap();
        bytes
    }
    #[test]
    fn protocol_rng_reproduces_supplied_seed_and_hides_state() {
        let mut a = SeededProtocolRng::from_seed([37; 32]);
        let mut b = SeededProtocolRng::from_seed([37; 32]);
        let mut c = SeededProtocolRng::from_seed([38; 32]);
        let original = sample(&mut a);
        assert_eq!(original, sample(&mut b));
        assert_ne!(original, sample(&mut c));
        assert_ne!(original, sample(&mut a));
        assert_eq!(format!("{a:?}"), "SeededProtocolRng { .. }");
    }
    #[test]
    fn protocol_rng_forks_advance_parent_and_produce_distinct_reproducible_children() {
        let mut parent = SeededProtocolRng::from_seed([41; 32]);
        let mut mirror = SeededProtocolRng::from_seed([41; 32]);
        let mut first = parent.fork();
        let mut second = parent.fork();
        let mut mirror_first = mirror.fork();
        let mut mirror_second = mirror.fork();
        let a = sample(&mut first);
        let b = sample(&mut second);
        assert_ne!(a, b);
        assert_eq!(a, sample(&mut mirror_first));
        assert_eq!(b, sample(&mut mirror_second));
        assert_eq!(sample(&mut parent), sample(&mut mirror));
    }
    #[derive(Default)]
    struct CountingSource(usize);
    impl CryptoRng for CountingSource {}
    impl RngCore for CountingSource {
        fn next_u32(&mut self) -> u32 {
            panic!("seed must use the supplied byte seam")
        }
        fn next_u64(&mut self) -> u64 {
            panic!("seed must use the supplied byte seam")
        }
        fn fill_bytes(&mut self, dest: &mut [u8]) {
            for byte in dest {
                *byte = self.0 as u8;
                self.0 += 1;
            }
        }
        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), crate::rng::RngError> {
            self.fill_bytes(dest);
            Ok(())
        }
    }
    #[test]
    fn protocol_rng_uses_exactly_one_original_seed_and_never_redraws_when_forking() {
        let mut source = CountingSource::default();
        let mut actual = SeededProtocolRng::seeded_by(&mut source);
        assert_eq!(source.0, 32);
        let mut expected = SeededProtocolRng::from_seed(std::array::from_fn(|i| i as u8));
        assert_eq!(sample(&mut actual), sample(&mut expected));
        let mut child = actual.fork();
        let _ = sample(&mut child);
        let _ = sample(&mut actual);
        assert_eq!(source.0, 32);
    }
}
