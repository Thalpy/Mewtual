//! Authenticated two-way invite reply codes.
//!
//! A normal invite only tells the joiner how to dial the inviter.  When that route is stale or
//! one-way, the joiner can send a short-lived reply through the same human channel.  It contains
//! the *full signed invite* as an issuance permit (important: the inviter may have auto-reminted a
//! newer invite since the pasted one was copied), the joiner's Noise `PeerId`, and up to four
//! direct public listener candidates.  The inviter then dials those candidates while the joiner
//! keeps its listener and original join request alive.
//!
//! The MAC key is derived from the invite's random nonce, so this is an authenticated extension of
//! the bearer invite, not a new identity proof.  Anyone who saw the invite can produce a reply and
//! could already redeem that invite; callers must therefore require confirmation before replacing
//! a different active joiner key.  The signed permit is verified by the application/MLS layer.

use std::fmt;

use catcoms_rt::{Clock, RngCore};
use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};
use thiserror::Error;

const DOMAIN: &str = "catcoms/join-reply/v1";
const CODE_PREFIX: &str = "mewtual-reply-v1:";
const MAX_CODE_BYTES: usize = 32 * 1024;
const MAX_PERMIT_BYTES: usize = 24 * 1024;
const MAX_CANDIDATES: usize = 4;
const MAX_ADDR_BYTES: usize = 512;
/// A NAT mapping is commonly recycled within minutes.  The short window is functional as well as
/// security-sensitive: both applications need overlapping sessions for the punch to work.
pub const JOIN_REPLY_LIFETIME_MS: u64 = 60_000;
const MAX_FUTURE_SKEW_MS: u64 = 30_000;
const DIALBACK_PROOF_DOMAIN: &str = "catcoms/join-reply/dialback-proof/v1";

/// A verified, bounded two-way invite reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinReply {
    /// The exact signed invite bytes the joiner received.  This proves the nonce was actually
    /// issued even if the inviter's cached “latest invite” has since changed.
    pub invite_permit: Vec<u8>,
    /// The joiner's libp2p Noise identity. Candidate addresses deliberately omit `/p2p`; callers
    /// reconstruct that suffix from this one authenticated field.
    pub joiner: PeerId,
    /// Direct, globally-routable TCP/QUIC endpoints with no DNS, circuit or trailing peer id.
    pub candidates: Vec<Multiaddr>,
    /// Makes refresh/replay policy idempotent without consuming the MLS invite.
    pub joiner_nonce: [u8; 16],
    /// Issuer clock, milliseconds since epoch.
    pub issued_at_ms: u64,
    /// Exactly [`JOIN_REPLY_LIFETIME_MS`] after `issued_at_ms`.
    pub expires_at_ms: u64,
    mac: [u8; 32],
}

/// Why a reply code was rejected before any network dial.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JoinReplyError {
    /// Prefix, hex, canonical frame or field shape was invalid.
    #[error("malformed connection reply code")]
    Malformed,
    /// More than four candidates, duplicate candidates, or a non-public/non-direct route.
    #[error("connection reply contains an unsafe or unsupported address")]
    UnsafeAddress,
    /// The code was not MACed by a holder of this exact invite nonce.
    #[error("connection reply does not match the invite")]
    BadMac,
    /// The 60-second route window elapsed.
    #[error("connection reply expired; ask the joiner to generate a fresh one")]
    Expired,
    /// The issuer clock is implausibly far ahead of the verifier.
    #[error("connection reply clock is too far ahead")]
    Future,
}

impl JoinReply {
    /// Mint a reply using the original signed invite as an issuance permit.
    pub fn mint(
        invite_permit: Vec<u8>,
        invite_nonce: &[u8; 16],
        joiner: PeerId,
        candidates: Vec<Multiaddr>,
        clock: &dyn Clock,
        rng: &mut dyn RngCore,
    ) -> Result<Self, JoinReplyError> {
        validate_fields(&invite_permit, &candidates)?;
        let mut joiner_nonce = [0u8; 16];
        rng.fill_bytes(&mut joiner_nonce);
        let issued_at_ms = clock.now_ms();
        let expires_at_ms = issued_at_ms.saturating_add(JOIN_REPLY_LIFETIME_MS);
        let mut reply = Self {
            invite_permit,
            joiner,
            candidates,
            joiner_nonce,
            issued_at_ms,
            expires_at_ms,
            mac: [0; 32],
        };
        reply.mac = reply.expected_mac(invite_nonce);
        Ok(reply)
    }

    /// Encode as a pasteable versioned hex string.
    pub fn encode(&self) -> String {
        let bytes = self.canonical_bytes(true);
        format!("{CODE_PREFIX}{}", hex::encode(bytes))
    }

    /// Decode and enforce structural/address bounds.  Call [`JoinReply::verify`] after decoding
    /// the embedded signed invite and obtaining its nonce.
    pub fn decode(code: &str) -> Result<Self, JoinReplyError> {
        let body = code
            .trim()
            .strip_prefix(CODE_PREFIX)
            .ok_or(JoinReplyError::Malformed)?;
        if body.len() > MAX_CODE_BYTES.saturating_mul(2) {
            return Err(JoinReplyError::Malformed);
        }
        let raw = hex::decode(body).map_err(|_| JoinReplyError::Malformed)?;
        if raw.len() > MAX_CODE_BYTES {
            return Err(JoinReplyError::Malformed);
        }
        let mut cursor = Cursor::new(&raw);
        if cursor.string()? != DOMAIN {
            return Err(JoinReplyError::Malformed);
        }
        let invite_permit = cursor.bytes(MAX_PERMIT_BYTES)?.to_vec();
        let joiner =
            PeerId::from_bytes(cursor.bytes(128)?).map_err(|_| JoinReplyError::Malformed)?;
        let count = usize::try_from(cursor.u32()?).map_err(|_| JoinReplyError::Malformed)?;
        if count == 0 || count > MAX_CANDIDATES {
            return Err(JoinReplyError::UnsafeAddress);
        }
        let mut candidates = Vec::with_capacity(count);
        for _ in 0..count {
            let text = cursor.string_limited(MAX_ADDR_BYTES)?;
            candidates.push(text.parse().map_err(|_| JoinReplyError::Malformed)?);
        }
        let joiner_nonce = cursor
            .bytes(16)?
            .try_into()
            .map_err(|_| JoinReplyError::Malformed)?;
        let issued_at_ms = cursor.u64()?;
        let expires_at_ms = cursor.u64()?;
        let mac = cursor
            .bytes(32)?
            .try_into()
            .map_err(|_| JoinReplyError::Malformed)?;
        cursor.finish()?;
        validate_fields(&invite_permit, &candidates)?;
        Ok(Self {
            invite_permit,
            joiner,
            candidates,
            joiner_nonce,
            issued_at_ms,
            expires_at_ms,
            mac,
        })
    }

    /// Verify invite binding, fixed lifetime and wall-clock validity.
    pub fn verify(&self, invite_nonce: &[u8; 16], clock: &dyn Clock) -> Result<(), JoinReplyError> {
        if self.expires_at_ms.saturating_sub(self.issued_at_ms) != JOIN_REPLY_LIFETIME_MS {
            return Err(JoinReplyError::Malformed);
        }
        let now = clock.now_ms();
        if self.issued_at_ms > now.saturating_add(MAX_FUTURE_SKEW_MS) {
            return Err(JoinReplyError::Future);
        }
        if now > self.expires_at_ms {
            return Err(JoinReplyError::Expired);
        }
        if !constant_time_eq(&self.mac, &self.expected_mac(invite_nonce)) {
            return Err(JoinReplyError::BadMac);
        }
        Ok(())
    }

    /// Rebuild dial targets with the authenticated joiner identity as the only `/p2p` suffix.
    pub fn dial_targets(&self) -> Vec<Multiaddr> {
        self.candidates
            .iter()
            .cloned()
            .map(|mut address| {
                address.push(Protocol::P2p(self.joiner));
                address
            })
            .collect()
    }

    /// Proof sent by the code holder after connecting back. It keeps an unrelated Internet
    /// scanner that merely found the joiner's public listener from receiving the bearer invite
    /// and KeyPackage. This authenticates possession of the reply channel, not group membership;
    /// the inviter-signed Welcome remains the admission proof.
    pub fn dialback_proof(&self, invite_nonce: &[u8; 16], dialer: &[u8; 32]) -> [u8; 32] {
        let key = blake3::derive_key("catcoms/join-reply/proof-key/v1", invite_nonce);
        let mut transcript = Vec::new();
        put_bytes(&mut transcript, DIALBACK_PROOF_DOMAIN.as_bytes());
        put_bytes(&mut transcript, &self.joiner_nonce);
        put_bytes(&mut transcript, &self.joiner.to_bytes());
        put_bytes(&mut transcript, dialer);
        transcript.extend_from_slice(&self.expires_at_ms.to_be_bytes());
        *blake3::keyed_hash(&key, &transcript).as_bytes()
    }

    pub fn verify_dialback_proof(
        &self,
        invite_nonce: &[u8; 16],
        dialer: &[u8; 32],
        proof: &[u8],
    ) -> bool {
        let Ok(proof): Result<[u8; 32], _> = proof.try_into() else {
            return false;
        };
        constant_time_eq(&proof, &self.dialback_proof(invite_nonce, dialer))
    }

    fn expected_mac(&self, invite_nonce: &[u8; 16]) -> [u8; 32] {
        let key = blake3::derive_key("catcoms/join-reply/key/v1", invite_nonce);
        *blake3::keyed_hash(&key, &self.canonical_bytes(false)).as_bytes()
    }

    fn canonical_bytes(&self, include_mac: bool) -> Vec<u8> {
        let mut out = Vec::new();
        put_bytes(&mut out, DOMAIN.as_bytes());
        put_bytes(&mut out, &self.invite_permit);
        put_bytes(&mut out, &self.joiner.to_bytes());
        out.extend_from_slice(
            &u32::try_from(self.candidates.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        for address in &self.candidates {
            put_bytes(&mut out, address.to_string().as_bytes());
        }
        put_bytes(&mut out, &self.joiner_nonce);
        out.extend_from_slice(&self.issued_at_ms.to_be_bytes());
        out.extend_from_slice(&self.expires_at_ms.to_be_bytes());
        if include_mac {
            put_bytes(&mut out, &self.mac);
        }
        out
    }
}

fn validate_fields(permit: &[u8], candidates: &[Multiaddr]) -> Result<(), JoinReplyError> {
    if permit.is_empty() || permit.len() > MAX_PERMIT_BYTES {
        return Err(JoinReplyError::Malformed);
    }
    if candidates.is_empty() || candidates.len() > MAX_CANDIDATES {
        return Err(JoinReplyError::UnsafeAddress);
    }
    for (index, candidate) in candidates.iter().enumerate() {
        if candidate.to_string().len() > MAX_ADDR_BYTES
            || !direct_candidate(candidate)
            || candidates[..index].contains(candidate)
        {
            return Err(JoinReplyError::UnsafeAddress);
        }
    }
    Ok(())
}

fn direct_candidate(address: &Multiaddr) -> bool {
    if !crate::addr_is_globally_routable(address) {
        return false;
    }
    let mut host = false;
    let mut tcp = false;
    let mut udp = false;
    let mut quic = false;
    for part in address.iter() {
        match part {
            Protocol::Ip4(_) | Protocol::Ip6(_) if !host => host = true,
            Protocol::Tcp(port) if host && port != 0 && !tcp && !udp => tcp = true,
            Protocol::Udp(port) if host && port != 0 && !udp && !tcp => udp = true,
            Protocol::QuicV1 if udp && !quic => quic = true,
            // The one authenticated PeerId lives outside each route and is appended by us.
            Protocol::P2p(_) | Protocol::P2pCircuit => return false,
            _ => return false,
        }
    }
    host && (tcp || (udp && quic))
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&u32::try_from(bytes.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

struct Cursor<'a> {
    input: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, at: 0 }
    }

    fn u32(&mut self) -> Result<u32, JoinReplyError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| JoinReplyError::Malformed)?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, JoinReplyError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| JoinReplyError::Malformed)?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn bytes(&mut self, max: usize) -> Result<&'a [u8], JoinReplyError> {
        let len = usize::try_from(self.u32()?).map_err(|_| JoinReplyError::Malformed)?;
        if len > max {
            return Err(JoinReplyError::Malformed);
        }
        self.take(len)
    }

    fn string(&mut self) -> Result<&'a str, JoinReplyError> {
        self.string_limited(128)
    }

    fn string_limited(&mut self, max: usize) -> Result<&'a str, JoinReplyError> {
        std::str::from_utf8(self.bytes(max)?).map_err(|_| JoinReplyError::Malformed)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], JoinReplyError> {
        let end = self.at.checked_add(len).ok_or(JoinReplyError::Malformed)?;
        let bytes = self
            .input
            .get(self.at..end)
            .ok_or(JoinReplyError::Malformed)?;
        self.at = end;
        Ok(bytes)
    }

    fn finish(self) -> Result<(), JoinReplyError> {
        if self.at == self.input.len() {
            Ok(())
        } else {
            Err(JoinReplyError::Malformed)
        }
    }
}

impl fmt::Display for JoinReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::rng::RngError;
    use catcoms_rt::ManualClock;

    #[derive(Default)]
    struct FixedRng(u8);

    impl RngCore for FixedRng {
        fn next_u32(&mut self) -> u32 {
            let mut bytes = [0; 4];
            self.fill_bytes(&mut bytes);
            u32::from_le_bytes(bytes)
        }

        fn next_u64(&mut self) -> u64 {
            let mut bytes = [0; 8];
            self.fill_bytes(&mut bytes);
            u64::from_le_bytes(bytes)
        }

        fn fill_bytes(&mut self, dest: &mut [u8]) {
            for byte in dest {
                *byte = self.0;
                self.0 = self.0.wrapping_add(1);
            }
        }

        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), RngError> {
            self.fill_bytes(dest);
            Ok(())
        }
    }

    fn candidate(text: &str) -> Multiaddr {
        text.parse().unwrap()
    }

    #[test]
    fn round_trip_binds_permit_peer_candidates_nonce_and_expiry() {
        let clock = ManualClock::new(1_000_000);
        let mut rng = FixedRng(7);
        let invite_nonce = [9; 16];
        let peer = PeerId::random();
        let reply = JoinReply::mint(
            b"signed invite".to_vec(),
            &invite_nonce,
            peer,
            vec![
                candidate("/ip4/45.79.12.34/tcp/22487"),
                candidate("/ip6/2606:4700:4700::1111/udp/22487/quic-v1"),
            ],
            &clock,
            &mut rng,
        )
        .unwrap();
        let decoded = JoinReply::decode(&reply.encode()).unwrap();
        assert_eq!(decoded, reply);
        assert_eq!(decoded.verify(&invite_nonce, &clock), Ok(()));
        assert!(decoded
            .dial_targets()
            .iter()
            .all(|address| address.iter().last() == Some(Protocol::P2p(peer))));
    }

    #[test]
    fn tamper_wrong_invite_and_expiry_are_rejected() {
        let clock = ManualClock::new(5_000);
        let mut rng = FixedRng(1);
        let mut reply = JoinReply::mint(
            vec![1, 2, 3],
            &[2; 16],
            PeerId::random(),
            vec![candidate("/ip4/45.79.12.34/udp/9/quic-v1")],
            &clock,
            &mut rng,
        )
        .unwrap();
        assert_eq!(reply.verify(&[3; 16], &clock), Err(JoinReplyError::BadMac));
        reply.expires_at_ms += 1;
        assert_eq!(
            reply.verify(&[2; 16], &clock),
            Err(JoinReplyError::Malformed)
        );

        let valid = JoinReply::mint(
            vec![1],
            &[4; 16],
            PeerId::random(),
            vec![candidate("/ip4/45.79.12.34/tcp/9")],
            &clock,
            &mut rng,
        )
        .unwrap();
        clock.advance_ms(JOIN_REPLY_LIFETIME_MS + 1);
        assert_eq!(valid.verify(&[4; 16], &clock), Err(JoinReplyError::Expired));
    }

    #[test]
    fn unsafe_candidates_are_refused_before_dial() {
        let clock = ManualClock::new(0);
        let mut rng = FixedRng(3);
        for unsafe_address in [
            "/ip4/127.0.0.1/tcp/22",
            "/ip4/192.168.1.1/tcp/22",
            "/dns4/example.com/tcp/443",
            "/ip4/45.79.12.34/tcp/9/p2p-circuit",
        ] {
            assert_eq!(
                JoinReply::mint(
                    vec![1],
                    &[1; 16],
                    PeerId::random(),
                    vec![candidate(unsafe_address)],
                    &clock,
                    &mut rng,
                ),
                Err(JoinReplyError::UnsafeAddress)
            );
        }
    }
}
