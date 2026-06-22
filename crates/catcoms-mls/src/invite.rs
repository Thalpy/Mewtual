//! Single-use, device-bound invites.
//!
//! An [`InviteToken`] is a pasteable capability signed by an inviting device. It
//! carries the target `group_id`, a one-time `invite_nonce`, an expiry, and
//! bootstrap hints — but **no group secrets**. Its security rests on three
//! independent checks performed by [`crate::ServerGroup::add_member_via_invite`]
//! (and re-checkable by every member):
//!
//! 1. **Inviter authenticity** — the token signature verifies under a *current
//!    group member's* key.
//! 2. **Credential binding** — the joiner's KeyPackage carries a
//!    [`MembershipCredential`] bound to exactly `(group_id, invite_nonce)`. Since
//!    the KeyPackage is self-signed by the joining device, a KeyPackage minted
//!    for group X cannot be replayed into group Y — the binding travels *inside
//!    MLS* in the leaf credential.
//! 3. **Single use** — the [`InviteLedger`] records consumed/revoked nonces, so a
//!    leaked or already-used invite is inert, and the token cannot be redeemed
//!    twice.

use std::collections::HashSet;

use catcoms_crypto::{verify_with_public_bytes, DeviceId};
use catcoms_wire::{Decoder, Encoder};
use openmls::prelude::*;
use thiserror::Error;

// Bumped v1 -> v2 in 6e-3d-9 (hard cutover, pre-release) when the token gained the
// `rendezvous` vector: the signed payload shape changed, so a v1 token never verifies
// against v2 and vice versa.
const INVITE_DOMAIN: &str = "catcoms/invite/v2";
const MEMBERSHIP_DOMAIN: &str = "catcoms/membership/v1";
/// Defensive cap on each of the two invite address vectors (bootstrap, rendezvous).
const MAX_INVITE_ADDRS: u32 = 64;

/// Why an invite admission was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InviteError {
    /// The token is for a different group than the one being joined.
    #[error("invite is for a different group")]
    WrongGroup,
    /// The token signature did not verify under the inviter's key.
    #[error("invite signature is invalid")]
    BadSignature,
    /// The named inviter is not a current member of the group.
    #[error("inviter is not a current group member")]
    InviterNotMember,
    /// The joiner's KeyPackage credential does not match the invite binding.
    #[error("key package credential does not match the invite binding")]
    CredentialMismatch,
    /// The invite nonce has already been consumed.
    #[error("invite has already been used")]
    AlreadyUsed,
    /// The invite nonce was revoked.
    #[error("invite has been revoked")]
    Revoked,
    /// The invite has expired.
    #[error("invite has expired")]
    Expired,
    /// The token or credential bytes were malformed.
    #[error("malformed invite or credential")]
    Malformed,
}

/// The binding carried in a joining device's MLS leaf credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipCredential {
    /// The joining device's content-addressed id.
    pub device_id: DeviceId,
    /// The group this credential is valid for.
    pub group_id: Vec<u8>,
    /// The one-time invite nonce this admission consumes.
    pub invite_nonce: [u8; 16],
}

impl MembershipCredential {
    /// Canonical encoding used as the MLS BasicCredential identity bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_str(MEMBERSHIP_DOMAIN).expect("label fits");
        e.put_bytes(self.device_id.as_bytes()).expect("32 fits");
        e.put_bytes(&self.group_id).expect("group id fits");
        e.put_bytes(&self.invite_nonce).expect("16 fits");
        e.finish()
    }

    /// Decode from credential identity bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, InviteError> {
        let mut d = Decoder::new(bytes);
        let domain = d.get_str().map_err(|_| InviteError::Malformed)?;
        if domain != MEMBERSHIP_DOMAIN {
            return Err(InviteError::Malformed);
        }
        let device_id: [u8; 32] = d
            .get_bytes()
            .map_err(|_| InviteError::Malformed)?
            .try_into()
            .map_err(|_| InviteError::Malformed)?;
        let group_id = d.get_bytes().map_err(|_| InviteError::Malformed)?.to_vec();
        let invite_nonce: [u8; 16] = d
            .get_bytes()
            .map_err(|_| InviteError::Malformed)?
            .try_into()
            .map_err(|_| InviteError::Malformed)?;
        d.finish().map_err(|_| InviteError::Malformed)?;
        Ok(Self {
            device_id: DeviceId::from_bytes(device_id),
            group_id,
            invite_nonce,
        })
    }
}

/// Read the [`MembershipCredential`] out of a KeyPackage's leaf credential.
pub(crate) fn membership_from_key_package(
    kp: &KeyPackage,
) -> Result<MembershipCredential, InviteError> {
    let credential = kp.leaf_node().credential().clone();
    let basic = BasicCredential::try_from(credential).map_err(|_| InviteError::Malformed)?;
    MembershipCredential::decode(basic.identity())
}

/// A pasteable, single-use, device-bound invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteToken {
    /// The target group's id.
    pub group_id: Vec<u8>,
    /// The inviting device's content-addressed id.
    pub inviter_device_id: DeviceId,
    /// The inviter's Ed25519 public key (raw bytes). Lets a *joiner* — who is not
    /// yet a group member and so has no roster — authenticate the invite and the
    /// admitter's signed Welcome. Must content-address `inviter_device_id`.
    pub inviter_public_key: Vec<u8>,
    /// The one-time nonce identifying this invite.
    pub invite_nonce: [u8; 16],
    /// Expiry (ms since epoch); compared against an injected clock.
    pub expires_at_ms: u64,
    /// Optional single inviter-seed bootstrap hint (an opaque multiaddr string). Since
    /// 6e-3d-9 this shrinks to a seed; discovery rides `rendezvous` + the pre-join
    /// `join_ns` instead, so a joiner needs no hard-coded server address.
    pub bootstrap: Vec<String>,
    /// Zero-knowledge **rendezvous** infra multiaddrs (≥2 recommended, distinct PeerIds,
    /// direct — never `/p2p-circuit`). The joiner registers/discovers under the pre-join
    /// `join_ns` at these to find the inviter; each is credited as ≤1 eclipse trust root
    /// (the distinct-PeerId check is misconfig defence, not anti-collusion). Validation
    /// (reject circuit, distinct PeerIds) lives in `catcoms-net` where multiaddrs parse.
    pub rendezvous: Vec<String>,
    /// Inviter's Ed25519 signature over the canonical unsigned encoding.
    pub signature: [u8; 64],
}

#[allow(clippy::too_many_arguments)]
fn write_unsigned(
    e: &mut Encoder,
    group_id: &[u8],
    inviter_device_id: &DeviceId,
    inviter_public_key: &[u8],
    invite_nonce: &[u8; 16],
    expires_at_ms: u64,
    bootstrap: &[String],
    rendezvous: &[String],
) {
    e.put_str(INVITE_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(inviter_device_id.as_bytes()).expect("32 fits");
    e.put_bytes(inviter_public_key).expect("pubkey fits");
    e.put_bytes(invite_nonce).expect("16 fits");
    e.put_u64(expires_at_ms);
    e.put_u32(bootstrap.len() as u32);
    for addr in bootstrap {
        e.put_str(addr).expect("addr fits");
    }
    // Length-prefixed second vector (round-trip + tamper tested). Bound into the
    // signature so a relay cannot strip or substitute the rendezvous set.
    e.put_u32(rendezvous.len() as u32);
    for addr in rendezvous {
        e.put_str(addr).expect("addr fits");
    }
}

impl InviteToken {
    /// The canonical bytes that the inviter signs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn signing_payload(
        group_id: &[u8],
        inviter_device_id: &DeviceId,
        inviter_public_key: &[u8],
        invite_nonce: &[u8; 16],
        expires_at_ms: u64,
        bootstrap: &[String],
        rendezvous: &[String],
    ) -> Vec<u8> {
        let mut e = Encoder::new();
        write_unsigned(
            &mut e,
            group_id,
            inviter_device_id,
            inviter_public_key,
            invite_nonce,
            expires_at_ms,
            bootstrap,
            rendezvous,
        );
        e.finish()
    }

    /// Verify the token signature under an externally-supplied inviter public key
    /// (e.g. one looked up from the group roster on the admitter side).
    pub fn verify(&self, inviter_public_key: &[u8]) -> bool {
        let payload = Self::signing_payload(
            &self.group_id,
            &self.inviter_device_id,
            &self.inviter_public_key,
            &self.invite_nonce,
            self.expires_at_ms,
            &self.bootstrap,
            &self.rendezvous,
        );
        verify_with_public_bytes(inviter_public_key, &payload, &self.signature)
    }

    /// Self-authenticate the token: the embedded public key must content-address
    /// the named inviter device, and must have signed the token. This lets a
    /// joiner (with no roster) check the invite is internally consistent.
    pub fn verify_self(&self) -> bool {
        if DeviceId::from_public_key_bytes(&self.inviter_public_key) != self.inviter_device_id {
            return false;
        }
        self.verify(&self.inviter_public_key)
    }

    /// Verify a signature made by the inviter (the embedded public key) over
    /// `message` — e.g. the admitter's signature over a join-response transcript,
    /// which authenticates that the Welcome really came from the inviter.
    pub fn verify_inviter_signature(&self, message: &[u8], signature: &[u8; 64]) -> bool {
        verify_with_public_bytes(&self.inviter_public_key, message, signature)
    }

    /// Serialize the full token (including signature) for pasting/transport.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        write_unsigned(
            &mut e,
            &self.group_id,
            &self.inviter_device_id,
            &self.inviter_public_key,
            &self.invite_nonce,
            self.expires_at_ms,
            &self.bootstrap,
            &self.rendezvous,
        );
        e.put_bytes(&self.signature).expect("64 fits");
        e.finish()
    }

    /// Parse a token produced by [`InviteToken::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, InviteError> {
        let mut d = Decoder::new(bytes);
        let domain = d.get_str().map_err(|_| InviteError::Malformed)?;
        if domain != INVITE_DOMAIN {
            return Err(InviteError::Malformed);
        }
        let group_id = d.get_bytes().map_err(|_| InviteError::Malformed)?.to_vec();
        let inviter_device_id: [u8; 32] = d
            .get_bytes()
            .map_err(|_| InviteError::Malformed)?
            .try_into()
            .map_err(|_| InviteError::Malformed)?;
        let inviter_public_key = d.get_bytes().map_err(|_| InviteError::Malformed)?.to_vec();
        let invite_nonce: [u8; 16] = d
            .get_bytes()
            .map_err(|_| InviteError::Malformed)?
            .try_into()
            .map_err(|_| InviteError::Malformed)?;
        let expires_at_ms = d.get_u64().map_err(|_| InviteError::Malformed)?;
        let addr_count = d.get_u32().map_err(|_| InviteError::Malformed)?;
        // Cap both address-vector counts up front (an invite carries at most a seed +
        // a handful of rendezvous). The Decoder already bounds each read against the
        // remaining bytes, so this is consistency/hardening, mirroring the bundle codecs.
        if addr_count > MAX_INVITE_ADDRS {
            return Err(InviteError::Malformed);
        }
        let mut bootstrap = Vec::new();
        for _ in 0..addr_count {
            bootstrap.push(d.get_str().map_err(|_| InviteError::Malformed)?.to_string());
        }
        let rz_count = d.get_u32().map_err(|_| InviteError::Malformed)?;
        if rz_count > MAX_INVITE_ADDRS {
            return Err(InviteError::Malformed);
        }
        let mut rendezvous = Vec::new();
        for _ in 0..rz_count {
            rendezvous.push(d.get_str().map_err(|_| InviteError::Malformed)?.to_string());
        }
        let signature: [u8; 64] = d
            .get_bytes()
            .map_err(|_| InviteError::Malformed)?
            .try_into()
            .map_err(|_| InviteError::Malformed)?;
        d.finish().map_err(|_| InviteError::Malformed)?;
        Ok(Self {
            group_id,
            inviter_device_id: DeviceId::from_bytes(inviter_device_id),
            inviter_public_key,
            invite_nonce,
            expires_at_ms,
            bootstrap,
            rendezvous,
            signature,
        })
    }
}

/// Records which one-time invite nonces have been consumed or revoked, enforcing
/// single use. (In a later phase this becomes a replicated CRDT inside the group;
/// here it is a local set with the same admission rules.)
#[derive(Debug, Default)]
pub struct InviteLedger {
    consumed: HashSet<[u8; 16]>,
    revoked: HashSet<[u8; 16]>,
}

impl InviteLedger {
    /// A fresh, empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Revoke an invite nonce so it can never be admitted.
    pub fn revoke(&mut self, invite_nonce: [u8; 16]) {
        self.revoked.insert(invite_nonce);
    }

    /// Whether a nonce has been consumed.
    pub fn is_consumed(&self, invite_nonce: &[u8; 16]) -> bool {
        self.consumed.contains(invite_nonce)
    }

    /// Whether a nonce has been revoked.
    pub fn is_revoked(&self, invite_nonce: &[u8; 16]) -> bool {
        self.revoked.contains(invite_nonce)
    }

    /// Check that `token` may currently be admitted (not expired, revoked, or
    /// already used). Does not consume it.
    pub fn check(&self, token: &InviteToken, now_ms: u64) -> Result<(), InviteError> {
        if now_ms > token.expires_at_ms {
            return Err(InviteError::Expired);
        }
        if self.revoked.contains(&token.invite_nonce) {
            return Err(InviteError::Revoked);
        }
        if self.consumed.contains(&token.invite_nonce) {
            return Err(InviteError::AlreadyUsed);
        }
        Ok(())
    }

    /// Mark a nonce consumed (single use). Errors if already consumed.
    pub fn consume(&mut self, invite_nonce: [u8; 16]) -> Result<(), InviteError> {
        if !self.consumed.insert(invite_nonce) {
            return Err(InviteError::AlreadyUsed);
        }
        Ok(())
    }

    /// Serialize the ledger (consumed + revoked nonces) for persistence (Phase 9e). The
    /// single-use guarantee must survive restart, so the inviter persists this — otherwise
    /// a restart would forget which invites were spent and a single-use invite could be
    /// redeemed again.
    pub fn snapshot(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u32(self.consumed.len() as u32);
        for n in &self.consumed {
            e.put_bytes(n).expect("16 fits");
        }
        e.put_u32(self.revoked.len() as u32);
        for n in &self.revoked {
            e.put_bytes(n).expect("16 fits");
        }
        e.finish()
    }

    /// Reconstruct a ledger from a [`InviteLedger::snapshot`] blob.
    pub fn restore(bytes: &[u8]) -> Result<Self, InviteError> {
        let mut d = Decoder::new(bytes);
        let mut consumed = HashSet::new();
        let c = d.get_u32().map_err(|_| InviteError::Malformed)?;
        for _ in 0..c {
            let n: [u8; 16] = d
                .get_bytes()
                .map_err(|_| InviteError::Malformed)?
                .try_into()
                .map_err(|_| InviteError::Malformed)?;
            consumed.insert(n);
        }
        let mut revoked = HashSet::new();
        let r = d.get_u32().map_err(|_| InviteError::Malformed)?;
        for _ in 0..r {
            let n: [u8; 16] = d
                .get_bytes()
                .map_err(|_| InviteError::Malformed)?
                .try_into()
                .map_err(|_| InviteError::Malformed)?;
            revoked.insert(n);
        }
        d.finish().map_err(|_| InviteError::Malformed)?;
        Ok(Self { consumed, revoked })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_snapshot_round_trips_consumed_and_revoked() {
        // The single-use guarantee must survive restart: both the consumed and revoked
        // nonce sets round-trip (9e).
        let mut led = InviteLedger::new();
        led.consume([1u8; 16]).unwrap();
        led.consume([2u8; 16]).unwrap();
        led.revoke([3u8; 16]);

        let restored = InviteLedger::restore(&led.snapshot()).unwrap();
        assert!(restored.is_consumed(&[1u8; 16]));
        assert!(restored.is_consumed(&[2u8; 16]));
        assert!(restored.is_revoked(&[3u8; 16]));
        assert!(!restored.is_consumed(&[9u8; 16]));
        assert!(InviteLedger::restore(b"x").is_err());
    }

    #[test]
    fn membership_credential_roundtrips() {
        let mc = MembershipCredential {
            device_id: DeviceId::from_bytes([9u8; 32]),
            group_id: vec![1, 2, 3, 4],
            invite_nonce: [7u8; 16],
        };
        assert_eq!(MembershipCredential::decode(&mc.encode()).unwrap(), mc);
    }

    #[test]
    fn membership_credential_rejects_wrong_domain() {
        let mut e = Encoder::new();
        e.put_str("wrong/domain").unwrap();
        e.put_bytes(&[0u8; 32]).unwrap();
        e.put_bytes(&[]).unwrap();
        e.put_bytes(&[0u8; 16]).unwrap();
        assert_eq!(
            MembershipCredential::decode(&e.finish()),
            Err(InviteError::Malformed)
        );
    }

    #[test]
    fn ledger_enforces_single_use_and_revocation() {
        let mut ledger = InviteLedger::new();
        let token = InviteToken {
            group_id: vec![1],
            inviter_device_id: DeviceId::from_bytes([0u8; 32]),
            inviter_public_key: vec![],
            invite_nonce: [1u8; 16],
            expires_at_ms: 1000,
            bootstrap: vec![],
            rendezvous: vec![],
            signature: [0u8; 64],
        };
        assert_eq!(ledger.check(&token, 500), Ok(()));
        assert_eq!(ledger.check(&token, 2000), Err(InviteError::Expired));
        ledger.consume(token.invite_nonce).unwrap();
        assert_eq!(ledger.check(&token, 500), Err(InviteError::AlreadyUsed));
        assert_eq!(
            ledger.consume(token.invite_nonce),
            Err(InviteError::AlreadyUsed)
        );

        let mut ledger2 = InviteLedger::new();
        ledger2.revoke(token.invite_nonce);
        assert_eq!(ledger2.check(&token, 500), Err(InviteError::Revoked));
    }
}
