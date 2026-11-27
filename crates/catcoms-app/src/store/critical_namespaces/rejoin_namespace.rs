//! Capability for a second native joined incarnation using the original pin.
//! An exact catalog insertion sets the old-reader fence before any A2 key
//! work. These codecs do not settle a mount or authorize a rejoin.
use super::{
    archive_current_namespace_codec::CurrentLayoutData, catalog_declaration,
    pending_join_codec as catalog, refused,
};
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(super) const ID: u32 = 9;
pub(super) const VERSION: u32 = 1;
const TAG: &[u8] = b"catcoms/private-chat-v2-same-vault-rejoin/v1\0";
const BYTES: usize = 4 + TAG.len() + 2 + 6 * 36;

fn fields(
    layout: CurrentLayoutData,
    (current, pin, registry, history): (&[u8], &[u8], &[u8], &[u8]),
) -> [[u8; 32]; 6] {
    [
        layout.vault,
        layout.root,
        blake3::derive_key("catcoms/rejoin-namespace/current/v1", current),
        blake3::derive_key("catcoms/rejoin-namespace/pin/v1", pin),
        blake3::derive_key("catcoms/rejoin-namespace/registry/v1", registry),
        blake3::derive_key("catcoms/rejoin-namespace/history/v1", history),
    ]
}

fn expected(prior: &catalog::Parsed<'_>) -> Result<[[u8; 32]; 6], AppError> {
    Ok(fields(
        prior.current_layout.ok_or_else(refused)?,
        (
            prior.current_payload.ok_or_else(refused)?,
            prior.joined_pin_payload.ok_or_else(refused)?,
            prior.native_registry_payload.ok_or_else(refused)?,
            prior.shared_history_payload.ok_or_else(refused)?,
        ),
    ))
}

fn encode(fields: [[u8; 32]; 6]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut encoder = Encoder::with_capacity(BYTES);
    let invalid = |_| refused();
    encoder.put_bytes(TAG).map_err(invalid)?;
    encoder.put_u16(VERSION as u16);
    for field in fields {
        encoder.put_bytes(&field).map_err(invalid)?;
    }
    Ok(Zeroizing::new(encoder.finish()))
}

pub(super) fn verify_selected_descriptor(
    raw: &[u8],
    layout: CurrentLayoutData,
    leaves: (&[u8], &[u8], &[u8], &[u8]),
) -> Result<(), AppError> {
    require_exact(raw, fields(layout, leaves))
}

fn require_exact(raw: &[u8], fields: [[u8; 32]; 6]) -> Result<(), AppError> {
    if raw.len() != BYTES {
        return Err(refused());
    }
    let mut decoder = Decoder::new(raw);
    let invalid = |_| refused();
    if decoder.get_bytes().map_err(invalid)? != TAG
        || decoder.get_u16().map_err(invalid)? != VERSION as u16
    {
        return Err(refused());
    }
    for field in fields {
        if decoder.get_bytes().map_err(invalid)? != field {
            return Err(refused());
        }
    }
    decoder.finish().map_err(invalid)
}

/// Insert only the new critical capability into the exact fully supported
/// joined-history catalog. Native registry and all historical roles are left
/// byte-for-byte identical through the declaration.
pub(super) fn prepare_declaration(
    original: &[u8],
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let prior = catalog::parse_shared_history_native_registry_layout(original, domain, true)?;
    let descriptor = encode(expected(&prior)?)?;
    let candidate =
        catalog_declaration::insert_entry_data(original, domain, ID, VERSION, &descriptor)?;
    verify_declaration(original, &candidate, domain)?;
    Ok(candidate)
}

/// Prove the single closed insertion against the exact selected parent.
/// The owning physical opener must independently hold both catalog objects
/// and the surviving pinned/current leaves before taking this declaration.
pub(super) fn verify_declaration(
    original: &[u8],
    declaration: &[u8],
    domain: &[u8; 32],
) -> Result<(), AppError> {
    let prior = catalog::parse_shared_history_native_registry_layout(original, domain, true)?;
    catalog_declaration::validate_insert_pair_data(original, declaration, domain, ID, VERSION)?;
    let next = catalog::parse_rejoin_native_registry_layout(declaration, domain, true)?;
    require_exact(next.rejoin_payload.ok_or_else(refused)?, expected(&prior)?)
}

/// Seal the one catalog successor. No rejoin output is admissible before
/// its actual named selection and parent directory have been covered.
pub(in crate::store) fn prepare_framed_declaration(
    key: &[u8; 32],
    original: &[u8],
    domain: &[u8; 32],
    rng: &mut dyn catcoms_rt::CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let plain = open_framed_catalog(key, original)?;
    let next = prepare_declaration(&plain, domain)?;
    Ok(Zeroizing::new(crate::store::frame(&catcoms_crypto::seal(
        key, &next, rng,
    )?)))
}

/// An authenticated full Stage can finish the same Catalog-only declaration
/// from its original named parent. A partial or unrelated frame grants no
/// authority to settle either name.
pub(in crate::store) fn verify_framed_declaration(
    key: &[u8; 32],
    original: &[u8],
    candidate: &[u8],
    domain: &[u8; 32],
) -> Result<(), AppError> {
    let old = open_framed_catalog(key, original)?;
    let new = open_framed_catalog(key, candidate)?;
    verify_declaration(&old, &new, domain)
}

fn open_framed_catalog(key: &[u8; 32], frame: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(41..=catalog::MAX_CATALOG_FRAMED).contains(&frame.len()) {
        return Err(refused());
    }
    Ok(Zeroizing::new(catcoms_crypto::unseal(
        key,
        &crate::store::unframe(frame)?,
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_vault_rejoin_namespace_requires_every_exact_original_descriptor() {
        let layout = CurrentLayoutData {
            vault: [1; 32],
            root: [2; 32],
        };
        let leaves = (
            b"current bytes".as_slice(),
            b"pin bytes".as_slice(),
            b"registry bytes".as_slice(),
            b"history bytes".as_slice(),
        );
        let agreed = fields(layout, leaves);
        let wire = encode(agreed).unwrap();
        require_exact(&wire, agreed).unwrap();
        assert!(require_exact(&wire[..wire.len() - 1], agreed).is_err());
        for index in 0..agreed.len() {
            let mut wrong = agreed;
            wrong[index][0] ^= 1;
            assert!(require_exact(&wire, wrong).is_err());
        }
        let mut extra = wire.to_vec();
        extra.push(0);
        assert!(require_exact(&extra, agreed).is_err());
    }
}
