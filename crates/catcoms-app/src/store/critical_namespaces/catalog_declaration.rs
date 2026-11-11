//! Shared exact whole-catalog declaration DATA: insert ONE namespace entry at its sorted
//! position over the COMPLETE original catalog plaintext.
//!
//! Why a shared helper. The outer envelope requires strictly ascending namespace IDs
//! (`catalog_envelope::parse`). Namespaces are declared independently and in either order:
//! the member outbox (ID5) may be declared on a vault that never declares governance (ID4),
//! and governance may later be declared on a vault that already holds ID5. Appending at the
//! end would produce `[.., 5, 4]`, which every parser refuses, so a later declaration would
//! become impossible. Inserting at the sorted position keeps the strict order; for a catalog
//! whose existing IDs are all below the new one, the output is byte-identical to the older
//! append-only builder (so already-written governance declarations do not change).
//!
//! What a valid pair means, and nothing more: the candidate is the original's direct
//! successor (checked generation + 1, parent = BLAKE3 digest of the WHOLE original
//! plaintext), it holds exactly one added entry with the expected ID/version, and every other
//! entry keeps its ID, version and payload byte for byte, in the same relative order. This is
//! DATA grammar only. It is not a mounted capability, an admitted write, a selected catalog or
//! any source/Current/governance authority; each concrete handler still validates the added
//! payload itself and owns the physical declaration protocol.
use super::{catalog_envelope as outer, refused, MAX_ENTRIES};
use crate::AppError;
use catcoms_wire::Encoder;
use zeroize::Zeroizing;

/// Fixed outer framing of one entry: u32 id, u32 version, u32 payload length.
const ENTRY_OVERHEAD: usize = 12;

/// Build the exact successor DATA with one new `(id, version, payload)` entry at its sorted
/// position. Refuses a legacy/empty original, an ID that is already present (whatever its
/// version), a full entry table, generation overflow and any candidate above the envelope
/// ceiling, all BEFORE growing the output encoder. The caller funds the original input, this
/// candidate buffer and every later sealing overlap independently; there is no payer here.
pub(super) fn insert_entry_data(
    original: &[u8],
    domain: &[u8; 32],
    id: u32,
    version: u32,
    payload: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if id == 0 || version == 0 {
        return Err(refused());
    }
    let old = outer::parse(original, domain, outer::MAX_ENVELOPE_PLAIN)?;
    if old.format != outer::Format::Namespaces || old.count >= MAX_ENTRIES {
        return Err(refused());
    }
    for index in 0..old.count {
        if old.entry(index)?.id == id {
            return Err(refused());
        }
    }
    let generation = old.generation.checked_add(1).ok_or_else(refused)?;
    let count = u32::try_from(old.count + 1).map_err(|_| refused())?;
    // The original's own framing already counts its entries; only the new entry is added.
    let length = original
        .len()
        .checked_add(ENTRY_OVERHEAD)
        .and_then(|n| n.checked_add(payload.len()))
        .filter(|n| *n <= outer::MAX_ENVELOPE_PLAIN)
        .ok_or_else(refused)?;
    let mut e = Encoder::with_capacity(length);
    e.put_str(outer::TAG_V2).map_err(|_| refused())?;
    e.put_bytes(domain).map_err(|_| refused())?;
    e.put_u64(generation);
    // WHOLE prior plaintext digest, never a ciphertext/frame identity.
    e.put_bytes(&old.digest).map_err(|_| refused())?;
    e.put_u32(count);
    let mut inserted = false;
    for index in 0..old.count {
        let entry = old.entry(index)?;
        if !inserted && entry.id > id {
            e.put_u32(id).put_u32(version);
            e.put_bytes(payload).map_err(|_| refused())?;
            inserted = true;
        }
        e.put_u32(entry.id).put_u32(entry.version);
        e.put_bytes(entry.payload).map_err(|_| refused())?;
    }
    if !inserted {
        e.put_u32(id).put_u32(version);
        e.put_bytes(payload).map_err(|_| refused())?;
    }
    if e.len() != length {
        return Err(refused());
    }
    let candidate = Zeroizing::new(e.finish());
    // Re-validate the produced bytes through the same pair checker every reader uses, and
    // make sure the inserted payload is exactly the one requested.
    if validate_insert_pair_data(original, &candidate, domain, id, version)? != payload {
        return Err(refused());
    }
    Ok(candidate)
}

/// Validate one exact original -> candidate pair with exactly one added `(id, version)` entry
/// and return the added payload (borrowed from `candidate`) for the handler's own descriptor
/// check. Every other entry must match the original's ID, version and payload byte for byte
/// in the same order. A complete MAC, a matching pair or a decodable payload is not an
/// admitted write, mounted capability or Source result.
pub(super) fn validate_insert_pair_data<'a>(
    original: &[u8],
    candidate: &'a [u8],
    domain: &[u8; 32],
    id: u32,
    version: u32,
) -> Result<&'a [u8], AppError> {
    let old = outer::parse(original, domain, outer::MAX_ENVELOPE_PLAIN)?;
    let next = outer::parse(candidate, domain, outer::MAX_ENVELOPE_PLAIN)?;
    if old.format != outer::Format::Namespaces
        || next.format != outer::Format::Namespaces
        || old.generation.checked_add(1) != Some(next.generation)
        || next.parent != Some(old.digest)
        || old.count.checked_add(1) != Some(next.count)
    {
        return Err(refused());
    }
    let mut added = None;
    let mut prior = 0;
    for index in 0..next.count {
        let after = next.entry(index)?;
        if after.id == id {
            // The outer parser's strict order already forbids two entries with this ID.
            if added.is_some() || after.version != version {
                return Err(refused());
            }
            added = Some(after.payload);
            continue;
        }
        let before = old.entry(prior)?;
        if before.id != after.id
            || before.version != after.version
            || before.payload != after.payload
        {
            return Err(refused());
        }
        prior += 1;
    }
    if prior != old.count {
        return Err(refused());
    }
    added.ok_or_else(refused)
}

#[cfg(test)]
mod tests;
