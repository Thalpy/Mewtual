//! Multi-device pairing primitives: pairing requests, the short authentication
//! string (SAS), and origin-signed device certificates / revocations.
//!
//! This is the **v2** multi-device model of `docs/design-multi-device.md`: a
//! member's *original device* is the identity root. There is no separate account
//! key — a device certificate is
//! `sig_origin(origin_id ‖ origin_pk ‖ new_device_id ‖ device_name ‖ issued_ts)`,
//! minted by the origin device during a grant ceremony for exactly one companion.
//! **Chain depth is 1**: only the origin may certify, so a companion certificate
//! can never itself authorize a further device — nothing in this module lets a
//! certificate name a signer other than the origin.
//!
//! (The older [`crate::cert`] module implements the superseded v1 model, where a
//! separate account key rooted a multi-hop chain. Its signing domains are `/v1`
//! and this module's are `/v2`, so the two can never cross-verify.)
//!
//! ## The ceremony, and what lives here
//!
//! 1. The **new device** generates its device key and emits a [`PairingRequest`]
//!    — its public key plus a fresh 32-byte nonce. The request is deliberately
//!    *unsigned*: the SAS, compared by a human, is the authenticator.
//! 2. **Both devices** compute [`sas`] over exactly the same three inputs
//!    (`new_device_pk`, `pairing_nonce`, `origin_id`) and display the same six
//!    digits. A man-in-the-middle who substitutes its own key or nonce changes
//!    the digits on one side, and the human declines.
//! 3. On confirmation the origin mints a [`DeviceCertificate`]; later,
//!    [`DeviceRevocation`] withdraws one.
//!
//! ## What this module deliberately does *not* do
//!
//! There is no I/O, no clock and no ambient randomness here: timestamps are
//! passed in and randomness is injected as `&mut impl CryptoRngCore`, so every
//! operation is deterministically testable.
//!
//! **Single use is enforced above this crate.** A [`PairingRequest`]'s nonce is
//! consumed by *ceremony state* — accepted or declined, it must never be usable
//! twice — and that state lives in the pairing transport (M2), exactly as
//! `InviteLedger` (not the token type) enforces single use for invites.
//! Certificates and revocations here are **immutable value types**: they carry no
//! consumption bit, and freshness (`issued_ts_ms`) and not-revoked checks are the
//! admitting layer's job (M3/M5). Verification in this module answers only
//! "is this a well-formed statement genuinely signed by that origin device?".

use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};
use ed25519_dalek::VerifyingKey;
use thiserror::Error;

use crate::identity::{verify_with_public_bytes, DeviceKeypair};
use crate::ids::DeviceId;

const PAIRING_REQUEST_DOMAIN: &str = "catcoms/pairing-request/v1";
const SAS_DOMAIN: &str = "catcoms/pairing-sas/v1";
// `/v2` domains: the v1 device certificate (`crate::cert`) is rooted in an
// account key and carries a `signer` discriminant. This shape is origin-rooted
// and depth-1, so a v1 statement must never verify as a v2 one, or vice versa.
const CERT_DOMAIN: &str = "catcoms/device-cert/v2";
const REVOKE_DOMAIN: &str = "catcoms/device-revocation/v2";

/// Maximum length of a human-set device name, in bytes (matches the badge-label
/// bound used elsewhere in the app).
pub const MAX_DEVICE_NAME_BYTES: usize = 24;

/// Number of decimal digits in a short authentication string.
pub const SAS_DIGITS: u32 = 6;

/// One past the largest SAS value: codes are `0..SAS_MODULUS`, i.e. `000000`
/// through `999999` once zero-padded to [`SAS_DIGITS`].
pub const SAS_MODULUS: u32 = 1_000_000;

/// Why a pairing statement was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PairingError {
    /// The device name was empty, longer than [`MAX_DEVICE_NAME_BYTES`], or
    /// contained control characters (which would let a name spoof UI chrome).
    #[error("device name must be 1..={MAX_DEVICE_NAME_BYTES} bytes of non-control UTF-8")]
    InvalidName,
    /// A certificate may not name the origin device as its own subject.
    #[error("a device cannot certify itself")]
    SelfCertification,
    /// The bytes were not a canonical encoding of this statement.
    #[error("malformed pairing request, certificate, or revocation")]
    Malformed,
}

/// Check a device name against the bounds bound into every certificate.
///
/// Names are compared byte-wise against [`MAX_DEVICE_NAME_BYTES`] (not chars) so
/// the wire cost is bounded regardless of script, and control characters are
/// rejected so a name can never inject newlines into the grant popup.
pub fn validate_device_name(name: &str) -> Result<(), PairingError> {
    if name.is_empty() || name.len() > MAX_DEVICE_NAME_BYTES {
        return Err(PairingError::InvalidName);
    }
    if name.chars().any(char::is_control) {
        return Err(PairingError::InvalidName);
    }
    Ok(())
}

/// Reduce a uniform 64-bit draw into `0..SAS_MODULUS` by wide multiply-shift.
///
/// `(x * m) >> 64` is exactly the fixed-point scaling of `x / 2^64` into
/// `[0, m)`: bucket sizes differ by at most one draw in `2^64 / m ≈ 1.8e13`, a
/// relative bias below `2^-43` — some eight orders of magnitude beneath the
/// `10^-6` an attacker gets by simply guessing the code. Unlike `x % m` this
/// consumes the *high* bits and needs no rejection loop, so the derivation is
/// branch-free and constant-work.
fn wide_reduce(x: u64) -> u32 {
    ((x as u128 * SAS_MODULUS as u128) >> 64) as u32
}

/// Derive the six-digit short authentication string for a pairing ceremony.
///
/// Returns a value in `0..`[`SAS_MODULUS`]; render it zero-padded, e.g.
/// `format!("{:06}", code)` → `"073821"`, conventionally grouped `073 821`.
///
/// **Both devices compute this from the same three inputs** — the new device's
/// public key, the pairing nonce it generated, and the *origin* device's id —
/// and a human compares the results. The origin id is included so a request
/// captured from one pairing can never be replayed to produce a matching code
/// against a different origin device. The derivation is a domain-separated
/// BLAKE3 over a length-prefixed canonical encoding, so no two distinct input
/// triples share a preimage.
///
/// This is an *authentication* string, not a secret: it is displayed on both
/// screens and carries ~20 bits, which is a bound on a blind attacker's single
/// guess, not on offline search. Its security comes from the human comparison
/// happening before any certificate is minted.
pub fn sas(new_device_pk: &[u8; 32], pairing_nonce: &[u8; 32], origin_id: &DeviceId) -> u32 {
    let mut e = Encoder::new();
    e.put_str(SAS_DOMAIN).expect("label fits");
    e.put_bytes(new_device_pk).expect("32 bytes fit");
    e.put_bytes(pairing_nonce).expect("32 bytes fit");
    e.put_bytes(origin_id.as_bytes()).expect("32 bytes fit");
    let digest = blake3::hash(&e.finish());
    let wide = u64::from_be_bytes(
        digest.as_bytes()[..8]
            .try_into()
            .expect("digest is 32 bytes"),
    );
    wide_reduce(wide)
}

/// What a new device emits to start a grant ceremony: its public key and a fresh
/// single-use nonce, shown as a QR / copy-paste blob.
///
/// Unsigned by design — see the module docs. The nonce's single-use property is
/// enforced by the ceremony state above this crate, not by this value type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingRequest {
    /// The new device's Ed25519 public key (raw bytes). Content-addresses its
    /// [`DeviceId`].
    pub new_device_pk: [u8; 32],
    /// Single-use random nonce binding this ceremony.
    pub pairing_nonce: [u8; 32],
}

impl PairingRequest {
    /// Build a request for `new_device_pk`, drawing a fresh nonce from injected
    /// randomness.
    pub fn new(new_device_pk: &VerifyingKey, rng: &mut impl CryptoRngCore) -> Self {
        let mut pairing_nonce = [0u8; 32];
        rng.fill_bytes(&mut pairing_nonce);
        Self {
            new_device_pk: *new_device_pk.as_bytes(),
            pairing_nonce,
        }
    }

    /// The id the new device will have once admitted.
    pub fn new_device_id(&self) -> DeviceId {
        DeviceId::from_public_key_bytes(&self.new_device_pk)
    }

    /// The SAS a human compares, as computed against `origin_id`. Convenience
    /// wrapper over [`sas`] — both devices call one or the other and must agree.
    pub fn sas(&self, origin_id: &DeviceId) -> u32 {
        sas(&self.new_device_pk, &self.pairing_nonce, origin_id)
    }

    /// Serialize for transport (QR / paste).
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_str(PAIRING_REQUEST_DOMAIN).expect("label fits");
        e.put_bytes(&self.new_device_pk).expect("32 bytes fit");
        e.put_bytes(&self.pairing_nonce).expect("32 bytes fit");
        e.finish()
    }

    /// Parse a request produced by [`PairingRequest::encode`]. Rejects a public
    /// key that is not a valid Ed25519 point, so a request that could never
    /// verify a later signature never reaches the grant popup.
    pub fn decode(bytes: &[u8]) -> Result<Self, PairingError> {
        let mut d = Decoder::new(bytes);
        let domain = d.get_str().map_err(|_| PairingError::Malformed)?;
        if domain != PAIRING_REQUEST_DOMAIN {
            return Err(PairingError::Malformed);
        }
        let new_device_pk = get_32(&mut d)?;
        let pairing_nonce = get_32(&mut d)?;
        d.finish().map_err(|_| PairingError::Malformed)?;
        if VerifyingKey::from_bytes(&new_device_pk).is_err() {
            return Err(PairingError::Malformed);
        }
        Ok(Self {
            new_device_pk,
            pairing_nonce,
        })
    }
}

/// A signed assertion by an **origin** device that `new_device_id` is a
/// companion of the same member, under the human-set `device_name`.
///
/// Carries `origin_public_key` deliberately: [`DeviceId`] is a content address
/// of the key, so a verifier who holds only the id cannot check a signature.
/// Embedding the key lets any verifier both (a) check the signature and
/// (b) re-derive the id and confirm it matches — the same self-authenticating
/// shape as `InviteToken::verify_self` in `catcoms-mls`, which exists so a
/// joiner with no roster can still authenticate an invite. The key is inside the
/// signed payload, so it cannot be swapped for another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCertificate {
    /// The certifying origin device's content-addressed id.
    pub origin_id: DeviceId,
    /// The origin's Ed25519 public key (raw bytes). Must content-address
    /// `origin_id`.
    pub origin_public_key: [u8; 32],
    /// The certified companion device's id.
    pub new_device_id: DeviceId,
    /// Human-set device name, `1..=`[`MAX_DEVICE_NAME_BYTES`] bytes.
    pub device_name: String,
    /// Issue time (ms since epoch), bound into the signature. Freshness policy
    /// lives at the admitting layer.
    pub issued_ts_ms: u64,
    /// The origin's Ed25519 signature over the canonical payload.
    pub signature: [u8; 64],
}

fn write_cert_unsigned(
    e: &mut Encoder,
    origin_id: &DeviceId,
    origin_public_key: &[u8; 32],
    new_device_id: &DeviceId,
    device_name: &str,
    issued_ts_ms: u64,
) {
    e.put_str(CERT_DOMAIN).expect("label fits");
    e.put_bytes(origin_id.as_bytes()).expect("32 bytes fit");
    e.put_bytes(origin_public_key).expect("32 bytes fit");
    e.put_bytes(new_device_id.as_bytes()).expect("32 bytes fit");
    e.put_str(device_name).expect("bounded name fits");
    e.put_u64(issued_ts_ms);
}

impl DeviceCertificate {
    /// The canonical bytes the origin signs.
    pub fn signing_payload(
        origin_id: &DeviceId,
        origin_public_key: &[u8; 32],
        new_device_id: &DeviceId,
        device_name: &str,
        issued_ts_ms: u64,
    ) -> Vec<u8> {
        let mut e = Encoder::new();
        write_cert_unsigned(
            &mut e,
            origin_id,
            origin_public_key,
            new_device_id,
            device_name,
            issued_ts_ms,
        );
        e.finish()
    }

    /// Mint a certificate for one companion device.
    ///
    /// Fails on a name outside [`validate_device_name`]'s bounds, and on
    /// self-certification: a device naming itself as its own companion would
    /// make the companion → origin mapping a self-loop while asserting nothing.
    pub fn issue(
        origin: &DeviceKeypair,
        new_device_id: DeviceId,
        device_name: &str,
        now_ms: u64,
    ) -> Result<Self, PairingError> {
        validate_device_name(device_name)?;
        let origin_id = origin.device_id();
        if new_device_id == origin_id {
            return Err(PairingError::SelfCertification);
        }
        let origin_public_key = *origin.verifying_key().as_bytes();
        let payload = Self::signing_payload(
            &origin_id,
            &origin_public_key,
            &new_device_id,
            device_name,
            now_ms,
        );
        Ok(Self {
            origin_id,
            origin_public_key,
            new_device_id,
            device_name: device_name.to_string(),
            issued_ts_ms: now_ms,
            signature: origin.sign(&payload),
        })
    }

    /// Whether this certificate is a well-formed statement genuinely signed by
    /// `expected_origin`.
    ///
    /// Checks, all of which must hold:
    /// 1. the embedded `origin_id` is exactly `expected_origin` — so a valid
    ///    certificate from another member can never be re-presented here;
    /// 2. `origin_public_key` content-addresses `origin_id`;
    /// 3. the signature verifies (strictly) under that key over the canonical
    ///    payload — which covers every field, so any tamper invalidates it;
    /// 4. the name is within bounds and the subject is not the origin itself.
    ///
    /// Freshness and revocation are *not* checked here; they are the admitting
    /// layer's responsibility.
    pub fn verify(&self, expected_origin: &DeviceId) -> bool {
        if self.origin_id != *expected_origin {
            return false;
        }
        if DeviceId::from_public_key_bytes(&self.origin_public_key) != self.origin_id {
            return false;
        }
        if self.new_device_id == self.origin_id {
            return false;
        }
        if validate_device_name(&self.device_name).is_err() {
            return false;
        }
        let payload = Self::signing_payload(
            &self.origin_id,
            &self.origin_public_key,
            &self.new_device_id,
            &self.device_name,
            self.issued_ts_ms,
        );
        verify_with_public_bytes(&self.origin_public_key, &payload, &self.signature)
    }

    /// Serialize the full certificate (including signature).
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        write_cert_unsigned(
            &mut e,
            &self.origin_id,
            &self.origin_public_key,
            &self.new_device_id,
            &self.device_name,
            self.issued_ts_ms,
        );
        e.put_bytes(&self.signature).expect("64 bytes fit");
        e.finish()
    }

    /// Parse a certificate produced by [`DeviceCertificate::encode`]. Structural
    /// only — call [`DeviceCertificate::verify`] before trusting it.
    pub fn decode(bytes: &[u8]) -> Result<Self, PairingError> {
        let mut d = Decoder::new(bytes);
        let domain = d.get_str().map_err(|_| PairingError::Malformed)?;
        if domain != CERT_DOMAIN {
            return Err(PairingError::Malformed);
        }
        let origin_id = get_32(&mut d)?;
        let origin_public_key = get_32(&mut d)?;
        let new_device_id = get_32(&mut d)?;
        // `get_str` rejects invalid UTF-8; the length bound is ours.
        let device_name = d.get_str().map_err(|_| PairingError::Malformed)?;
        validate_device_name(device_name)?;
        let issued_ts_ms = d.get_u64().map_err(|_| PairingError::Malformed)?;
        let signature = get_64(&mut d)?;
        d.finish().map_err(|_| PairingError::Malformed)?;
        Ok(Self {
            origin_id: DeviceId::from_bytes(origin_id),
            origin_public_key,
            new_device_id: DeviceId::from_bytes(new_device_id),
            device_name: device_name.to_string(),
            issued_ts_ms,
            signature,
        })
    }
}

/// A signed assertion by an origin device that `revoked_device_id` is no longer
/// one of its companions.
///
/// Same carry-the-public-key shape as [`DeviceCertificate`], for the same reason.
/// Only the origin can sign one: companions hold no grant authority, so a stolen
/// companion can neither mint siblings nor revoke them.
///
/// Unlike a certificate, a revocation *may* name the origin itself — "burn this
/// identity" is a meaningful statement, whereas self-certification is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRevocation {
    /// The revoking origin device's content-addressed id.
    pub origin_id: DeviceId,
    /// The origin's Ed25519 public key (raw bytes). Must content-address
    /// `origin_id`.
    pub origin_public_key: [u8; 32],
    /// The device being revoked.
    pub revoked_device_id: DeviceId,
    /// Revocation time (ms since epoch), bound into the signature.
    pub rev_ts_ms: u64,
    /// The origin's Ed25519 signature over the canonical payload.
    pub signature: [u8; 64],
}

fn write_revoke_unsigned(
    e: &mut Encoder,
    origin_id: &DeviceId,
    origin_public_key: &[u8; 32],
    revoked_device_id: &DeviceId,
    rev_ts_ms: u64,
) {
    e.put_str(REVOKE_DOMAIN).expect("label fits");
    e.put_bytes(origin_id.as_bytes()).expect("32 bytes fit");
    e.put_bytes(origin_public_key).expect("32 bytes fit");
    e.put_bytes(revoked_device_id.as_bytes())
        .expect("32 bytes fit");
    e.put_u64(rev_ts_ms);
}

impl DeviceRevocation {
    /// The canonical bytes the origin signs.
    pub fn signing_payload(
        origin_id: &DeviceId,
        origin_public_key: &[u8; 32],
        revoked_device_id: &DeviceId,
        rev_ts_ms: u64,
    ) -> Vec<u8> {
        let mut e = Encoder::new();
        write_revoke_unsigned(
            &mut e,
            origin_id,
            origin_public_key,
            revoked_device_id,
            rev_ts_ms,
        );
        e.finish()
    }

    /// Mint a revocation for one device.
    pub fn issue(origin: &DeviceKeypair, revoked_device_id: DeviceId, now_ms: u64) -> Self {
        let origin_id = origin.device_id();
        let origin_public_key = *origin.verifying_key().as_bytes();
        let payload =
            Self::signing_payload(&origin_id, &origin_public_key, &revoked_device_id, now_ms);
        Self {
            origin_id,
            origin_public_key,
            revoked_device_id,
            rev_ts_ms: now_ms,
            signature: origin.sign(&payload),
        }
    }

    /// Whether this revocation is genuinely signed by `expected_origin`. Same
    /// three checks as [`DeviceCertificate::verify`], minus the name bounds.
    pub fn verify(&self, expected_origin: &DeviceId) -> bool {
        if self.origin_id != *expected_origin {
            return false;
        }
        if DeviceId::from_public_key_bytes(&self.origin_public_key) != self.origin_id {
            return false;
        }
        let payload = Self::signing_payload(
            &self.origin_id,
            &self.origin_public_key,
            &self.revoked_device_id,
            self.rev_ts_ms,
        );
        verify_with_public_bytes(&self.origin_public_key, &payload, &self.signature)
    }

    /// Serialize the full revocation (including signature).
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        write_revoke_unsigned(
            &mut e,
            &self.origin_id,
            &self.origin_public_key,
            &self.revoked_device_id,
            self.rev_ts_ms,
        );
        e.put_bytes(&self.signature).expect("64 bytes fit");
        e.finish()
    }

    /// Parse a revocation produced by [`DeviceRevocation::encode`]. Structural
    /// only — call [`DeviceRevocation::verify`] before trusting it.
    pub fn decode(bytes: &[u8]) -> Result<Self, PairingError> {
        let mut d = Decoder::new(bytes);
        let domain = d.get_str().map_err(|_| PairingError::Malformed)?;
        if domain != REVOKE_DOMAIN {
            return Err(PairingError::Malformed);
        }
        let origin_id = get_32(&mut d)?;
        let origin_public_key = get_32(&mut d)?;
        let revoked_device_id = get_32(&mut d)?;
        let rev_ts_ms = d.get_u64().map_err(|_| PairingError::Malformed)?;
        let signature = get_64(&mut d)?;
        d.finish().map_err(|_| PairingError::Malformed)?;
        Ok(Self {
            origin_id: DeviceId::from_bytes(origin_id),
            origin_public_key,
            revoked_device_id: DeviceId::from_bytes(revoked_device_id),
            rev_ts_ms,
            signature,
        })
    }
}

fn get_32(d: &mut Decoder<'_>) -> Result<[u8; 32], PairingError> {
    d.get_bytes()
        .map_err(|_| PairingError::Malformed)?
        .try_into()
        .map_err(|_| PairingError::Malformed)
}

fn get_64(d: &mut Decoder<'_>) -> Result<[u8; 64], PairingError> {
    d.get_bytes()
        .map_err(|_| PairingError::Malformed)?
        .try_into()
        .map_err(|_| PairingError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn rng(seed: u64) -> ChaCha20Rng {
        ChaCha20Rng::seed_from_u64(seed)
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The two ceremony participants, from a fixed seed so every vector below is
    /// reproducible.
    fn origin_and_new(seed: u64) -> (DeviceKeypair, DeviceKeypair) {
        let mut r = rng(seed);
        (
            DeviceKeypair::generate(&mut r),
            DeviceKeypair::generate(&mut r),
        )
    }

    // ---------- pairing request ----------

    #[test]
    fn pairing_request_round_trips() {
        let (_, new_dev) = origin_and_new(1);
        let req = PairingRequest::new(&new_dev.verifying_key(), &mut rng(99));
        assert_eq!(PairingRequest::decode(&req.encode()).unwrap(), req);
        assert_eq!(req.new_device_id(), new_dev.device_id());
    }

    #[test]
    fn pairing_nonce_is_drawn_fresh_from_injected_randomness() {
        let (_, new_dev) = origin_and_new(2);
        let mut r = rng(7);
        let a = PairingRequest::new(&new_dev.verifying_key(), &mut r);
        let b = PairingRequest::new(&new_dev.verifying_key(), &mut r);
        assert_ne!(a.pairing_nonce, b.pairing_nonce);
        // Same seed -> same nonce: randomness is injected, never ambient.
        let c = PairingRequest::new(&new_dev.verifying_key(), &mut rng(7));
        assert_eq!(a.pairing_nonce, c.pairing_nonce);
    }

    #[test]
    fn pairing_request_decode_rejects_malformed_input() {
        let (_, new_dev) = origin_and_new(3);
        let req = PairingRequest::new(&new_dev.verifying_key(), &mut rng(5));

        // Wrong domain.
        let mut e = Encoder::new();
        e.put_str("catcoms/pairing-request/v0").unwrap();
        e.put_bytes(&req.new_device_pk).unwrap();
        e.put_bytes(&req.pairing_nonce).unwrap();
        assert_eq!(
            PairingRequest::decode(&e.finish()),
            Err(PairingError::Malformed)
        );

        // Trailing bytes.
        let mut bytes = req.encode();
        bytes.push(0);
        assert_eq!(PairingRequest::decode(&bytes), Err(PairingError::Malformed));

        // Truncated.
        let bytes = req.encode();
        assert_eq!(
            PairingRequest::decode(&bytes[..bytes.len() - 4]),
            Err(PairingError::Malformed)
        );

        // Wrong-length public key field.
        let mut e = Encoder::new();
        e.put_str(PAIRING_REQUEST_DOMAIN).unwrap();
        e.put_bytes(&[0u8; 31]).unwrap();
        e.put_bytes(&req.pairing_nonce).unwrap();
        assert_eq!(
            PairingRequest::decode(&e.finish()),
            Err(PairingError::Malformed)
        );

        // 32 bytes that are not a decompressable Ed25519 point, so no signature
        // could ever verify under them.
        assert!(VerifyingKey::from_bytes(&[2u8; 32]).is_err());
        let mut e = Encoder::new();
        e.put_str(PAIRING_REQUEST_DOMAIN).unwrap();
        e.put_bytes(&[2u8; 32]).unwrap();
        e.put_bytes(&req.pairing_nonce).unwrap();
        assert_eq!(
            PairingRequest::decode(&e.finish()),
            Err(PairingError::Malformed)
        );
    }

    // ---------- SAS ----------

    #[test]
    fn both_devices_derive_the_same_sas() {
        let (origin, new_dev) = origin_and_new(4);
        let req = PairingRequest::new(&new_dev.verifying_key(), &mut rng(11));
        // New device computes it from its own material; origin from the decoded
        // request. Same three inputs, same code.
        let on_new = sas(&req.new_device_pk, &req.pairing_nonce, &origin.device_id());
        let on_origin = PairingRequest::decode(&req.encode())
            .unwrap()
            .sas(&origin.device_id());
        assert_eq!(on_new, on_origin);
        assert!(on_new < SAS_MODULUS);
    }

    #[test]
    fn sas_changes_when_any_input_changes() {
        let (origin, new_dev) = origin_and_new(5);
        let other_origin = DeviceKeypair::generate(&mut rng(500));
        let req = PairingRequest::new(&new_dev.verifying_key(), &mut rng(12));
        let base = req.sas(&origin.device_id());

        // Different origin id (replay to another origin yields a different code).
        assert_ne!(base, req.sas(&other_origin.device_id()));
        // Different nonce.
        let mut other_nonce = req.pairing_nonce;
        other_nonce[0] ^= 1;
        assert_ne!(
            base,
            sas(&req.new_device_pk, &other_nonce, &origin.device_id())
        );
        // Different public key (the MITM-substitution case).
        let mut other_pk = req.new_device_pk;
        other_pk[31] ^= 1;
        assert_ne!(
            base,
            sas(&other_pk, &req.pairing_nonce, &origin.device_id())
        );
    }

    #[test]
    fn sas_reduction_spans_the_full_six_digit_range() {
        // The reduction, not the statistics: both endpoints are representable and
        // nothing escapes 000000..=999999.
        assert_eq!(wide_reduce(0), 0);
        assert_eq!(wide_reduce(u64::MAX), SAS_MODULUS - 1);
        assert_eq!(wide_reduce(u64::MAX), 999_999);
        // The first draw that maps to each of the next few codes, and the last
        // that maps to 0: the buckets are contiguous and evenly cut.
        let bucket = u64::MAX / u64::from(SAS_MODULUS); // ~2^44 draws per code
        assert_eq!(wide_reduce(bucket), 0);
        assert_eq!(wide_reduce(bucket + 1), 1);
        assert_eq!(wide_reduce(u64::MAX / 2), SAS_MODULUS / 2 - 1);
        for x in [0u64, 1, 12_345, bucket, u64::MAX / 3, u64::MAX] {
            assert!(wide_reduce(x) < SAS_MODULUS);
        }
        assert_eq!(SAS_DIGITS, 6);
        assert_eq!(SAS_MODULUS, 10u32.pow(SAS_DIGITS));
    }

    #[test]
    fn sas_golden_vector() {
        // Fixed inputs -> fixed code. If the domain string, the field order, or
        // the reduction ever changes, this breaks: two devices on different
        // builds would otherwise silently show different codes.
        let pk = [0x11u8; 32];
        let nonce = [0x22u8; 32];
        let origin = DeviceId::from_bytes([0x33u8; 32]);
        assert_eq!(sas(&pk, &nonce, &origin), 129_416);
        assert_eq!(
            format!(
                "{:0width$}",
                sas(&pk, &nonce, &origin),
                width = SAS_DIGITS as usize
            ),
            "129416"
        );

        // All-zero input: in range, non-degenerate, and it exercises the
        // zero-padded rendering the two screens must agree on.
        let zero = sas(&[0u8; 32], &[0u8; 32], &DeviceId::from_bytes([0u8; 32]));
        assert_eq!(zero, 23_602);
        assert_eq!(format!("{zero:06}"), "023602");
    }

    // ---------- certificate ----------

    #[test]
    fn certificate_round_trips_and_verifies() {
        let (origin, new_dev) = origin_and_new(6);
        let cert =
            DeviceCertificate::issue(&origin, new_dev.device_id(), "phone", 1_700_000_000_000)
                .unwrap();
        assert!(cert.verify(&origin.device_id()));

        let decoded = DeviceCertificate::decode(&cert.encode()).unwrap();
        assert_eq!(decoded, cert);
        assert!(decoded.verify(&origin.device_id()));
        assert_eq!(decoded.device_name, "phone");
        assert_eq!(decoded.new_device_id, new_dev.device_id());
        assert_eq!(decoded.issued_ts_ms, 1_700_000_000_000);
    }

    #[test]
    fn certificate_rejects_a_swapped_origin_key() {
        let (origin, new_dev) = origin_and_new(7);
        let impostor = DeviceKeypair::generate(&mut rng(700));
        let cert = DeviceCertificate::issue(&origin, new_dev.device_id(), "phone", 1).unwrap();

        // Swap in another key while keeping the claimed id: the id no longer
        // content-addresses the key.
        let mut swapped = cert.clone();
        swapped.origin_public_key = *impostor.verifying_key().as_bytes();
        assert!(!swapped.verify(&origin.device_id()));

        // Swap both id and key so they agree: the signature no longer verifies.
        let mut relabelled = cert.clone();
        relabelled.origin_id = impostor.device_id();
        relabelled.origin_public_key = *impostor.verifying_key().as_bytes();
        assert!(!relabelled.verify(&impostor.device_id()));

        // A certificate genuinely signed by the impostor is still not accepted
        // when the caller expects `origin`.
        let other = DeviceCertificate::issue(&impostor, new_dev.device_id(), "phone", 1).unwrap();
        assert!(other.verify(&impostor.device_id()));
        assert!(!other.verify(&origin.device_id()));
    }

    #[test]
    fn certificate_rejects_every_tampered_field() {
        let (origin, new_dev) = origin_and_new(8);
        let cert = DeviceCertificate::issue(&origin, new_dev.device_id(), "phone", 42).unwrap();
        let expected = origin.device_id();
        assert!(cert.verify(&expected));

        let mut t = cert.clone();
        t.new_device_id = DeviceKeypair::generate(&mut rng(800)).device_id();
        assert!(!t.verify(&expected));

        let mut t = cert.clone();
        t.device_name = "laptop".into();
        assert!(!t.verify(&expected));

        let mut t = cert.clone();
        t.issued_ts_ms = 43;
        assert!(!t.verify(&expected));

        let mut t = cert.clone();
        t.signature[0] ^= 0xFF;
        assert!(!t.verify(&expected));

        let mut t = cert.clone();
        t.origin_public_key[0] ^= 0xFF;
        assert!(!t.verify(&expected));

        let mut t = cert.clone();
        t.origin_id = DeviceId::from_bytes([0u8; 32]);
        assert!(!t.verify(&expected));
        assert!(!t.verify(&DeviceId::from_bytes([0u8; 32])));
    }

    #[test]
    fn certificate_rejects_mismatched_expected_origin() {
        let (origin, new_dev) = origin_and_new(9);
        let cert = DeviceCertificate::issue(&origin, new_dev.device_id(), "phone", 1).unwrap();
        // A perfectly valid certificate, checked against the wrong origin.
        assert!(cert.verify(&origin.device_id()));
        assert!(!cert.verify(&new_dev.device_id()));
        assert!(!cert.verify(&DeviceId::from_bytes([7u8; 32])));
    }

    #[test]
    fn certificate_rejects_bad_names() {
        let (origin, new_dev) = origin_and_new(10);
        let id = new_dev.device_id();

        assert_eq!(
            DeviceCertificate::issue(&origin, id, "", 1),
            Err(PairingError::InvalidName)
        );
        // 25 bytes: one over the bound.
        assert_eq!(
            DeviceCertificate::issue(&origin, id, &"x".repeat(MAX_DEVICE_NAME_BYTES + 1), 1),
            Err(PairingError::InvalidName)
        );
        // Exactly at the bound is fine, and bytes (not chars) are what count:
        // six 4-byte emoji are 24 bytes and allowed, seven are not.
        assert!(
            DeviceCertificate::issue(&origin, id, &"x".repeat(MAX_DEVICE_NAME_BYTES), 1).is_ok()
        );
        assert!(DeviceCertificate::issue(&origin, id, &"🐱".repeat(6), 1).is_ok());
        assert_eq!(
            DeviceCertificate::issue(&origin, id, &"🐱".repeat(7), 1),
            Err(PairingError::InvalidName)
        );
        // Control characters would let a name inject lines into the grant popup.
        assert_eq!(
            DeviceCertificate::issue(&origin, id, "phone\nADMIN", 1),
            Err(PairingError::InvalidName)
        );

        // A signed-but-oversize name (forged by hand) fails verification too, so
        // the bound is not merely an issue-time courtesy.
        let long = "y".repeat(MAX_DEVICE_NAME_BYTES + 1);
        let origin_id = origin.device_id();
        let pk = *origin.verifying_key().as_bytes();
        let payload = DeviceCertificate::signing_payload(&origin_id, &pk, &id, &long, 1);
        let forged = DeviceCertificate {
            origin_id,
            origin_public_key: pk,
            new_device_id: id,
            device_name: long,
            issued_ts_ms: 1,
            signature: origin.sign(&payload),
        };
        assert!(!forged.verify(&origin_id));
        // ...and it cannot even be decoded off the wire.
        assert_eq!(
            DeviceCertificate::decode(&forged.encode()),
            Err(PairingError::InvalidName)
        );
    }

    #[test]
    fn certificate_decode_rejects_invalid_utf8_name() {
        let (origin, new_dev) = origin_and_new(11);
        let mut e = Encoder::new();
        e.put_str(CERT_DOMAIN).unwrap();
        e.put_bytes(origin.device_id().as_bytes()).unwrap();
        e.put_bytes(origin.verifying_key().as_bytes()).unwrap();
        e.put_bytes(new_dev.device_id().as_bytes()).unwrap();
        e.put_bytes(&[0xFF, 0xFE]).unwrap(); // not UTF-8
        e.put_u64(1);
        e.put_bytes(&[0u8; 64]).unwrap();
        assert_eq!(
            DeviceCertificate::decode(&e.finish()),
            Err(PairingError::Malformed)
        );
    }

    #[test]
    fn certificate_decode_rejects_wrong_domain_and_trailing_bytes() {
        let (origin, new_dev) = origin_and_new(12);
        let cert = DeviceCertificate::issue(&origin, new_dev.device_id(), "phone", 1).unwrap();

        let mut bytes = cert.encode();
        bytes.push(0);
        assert_eq!(
            DeviceCertificate::decode(&bytes),
            Err(PairingError::Malformed)
        );

        // A revocation must never decode as a certificate: distinct domains.
        let rev = DeviceRevocation::issue(&origin, new_dev.device_id(), 1);
        assert_eq!(
            DeviceCertificate::decode(&rev.encode()),
            Err(PairingError::Malformed)
        );
        assert_eq!(
            DeviceRevocation::decode(&cert.encode()),
            Err(PairingError::Malformed)
        );
    }

    #[test]
    fn certificate_rejects_self_certification() {
        let (origin, _) = origin_and_new(13);
        assert_eq!(
            DeviceCertificate::issue(&origin, origin.device_id(), "me", 1),
            Err(PairingError::SelfCertification)
        );
        // Even hand-signed, a self-certificate does not verify (depth stays 1).
        let origin_id = origin.device_id();
        let pk = *origin.verifying_key().as_bytes();
        let payload = DeviceCertificate::signing_payload(&origin_id, &pk, &origin_id, "me", 1);
        let forged = DeviceCertificate {
            origin_id,
            origin_public_key: pk,
            new_device_id: origin_id,
            device_name: "me".into(),
            issued_ts_ms: 1,
            signature: origin.sign(&payload),
        };
        assert!(!forged.verify(&origin_id));
    }

    // ---------- revocation ----------

    #[test]
    fn revocation_round_trips_and_verifies() {
        let (origin, new_dev) = origin_and_new(14);
        let rev = DeviceRevocation::issue(&origin, new_dev.device_id(), 1_700_000_000_001);
        assert!(rev.verify(&origin.device_id()));
        let decoded = DeviceRevocation::decode(&rev.encode()).unwrap();
        assert_eq!(decoded, rev);
        assert!(decoded.verify(&origin.device_id()));
        assert_eq!(decoded.revoked_device_id, new_dev.device_id());
    }

    #[test]
    fn revocation_rejects_every_tampered_field_and_wrong_origin() {
        let (origin, new_dev) = origin_and_new(15);
        let impostor = DeviceKeypair::generate(&mut rng(1500));
        let rev = DeviceRevocation::issue(&origin, new_dev.device_id(), 9);
        let expected = origin.device_id();

        let mut t = rev.clone();
        t.revoked_device_id = impostor.device_id();
        assert!(!t.verify(&expected));

        let mut t = rev.clone();
        t.rev_ts_ms = 10;
        assert!(!t.verify(&expected));

        let mut t = rev.clone();
        t.signature[63] ^= 0xFF;
        assert!(!t.verify(&expected));

        // Swapped key: no longer content-addresses the claimed origin id.
        let mut t = rev.clone();
        t.origin_public_key = *impostor.verifying_key().as_bytes();
        assert!(!t.verify(&expected));

        // Valid revocation from someone else, checked against `origin`.
        let other = DeviceRevocation::issue(&impostor, new_dev.device_id(), 9);
        assert!(other.verify(&impostor.device_id()));
        assert!(!other.verify(&expected));
    }

    #[test]
    fn origin_may_revoke_itself() {
        // Self-revocation ("burn this identity") is meaningful, unlike
        // self-certification.
        let (origin, _) = origin_and_new(16);
        let rev = DeviceRevocation::issue(&origin, origin.device_id(), 1);
        assert!(rev.verify(&origin.device_id()));
    }

    // ---------- golden vectors ----------

    #[test]
    fn encoding_golden_vectors() {
        // Everything below is derived from a fixed seed and fixed timestamp, so
        // any change to a domain string, field order, or field width breaks this
        // test rather than silently forking the wire format between builds.
        // Ed25519 signing is deterministic (RFC 8032), so signatures pin too.
        let (origin, new_dev) = origin_and_new(1234);
        assert_eq!(
            hex(origin.verifying_key().as_bytes()),
            "acdd6d5b53bfee478bf689f8e012fe7988bf755e3d7c5152947abc149bc20189"
        );
        assert_eq!(
            hex(origin.device_id().as_bytes()),
            "1eb7a17a7c614ef23a4df230e7b0072bd57c757cfe0915419f788264a697c235"
        );
        assert_eq!(
            hex(new_dev.device_id().as_bytes()),
            "405e4106e081dd008d77dc69bf66be5650f06efd5273661e7a7086bb77d17c44"
        );

        // Certificate: 26-byte domain field + 3×36 id/key fields + 9-byte name
        // field + 8-byte timestamp + 68-byte signature field = 219 bytes.
        let cert =
            DeviceCertificate::issue(&origin, new_dev.device_id(), "phone", 1_700_000_000_000)
                .unwrap();
        assert_eq!(cert.encode().len(), 219);
        assert_eq!(
            hex(&cert.signature),
            "d5c37c1e17b49fb8af440292989b315873a5864ba639bb0c8ccf51950f21a31b\
             4e0ead7cdb458e3b92a368a2f46ed0cc5780f3b467a3e7a4af13ed5274d3c703"
        );
        assert_eq!(
            hex(blake3::hash(&cert.encode()).as_bytes()),
            "fa0aa4fbc0c6a30a2ad1282576e95ad85af2d1965b74bfa0a6f029a5fe1dbde5"
        );

        let rev = DeviceRevocation::issue(&origin, new_dev.device_id(), 1_700_000_000_000);
        assert_eq!(
            hex(&rev.signature),
            "3329a03241f02436f9f4a784a91b00b7f07a8ac337d82c0c98da9e07ad5b3a60\
             e08149b9b95d492d376ce39a9537860cced33380e35fd9ead5ee97e69956710e"
        );
        assert_eq!(
            hex(blake3::hash(&rev.encode()).as_bytes()),
            "1d60b3306288b73401a7af721a84d4b7a0b9133df5c6a0116a36b946d267fca7"
        );
        // The two statements share ids and timestamp yet cannot collide: the
        // domain label is the first signed field.
        assert_ne!(cert.signature, rev.signature);

        // The pairing request is small enough to pin byte-for-byte:
        // `0000001a` len(26) ‖ "catcoms/pairing-request/v1"
        // `00000020` len(32) ‖ new_device_pk ‖ `00000020` len(32) ‖ pairing_nonce.
        let req = PairingRequest::new(&new_dev.verifying_key(), &mut rng(4321));
        assert_eq!(
            hex(&req.encode()),
            "0000001a636174636f6d732f70616972696e672d726571756573742f7631\
             00000020a060270db7e9c9f06e8f9cc33a64e99f6596af12cb01c4b638df8afc7b642463\
             0000002073261c3c42f877ec5c101a9818cc12554700398fce626a4fa7acc484ccebf7f0"
        );
        assert_eq!(req.sas(&origin.device_id()), 149_313);
    }
}
