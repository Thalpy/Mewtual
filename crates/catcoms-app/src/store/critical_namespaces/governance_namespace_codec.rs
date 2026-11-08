//! Immutable critical namespace4/version1 descriptor and exact declaration DATA.
//!
//! The separate concrete private Current4 metadata inspector consumes this codec.
//! Every older opener still refuses4 before settlement or cleanup. This paired
//! physical handler grants no Source4/Scope/Current owner or governance request.
//! DATA here is never Saved, SDK/key, source, publication or disposal authority.

use super::{archive_current_namespace_codec as current, catalog_envelope as outer};
use super::{pending_join_codec as prior, refused, MAX_ENTRIES};
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(super) const NAMESPACE_ID: u32 = 4;
pub(super) const NAMESPACE_VERSION: u32 = 1;
const TAG: &[u8] = b"catcoms/private-chat-v2-governance-layout/v1\0";
const VERSION: u16 = 1;
const DESCRIPTOR_BYTES: usize = 4 + TAG.len() + 2 + 3 * 36;
const ENTRY_OVERHEAD: usize = 12;

/// Immutable capability labels only. Request inventory belongs in a separate, future
/// journal and cannot mutate this descriptor or normalize any preceding namespace.
#[derive(Clone, Copy, PartialEq, Eq)]
struct GovernanceLayoutData {
    vault: [u8; 32],
    root: [u8; 32],
    current_descriptor: [u8; 32],
}

fn field32(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    d.get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())
}

/// This is the canonical ID3 DESCRIPTOR commitment, not a whole catalog plaintext
/// digest or exact ciphertext CAS identity. All three domains must remain distinct.
fn current_descriptor_digest(raw: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"catcoms/private-chat-v2-governance-current-descriptor/v1\0");
    h.update(&(raw.len() as u64).to_be_bytes());
    h.update(raw);
    *h.finalize().as_bytes()
}

impl GovernanceLayoutData {
    fn encode_data(self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(DESCRIPTOR_BYTES);
        e.put_bytes(TAG).map_err(|_| refused())?;
        e.put_u16(VERSION);
        for field in [self.vault, self.root, self.current_descriptor] {
            e.put_bytes(&field).map_err(|_| refused())?;
        }
        if e.len() != DESCRIPTOR_BYTES {
            return Err(refused());
        }
        Ok(Zeroizing::new(e.finish()))
    }

    fn decode_data(raw: &[u8], expected: Self) -> Result<Self, AppError> {
        if raw.len() != DESCRIPTOR_BYTES {
            return Err(refused());
        }
        let mut d = Decoder::new(raw);
        if d.get_bytes().map_err(|_| refused())? != TAG
            || d.get_u16().map_err(|_| refused())? != VERSION
        {
            return Err(refused());
        }
        let value = Self {
            vault: field32(&mut d)?,
            root: field32(&mut d)?,
            current_descriptor: field32(&mut d)?,
        };
        d.finish().map_err(|_| refused())?;
        if value != expected {
            return Err(refused());
        }
        Ok(value)
    }
}

fn original_data<'a>(
    raw: &'a [u8],
    domain: &[u8; 32],
) -> Result<(outer::Envelope<'a>, GovernanceLayoutData), AppError> {
    // This is DATA grammar validation, not selection of a runtime JOIN executor.
    // The existing complete parser accepts only its actual known JOINv1/v2, ID2,
    // and ID3. No namespace3 parser/descriptor is widened or relaxed here.
    let parsed = prior::parse_current_layout(raw, domain, true)?;
    let layout = parsed.current_layout.ok_or_else(refused)?;
    let archive = parsed.archive.as_ref().ok_or_else(refused)?;
    if layout.vault != *domain || archive.vault != layout.vault || archive.root != layout.root {
        return Err(refused());
    }
    let envelope = outer::parse(raw, domain, outer::MAX_ENVELOPE_PLAIN)?;
    if envelope.format != outer::Format::Namespaces || envelope.count >= MAX_ENTRIES {
        return Err(refused());
    }
    let mut descriptor = None;
    for index in 0..envelope.count {
        let entry = envelope.entry(index)?;
        if entry.id == current::NAMESPACE_ID {
            if entry.version != current::NAMESPACE_VERSION {
                return Err(refused());
            }
            current::CurrentLayoutData::decode_data(entry.payload, layout)?;
            descriptor = Some(entry.payload);
        }
    }
    Ok((
        envelope,
        GovernanceLayoutData {
            vault: layout.vault,
            root: layout.root,
            current_descriptor: current_descriptor_digest(descriptor.ok_or_else(refused)?),
        },
    ))
}

fn candidate_len(original: usize) -> Result<usize, AppError> {
    original
        .checked_add(ENTRY_OVERHEAD)
        .and_then(|n| n.checked_add(DESCRIPTOR_BYTES))
        .filter(|n| *n <= outer::MAX_ENVELOPE_PLAIN)
        .ok_or_else(refused)
}

/// Produce a complete catalog successor DATA candidate, with exactly one new4.
/// Caller must fund the original input, candidate buffer, parser metadata and later
/// sealing/ciphertext overlap independently BEFORE calling. There is no payer here.
/// A future declaration seed must retain BOTH whole original and candidate frames;
/// after promotion it may not synthesize missing original bytes from a Summary.
pub(super) fn prepare_declaration_data(
    original: &[u8],
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let (old, layout) = original_data(original, domain)?;
    let generation = old.generation.checked_add(1).ok_or_else(refused)?;
    let count = old
        .count
        .checked_add(1)
        .filter(|n| *n <= MAX_ENTRIES)
        .ok_or_else(refused)?;
    let length = candidate_len(original.len())?;
    // Overflow/whole-envelope/namespace checks finish before growing any encoder.
    let descriptor = layout.encode_data()?;
    let mut e = Encoder::with_capacity(length);
    e.put_str(outer::TAG_V2).map_err(|_| refused())?;
    e.put_bytes(domain).map_err(|_| refused())?;
    e.put_u64(generation);
    // WHOLE prior plaintext digest, never its ciphertext/frame identity.
    e.put_bytes(&old.digest).map_err(|_| refused())?;
    e.put_u32(u32::try_from(count).map_err(|_| refused())?);
    for index in 0..old.count {
        let entry = old.entry(index)?;
        e.put_u32(entry.id).put_u32(entry.version);
        e.put_bytes(entry.payload).map_err(|_| refused())?;
    }
    e.put_u32(NAMESPACE_ID).put_u32(NAMESPACE_VERSION);
    e.put_bytes(&descriptor).map_err(|_| refused())?;
    if e.len() != length {
        return Err(refused());
    }
    let candidate = Zeroizing::new(e.finish());
    validate_declaration_pair_data(original, &candidate, domain)?;
    Ok(candidate)
}

/// Validate one exact original→declaration DATA pair. Neither a complete MAC nor
/// this pair comparison is an admitted write, mounted capability or Source result.
pub(super) fn validate_declaration_pair_data(
    original: &[u8],
    candidate: &[u8],
    domain: &[u8; 32],
) -> Result<(), AppError> {
    let (old, expected) = original_data(original, domain)?;
    let next = outer::parse(candidate, domain, outer::MAX_ENVELOPE_PLAIN)?;
    if next.format != outer::Format::Namespaces
        || old.generation.checked_add(1) != Some(next.generation)
        || next.parent != Some(old.digest)
        || old.count.checked_add(1) != Some(next.count)
    {
        return Err(refused());
    }
    for index in 0..old.count {
        let before = old.entry(index)?;
        let after = next.entry(index)?;
        if before.id != after.id
            || before.version != after.version
            || before.payload != after.payload
        {
            return Err(refused());
        }
    }
    let added = next.entry(old.count)?;
    if added.id != NAMESPACE_ID || added.version != NAMESPACE_VERSION {
        return Err(refused());
    }
    GovernanceLayoutData::decode_data(added.payload, expected)?;
    Ok(())
}

pub(in crate::store) mod capability;

#[cfg(test)]
mod tests;
