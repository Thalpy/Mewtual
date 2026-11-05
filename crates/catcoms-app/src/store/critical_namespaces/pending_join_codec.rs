//! Closed borrowed Pending-only JOIN1 grammar. This module is being connected with the actual
//! configured opener/producer; it must NOT be declared as a supported namespace independently.
//! Parsing returns inert fixed metadata only, never admission, Fresh or an active worker lease.
use super::*;
use crate::store::chat_admission::journal_basis::{JournalBasis, MAX_ENCODED_BASIS};

pub(super) const TAG_V2: &str = "catcoms/critical-vault-namespaces/v2";
pub(super) const JOIN_TAG: &str = "catcoms/destination-join-journal/v1";
pub(super) const IMMUTABLE_TAG: &str = "catcoms/destination-join-source-binding/v1";
pub(super) const JOIN_ID: u32 = 1;
pub(super) const JOIN_VERSION: u32 = 1;
pub(super) const MAX_ROWS: usize = 5;
pub(super) const MAX_JOIN_PAYLOAD: usize = 16 * 1024 * 1024;
pub(super) const MAX_CATALOG_PLAIN: usize = MAX_JOIN_PAYLOAD + 4 * 1024 + 512;
pub(super) const MAX_CATALOG_FRAMED: usize = MAX_CATALOG_PLAIN + 40;
pub(super) const MAX_SOURCE: usize = 640 * 1024 + 40; // nonce24 + ciphertext(P+tag16)

/// No Vec backing is retained for dormant group labels. The complete bounded metadata array
/// has a fixed layout that the configured resource invoice can enumerate before copying it.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Group {
    bytes: [u8; 256],
    length: u16,
}
impl Group {
    pub(super) fn from_slice(bytes: &[u8]) -> Result<Self, AppError> {
        if bytes.is_empty() || bytes.len() > 256 {
            return Err(refused());
        }
        let mut group = Self {
            bytes: [0; 256],
            length: bytes.len() as u16,
        };
        group.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(group)
    }
    pub(super) fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.length as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Scheduling {
    Active,
    Paused,
}

/// DATA labels. `binding` covers the complete canonical immutable frame including explicit
/// protocol/root kind and original optional admission basis. Cipher digest is negative equality
/// metadata only; a real reader still authenticates and opens the exact original source.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Metadata {
    pub(super) source: [u8; 32],
    pub(super) revision: u64,
    pub(super) scheduling: Scheduling,
    pub(super) group: Group,
    pub(super) signer: [u8; 32],
    pub(super) profile: [u8; 32],
    pub(super) founder: [u8; 32],
    pub(super) invite_nonce: [u8; 16],
    pub(super) invite: [u8; 32],
    pub(super) request: [u8; 32],
    pub(super) package: [u8; 32],
    pub(super) basis: JournalBasis,
    pub(super) binding: [u8; 32],
    pub(super) ciphertext: [u8; 32],
}

pub(super) struct Row<'a> {
    pub(super) metadata: Metadata,
    pub(super) immutable: &'a [u8],
    pub(super) encrypted: &'a [u8],
}
pub(super) struct Parsed<'a> {
    pub(super) generation: u64,
    pub(super) parent: Option<[u8; 32]>,
    pub(super) digest: [u8; 32],
    pub(super) rows: [Option<Row<'a>>; MAX_ROWS],
    pub(super) count: usize,
}

/// Fixed, inert classification retained after dropping a complete plaintext. Digest equality
/// is negative cryptographic data equality, not a file/root barrier or publication witness.
#[derive(Clone)]
pub(in crate::store) struct Summary {
    pub(super) generation: u64,
    pub(super) parent: Option<[u8; 32]>,
    pub(super) digest: [u8; 32],
    pub(in crate::store) framed: [u8; 32],
    pub(super) rows: [Option<Metadata>; MAX_ROWS],
    pub(super) count: usize,
}
impl Parsed<'_> {
    pub(super) fn summary(&self, framed: [u8; 32]) -> Summary {
        Summary {
            generation: self.generation,
            parent: self.parent,
            digest: self.digest,
            framed,
            rows: std::array::from_fn(|index| {
                self.rows[index].as_ref().map(|row| row.metadata.clone())
            }),
            count: self.count,
        }
    }
}

pub(super) fn catalog_digest(plain: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/critical-catalog-plaintext/v1\0");
    hash.update(plain);
    *hash.finalize().as_bytes()
}
pub(super) fn binding_digest(immutable: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/destination-join-source/v1\0");
    hash.update(immutable);
    *hash.finalize().as_bytes()
}

/// Scan the complete outer candidate before deciding supported IDs. An unknown descriptor,
/// including reserved ARCHIVE2, is never skipped/truncated into a known JOIN-only history.
pub(super) fn parse<'a>(plain: &'a [u8], domain: &[u8; 32]) -> Result<Parsed<'a>, AppError> {
    if plain.len() > MAX_CATALOG_PLAIN {
        return Err(refused());
    }
    let mut input = Decoder::new(plain);
    let tag = input.get_str().map_err(|_| refused())?;
    if tag == TAG {
        validate_empty(plain, domain)?;
        return Ok(Parsed {
            generation: 1,
            parent: None,
            digest: catalog_digest(plain),
            rows: std::array::from_fn(|_| None),
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
    if count > MAX_ENTRIES || count > input.remaining() / 12 {
        return Err(refused());
    }
    let mut previous = 0;
    let mut join = None;
    let mut unsupported = false;
    for _ in 0..count {
        let id = input.get_u32().map_err(|_| refused())?;
        let version = input.get_u32().map_err(|_| refused())?;
        let payload = input.get_bytes().map_err(|_| refused())?;
        if id <= previous || version == 0 {
            return Err(refused());
        }
        previous = id;
        if id == JOIN_ID && version == JOIN_VERSION && payload.len() <= MAX_JOIN_PAYLOAD {
            join = Some(payload);
        } else {
            unsupported = true;
        }
    }
    input.finish().map_err(|_| refused())?;
    if generation < 2 || count != 1 || unsupported {
        return Err(refused());
    }
    let (rows, count) = parse_rows(join.ok_or_else(refused)?, domain)?;
    Ok(Parsed {
        generation,
        parent: Some(parent),
        digest: catalog_digest(plain),
        rows,
        count,
    })
}

fn parse_rows<'a>(
    bytes: &'a [u8],
    domain: &[u8; 32],
) -> Result<([Option<Row<'a>>; MAX_ROWS], usize), AppError> {
    let mut input = Decoder::new(bytes);
    if input.get_str().map_err(|_| refused())? != JOIN_TAG {
        return Err(refused());
    }
    let count = input.get_u32().map_err(|_| refused())? as usize;
    if count == 0 || count > MAX_ROWS || count > input.remaining() / 4 {
        return Err(refused());
    }
    let mut rows: [Option<Row<'a>>; MAX_ROWS] = std::array::from_fn(|_| None);
    let mut active = 0;
    for index in 0..count {
        let row = parse_row(input.get_bytes().map_err(|_| refused())?, domain)?;
        if row.metadata.scheduling == Scheduling::Active {
            active += 1;
        }
        if active > 1
            || rows[..index].iter().flatten().any(|prior| {
                prior.metadata.source >= row.metadata.source
                    || (prior.metadata.group == row.metadata.group
                        && (prior.metadata.signer == row.metadata.signer
                            || prior.metadata.invite_nonce == row.metadata.invite_nonce))
            })
        {
            return Err(refused());
        }
        rows[index] = Some(row);
    }
    input.finish().map_err(|_| refused())?;
    // FIVE retained rows total and at most one Active. All five may be Paused: otherwise a full
    // pool could neither pause its unavailable Active row nor resume another without exceeding
    // one Active. Pausing then resuming is two durable steps and never admits a sixth row.
    Ok((rows, count))
}

fn array<const N: usize>(input: &mut Decoder<'_>) -> Result<[u8; N], AppError> {
    input
        .get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())
}

fn parse_row<'a>(bytes: &'a [u8], domain: &[u8; 32]) -> Result<Row<'a>, AppError> {
    let mut input = Decoder::new(bytes);
    let source = array(&mut input)?;
    let revision = input.get_u64().map_err(|_| refused())?;
    let scheduling = match input.get_u8().map_err(|_| refused())? {
        0 => Scheduling::Active,
        1 => Scheduling::Paused,
        _ => return Err(refused()),
    };
    // Progress and scheduling are independent. This first handler supports ONLY Pending=1;
    // later ValidatedReply/Intent/Frozen/Settled records refuse before any old cleaner.
    if input.get_u8().map_err(|_| refused())? != 1 {
        return Err(refused());
    }
    let immutable = input.get_bytes().map_err(|_| refused())?;
    let encrypted = input.get_bytes().map_err(|_| refused())?;
    input.finish().map_err(|_| refused())?;
    if revision == 0
        || immutable.len() > 2048
        || encrypted.len() < 40
        || encrypted.len() > MAX_SOURCE
    {
        return Err(refused());
    }
    let mut original = Decoder::new(immutable);
    if original.get_str().map_err(|_| refused())? != IMMUTABLE_TAG
        || array::<32>(&mut original)? != *domain
        || array::<32>(&mut original)? != source
        || original.get_u8().map_err(|_| refused())? != 1 // direct checkpoint protocol
        || original.get_u8().map_err(|_| refused())? != 1
    // finite OwnerFact v1 root kind
    {
        return Err(refused());
    }
    let group = Group::from_slice(original.get_bytes().map_err(|_| refused())?)?;
    let signer = array(&mut original)?;
    let profile = array(&mut original)?;
    let founder = array(&mut original)?;
    let invite_nonce = array(&mut original)?;
    let invite = array(&mut original)?;
    let request = array(&mut original)?;
    let package = array(&mut original)?;
    let basis = original.get_bytes().map_err(|_| refused())?;
    if basis.len() > MAX_ENCODED_BASIS {
        return Err(refused());
    }
    let basis = JournalBasis::decode(basis)?;
    original.finish().map_err(|_| refused())?;
    Ok(Row {
        metadata: Metadata {
            source,
            revision,
            scheduling,
            group,
            signer,
            profile,
            founder,
            invite_nonce,
            invite,
            request,
            package,
            basis,
            binding: binding_digest(immutable),
            ciphertext: *blake3::hash(encrypted).as_bytes(),
        },
        immutable,
        encrypted,
    })
}

/// Validate a COMPLETE known successor, not merely a matching JOIN subpayload. Old rows keep
/// exact immutable/ciphertext custody; one original add or one scheduling toggle is permitted.
/// Duplicate same-plaintext files are handled separately by actual source classification.
pub(super) fn successor(current: &Summary, next: &Summary) -> Result<(), AppError> {
    if current.generation.checked_add(1) != Some(next.generation)
        || next.parent != Some(current.digest)
        || next.count < current.count
        || next.count > current.count + 1
    {
        return Err(refused());
    }
    let mut toggles = 0;
    for old in current.rows.iter().flatten() {
        let new = next
            .rows
            .iter()
            .flatten()
            .find(|new| new.source == old.source)
            .ok_or_else(refused)?;
        // These two hashes cover EVERY immutable and encrypted byte in the fully closed parsed
        // row. Keeping fixed metadata lets the actual reader drop current plaintext before
        // allocating/decrypting stage; no unknown field can be silently omitted by this codec.
        if new.binding != old.binding || new.ciphertext != old.ciphertext {
            return Err(refused());
        }
        if new.revision != old.revision || new.scheduling != old.scheduling {
            if old.revision.checked_add(1) != Some(new.revision) || old.scheduling == new.scheduling
            {
                return Err(refused());
            }
            toggles += 1;
        }
    }
    if next.count == current.count {
        if toggles != 1 {
            return Err(refused());
        }
    } else {
        if toggles != 0 {
            return Err(refused());
        }
        let added = next
            .rows
            .iter()
            .flatten()
            .find(|new| {
                !current
                    .rows
                    .iter()
                    .flatten()
                    .any(|old| old.source == new.source)
            })
            .ok_or_else(refused)?;
        if added.revision != 1 || added.scheduling != Scheduling::Active {
            return Err(refused());
        }
    }
    Ok(())
}
