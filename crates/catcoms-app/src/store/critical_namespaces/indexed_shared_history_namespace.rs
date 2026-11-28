//! Critical companion to the original received-history entry. Existing
//! moderation and membership descriptors continue to bind the exact ID8
//! bytes they were originally issued under.
use super::{
    archive_current_namespace_codec::CurrentLayoutData, catalog_declaration,
    pending_join_codec as catalog, refused,
};
use crate::AppError;
use zeroize::Zeroizing;

pub(in crate::store) mod bootstrap;

pub(super) const ID: u32 = 13;
pub(super) const VERSION: u32 = 1;

fn descriptor(layout: CurrentLayoutData, original_history: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key(
        "catcoms/private-chat-v2-indexed-shared-history-namespace/v1",
    );
    hash.update(&layout.vault);
    hash.update(&layout.root);
    hash.update(blake3::hash(original_history).as_bytes());
    *hash.finalize().as_bytes()
}

pub(super) fn require_descriptor(
    raw: &[u8],
    layout: CurrentLayoutData,
    original_history: &[u8],
) -> Result<(), AppError> {
    if raw != descriptor(layout, original_history) {
        return Err(refused());
    }
    Ok(())
}

pub(in crate::store) fn marker_data(
    vault_key: &[u8; 32],
    selected: &[u8],
) -> Result<[u8; 32], AppError> {
    if selected.len() != 32 {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(
        "catcoms/private-chat-v2-indexed-shared-history-marker-key/v1",
        vault_key,
    ));
    Ok(*blake3::keyed_hash(&key, selected).as_bytes())
}

pub(super) fn prepare_declaration(
    original: &[u8],
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    // The fresh declaration runs before another namespace depends on the
    // complete encrypted catalog. A previously started rejoin keeps it.
    let parent = catalog::parse_shared_history_native_registry_layout(original, domain, true)?;
    let value = descriptor(
        parent.current_layout.ok_or_else(refused)?,
        parent.shared_history_payload.ok_or_else(refused)?,
    );
    let candidate = catalog_declaration::insert_entry_data(original, domain, ID, VERSION, &value)?;
    verify_declaration(original, &candidate, domain)?;
    Ok(candidate)
}

pub(super) fn verify_declaration(
    original: &[u8],
    candidate: &[u8],
    domain: &[u8; 32],
) -> Result<(), AppError> {
    catalog::parse_shared_history_native_registry_layout(original, domain, true)?;
    catalog_declaration::validate_insert_pair_data(original, candidate, domain, ID, VERSION)?;
    catalog::parse_indexed_shared_history_base(candidate, domain, true)?;
    Ok(())
}
