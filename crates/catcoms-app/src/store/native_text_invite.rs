//! The initial founder invitation packet is inert wire data. Its only routing
//! profile describes a reservation with zero contact addresses.
catcoms_rt::redacted_debug! {
    NativeTextSavedJoinResponse;
}

use super::critical_namespaces::{
    read_pool::{CatalogReadReservation, CriticalCatalogReadPool},
    refused,
};
use crate::AppError;
use catcoms_mls::{GroupMode, GroupPolicy};
use catcoms_net::SeededTransportIdentityData;
use catcoms_replication::chat::{
    join_bindings::{SignedInviteBindingData, MAX_JOIN_BINDING_BYTES},
    ChatError, MAX_GROUP_ID_BYTES,
};
use catcoms_wire::{Decoder, Encoder};
use std::sync::Arc;
use zeroize::Zeroizing;

const PACKET: &[u8] = b"mewtual/chat/native-invite-packet/v2\0";
const RESERVATION: &[u8] = b"mewtual/chat/native-invite-routing-plan/v2\0";
const VERSION: u16 = 2;
const TCP_RESERVATION: u16 = 1;
const MAX_POLICY: usize = 1024;
const MAX_ROUTING: usize = 1024;
pub const MAX_NATIVE_TEXT_INVITE_PACKET_BYTES: usize =
    4 + PACKET.len() + 2 + 3 * 4 + MAX_JOIN_BINDING_BYTES + MAX_POLICY + MAX_ROUTING;

/// Observed original journal state. Incomplete records grant no key retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextInviteRecovery {
    IntentOnly,
    KeyAttemptHeld,
    SignatureRecovered {
        binding_id: [u8; 32],
        expires_at_ms: u64,
    },
}

/// Verified signature, policy and reservation relationships. No accepted pin,
/// present membership, expiry, possession or permission to dial is implied.
pub struct NativeTextInvitePacketData<'a> {
    encoded: &'a [u8],
    binding: SignedInviteBindingData,
    policy: &'a [u8],
    reservation: &'a [u8],
    peer: catcoms_rt::PeerId,
    peer_address_id: &'a str,
    tcp_port: u16,
}

impl std::fmt::Debug for NativeTextInvitePacketData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextInvitePacketData")
            .finish_non_exhaustive()
    }
}

impl<'a> NativeTextInvitePacketData<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_NATIVE_TEXT_INVITE_PACKET_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes()? != PACKET || d.get_u16()? != VERSION {
            return Err(ChatError::Malformed);
        }
        let binding = d.get_bytes()?;
        let policy = d.get_bytes()?;
        let reservation = d.get_bytes()?;
        d.finish()?;
        if policy.len() > MAX_POLICY || reservation.len() > MAX_ROUTING {
            return Err(ChatError::Bound);
        }
        let binding = SignedInviteBindingData::decode(binding)?;
        let (peer, peer_address_id, tcp_port) = check_parts(&binding, policy, reservation)?;
        Ok(Self {
            encoded: bytes,
            binding,
            policy,
            reservation,
            peer,
            peer_address_id,
            tcp_port,
        })
    }

    /// The complete validated packet, including the exact authenticated
    /// policy and transport reservation retained by a native attempt.
    pub fn encoded_data(&self) -> &'a [u8] {
        self.encoded
    }

    pub fn binding(&self) -> &SignedInviteBindingData {
        &self.binding
    }

    pub fn policy_bytes(&self) -> &'a [u8] {
        self.policy
    }

    pub fn reservation_bytes(&self) -> &'a [u8] {
        self.reservation
    }

    /// The invite's authenticated transport identity for connected routing.
    pub fn connected_peer_data(&self) -> catcoms_rt::PeerId {
        self.peer
    }

    pub(super) fn direct_address_data(&self, host: std::net::IpAddr) -> String {
        let family = if host.is_ipv4() { "ip4" } else { "ip6" };
        format!(
            "/{family}/{host}/tcp/{}/p2p/{}",
            self.tcp_port, self.peer_address_id
        )
    }
}

fn check_parts<'a>(
    binding: &SignedInviteBindingData,
    policy: &[u8],
    reservation: &'a [u8],
) -> Result<(catcoms_rt::PeerId, &'a str, u16), ChatError> {
    if policy.len() > MAX_POLICY || reservation.len() > MAX_ROUTING {
        return Err(ChatError::Bound);
    }
    binding.check_public_bodies_data(policy, reservation)?;
    let anchor = &binding.spec_data().anchor;
    let policy = GroupPolicy::decode(policy).map_err(|_| ChatError::Malformed)?;
    if binding.spec_data().inviter != anchor.founder
        || policy.mode() != GroupMode::PeerToPeer
        || policy.issued_epoch() != 0
        || policy.group_id() != anchor.group.as_slice()
        || policy.issuer_public_key() != anchor.founder.as_slice()
    {
        return Err(ChatError::Scope);
    }
    check_reservation(reservation, &anchor.group)
}

pub(in crate::store) fn checked_founder_peer(
    binding: &SignedInviteBindingData,
    reservation: &[u8],
) -> Result<catcoms_rt::PeerId, ChatError> {
    checked_founder_reservation(binding, reservation).map(|value| value.0)
}

/// Reconstruct a numeric route from the signed reservation and separate local consent.
pub(in crate::store) fn checked_founder_route(
    binding: &SignedInviteBindingData,
    reservation: &[u8],
    host: std::net::IpAddr,
) -> Result<(catcoms_rt::PeerId, String), ChatError> {
    let (peer, address, port) = checked_founder_reservation(binding, reservation)?;
    let family = if host.is_ipv4() { "ip4" } else { "ip6" };
    Ok((peer, format!("/{family}/{host}/tcp/{port}/p2p/{address}")))
}

fn checked_founder_reservation<'a>(
    binding: &SignedInviteBindingData,
    reservation: &'a [u8],
) -> Result<(catcoms_rt::PeerId, &'a str, u16), ChatError> {
    use catcoms_replication::chat::join_bindings::JoinPayloadKindData;
    if reservation.len() > MAX_ROUTING
        || !binding.verify()
        || binding.spec_data().inviter != binding.spec_data().anchor.founder
        || JoinPayloadKindData::RoutingPlan.commitment_data(reservation)?
            != binding.spec_data().public_routing_plan
    {
        return Err(ChatError::Scope);
    }
    // The same closed reservation grammar as the original invitation:
    // authenticated identity, TCP port and exactly zero contact addresses.
    check_reservation(reservation, &binding.spec_data().anchor.group)
}

fn check_reservation<'a>(
    bytes: &'a [u8],
    group: &[u8],
) -> Result<(catcoms_rt::PeerId, &'a str, u16), ChatError> {
    let mut d = Decoder::new(bytes);
    if d.get_bytes()? != RESERVATION || d.get_u16()? != VERSION {
        return Err(ChatError::Malformed);
    }
    if d.get_bytes()? != group {
        return Err(ChatError::Scope);
    }
    let key = d
        .get_bytes()?
        .try_into()
        .map_err(|_| ChatError::Malformed)?;
    let peer = d.get_str()?;
    let commitment = d.get_bytes()?;
    let kind = d.get_u16()?;
    let port = d.get_u16()?;
    if kind != TCP_RESERVATION || port == 0 || d.get_u32()? != 0 {
        return Err(ChatError::Malformed);
    }
    d.finish()?;
    let identity =
        SeededTransportIdentityData::authenticated_peer_data(key, peer.as_bytes(), commitment)
            .map_err(|_| ChatError::Malformed)?
            .ok_or(ChatError::Scope)?;
    Ok((identity, peer, port))
}

pub(super) fn encode_reservation(
    group: &[u8],
    identity: &SeededTransportIdentityData,
    port: u16,
) -> Result<Vec<u8>, ChatError> {
    if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES || port == 0 {
        return Err(ChatError::Bound);
    }
    let mut e = Encoder::with_capacity(MAX_ROUTING);
    e.put_bytes(RESERVATION)?;
    e.put_u16(VERSION);
    e.put_bytes(group)?;
    e.put_bytes(identity.public_key_data())?;
    e.put_str(identity.peer_id_data())?;
    e.put_bytes(&identity.commitment_data())?;
    e.put_u16(TCP_RESERVATION);
    e.put_u16(port);
    e.put_u32(0); // no certified contacts
    Ok(e.finish())
}

fn encode_packet(binding: &[u8], policy: &[u8], reservation: &[u8]) -> Result<Vec<u8>, ChatError> {
    if binding.len() > MAX_JOIN_BINDING_BYTES
        || policy.len() > MAX_POLICY
        || reservation.len() > MAX_ROUTING
    {
        return Err(ChatError::Bound);
    }
    let mut e = Encoder::with_capacity(MAX_NATIVE_TEXT_INVITE_PACKET_BYTES);
    e.put_bytes(PACKET)?;
    e.put_u16(VERSION);
    e.put_bytes(binding)?;
    e.put_bytes(policy)?;
    e.put_bytes(reservation)?;
    Ok(e.finish())
}

/// A paid original result from the warm native invitation save. The packet
/// can be inspected by trusted Rust. It has no certified address, publication
/// permission or current Add authority.
pub struct NativeTextSavedInvite(PreparedNativeTextInvitePacket);

/// The exact response released by a covered, installed governed Add.
pub struct NativeTextSavedJoinResponse(PreparedNativeTextJoinResponse);

pub(super) struct PreparedNativeTextJoinResponse(Arc<PaidJoinResponse>);

struct PaidJoinResponse {
    packet: Zeroizing<Vec<u8>>,
    proof: Vec<u8>,
    later: bool,
    _payment: CatalogReadReservation,
}

impl PreparedNativeTextJoinResponse {
    pub(super) fn prepare(
        pool: &CriticalCatalogReadPool,
        packet: Zeroizing<Vec<u8>>,
        proof: Vec<u8>,
        later: bool,
    ) -> Result<Self, AppError> {
        let proof_cap = if later {
            catcoms_sync::MAX_V2_GOVERNING_JOIN_PREFIX_BYTES
        } else {
            catcoms_sync::MAX_V2_FIRST_ADD_PROOF_BYTES
        };
        if packet.is_empty()
            || packet.capacity() > catcoms_sync::MAX_V2_JOIN_RESPONSE_PACKET_BYTES
            || proof.is_empty()
            || proof.capacity() > proof_cap
        {
            return Err(refused());
        }
        let bytes = packet.capacity()
            + proof.capacity()
            + 4 * std::mem::size_of::<PaidJoinResponse>()
            + 256;
        let payment = pool.try_reserve(bytes)?;
        payment.validate(pool, bytes)?;
        Ok(Self(Arc::new(PaidJoinResponse {
            packet,
            proof,
            later,
            _payment: payment,
        })))
    }

    pub(super) fn packet_bytes_data(&self) -> &[u8] {
        &self.0.packet
    }

    fn governing_proof_bytes_data(&self) -> &[u8] {
        &self.0.proof
    }

    pub(super) fn saved_data(&self) -> NativeTextSavedJoinResponse {
        NativeTextSavedJoinResponse(Self(self.0.clone()))
    }
}

impl NativeTextSavedJoinResponse {
    pub fn packet_bytes_data(&self) -> &[u8] {
        self.0.packet_bytes_data()
    }

    /// Exact signed governing records through the joiner's selected Add.
    pub fn governing_proof_bytes_data(&self) -> &[u8] {
        self.0.governing_proof_bytes_data()
    }

    /// The transport owner supplies the separate bounded output allocation.
    pub fn encode_delivery_data(&self) -> Result<Vec<u8>, AppError> {
        let packet = self.packet_bytes_data();
        let proof = self.governing_proof_bytes_data();
        if self.0 .0.later {
            Ok(catcoms_sync::V2GoverningJoinDeliveryData::encode(
                packet, proof,
            )?)
        } else {
            Ok(catcoms_sync::V2FirstJoinDeliveryData::encode(
                packet, proof,
            )?)
        }
    }
}

pub(super) struct PreparedNativeTextInvitePacket {
    binding_id: [u8; 32],
    expires_at_ms: u64,
    packet: Zeroizing<Vec<u8>>,
    _payment: CatalogReadReservation,
}

impl PreparedNativeTextInvitePacket {
    pub(super) fn prepare(
        pool: &CriticalCatalogReadPool,
        binding: &SignedInviteBindingData,
        policy: &[u8],
        reservation: &[u8],
    ) -> Result<Self, AppError> {
        // Pays the returned packet and validation overlap independently of
        // the Host. The packet can outlive the runtime which constructed it.
        const BYTES: usize =
            4 * MAX_NATIVE_TEXT_INVITE_PACKET_BYTES + std::mem::size_of::<NativeTextSavedInvite>();
        let payment = pool.try_reserve(BYTES)?;
        payment.validate(pool, BYTES)?;
        check_parts(binding, policy, reservation).map_err(|_| refused())?;
        let packet =
            encode_packet(&binding.encode(), policy, reservation).map_err(|_| refused())?;
        Ok(Self {
            binding_id: binding.id_data(),
            expires_at_ms: binding.spec_data().expires_at_ms,
            packet: Zeroizing::new(packet),
            _payment: payment,
        })
    }

    /// The owning Host calls this only after the Raw barrier and live checks.
    pub(super) fn into_saved(self) -> NativeTextSavedInvite {
        NativeTextSavedInvite(self)
    }
}

impl NativeTextSavedInvite {
    pub fn binding_id(&self) -> [u8; 32] {
        self.0.binding_id
    }

    pub fn expires_at_ms(&self) -> u64 {
        self.0.expires_at_ms
    }

    /// Original signed packet bytes for a later owning publication consumer.
    pub fn packet_bytes_data(&self) -> &[u8] {
        self.0.packet.as_slice()
    }
}

impl PartialEq for NativeTextSavedInvite {
    fn eq(&self, other: &Self) -> bool {
        self.packet_bytes_data() == other.packet_bytes_data()
    }
}
impl Eq for NativeTextSavedInvite {}
impl std::fmt::Debug for NativeTextSavedInvite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextSavedInvite")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
