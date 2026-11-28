//! Catalog data for the joined recipient's independent review journal.
//! Selecting the namespace requires its physical declaration and mount.
use super::{
    archive_current_namespace_codec::CurrentLayoutData, catalog_declaration,
    pending_join_codec as catalog, refused,
};
use crate::AppError;
use zeroize::Zeroizing;

pub(in crate::store) mod bootstrap;

pub(super) const ID: u32 = 12;
pub(super) const VERSION: u32 = 1;

/// Successor JOIN or ID9 declarations preserve these original descriptor
/// bytes. The framed catalog changes on either operation.
fn descriptor(
    layout: CurrentLayoutData,
    joined: &[u8],
    registry: &[u8],
    history: &[u8],
) -> [u8; 32] {
    let mut hash =
        blake3::Hasher::new_derive_key("catcoms/private-chat-v2-review-receiver-namespace/v1");
    hash.update(&layout.vault);
    hash.update(&layout.root);
    for field in [joined, registry, history] {
        hash.update(blake3::hash(field).as_bytes());
    }
    *hash.finalize().as_bytes()
}

pub(super) fn require_descriptor(
    raw: &[u8],
    layout: CurrentLayoutData,
    (joined, registry, history): (&[u8], &[u8], &[u8]),
) -> Result<(), AppError> {
    if raw != descriptor(layout, joined, registry, history) {
        return Err(refused());
    }
    Ok(())
}

pub(in crate::store) fn marker_data(
    vault_key: &[u8; 32],
    descriptor: &[u8],
) -> Result<[u8; 32], AppError> {
    if descriptor.len() != 32 {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(
        "catcoms/private-chat-v2-review-receiver-marker-key/v1",
        vault_key,
    ));
    Ok(*blake3::keyed_hash(&key, descriptor).as_bytes())
}

pub(super) fn prepare_declaration(
    original: &[u8],
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    // This initial declaration precedes ID9. The already-created A2
    // storage binds its whole catalog ciphertext and needs its own
    // migration before that identity can change.
    let parent = catalog::parse_shared_history_native_registry_layout(original, domain, true)?;
    let value = descriptor(
        parent.current_layout.ok_or_else(refused)?,
        parent.joined_pin_payload.ok_or_else(refused)?,
        parent.native_registry_payload.ok_or_else(refused)?,
        parent.shared_history_payload.ok_or_else(refused)?,
    );
    let updated = catalog_declaration::insert_entry_data(original, domain, ID, VERSION, &value)?;
    verify_declaration(original, &updated, domain)?;
    Ok(updated)
}

pub(super) fn verify_declaration(
    original: &[u8],
    updated: &[u8],
    domain: &[u8; 32],
) -> Result<(), AppError> {
    catalog::parse_shared_history_native_registry_layout(original, domain, true)?;
    catalog_declaration::validate_insert_pair_data(original, updated, domain, ID, VERSION)?;
    let selected = catalog::parse_review_receive_family(updated, domain, true)?;
    if selected.review_receive_payload.is_none() || selected.rejoin_payload.is_some() {
        return Err(refused());
    }
    Ok(())
}
