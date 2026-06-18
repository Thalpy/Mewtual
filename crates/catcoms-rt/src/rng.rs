//! The RNG seam: cryptographically-secure randomness, injectable everywhere.
//!
//! Like [`crate::Clock`], randomness is a dependency-injection point so the whole
//! stack is deterministically testable. Production code obtains randomness from
//! [`OsCryptoRng`]; tests seed a CSPRNG (e.g. `rand_chacha::ChaCha20Rng`). Every
//! consumer takes `&mut impl CryptoRngCore`, so it never names a concrete RNG.
//!
//! This module is the **only** sanctioned place to construct an OS RNG; the CI
//! ambient-dependency gate forbids `OsRng` / `thread_rng` everywhere else.

pub use rand_core::{CryptoRng, CryptoRngCore, Error as RngError, RngCore};

/// The operating-system CSPRNG — the single sanctioned source of OS randomness.
///
/// Pass `&mut OsCryptoRng` wherever a `CryptoRngCore` is required in production.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsCryptoRng;

impl RngCore for OsCryptoRng {
    fn next_u32(&mut self) -> u32 {
        rand_core::OsRng.next_u32()
    }

    fn next_u64(&mut self) -> u64 {
        rand_core::OsRng.next_u64()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        rand_core::OsRng.fill_bytes(dest)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), RngError> {
        rand_core::OsRng.try_fill_bytes(dest)
    }
}

impl CryptoRng for OsCryptoRng {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_rng_fills_and_varies() {
        let mut rng = OsCryptoRng;
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        rng.fill_bytes(&mut a);
        rng.fill_bytes(&mut b);
        // Two independent 256-bit draws colliding has probability ~2^-256.
        assert_ne!(a, b);
    }

    #[test]
    fn usable_as_dyn_crypto_rng_core() {
        fn draw(rng: &mut dyn CryptoRngCore) -> u64 {
            rng.next_u64()
        }
        let mut rng = OsCryptoRng;
        let _ = draw(&mut rng);
    }
}
