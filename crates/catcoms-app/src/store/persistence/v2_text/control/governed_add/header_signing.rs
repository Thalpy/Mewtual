//! Fixed private Header signing journal. An authenticated Attempt records
//! possible key entry. Only its exact Raw descendant may supply signature data.
use super::outcome::{MAX_FRAME, MIN_FRAME};
use super::*;

const TAG: &[u8] = b"catcoms/private-chat-text-governed-add-header-signing/v2\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-header-signing-key/v2";
const OUTCOME_DOMAIN: &str = "catcoms/private-chat-text-governed-add-header-outcome/v2";
const ATTEMPT_DOMAIN: &str = "catcoms/private-chat-text-governed-add-header-attempt/v2";
const VERSION: u16 = 2;
const ATTEMPT: u8 = 1;
const RAW: u8 = 2;
const ATTEMPT_PLAIN: usize = 4 + TAG.len() + 2 + 1 + 4 + 32;
const RAW_PLAIN: usize = ATTEMPT_PLAIN + 4 + 32 + 4 + 64;
pub(super) const ATTEMPT_BYTES: usize = ATTEMPT_PLAIN + 40;
pub(super) const RAW_BYTES: usize = RAW_PLAIN + 40;
pub(super) const ATTEMPT_NAME: &str = "text.add.header-attempt.v2";
pub(super) const RAW_NAME: &str = "text.add.raw-header.v2";

/// Exact original outcome commitment. Construct it outside all writers; it
/// identifies data only and cannot authenticate a captured outcome file.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct HeaderOutcomeCommitment([u8; 32]);

impl HeaderOutcomeCommitment {
    pub(super) fn from_outcome(raw: &[u8]) -> Result<Self, AppError> {
        if !(MIN_FRAME..=MAX_FRAME).contains(&raw.len()) {
            return Err(refused());
        }
        Ok(Self(blake3::derive_key(OUTCOME_DOMAIN, raw)))
    }

    pub(super) fn seal_attempt(
        self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.seal(key, None, rng)
    }

    pub(super) fn require_attempt(self, key: &[u8; 32], raw: &[u8]) -> Result<(), AppError> {
        let plain = Self::open(key, raw, ATTEMPT_BYTES)?;
        let mut d = Decoder::new(&plain);
        self.require_prefix(&mut d, ATTEMPT)?;
        d.finish().map_err(|_| refused())
    }

    pub(super) fn seal_raw(
        self,
        key: &[u8; 32],
        attempt: &[u8],
        signature: &[u8; 64],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.require_attempt(key, attempt)?;
        self.seal(key, Some((attempt, signature)), rng)
    }

    /// Read the exact original Raw bytes. The receiver independently verifies
    /// their signature over the authenticated outcome's original preimage.
    pub(super) fn open_raw(
        self,
        key: &[u8; 32],
        attempt: &[u8],
        raw: &[u8],
    ) -> Result<[u8; 64], AppError> {
        self.require_attempt(key, attempt)?;
        let plain = Self::open(key, raw, RAW_BYTES)?;
        let mut d = Decoder::new(&plain);
        self.require_prefix(&mut d, RAW)?;
        if b32(&mut d)? != blake3::derive_key(ATTEMPT_DOMAIN, attempt) {
            return Err(refused());
        }
        let signature = field(&mut d, 64)?.try_into().map_err(|_| refused())?;
        d.finish().map_err(|_| refused())?;
        Ok(signature)
    }

    fn seal(
        self,
        key: &[u8; 32],
        raw: Option<(&[u8], &[u8; 64])>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let (size, role) = if raw.is_some() {
            (RAW_BYTES, RAW)
        } else {
            (ATTEMPT_BYTES, ATTEMPT)
        };
        let mut e = Encoder::with_capacity(size - 40);
        put(&mut e, TAG)?;
        e.put_u16(VERSION).put_u8(role);
        put(&mut e, &self.0)?;
        if let Some((parent, signature)) = raw {
            put(&mut e, &blake3::derive_key(ATTEMPT_DOMAIN, parent))?;
            put(&mut e, signature)?;
        }
        let plain = Zeroizing::new(e.finish());
        let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        let sealed = Zeroizing::new(frame(&seal(&key, &plain, rng)?));
        if sealed.len() != size {
            return Err(refused());
        }
        Ok(sealed)
    }

    fn open(key: &[u8; 32], raw: &[u8], size: usize) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if raw.len() != size {
            return Err(refused());
        }
        let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        Ok(Zeroizing::new(unseal(&key, &unframe(raw)?)?))
    }

    fn require_prefix(self, d: &mut Decoder<'_>, role: u8) -> Result<(), AppError> {
        if field(d, TAG.len())? != TAG
            || d.get_u16().map_err(|_| refused())? != VERSION
            || d.get_u8().map_err(|_| refused())? != role
            || b32(d)? != self.0
        {
            return Err(refused());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
