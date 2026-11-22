//! Exact private Welcome response, linked to its covered native signature.
use super::header_signing::WELCOME_RAW_BYTES;
use super::*;
use catcoms_replication::chat::join_bindings::{SignedWelcomeBindingData, WelcomeBindingSpecData};
use catcoms_sync::{
    V2JoinResponseData, V2JoinResponsePacketData, MAX_V2_JOIN_RESPONSE_PACKET_BYTES,
};

pub(super) const NAME: &str = "text.add.response.v2";
const TAG: &[u8] = b"catcoms/private-chat-text-governed-add-response/v2\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-response-key/v2";
const PARENT_DOMAIN: &str = "catcoms/private-chat-text-governed-add-response-parent/v2";
const VERSION: u16 = 2;
const OVERHEAD: usize = 4 + TAG.len() + 2 + 36 + 4 + 40;
pub(super) const MAX_BYTES: usize = OVERHEAD + MAX_V2_JOIN_RESPONSE_PACKET_BYTES;

pub(super) struct OriginalResponse<'a> {
    pub(super) preimage: &'a [u8],
    pub(super) signature: &'a [u8; 64],
    pub(super) payloads: [&'a [u8]; 2],
}

impl OriginalResponse<'_> {
    pub(super) fn require_packet(&self, actual: &V2JoinResponseData<'_>) -> Result<(), AppError> {
        let spec = WelcomeBindingSpecData::decode_signing_bytes_data(self.preimage)
            .map_err(|_| refused())?;
        let binding = SignedWelcomeBindingData::sign_using(spec, |preimage| {
            if preimage == self.preimage {
                Ok(*self.signature)
            } else {
                Err(catcoms_replication::chat::ChatError::Dependency)
            }
        })
        .map_err(|_| refused())?;
        if [actual.welcome, actual.sealed_routing] != self.payloads
            || actual.binding.encode() != binding.encode()
        {
            return Err(refused());
        }
        Ok(())
    }
}

fn parent(raw_welcome: &[u8]) -> Result<[u8; 32], AppError> {
    if raw_welcome.len() != WELCOME_RAW_BYTES {
        return Err(refused());
    }
    Ok(blake3::derive_key(PARENT_DOMAIN, raw_welcome))
}

/// Caller has prepaid all packet and cipher scratch and checked the original
/// binding and payloads. This function never hands out shareable bytes.
pub(super) fn seal_data(
    key: &[u8; 32],
    raw_welcome: &[u8],
    packet: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if packet.is_empty() || packet.len() > MAX_V2_JOIN_RESPONSE_PACKET_BYTES {
        return Err(refused());
    }
    let mut e = Encoder::with_capacity(OVERHEAD - 40 + packet.len());
    put(&mut e, TAG)?;
    e.put_u16(VERSION);
    put(&mut e, &parent(raw_welcome)?)?;
    put(&mut e, packet)?;
    let plain = Zeroizing::new(e.finish());
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    Ok(Zeroizing::new(frame(&seal(&key, &plain, rng)?)))
}

pub(super) fn open_data(
    key: &[u8; 32],
    raw_welcome: &[u8],
    raw: &[u8],
    original: OriginalResponse<'_>,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(OVERHEAD + 1..=MAX_BYTES).contains(&raw.len()) {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
    let mut d = Decoder::new(&plain);
    if field(&mut d, TAG.len())? != TAG
        || d.get_u16().map_err(|_| refused())? != VERSION
        || b32(&mut d)? != parent(raw_welcome)?
    {
        return Err(refused());
    }
    let packet = field(&mut d, MAX_V2_JOIN_RESPONSE_PACKET_BYTES)?;
    d.finish().map_err(|_| refused())?;
    let decoded = V2JoinResponsePacketData::decode(packet)?;
    original.require_packet(&decoded.response_data())?;
    Ok(Zeroizing::new(packet.to_vec()))
}

#[cfg(test)]
mod tests;
