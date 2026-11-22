//! The original unsigned Welcome preimage, bound to the exact governing row.
use super::*;
use catcoms_replication::chat::{
    join_bindings::{JoinPayloadKindData, WelcomeBindingSpecData, MAX_JOIN_BINDING_BYTES},
    owner_facts::OwnerFactId,
    owner_header::OwnerHeaderId,
};

pub(super) const NAME: &str = "text.add.welcome-intent.v2";

const TAG: &[u8] = b"catcoms/private-chat-text-governed-add-welcome-intent/v2\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-welcome-intent-key/v2";
const PARENT_DOMAIN: &str = "catcoms/private-chat-text-governed-add-welcome-intent-parent/v2";
const VERSION: u16 = 2;
const BASE: usize = 4 + TAG.len() + 2 + 36 + 4 + 40;
pub(super) const MIN_BYTES: usize = BASE + 1;
pub(super) const MAX_BYTES: usize = BASE + MAX_JOIN_BINDING_BYTES;

fn parent(frame: &[u8]) -> Result<[u8; 32], AppError> {
    if !(governing_record::MIN_BYTES..=governing_record::MAX_BYTES).contains(&frame.len()) {
        return Err(refused());
    }
    Ok(blake3::derive_key(PARENT_DOMAIN, frame))
}

pub(super) fn require_original_binding(
    spec: &WelcomeBindingSpecData,
    packet: &V2JoinRequestPacketData<'_>,
    signer: [u8; 32],
    header: OwnerHeaderId,
    admission: OwnerFactId,
    [welcome, routing]: [&[u8]; 2],
) -> Result<(), AppError> {
    let input = packet.founder_add_data();
    let request = input.request.spec_data();
    let invite = input.invite.spec_data();
    if spec.invite != input.invite.id_data()
        || spec.anchor != request.anchor
        || spec.request_binding != input.request.id_data()
        || spec.key_package_wire != request.key_package_wire
        || spec.key_package_reference != request.key_package_reference
        || spec.admitter != signer
        || spec.add_header != *header.as_bytes()
        || spec.admission != *admission.as_bytes()
        || spec.policy != invite.policy
        || spec.public_routing_plan != invite.public_routing_plan
        || spec.welcome
            != JoinPayloadKindData::Welcome
                .commitment_data(welcome)
                .map_err(|_| refused())?
        || spec.sealed_routing_transfer
            != JoinPayloadKindData::RoutingTransfer
                .commitment_data(routing)
                .map_err(|_| refused())?
    {
        return Err(refused());
    }
    Ok(())
}

pub(super) fn seal_data(
    key: &[u8; 32],
    governing: &[u8],
    preimage: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let parent = parent(governing)?;
    WelcomeBindingSpecData::decode_signing_bytes_data(preimage).map_err(|_| refused())?;
    let mut e = Encoder::with_capacity(BASE - 40 + preimage.len());
    put(&mut e, TAG)?;
    e.put_u16(VERSION);
    put(&mut e, &parent)?;
    put(&mut e, preimage)?;
    let plain = Zeroizing::new(e.finish());
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    Ok(Zeroizing::new(frame(&seal(&key, &plain, rng)?)))
}

/// Recover the bounded DATA only. The owner compares it to the original
/// request, covered Admission and cached MLS Welcome/routing outputs.
pub(super) fn open_data(
    key: &[u8; 32],
    governing: &[u8],
    raw: &[u8],
) -> Result<WelcomeBindingSpecData, AppError> {
    if !(BASE..=MAX_BYTES).contains(&raw.len()) {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
    let mut d = Decoder::new(&plain);
    if field(&mut d, TAG.len())? != TAG
        || d.get_u16().map_err(|_| refused())? != VERSION
        || b32(&mut d)? != parent(governing)?
    {
        return Err(refused());
    }
    let preimage = field(&mut d, MAX_JOIN_BINDING_BYTES)?;
    d.finish().map_err(|_| refused())?;
    WelcomeBindingSpecData::decode_signing_bytes_data(preimage).map_err(|_| refused())
}

#[cfg(test)]
mod tests;
