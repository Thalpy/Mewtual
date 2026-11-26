//! Fixed private Header and Welcome signing journals. An Attempt records
//! possible key entry. Only its exact Raw descendant may supply signature data.
use super::outcome::{MAX_FRAME, MIN_FRAME};
use super::*;

const TAG: &[u8] = b"catcoms/private-chat-text-governed-add-header-signing/v2\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-header-signing-key/v2";
const OUTCOME_DOMAIN: &str = "catcoms/private-chat-text-governed-add-header-outcome/v2";
const ATTEMPT_DOMAIN: &str = "catcoms/private-chat-text-governed-add-header-attempt/v2";
const WELCOME_INTENT_DOMAIN: &str =
    "catcoms/private-chat-text-governed-add-welcome-attempt-intent/v2";
const VERSION: u16 = 2;
const ATTEMPT: u8 = 1;
const RAW: u8 = 2;
pub(super) const ATTEMPT_BYTES: usize = HEADER.attempt_bytes();
pub(super) const RAW_BYTES: usize = HEADER.raw_bytes();
pub(super) const WELCOME_ATTEMPT_BYTES: usize = WELCOME.attempt_bytes();
pub(super) const WELCOME_RAW_BYTES: usize = WELCOME.raw_bytes();
pub(super) const ATTEMPT_NAME: &str = "text.add.header-attempt.v2";
pub(super) const RAW_NAME: &str = "text.add.raw-header.v2";
pub(super) const WELCOME_ATTEMPT_NAME: &str = "text.add.welcome-attempt.v2";
pub(super) const WELCOME_RAW_NAME: &str = "text.add.raw-welcome.v2";

struct SigningDomains {
    tag: &'static [u8],
    key: &'static str,
    attempt: &'static str,
}

impl SigningDomains {
    const fn attempt_bytes(&self) -> usize {
        4 + self.tag.len() + 2 + 1 + 4 + 32 + 40
    }
    const fn raw_bytes(&self) -> usize {
        self.attempt_bytes() + 4 + 32 + 4 + 64
    }
}

const HEADER: SigningDomains = SigningDomains {
    tag: TAG,
    key: KEY_DOMAIN,
    attempt: ATTEMPT_DOMAIN,
};
const WELCOME: SigningDomains = SigningDomains {
    tag: b"catcoms/private-chat-text-governed-add-welcome-signing/v2\0",
    key: "catcoms/private-chat-text-governed-add-welcome-signing-key/v2",
    attempt: "catcoms/private-chat-text-governed-add-welcome-attempt/v2",
};

/// Exact original sealed parent commitment. Construct it outside all writers;
/// authenticating the parent itself requires the preceding record's reader.
#[derive(Clone, Copy)]
pub(super) struct AddSigningParent {
    domains: &'static SigningDomains,
    commitment: [u8; 32],
}

impl AddSigningParent {
    pub(super) fn from_outcome(raw: &[u8]) -> Result<Self, AppError> {
        if !(MIN_FRAME..=MAX_FRAME).contains(&raw.len()) {
            return Err(refused());
        }
        Ok(Self {
            domains: &HEADER,
            commitment: blake3::derive_key(OUTCOME_DOMAIN, raw),
        })
    }

    pub(super) fn from_welcome_intent(raw: &[u8]) -> Result<Self, AppError> {
        if !(welcome_intent::MIN_BYTES..=welcome_intent::MAX_BYTES).contains(&raw.len()) {
            return Err(refused());
        }
        Ok(Self {
            domains: &WELCOME,
            commitment: blake3::derive_key(WELCOME_INTENT_DOMAIN, raw),
        })
    }

    pub(super) fn seal_attempt(
        self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.seal(key, None, rng)
    }

    pub(super) fn require_attempt(self, key: &[u8; 32], raw: &[u8]) -> Result<(), AppError> {
        let plain = self.open(key, raw, self.domains.attempt_bytes())?;
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
        let plain = self.open(key, raw, self.domains.raw_bytes())?;
        let mut d = Decoder::new(&plain);
        self.require_prefix(&mut d, RAW)?;
        if b32(&mut d)? != blake3::derive_key(self.domains.attempt, attempt) {
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
            (self.domains.raw_bytes(), RAW)
        } else {
            (self.domains.attempt_bytes(), ATTEMPT)
        };
        let mut e = Encoder::with_capacity(size - 40);
        put(&mut e, self.domains.tag)?;
        e.put_u16(VERSION).put_u8(role);
        put(&mut e, &self.commitment)?;
        if let Some((parent, signature)) = raw {
            put(&mut e, &blake3::derive_key(self.domains.attempt, parent))?;
            put(&mut e, signature)?;
        }
        let plain = Zeroizing::new(e.finish());
        let key = Zeroizing::new(blake3::derive_key(self.domains.key, key));
        let sealed = Zeroizing::new(frame(&seal(&key, &plain, rng)?));
        if sealed.len() != size {
            return Err(refused());
        }
        Ok(sealed)
    }

    fn open(self, key: &[u8; 32], raw: &[u8], size: usize) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if raw.len() != size {
            return Err(refused());
        }
        let key = Zeroizing::new(blake3::derive_key(self.domains.key, key));
        Ok(Zeroizing::new(unseal(&key, &unframe(raw)?)?))
    }

    fn require_prefix(self, d: &mut Decoder<'_>, role: u8) -> Result<(), AppError> {
        if field(d, self.domains.tag.len())? != self.domains.tag
            || d.get_u16().map_err(|_| refused())? != VERSION
            || d.get_u8().map_err(|_| refused())? != role
            || b32(d)? != self.commitment
        {
            return Err(refused());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
