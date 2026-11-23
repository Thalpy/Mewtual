//! Exact catalog DATA for a received-history owner on the native joined layout.
//! This only describes a declaration. A physical owner must still admit the
//! exact original and candidate files, directory and resource account.
use super::{
    archive_current_namespace_codec::CurrentLayoutData, catalog_declaration,
    pending_join_codec as catalog, refused,
};
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(in crate::store) mod declaration;

pub(super) const ID: u32 = 8;
pub(super) const VERSION: u32 = 1;
const TAG: &[u8] = b"catcoms/private-chat-v2-shared-history-namespace/v1\0";
const BYTES: usize = 4 + TAG.len() + 2 + 5 * 36;

fn fields(
    layout: CurrentLayoutData,
    current: &[u8],
    joined: &[u8],
    native: &[u8],
) -> [[u8; 32]; 5] {
    [
        layout.vault,
        layout.root,
        blake3::derive_key("catcoms/shared-history-namespace/current/v1", current),
        blake3::derive_key("catcoms/shared-history-namespace/joined/v1", joined),
        blake3::derive_key("catcoms/shared-history-namespace/native/v1", native),
    ]
}

pub(super) fn validate_descriptor(
    raw: &[u8],
    layout: CurrentLayoutData,
    current: &[u8],
    joined: &[u8],
    native: &[u8],
) -> Result<(), AppError> {
    if raw.len() != BYTES {
        return Err(refused());
    }
    let invalid = |_| refused();
    let mut input = Decoder::new(raw);
    if input.get_bytes().map_err(invalid)? != TAG
        || input.get_u16().map_err(invalid)? != VERSION as u16
    {
        return Err(refused());
    }
    for expected in fields(layout, current, joined, native) {
        if input.get_bytes().map_err(invalid)? != expected {
            return Err(refused());
        }
    }
    input.finish().map_err(invalid)
}

pub(super) fn prepare_declaration(
    original: &[u8],
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let prior = catalog::parse_joined_native_registry_layout(original, domain, true)?;
    let mut descriptor = Encoder::with_capacity(BYTES);
    let invalid = |_| refused();
    descriptor.put_bytes(TAG).map_err(invalid)?;
    descriptor.put_u16(VERSION as u16);
    for field in fields(
        prior.current_layout.ok_or_else(refused)?,
        prior.current_payload.ok_or_else(refused)?,
        prior.joined_pin_payload.ok_or_else(refused)?,
        prior.native_registry_payload.ok_or_else(refused)?,
    ) {
        descriptor.put_bytes(&field).map_err(invalid)?;
    }
    let declaration = catalog_declaration::insert_entry_data(
        original,
        domain,
        ID,
        VERSION,
        &descriptor.finish(),
    )?;
    verify_declaration(original, &declaration, domain)?;
    Ok(declaration)
}

pub(super) fn verify_declaration(
    original: &[u8],
    declaration: &[u8],
    domain: &[u8; 32],
) -> Result<(), AppError> {
    catalog::parse_joined_native_registry_layout(original, domain, true)?;
    catalog_declaration::validate_insert_pair_data(original, declaration, domain, ID, VERSION)?;
    catalog::parse_shared_history_native_registry_layout(declaration, domain)?;
    Ok(())
}
