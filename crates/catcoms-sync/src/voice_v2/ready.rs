//! Signed causal readiness metadata. This public wire value is NOT a native receipt or
//! permission to use a key/engine/RTC. Only the exact protected pair may accept it, after
//! comparing its own original Finish identity/hash and repeating current native authority.
use super::*;

pub(crate) mod admission;
pub(in crate::voice_v2) mod capacity;
pub(in crate::voice_v2) mod producer;
pub(crate) mod source_authority;

/// Untrusted, allocation-free header view used ONLY to select an existing native verifier.
/// Signature, helper policy, immutable identity and final admission remain separate checks.
pub(super) struct ReadyPeek {
    pub epoch: u64,
    pub target: DeviceId,
    pub source_join: Nonce,
    pub target_join: Nonce,
    pub source_key_ref: Nonce,
    pub target_key_ref: Nonce,
    pub handshake: Nonce,
    pub finish_message: MessageId,
    pub finish_object_hash: [u8; 32],
    pub helpers: [Option<DeviceId>; 2],
}
impl ReadyPeek {
    pub(super) fn get(d: &mut Decoder<'_>) -> Result<Self, VoiceError> {
        let mut value = Self {
            epoch: d.get_u64()?,
            target: DeviceId::from_bytes(array(d)?),
            source_join: array(d)?,
            target_join: array(d)?,
            source_key_ref: array(d)?,
            target_key_ref: array(d)?,
            handshake: array(d)?,
            finish_message: array(d)?,
            finish_object_hash: array(d)?,
            helpers: [None; 2],
        };
        let count = d.get_u8()?;
        if count > 2 {
            return Err(VoiceError::Bounds);
        }
        for helper in value.helpers.iter_mut().take(usize::from(count)) {
            *helper = Some(DeviceId::from_bytes(array(d)?));
        }
        Ok(value)
    }
}

/// High endpoint's immutable receive-readiness claim for one original native Finish. The
/// hash is the native SIGNED Finish object's hash, not a WebCrypto output fingerprint.
/// Helpers are selected once by high, then adopted exactly by low; endpoint-independent
/// helper selection could otherwise consume all original Offer attempts on incompatible paths.
#[derive(Clone, PartialEq, Eq)]
pub struct ReadyHeader {
    pub epoch: u64,
    pub target: DeviceId,
    pub source_join: Nonce,
    pub target_join: Nonce,
    pub source_key_ref: Nonce,
    pub target_key_ref: Nonce,
    pub handshake: Nonce,
    pub finish_message: MessageId,
    pub finish_object_hash: [u8; 32],
    /// Strictly increasing, at most two current members; neither endpoint can be a helper.
    /// Membership and current policy remain native admission checks, independent of signing.
    pub helpers: Vec<DeviceId>,
}

#[cfg(test)]
mod tests;
redacted_debug!(ReadyHeader);

impl ReadyHeader {
    pub(super) fn validate(&self, issuer: DeviceId, scope: Scope) -> Result<(), VoiceError> {
        // Confirmation's low endpoint signs Finish. Only its HIGH counterpart can claim
        // receive readiness; a low-side Ready must never authorize its own first Offer.
        if issuer <= self.target {
            return Err(VoiceError::Malformed);
        }
        if self.helpers.len() > 2 {
            return Err(VoiceError::Bounds);
        }
        if self.helpers.windows(2).any(|pair| pair[0] >= pair[1])
            || self
                .helpers
                .iter()
                .any(|helper| *helper == issuer || *helper == self.target)
        {
            return Err(VoiceError::Malformed);
        }
        if scope.policy.is_none() && !self.helpers.is_empty() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    pub(super) fn put(&self, e: &mut Encoder) {
        e.put_u64(self.epoch);
        fixed(e, self.target.as_bytes());
        fixed(e, &self.source_join);
        fixed(e, &self.target_join);
        fixed(e, &self.source_key_ref);
        fixed(e, &self.target_key_ref);
        fixed(e, &self.handshake);
        fixed(e, &self.finish_message);
        fixed(e, &self.finish_object_hash);
        // validate() precedes every signing/encoding entry. This count has exactly one
        // canonical representation, with no optional-field or extra-candidate fallback.
        e.put_u8(self.helpers.len() as u8);
        for helper in &self.helpers {
            fixed(e, helper.as_bytes());
        }
    }
    pub(super) fn get(d: &mut Decoder<'_>) -> Result<Self, VoiceError> {
        let epoch = d.get_u64()?;
        let target = DeviceId::from_bytes(array(d)?);
        let source_join = array(d)?;
        let target_join = array(d)?;
        let source_key_ref = array(d)?;
        let target_key_ref = array(d)?;
        let handshake = array(d)?;
        let finish_message = array(d)?;
        let finish_object_hash = array(d)?;
        let count = d.get_u8()?;
        if count > 2 {
            return Err(VoiceError::Bounds);
        }
        let mut helpers = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            helpers.push(DeviceId::from_bytes(array(d)?));
        }
        Ok(Self {
            epoch,
            target,
            source_join,
            target_join,
            source_key_ref,
            target_key_ref,
            handshake,
            finish_message,
            finish_object_hash,
            helpers,
        })
    }
}
