//! Exact ID10 catalog DATA. The original founder's physical owner must
//! still prove the pinned row, own both files, and cover its private marker.
use super::{
    archive_current_namespace_codec::CurrentLayoutData, catalog_declaration,
    pending_join_codec as catalog, refused,
};
use crate::AppError;
use zeroize::Zeroizing;

pub(super) const ID: u32 = 10;
pub(super) const VERSION: u32 = 1;
const DOMAIN: &str = "catcoms/private-chat-v2-review-hold-namespace/v1";
const MARKER_KEY: &str = "catcoms/private-chat-v2-review-hold-marker-key/v1";
pub(in crate::store) const MARKER_LEN: usize = 32;

/// Authenticate the selected descriptor under the original vault key. The
/// physical holder still has to tie these bytes to the actual marker name
/// and the original directory. Catalog or marker bytes alone confer no role.
pub(in crate::store) fn marker_data(
    vault_key: &[u8; 32],
    descriptor: &[u8],
) -> Result<[u8; MARKER_LEN], AppError> {
    if descriptor.len() != 32 {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(MARKER_KEY, vault_key));
    Ok(*blake3::keyed_hash(&key, descriptor).as_bytes())
}

fn descriptor(layout: CurrentLayoutData, shared_history: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key(DOMAIN);
    hash.update(&layout.vault);
    hash.update(&layout.root);
    // ID8 already binds exact Current, joined pins and native registry.
    hash.update(shared_history);
    *hash.finalize().as_bytes()
}

pub(super) fn verify_descriptor(
    raw: &[u8],
    layout: CurrentLayoutData,
    shared_history: &[u8],
) -> Result<(), AppError> {
    if raw != descriptor(layout, shared_history) {
        return Err(refused());
    }
    Ok(())
}

/// Add only ID10 to an exact ID8 parent. The old grammar rejects ID9 and
/// ID10, so even a genuine A2 cannot be passed as a declaration source.
pub(super) fn prepare_declaration(
    original: &[u8],
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let parent = catalog::parse_shared_history_native_registry_layout(original, domain, true)?;
    let data = descriptor(
        parent.current_layout.ok_or_else(refused)?,
        parent.shared_history_payload.ok_or_else(refused)?,
    );
    let candidate = catalog_declaration::insert_entry_data(original, domain, ID, VERSION, &data)?;
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
    let parsed = catalog::parse_review_hold_family(candidate, domain, true)?;
    parsed.review_hold_payload.ok_or_else(refused).map(|_| ())
}
