//! Exact successor data for a founder's independent decision journal.
//! Durable selection still needs the original founder and filesystem owner.
use super::{
    archive_current_namespace_codec::CurrentLayoutData, catalog_declaration,
    pending_join_codec as catalog, refused,
};
use crate::AppError;
use zeroize::Zeroizing;

pub(super) const ID: u32 = 11;
pub(super) const VERSION: u32 = 1;

fn descriptor(layout: CurrentLayoutData, shared: &[u8], held: &[u8]) -> [u8; 32] {
    let mut hash =
        blake3::Hasher::new_derive_key("catcoms/private-chat-v2-review-decisions-namespace/v1");
    hash.update(&layout.vault);
    hash.update(&layout.root);
    hash.update(shared);
    hash.update(held);
    *hash.finalize().as_bytes()
}

pub(super) fn verify_descriptor(
    raw: &[u8],
    layout: CurrentLayoutData,
    shared: &[u8],
    held: &[u8],
) -> Result<(), AppError> {
    if raw != descriptor(layout, shared, held) {
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
        "catcoms/private-chat-v2-review-decisions-marker-key/v1",
        vault_key,
    ));
    Ok(*blake3::keyed_hash(&key, descriptor).as_bytes())
}

pub(super) fn prepare_declaration(
    original: &[u8],
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let parent = catalog::parse_review_hold_family(original, domain, true)?;
    let expected = descriptor(
        parent.current_layout.ok_or_else(refused)?,
        parent.shared_history_payload.ok_or_else(refused)?,
        parent.review_hold_payload.ok_or_else(refused)?,
    );
    let candidate =
        catalog_declaration::insert_entry_data(original, domain, ID, VERSION, &expected)?;
    verify_declaration(original, &candidate, domain)?;
    Ok(candidate)
}

pub(super) fn verify_declaration(
    original: &[u8],
    candidate: &[u8],
    domain: &[u8; 32],
) -> Result<(), AppError> {
    let parent = catalog::parse_review_hold_family(original, domain, true)?;
    if parent.review_hold_payload.is_none() || parent.count != 0 {
        return Err(refused());
    }
    catalog_declaration::validate_insert_pair_data(original, candidate, domain, ID, VERSION)?;
    let parsed = catalog::parse_review_decision_family(candidate, domain, true)?;
    if parsed.review_decision_payload.is_none() || parsed.count != 0 {
        return Err(refused());
    }
    Ok(())
}
