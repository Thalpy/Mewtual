//! Exact whole-catalog 7/v2 -> 7/v3 DATA successor. The migration changes only
//! the native entry, never the JOIN ledger or any other namespace's payload.
use super::super::super::{catalog_envelope as outer, pending_join_codec as grammar, refused};
use super::super::{capability::NativeCapabilityData, operational::capability as v2};
use super::capability::Capability;
use crate::AppError;
use catcoms_wire::Encoder;
use zeroize::Zeroizing;

pub(super) fn upgrade_data(
    original: &[u8],
    domain: &[u8; 32],
    expected: NativeCapabilityData,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let known = grammar::parse_operational_native_registry_layout(original, domain, true)?;
    let actual = NativeCapabilityData::for_current_data(
        known.current_layout.ok_or_else(refused)?,
        known.current_payload.ok_or_else(refused)?,
    )?;
    if actual != expected {
        return Err(refused());
    }
    let old_cap = v2::Capability { original: expected };
    v2::Capability::decode_exact(known.native_registry_payload.ok_or_else(refused)?, old_cap)?;
    let old = outer::parse(original, domain, outer::MAX_ENVELOPE_PLAIN)?;
    if old.format != outer::Format::Namespaces {
        return Err(refused());
    }
    let descriptor = Capability { original: expected }.encode_data()?;
    let old_descriptor = old_cap.encode_data()?;
    let generation = old.generation.checked_add(1).ok_or_else(refused)?;
    let length = original
        .len()
        .checked_sub(old_descriptor.len())
        .and_then(|n| n.checked_add(descriptor.len()))
        .filter(|n| *n <= outer::MAX_ENVELOPE_PLAIN)
        .ok_or_else(refused)?;
    let mut output = Encoder::with_capacity(length);
    output.put_str(outer::TAG_V2).map_err(|_| refused())?;
    output.put_bytes(domain).map_err(|_| refused())?;
    output.put_u64(generation);
    output.put_bytes(&old.digest).map_err(|_| refused())?;
    output.put_u32(u32::try_from(old.count).map_err(|_| refused())?);
    let mut replaced = false;
    for index in 0..old.count {
        let entry = old.entry(index)?;
        if entry.id == 7 {
            if replaced || entry.version != 2 || entry.payload != old_descriptor.as_slice() {
                return Err(refused());
            }
            output
                .put_u32(7)
                .put_u32(super::capability::NAMESPACE_VERSION);
            output.put_bytes(&descriptor).map_err(|_| refused())?;
            replaced = true;
        } else {
            // In particular, retain complete JOIN v1/v2 bytes. Rebuilding rows
            // from a summary would silently drop execution or paused history.
            output.put_u32(entry.id).put_u32(entry.version);
            output.put_bytes(entry.payload).map_err(|_| refused())?;
        }
    }
    if !replaced || output.len() != length {
        return Err(refused());
    }
    let bytes = Zeroizing::new(output.finish());
    grammar::parse_lifecycle_native_registry_layout(&bytes, domain, true)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests;
