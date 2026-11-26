//! An independent durable record of possible SDK entry. A successful decode
//! only identifies the covered fence. It cannot resume or renew SDK admission.
use super::*;

const TAG: &[u8] = b"catcoms/private-chat-text-governed-add-sdk-entry/v2\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-sdk-entry-key/v2";
const FENCE_DOMAIN: &str = "catcoms/private-chat-text-governed-add-sdk-entry-parent/v2";
const VERSION: u16 = 2;
const PLAIN_BYTES: usize = 4 + TAG.len() + 2 + 4 + 32;
pub(in crate::store::persistence::v2_text) const SDK_ENTRY_NAME: &str = "text.add.sdk-entry.v2";
pub(in crate::store::persistence::v2_text) const ENTRY_BYTES: usize = PLAIN_BYTES + 40;

/// This commits the exact encrypted fence representation, including its
/// nonce. Independently sealing equal semantic labels changes this identity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::persistence::v2_text) struct GovernedAddSdkEntryData {
    fence: [u8; 32],
}

impl GovernedAddSdkEntryData {
    pub(in crate::store::persistence::v2_text) fn from_fence_data(
        raw: &[u8],
    ) -> Result<Self, AppError> {
        if raw.len() != FRAME_BYTES {
            return Err(refused());
        }
        Ok(Self {
            fence: blake3::derive_key(FENCE_DOMAIN, raw),
        })
    }

    pub(in crate::store::persistence::v2_text) fn seal_data(
        self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(PLAIN_BYTES);
        put(&mut e, TAG)?;
        e.put_u16(VERSION);
        put(&mut e, &self.fence)?;
        let plain = Zeroizing::new(e.finish());
        let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        let raw = Zeroizing::new(frame(&seal(&key, &plain, rng)?));
        if raw.len() != ENTRY_BYTES {
            return Err(refused());
        }
        Ok(raw)
    }

    /// A complete authenticated entry binds only the supplied exact fence.
    /// The caller has to independently prove that fence is the original held
    /// reservation in the current group directory.
    pub(in crate::store::persistence::v2_text) fn require_fence_data(
        self,
        key: &[u8; 32],
        frame: &[u8],
    ) -> Result<(), AppError> {
        if frame.len() != ENTRY_BYTES {
            return Err(refused());
        }
        let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        let plain = Zeroizing::new(unseal(&key, &unframe(frame)?)?);
        let mut d = Decoder::new(&plain);
        if field(&mut d, TAG.len())? != TAG
            || d.get_u16().map_err(|_| refused())? != VERSION
            || b32(&mut d)? != self.fence
        {
            return Err(refused());
        }
        d.finish().map_err(|_| refused())
    }
}

#[cfg(test)]
mod tests;
