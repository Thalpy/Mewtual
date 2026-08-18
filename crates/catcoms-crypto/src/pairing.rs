//! Multi-device pairing primitives: pairing requests, the short authentication
//! string (SAS), origin-signed device certificates / revocations, and the signed
//! transfer of grant authority between a member's own devices.
//!
//! This is the **v2** multi-device model of `docs/design-multi-device.md`: a
//! member's *original device* is the identity root. There is no separate account
//! key; a device certificate is
//! `sig_origin(origin_id ‖ origin_pk ‖ new_device_id ‖ device_name ‖ issued_ts)`,
//! minted by the origin device during a grant ceremony for exactly one companion.
//! **Chain depth is 1**: only the origin may certify, so a companion certificate
//! can never itself authorize a further device; nothing in this module lets a
//! certificate name a signer other than the origin.
//!
//! (The superseded v1 model; a separate account key rooting a multi-hop chain;
//! was deleted with its module; every domain here is `/v2`, so no statement from
//! that era could verify against this one even if one were replayed from a
//! backup.)
//!
//! ## The ceremony, and what lives here
//!
//! 1. The **new device** generates its device key and emits a [`PairingRequest`]
//!    its public key plus a fresh 32-byte nonce. The request is deliberately
//!    *unsigned*: the SAS, compared by a human, is the authenticator.
//! 2. **Both devices** compute [`sas`] over exactly the same three inputs
//!    (`new_device_pk`, `pairing_nonce`, `origin_id`) and display the same six
//!    digits. A man-in-the-middle who substitutes its own key or nonce changes
//!    the digits on one side, and the human declines.
//! 3. On confirmation the origin mints a [`DeviceCertificate`]; later,
//!    [`DeviceRevocation`] withdraws one, and [`MasterHandoff`] moves the right to
//!    mint either one to another device the member already controls.
//!
//! ## What this module deliberately does *not* do
//!
//! There is no I/O, no clock and no ambient randomness here: timestamps are
//! passed in and randomness is injected as `&mut impl CryptoRngCore`, so every
//! operation is deterministically testable.
//!
//! **Single use is enforced above this crate.** A [`PairingRequest`]'s nonce is
//! consumed by *ceremony state*; accepted or declined, it must never be usable
//! twice; and that state lives in the pairing transport (M2), exactly as
//! `InviteLedger` (not the token type) enforces single use for invites.
//! Certificates, revocations and handoffs here are **immutable value types**: they
//! carry no consumption bit, and freshness (`issued_ts_ms`), not-revoked and
//! monotonic-`master_seq` checks are the admitting layer's job (M3/M5).
//! Verification in this module answers only "is this a well-formed statement
//! genuinely signed by that origin device?".

use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};
use ed25519_dalek::VerifyingKey;
use thiserror::Error;

use crate::identity::{verify_with_public_bytes, DeviceKeypair};
use crate::ids::DeviceId;

const PAIRING_REQUEST_DOMAIN: &str = "catcoms/pairing-request/v1";
const SAS_DOMAIN: &str = "catcoms/pairing-sas/v1";
// `/v2` domains: the deleted v1 device certificate was rooted in an account key
// and carried a `signer` discriminant. This shape is origin-rooted and depth-1,
// so a v1 statement must never verify as a v2 one, or vice versa.
const CERT_DOMAIN: &str = "catcoms/device-cert/v2";
const REVOKE_DOMAIN: &str = "catcoms/device-revocation/v2";
const HANDOFF_DOMAIN: &str = "catcoms/master-handoff/v2";

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
    /// A master handoff may not name the current master as its own successor.
    #[error("a device cannot hand the master role to itself")]
    SelfHandoff,
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
    // Control characters could inject lines into the grant popup; bidi overrides and
    // zero-width characters could make one name render as another wherever it appears
    // (the popup now, message attribution at M4).
    if name.chars().any(|c| {
        char::is_control(c)
            || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
            || matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}')
    }) {
        return Err(PairingError::InvalidName);
    }
    Ok(())
}

/// Reduce a uniform 64-bit draw into `0..SAS_MODULUS` by wide multiply-shift.
///
/// `(x * m) >> 64` is exactly the fixed-point scaling of `x / 2^64` into
/// `[0, m)`: bucket sizes differ by at most one draw in `2^64 / m ≈ 1.8e13`, a
/// relative bias below `2^-43`; some eight orders of magnitude beneath the
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
/// **Both devices compute this from the same three inputs**; the new device's
/// public key, the pairing nonce it generated, and the *origin* device's id;
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
/// Unsigned by design; see the module docs. The nonce's single-use property is
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
    /// wrapper over [`sas`]; both devices call one or the other and must agree.
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
/// (b) re-derive the id and confirm it matches; the same self-authenticating
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
    /// The MLS group id this certificate admits into, `1..=`[`MAX_CERT_GROUP_ID_BYTES`]
    /// bytes, **bound into the signature**; so a certificate minted for one server can
    /// never be replayed to admit the same device somewhere else, even if a member ever
    /// reuses an origin key across groups. (Adversarial-review finding on the M2 slice;
    /// mirrors `InviteToken`, which signs its `group_id` for the same reason.)
    pub group_id: Vec<u8>,
    /// Human-set device name, `1..=`[`MAX_DEVICE_NAME_BYTES`] bytes.
    pub device_name: String,
    /// Issue time (ms since epoch), bound into the signature. Freshness policy
    /// lives at the admitting layer.
    pub issued_ts_ms: u64,
    /// The origin's Ed25519 signature over the canonical payload.
    pub signature: [u8; 64],
}

/// Upper bound on [`DeviceCertificate::group_id`] (MLS group ids are far smaller).
pub const MAX_CERT_GROUP_ID_BYTES: usize = 64;

fn validate_cert_group_id(group_id: &[u8]) -> Result<(), PairingError> {
    if group_id.is_empty() || group_id.len() > MAX_CERT_GROUP_ID_BYTES {
        return Err(PairingError::Malformed);
    }
    Ok(())
}

fn write_cert_unsigned(
    e: &mut Encoder,
    origin_id: &DeviceId,
    origin_public_key: &[u8; 32],
    new_device_id: &DeviceId,
    group_id: &[u8],
    device_name: &str,
    issued_ts_ms: u64,
) {
    e.put_str(CERT_DOMAIN).expect("label fits");
    e.put_bytes(origin_id.as_bytes()).expect("32 bytes fit");
    e.put_bytes(origin_public_key).expect("32 bytes fit");
    e.put_bytes(new_device_id.as_bytes()).expect("32 bytes fit");
    e.put_bytes(group_id).expect("bounded group id fits");
    e.put_str(device_name).expect("bounded name fits");
    e.put_u64(issued_ts_ms);
}

impl DeviceCertificate {
    /// The canonical bytes the origin signs.
    pub fn signing_payload(
        origin_id: &DeviceId,
        origin_public_key: &[u8; 32],
        new_device_id: &DeviceId,
        group_id: &[u8],
        device_name: &str,
        issued_ts_ms: u64,
    ) -> Vec<u8> {
        let mut e = Encoder::new();
        write_cert_unsigned(
            &mut e,
            origin_id,
            origin_public_key,
            new_device_id,
            group_id,
            device_name,
            issued_ts_ms,
        );
        e.finish()
    }

    /// Mint a certificate admitting one companion device into one group.
    ///
    /// Fails on a name outside [`validate_device_name`]'s bounds, a group id outside
    /// its bound, and on self-certification: a device naming itself as its own
    /// companion would make the companion → origin mapping a self-loop while asserting
    /// nothing.
    pub fn issue(
        origin: &DeviceKeypair,
        new_device_id: DeviceId,
        group_id: &[u8],
        device_name: &str,
        now_ms: u64,
    ) -> Result<Self, PairingError> {
        validate_device_name(device_name)?;
        validate_cert_group_id(group_id)?;
        let origin_id = origin.device_id();
        if new_device_id == origin_id {
            return Err(PairingError::SelfCertification);
        }
        let origin_public_key = *origin.verifying_key().as_bytes();
        let payload = Self::signing_payload(
            &origin_id,
            &origin_public_key,
            &new_device_id,
            group_id,
            device_name,
            now_ms,
        );
        Ok(Self {
            origin_id,
            origin_public_key,
            new_device_id,
            group_id: group_id.to_vec(),
            device_name: device_name.to_string(),
            issued_ts_ms: now_ms,
            signature: origin.sign(&payload),
        })
    }

    /// Whether this certificate is a well-formed statement genuinely signed by
    /// `expected_origin`.
    ///
    /// Checks, all of which must hold:
    /// 1. the embedded `origin_id` is exactly `expected_origin`; so a valid
    ///    certificate from another member can never be re-presented here;
    /// 2. `origin_public_key` content-addresses `origin_id`;
    /// 3. the signature verifies (strictly) under that key over the canonical
    ///    payload; which covers every field, so any tamper invalidates it;
    /// 4. the name and group id are within bounds and the subject is not the origin
    ///    itself.
    ///
    /// Freshness, revocation, and *which* group this certificate must name are the
    /// admitting layer's responsibility (it compares `group_id` to its own).
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
        if validate_device_name(&self.device_name).is_err()
            || validate_cert_group_id(&self.group_id).is_err()
        {
            return false;
        }
        let payload = Self::signing_payload(
            &self.origin_id,
            &self.origin_public_key,
            &self.new_device_id,
            &self.group_id,
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
            &self.group_id,
            &self.device_name,
            self.issued_ts_ms,
        );
        e.put_bytes(&self.signature).expect("64 bytes fit");
        e.finish()
    }

    /// Parse a certificate produced by [`DeviceCertificate::encode`]. Structural
    /// only; call [`DeviceCertificate::verify`] before trusting it.
    pub fn decode(bytes: &[u8]) -> Result<Self, PairingError> {
        let mut d = Decoder::new(bytes);
        let domain = d.get_str().map_err(|_| PairingError::Malformed)?;
        if domain != CERT_DOMAIN {
            return Err(PairingError::Malformed);
        }
        let origin_id = get_32(&mut d)?;
        let origin_public_key = get_32(&mut d)?;
        let new_device_id = get_32(&mut d)?;
        let group_id = d.get_bytes().map_err(|_| PairingError::Malformed)?;
        validate_cert_group_id(group_id)?;
        let group_id = group_id.to_vec();
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
            group_id,
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
/// Unlike a certificate, a revocation *may* name the origin itself; "burn this
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
    /// only; call [`DeviceRevocation::verify`] before trusting it.
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

/// A signed transfer of **grant authority** from the current master device to one
/// other device the member already controls.
///
/// The master is *transferable, not distributable*
/// (`docs/design-multi-device.md`): the member may **move** the right to mint
/// [`DeviceCertificate`]s and [`DeviceRevocation`]s to an accepted device; the
/// safe form of "elected master"; but nothing here hands out a second copy of it.
/// Companion **self-election is rejected by construction**: only the current
/// master's key produces a handoff that verifies, so surviving devices cannot
/// crown one of their own without it.
///
/// Same carry-the-public-key shape as [`DeviceCertificate`], for the same reason:
/// a verifier holding only the content-addressed id could not check a signature.
///
/// # `master_seq` is a fence this module does not enforce
///
/// `master_seq` exists so a *consumer* can reject a stale handoff; a replayed
/// older statement that would hand the master back to a device the member has
/// since retired. **Enforcing that is the admitting layer's job (M3/M5)**: it must
/// remember the highest `master_seq` it has accepted for this identity and reject
/// anything at or below it, exactly as it enforces certificate freshness and
/// revocation. [`MasterHandoff::verify`] answers only "is this a well-formed
/// statement genuinely signed by that master?" and will happily verify a handoff
/// with `master_seq = 0` presented after one with `master_seq = 9`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterHandoff {
    /// The **current** master (the identity root) making the transfer.
    pub origin_id: DeviceId,
    /// The current master's Ed25519 public key (raw bytes). Must content-address
    /// `origin_id`.
    pub origin_public_key: [u8; 32],
    /// The device that becomes the master once this handoff is accepted.
    pub new_master_device_id: DeviceId,
    /// Monotonic transfer counter, bound into the signature. See the type docs:
    /// monotonicity is enforced above this crate.
    pub master_seq: u64,
    /// Handoff time (ms since epoch), bound into the signature.
    pub ts_ms: u64,
    /// The current master's Ed25519 signature over the canonical payload.
    pub signature: [u8; 64],
}

fn write_handoff_unsigned(
    e: &mut Encoder,
    origin_id: &DeviceId,
    origin_public_key: &[u8; 32],
    new_master_device_id: &DeviceId,
    master_seq: u64,
    ts_ms: u64,
) {
    e.put_str(HANDOFF_DOMAIN).expect("label fits");
    e.put_bytes(origin_id.as_bytes()).expect("32 bytes fit");
    e.put_bytes(origin_public_key).expect("32 bytes fit");
    e.put_bytes(new_master_device_id.as_bytes())
        .expect("32 bytes fit");
    e.put_u64(master_seq);
    e.put_u64(ts_ms);
}

impl MasterHandoff {
    /// The canonical bytes the current master signs.
    pub fn signing_payload(
        origin_id: &DeviceId,
        origin_public_key: &[u8; 32],
        new_master_device_id: &DeviceId,
        master_seq: u64,
        ts_ms: u64,
    ) -> Vec<u8> {
        let mut e = Encoder::new();
        write_handoff_unsigned(
            &mut e,
            origin_id,
            origin_public_key,
            new_master_device_id,
            master_seq,
            ts_ms,
        );
        e.finish()
    }

    /// Sign a transfer of the master role to `new_master_device_id` at
    /// `master_seq`.
    ///
    /// Fails on a self-handoff: naming yourself as your own successor transfers
    /// nothing while still consuming a sequence number, so its only effect would
    /// be to move the replay fence. Rejecting it here beats reasoning about it at
    /// every consumer.
    pub fn issue(
        master: &DeviceKeypair,
        new_master_device_id: DeviceId,
        master_seq: u64,
        now_ms: u64,
    ) -> Result<Self, PairingError> {
        let origin_id = master.device_id();
        if new_master_device_id == origin_id {
            return Err(PairingError::SelfHandoff);
        }
        let origin_public_key = *master.verifying_key().as_bytes();
        let payload = Self::signing_payload(
            &origin_id,
            &origin_public_key,
            &new_master_device_id,
            master_seq,
            now_ms,
        );
        Ok(Self {
            origin_id,
            origin_public_key,
            new_master_device_id,
            master_seq,
            ts_ms: now_ms,
            signature: master.sign(&payload),
        })
    }

    /// Whether this handoff is a well-formed statement genuinely signed by
    /// `expected_origin`; the master the verifier currently believes in.
    ///
    /// Same checks as [`DeviceCertificate::verify`]: the embedded `origin_id` is
    /// exactly `expected_origin`, the key content-addresses it, the signature
    /// verifies strictly over every field, and the successor is not the master
    /// itself. **`master_seq` monotonicity is not checked here**; see the type
    /// docs.
    pub fn verify(&self, expected_origin: &DeviceId) -> bool {
        if self.origin_id != *expected_origin {
            return false;
        }
        if DeviceId::from_public_key_bytes(&self.origin_public_key) != self.origin_id {
            return false;
        }
        if self.new_master_device_id == self.origin_id {
            return false;
        }
        let payload = Self::signing_payload(
            &self.origin_id,
            &self.origin_public_key,
            &self.new_master_device_id,
            self.master_seq,
            self.ts_ms,
        );
        verify_with_public_bytes(&self.origin_public_key, &payload, &self.signature)
    }

    /// Serialize the full handoff (including signature).
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        write_handoff_unsigned(
            &mut e,
            &self.origin_id,
            &self.origin_public_key,
            &self.new_master_device_id,
            self.master_seq,
            self.ts_ms,
        );
        e.put_bytes(&self.signature).expect("64 bytes fit");
        e.finish()
    }

    /// Parse a handoff produced by [`MasterHandoff::encode`]. Structural only;
    /// call [`MasterHandoff::verify`] before trusting it.
    pub fn decode(bytes: &[u8]) -> Result<Self, PairingError> {
        let mut d = Decoder::new(bytes);
        let domain = d.get_str().map_err(|_| PairingError::Malformed)?;
        if domain != HANDOFF_DOMAIN {
            return Err(PairingError::Malformed);
        }
        let origin_id = get_32(&mut d)?;
        let origin_public_key = get_32(&mut d)?;
        let new_master_device_id = get_32(&mut d)?;
        let master_seq = d.get_u64().map_err(|_| PairingError::Malformed)?;
        let ts_ms = d.get_u64().map_err(|_| PairingError::Malformed)?;
        let signature = get_64(&mut d)?;
        d.finish().map_err(|_| PairingError::Malformed)?;
        Ok(Self {
            origin_id: DeviceId::from_bytes(origin_id),
            origin_public_key,
            new_master_device_id: DeviceId::from_bytes(new_master_device_id),
            master_seq,
            ts_ms,
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

    /// The group id every certificate test binds; group-scoping is part of the
    /// signed payload (adversarial-review finding on the M2 slice).
    const TG: &[u8] = b"test-group-id";

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
            DeviceCertificate::issue(&origin, new_dev.device_id(), TG, "phone", 1_700_000_000_000)
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
        let cert = DeviceCertificate::issue(&origin, new_dev.device_id(), TG, "phone", 1).unwrap();

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
        let other =
            DeviceCertificate::issue(&impostor, new_dev.device_id(), TG, "phone", 1).unwrap();
        assert!(other.verify(&impostor.device_id()));
        assert!(!other.verify(&origin.device_id()));
    }

    #[test]
    fn certificate_rejects_every_tampered_field() {
        let (origin, new_dev) = origin_and_new(8);
        let cert = DeviceCertificate::issue(&origin, new_dev.device_id(), TG, "phone", 42).unwrap();
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
        let cert = DeviceCertificate::issue(&origin, new_dev.device_id(), TG, "phone", 1).unwrap();
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
            DeviceCertificate::issue(&origin, id, TG, "", 1),
            Err(PairingError::InvalidName)
        );
        // 25 bytes: one over the bound.
        assert_eq!(
            DeviceCertificate::issue(&origin, id, TG, &"x".repeat(MAX_DEVICE_NAME_BYTES + 1), 1),
            Err(PairingError::InvalidName)
        );
        // Exactly at the bound is fine, and bytes (not chars) are what count:
        // six 4-byte emoji are 24 bytes and allowed, seven are not.
        assert!(
            DeviceCertificate::issue(&origin, id, TG, &"x".repeat(MAX_DEVICE_NAME_BYTES), 1)
                .is_ok()
        );
        assert!(DeviceCertificate::issue(&origin, id, TG, &"🐱".repeat(6), 1).is_ok());
        assert_eq!(
            DeviceCertificate::issue(&origin, id, TG, &"🐱".repeat(7), 1),
            Err(PairingError::InvalidName)
        );
        // Control characters would let a name inject lines into the grant popup.
        assert_eq!(
            DeviceCertificate::issue(&origin, id, TG, "phone\nADMIN", 1),
            Err(PairingError::InvalidName)
        );

        // A signed-but-oversize name (forged by hand) fails verification too, so
        // the bound is not merely an issue-time courtesy.
        let long = "y".repeat(MAX_DEVICE_NAME_BYTES + 1);
        let origin_id = origin.device_id();
        let pk = *origin.verifying_key().as_bytes();
        let payload = DeviceCertificate::signing_payload(&origin_id, &pk, &id, TG, &long, 1);
        let forged = DeviceCertificate {
            origin_id,
            origin_public_key: pk,
            new_device_id: id,
            group_id: TG.to_vec(),
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
        e.put_bytes(TG).unwrap();
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
        let cert = DeviceCertificate::issue(&origin, new_dev.device_id(), TG, "phone", 1).unwrap();

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
            DeviceCertificate::issue(&origin, origin.device_id(), TG, "me", 1),
            Err(PairingError::SelfCertification)
        );
        // Even hand-signed, a self-certificate does not verify (depth stays 1).
        let origin_id = origin.device_id();
        let pk = *origin.verifying_key().as_bytes();
        let payload = DeviceCertificate::signing_payload(&origin_id, &pk, &origin_id, TG, "me", 1);
        let forged = DeviceCertificate {
            origin_id,
            origin_public_key: pk,
            new_device_id: origin_id,
            group_id: TG.to_vec(),
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

    // ---------- master handoff ----------

    #[test]
    fn master_handoff_round_trips_and_verifies() {
        let (master, successor) = origin_and_new(20);
        let h = MasterHandoff::issue(&master, successor.device_id(), 1, 1_700_000_000_002).unwrap();
        assert!(h.verify(&master.device_id()));

        let decoded = MasterHandoff::decode(&h.encode()).unwrap();
        assert_eq!(decoded, h);
        assert!(decoded.verify(&master.device_id()));
        assert_eq!(decoded.new_master_device_id, successor.device_id());
        assert_eq!(decoded.master_seq, 1);
        assert_eq!(decoded.ts_ms, 1_700_000_000_002);
    }

    #[test]
    fn master_handoff_rejects_every_tampered_field_and_wrong_master() {
        let (master, successor) = origin_and_new(21);
        let impostor = DeviceKeypair::generate(&mut rng(2100));
        let h = MasterHandoff::issue(&master, successor.device_id(), 4, 99).unwrap();
        let expected = master.device_id();
        assert!(h.verify(&expected));

        // Re-point the transfer at another device.
        let mut t = h.clone();
        t.new_master_device_id = impostor.device_id();
        assert!(!t.verify(&expected));

        // Move the replay fence.
        let mut t = h.clone();
        t.master_seq = 5;
        assert!(!t.verify(&expected));

        let mut t = h.clone();
        t.ts_ms = 100;
        assert!(!t.verify(&expected));

        let mut t = h.clone();
        t.signature[0] ^= 0xFF;
        assert!(!t.verify(&expected));

        // Swap in another key while keeping the claimed id: the id no longer
        // content-addresses the key.
        let mut t = h.clone();
        t.origin_public_key = *impostor.verifying_key().as_bytes();
        assert!(!t.verify(&expected));

        // Swap both so they agree: the signature no longer verifies.
        let mut t = h.clone();
        t.origin_id = impostor.device_id();
        t.origin_public_key = *impostor.verifying_key().as_bytes();
        assert!(!t.verify(&impostor.device_id()));

        // A handoff genuinely signed by someone else is not accepted for `master`
        //; this is what blocks a companion from crowning itself.
        let other = MasterHandoff::issue(&impostor, successor.device_id(), 4, 99).unwrap();
        assert!(other.verify(&impostor.device_id()));
        assert!(!other.verify(&expected));
    }

    #[test]
    fn master_handoff_rejects_self_handoff() {
        let (master, _) = origin_and_new(22);
        assert_eq!(
            MasterHandoff::issue(&master, master.device_id(), 1, 1),
            Err(PairingError::SelfHandoff)
        );
        // Even hand-signed, a self-handoff does not verify.
        let origin_id = master.device_id();
        let pk = *master.verifying_key().as_bytes();
        let payload = MasterHandoff::signing_payload(&origin_id, &pk, &origin_id, 1, 1);
        let forged = MasterHandoff {
            origin_id,
            origin_public_key: pk,
            new_master_device_id: origin_id,
            master_seq: 1,
            ts_ms: 1,
            signature: master.sign(&payload),
        };
        assert!(!forged.verify(&origin_id));
    }

    #[test]
    fn master_handoff_does_not_enforce_monotonic_seq() {
        // Documented contract: this module verifies authenticity only. A *stale*
        // handoff (a lower `master_seq` presented after a higher one) is perfectly
        // well-signed and verifies here; rejecting it is the admitting layer's
        // job (M3/M5), which must track the highest seq it has accepted.
        let (master, successor) = origin_and_new(23);
        let retired = DeviceKeypair::generate(&mut rng(2300));
        let newer = MasterHandoff::issue(&master, successor.device_id(), 9, 2).unwrap();
        let stale = MasterHandoff::issue(&master, retired.device_id(), 0, 1).unwrap();
        assert!(newer.verify(&master.device_id()));
        assert!(stale.verify(&master.device_id()));
        assert!(stale.master_seq < newer.master_seq);
    }

    #[test]
    fn master_handoff_decode_rejects_wrong_domain_and_trailing_bytes() {
        let (master, successor) = origin_and_new(24);
        let h = MasterHandoff::issue(&master, successor.device_id(), 1, 1).unwrap();

        let mut bytes = h.encode();
        bytes.push(0);
        assert_eq!(MasterHandoff::decode(&bytes), Err(PairingError::Malformed));

        let bytes = h.encode();
        assert_eq!(
            MasterHandoff::decode(&bytes[..bytes.len() - 4]),
            Err(PairingError::Malformed)
        );

        // No cross-decoding between the three signed statements: the domain label
        // is the first field of each.
        let cert =
            DeviceCertificate::issue(&master, successor.device_id(), TG, "phone", 1).unwrap();
        let rev = DeviceRevocation::issue(&master, successor.device_id(), 1);
        assert_eq!(
            MasterHandoff::decode(&cert.encode()),
            Err(PairingError::Malformed)
        );
        assert_eq!(
            MasterHandoff::decode(&rev.encode()),
            Err(PairingError::Malformed)
        );
        assert_eq!(
            DeviceCertificate::decode(&h.encode()),
            Err(PairingError::Malformed)
        );
        assert_eq!(
            DeviceRevocation::decode(&h.encode()),
            Err(PairingError::Malformed)
        );
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

        // Certificate: 26-byte domain field + 3×36 id/key fields + 17-byte group-id
        // field ("test-group-id") + 9-byte name field + 8-byte timestamp + 68-byte
        // signature field = 236 bytes.
        let cert =
            DeviceCertificate::issue(&origin, new_dev.device_id(), TG, "phone", 1_700_000_000_000)
                .unwrap();
        assert_eq!(cert.encode().len(), 236);
        assert_eq!(
            hex(&cert.signature),
            "edbd50ac16b22ac4669a55d76d14101fe9312243034002086b88eff6de8679b4\
             464a3accbfee2565127977308f3aa6a557d46222b3c70ff23981dcf9d795490d"
        );
        assert_eq!(
            hex(blake3::hash(&cert.encode()).as_bytes()),
            "9582fd066e5b59bc0f3cda540db16e8f9008fc1d038c49e3bad838a748992837"
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
        // Handoff: 29-byte domain field + 3×36 id/key fields + 2×8 for
        // `master_seq`/`ts_ms` + 68-byte signature field = 221 bytes.
        let handoff =
            MasterHandoff::issue(&origin, new_dev.device_id(), 1, 1_700_000_000_000).unwrap();
        assert_eq!(handoff.encode().len(), 221);
        assert_eq!(
            hex(&handoff.signature),
            "777d1b23858aaa1f1a476678bcbd17558f0c57769b7ae7dbcbb76bdcc71b210f\
             d77efdae045c2764344b7edd80a96fc91409d48eb6a00d3ccd51ea32ea37c10c"
        );
        assert_eq!(
            hex(blake3::hash(&handoff.encode()).as_bytes()),
            "d1faabb1d2fabd9e967a799c987490fd5f08993aa7f9ed12c7f77af240281f54"
        );

        // The three statements share ids and timestamp yet cannot collide: the
        // domain label is the first signed field.
        assert_ne!(cert.signature, rev.signature);
        assert_ne!(cert.signature, handoff.signature);
        assert_ne!(rev.signature, handoff.signature);

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
