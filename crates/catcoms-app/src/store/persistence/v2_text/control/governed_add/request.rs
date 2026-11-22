//! The encrypted original request makes the reservation recoverable without
//! relying on a new peer delivery. Its identity includes the sealed fence.
use super::*;
use catcoms_sync::MAX_V2_JOIN_REQUEST_PACKET_BYTES;

pub(super) const NAME: &str = "text.add.request.v2";
const TAG: &[u8] = b"catcoms/private-chat-text-governed-add-request/v2\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-request-key/v2";
const PARENT_DOMAIN: &str = "catcoms/private-chat-text-governed-add-request-parent/v2";
const VERSION: u16 = 2;
const OVERHEAD: usize = 4 + TAG.len() + 2 + 36 + 4 + 40;
pub(super) const MIN_BYTES: usize = OVERHEAD + 1;
pub(super) const MAX_BYTES: usize = OVERHEAD + MAX_V2_JOIN_REQUEST_PACKET_BYTES;

fn parent(fence: &[u8]) -> Result<[u8; 32], AppError> {
    if fence.len() != FRAME_BYTES {
        return Err(refused());
    }
    Ok(blake3::derive_key(PARENT_DOMAIN, fence))
}

pub(super) fn seal_data(
    key: &[u8; 32],
    fence: &[u8],
    packet: &V2JoinRequestPacketData<'_>,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let reservation = GovernedAddFenceData::open_data(key, fence)?;
    reservation.require_packet(packet)?;
    let packet = packet.packet_bytes_data();
    if packet.is_empty() || packet.len() > MAX_V2_JOIN_REQUEST_PACKET_BYTES {
        return Err(refused());
    }
    let mut e = Encoder::with_capacity(OVERHEAD - 40 + packet.len());
    put(&mut e, TAG)?;
    e.put_u16(VERSION);
    put(&mut e, &parent(fence)?)?;
    put(&mut e, packet)?;
    let plain = Zeroizing::new(e.finish());
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    Ok(Zeroizing::new(frame(&seal(&key, &plain, rng)?)))
}

pub(super) fn open_data(
    key: &[u8; 32],
    fence: &[u8],
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(MIN_BYTES..=MAX_BYTES).contains(&raw.len()) {
        return Err(refused());
    }
    let reservation = GovernedAddFenceData::open_data(key, fence)?;
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
    let mut d = Decoder::new(&plain);
    if field(&mut d, TAG.len())? != TAG
        || d.get_u16().map_err(|_| refused())? != VERSION
        || b32(&mut d)? != parent(fence)?
    {
        return Err(refused());
    }
    let packet = field(&mut d, MAX_V2_JOIN_REQUEST_PACKET_BYTES)?;
    d.finish().map_err(|_| refused())?;
    reservation.require_packet(&V2JoinRequestPacketData::decode(packet)?)?;
    Ok(Zeroizing::new(packet.to_vec()))
}
