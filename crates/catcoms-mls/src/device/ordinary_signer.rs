//! Narrow retained-device signing for Chat V2 ordinary member actions.
//!
//! Ordinary Create/reply, own Edit/Delete and Join are signed by the member's own retained
//! MLS leaf key, never by the Profile founder's governing Header path. This is the one entry
//! point the V2 ordinary signer in `catcoms-sync` uses. It differs from [`MlsDevice::sign`] in
//! three ways a caller cannot get wrong silently:
//!
//! - the caller names the FULL 32-byte public key it was admitted for. A device whose retained
//!   keypair, credential or content-addressed id disagrees with that key refuses before the
//!   private key is touched, so a replacement/restored device with another key cannot answer
//!   for an admission made against the original one;
//! - the payload must be non-empty and within an explicit caller bound, so an unbounded buffer
//!   never reaches the signer;
//! - it touches neither provider storage, SDK group state, channel-secret export nor the sender
//!   ratchet, and it does not require SDK operation admission: signing with the retained
//!   `SignatureKeyPair` mutates nothing in a closed (restored) SDK graph.
//!
//! This is NOT authorization. The caller must already hold its own Core plan admission and
//! native/source guard; this function only proves "the key that signs is the key expected".
use super::*;
use openmls_traits::types::SignatureScheme;

impl MlsDevice {
    /// Allocation-free comparison of this device's retained Ed25519 keypair, credential key
    /// and content-addressed id with `expected_public`. Callers holding a Core signing guard
    /// use it BEFORE entering the key, so a mismatch refuses without consuming the key entry.
    pub fn retained_ed25519_key_matches(&self, expected_public: &[u8; 32]) -> bool {
        let public = self.signer.public();
        self.signer.signature_scheme() == SignatureScheme::ED25519
            && public == expected_public.as_slice()
            && self.credential.signature_key.as_slice() == public
            && self.device_id == DeviceId::from_public_key_bytes(public)
    }

    /// Sign `payload` with this device's retained Ed25519 leaf key, after checking that the
    /// retained key is exactly `expected_public`.
    ///
    /// Errors (all before the private-key operation):
    /// - the signature scheme is not Ed25519;
    /// - the retained public key, its credential key, or the derived device id disagrees with
    ///   `expected_public`;
    /// - `payload` is empty or longer than `max_payload`.
    ///
    /// A signer failure after the checks is also an error. No partial result is returned.
    pub fn sign_retained_ed25519_bounded(
        &self,
        expected_public: &[u8; 32],
        payload: &[u8],
        max_payload: usize,
    ) -> Result<[u8; 64], MlsError> {
        if !self.retained_ed25519_key_matches(expected_public) {
            return Err(MlsError::Internal(
                "retained device key does not match the admitted signer",
            ));
        }
        if payload.is_empty() || payload.len() > max_payload {
            return Err(MlsError::Internal(
                "ordinary signing payload is outside its explicit bound",
            ));
        }
        self.sign_raw(payload)
    }
}

#[cfg(test)]
mod tests;
