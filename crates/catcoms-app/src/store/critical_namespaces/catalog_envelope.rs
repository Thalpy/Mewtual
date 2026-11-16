//! Strict borrowed OUTER catalog DATA. Parsing a descriptor never enables its handler or
//! authorizes cleanup, admission or a positive source. Each concrete reader must reject every
//! namespace/version it cannot fully inspect and preserve both complete source candidates.
use super::{refused, validate_empty, MAX_ENTRIES, TAG};
use crate::AppError;
use catcoms_wire::Decoder;

pub(super) const TAG_V2: &str = "catcoms/critical-vault-namespaces/v2";
/// Shared hard framing ceiling: existing JOIN16MiB, reserved ARCHIVE4KiB, outer header512.
/// A concrete policy can impose a smaller limit; decoded metadata never enlarges this ceiling.
pub(super) const MAX_ENVELOPE_PLAIN: usize = 16 * 1024 * 1024 + 4 * 1024 + 512;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Format {
    LegacyEmpty,
    Namespaces,
}

/// Untrusted namespace declaration. Payload is borrowed, never copied or treated as a path,
/// mounted source, supported handler or resource lease. Unknown IDs/versions remain DATA here.
#[derive(Clone, Copy)]
pub(super) struct Entry<'a> {
    pub(super) id: u32,
    pub(super) version: u32,
    pub(super) payload: &'a [u8],
}
impl std::fmt::Debug for Entry<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogEntryData")
            .field("id", &self.id)
            .field("version", &self.version)
            .field("payload_bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}

pub(super) struct Envelope<'a> {
    pub(super) format: Format,
    pub(super) generation: u64,
    pub(super) parent: Option<[u8; 32]>,
    pub(super) digest: [u8; 32],
    entries: [Option<Entry<'a>>; MAX_ENTRIES],
    pub(super) count: usize,
}
impl std::fmt::Debug for Envelope<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogEnvelopeData")
            .field("generation", &self.generation)
            .field("entries", &self.count)
            .finish_non_exhaustive()
    }
}
impl<'a> Envelope<'a> {
    pub(super) fn entry(&self, index: usize) -> Result<Entry<'a>, AppError> {
        if index >= self.count {
            return Err(refused());
        }
        self.entries[index].ok_or_else(refused)
    }
}

/// Additional fixed stack layout while a concrete reader builds its own classified metadata.
/// The configured reader pays this before raw allocation alongside its other full inventories.
pub(super) const OUTER_METADATA_BYTES: usize = std::mem::size_of::<Envelope<'static>>();

pub(super) fn catalog_digest(plain: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/critical-catalog-plaintext/v1\0");
    hash.update(plain);
    *hash.finalize().as_bytes()
}

/// Complete outer syntax/EOF precedes return or concrete support decisions. ALL variable fields
/// borrow input and the entry array has fixed16 slots. Domain, parent size, generation, positive
/// strictly sorted IDs and nonzero versions are checked before any payload-specific parser.
pub(super) fn parse<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
    policy_limit: usize,
) -> Result<Envelope<'a>, AppError> {
    if policy_limit == 0 || policy_limit > MAX_ENVELOPE_PLAIN || plain.len() > policy_limit {
        return Err(refused());
    }
    let mut input = Decoder::new(plain);
    let tag = input.get_str().map_err(|_| refused())?;
    if tag == TAG {
        // The existing legacy frame is EXACT generation1/count0, with no parent field. Never
        // reinterpret a nonempty legacy candidate as an Empty/new-format upgrade permission.
        validate_empty(plain, domain)?;
        return Ok(Envelope {
            format: Format::LegacyEmpty,
            generation: 1,
            parent: None,
            digest: catalog_digest(plain),
            entries: [None; MAX_ENTRIES],
            count: 0,
        });
    }
    if tag != TAG_V2 || input.get_bytes().map_err(|_| refused())? != domain {
        return Err(refused());
    }
    let generation = input.get_u64().map_err(|_| refused())?;
    let parent = input
        .get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())?;
    let count = input.get_u32().map_err(|_| refused())? as usize;
    if generation < 2 || count == 0 || count > MAX_ENTRIES || count > input.remaining() / 12 {
        return Err(refused());
    }
    let mut entries = [None; MAX_ENTRIES];
    let mut previous = 0;
    for slot in entries.iter_mut().take(count) {
        let id = input.get_u32().map_err(|_| refused())?;
        let version = input.get_u32().map_err(|_| refused())?;
        let payload = input.get_bytes().map_err(|_| refused())?;
        if id <= previous || version == 0 {
            return Err(refused());
        }
        previous = id;
        *slot = Some(Entry {
            id,
            version,
            payload,
        });
    }
    input.finish().map_err(|_| refused())?;
    Ok(Envelope {
        format: Format::Namespaces,
        generation,
        parent: Some(parent),
        digest: catalog_digest(plain),
        entries,
        count,
    })
}

#[cfg(test)]
mod tests;
