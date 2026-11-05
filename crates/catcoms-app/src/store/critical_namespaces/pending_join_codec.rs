//! Closed borrowed Pending-only JOIN1 grammar used by the explicit configured opener/producer.
//! The ordinary Store opener still rejects nonempty catalogs; this codec alone grants no handler.
//! Parsing returns inert fixed metadata only, never admission, Fresh or an active worker lease.
use super::*;
use crate::store::chat_admission::journal_basis::{JournalBasis, MAX_ENCODED_BASIS};

pub(super) use super::catalog_envelope::{catalog_digest, TAG_V2};
pub(super) const JOIN_TAG: &str = "catcoms/destination-join-journal/v1";
pub(super) const IMMUTABLE_TAG: &str = "catcoms/destination-join-source-binding/v1";
pub(super) const JOIN_ID: u32 = 1;
pub(super) const JOIN_VERSION: u32 = 1;
pub(super) const MAX_ROWS: usize = 5;
pub(super) const MAX_JOIN_PAYLOAD: usize = 16 * 1024 * 1024;
pub(super) const MAX_CATALOG_PLAIN: usize = super::catalog_envelope::MAX_ENVELOPE_PLAIN;
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
    // Version1 source rows have no execution record. Parsed version2 DATA is separate from
    // immutable source identity and never provides a resource grant/submission ticket by itself.
    pub(super) execution: Option<super::pending_join::execution_state::State>,
}

pub(super) struct Row<'a> {
    pub(super) metadata: Metadata,
    pub(super) immutable: &'a [u8],
    pub(super) encrypted: &'a [u8],
    pub(super) execution_input: Option<super::pending_join::execution_state::Input<'a>>,
}
pub(super) struct Parsed<'a> {
    pub(super) generation: u64,
    pub(super) parent: Option<[u8; 32]>,
    pub(super) digest: [u8; 32],
    pub(super) rows: [Option<Row<'a>>; MAX_ROWS],
    pub(super) count: usize,
    // Only the concrete combined reader can populate these. The strict Pending-only parser
    // still rejects ID2. Preserve the complete original bytes when a JOIN writer re-encodes
    // the surrounding catalog; a shared outer hash does not permit dropping another namespace.
    pub(super) archive: Option<super::archive_bootstrap::Descriptor>,
    pub(super) archive_payload: Option<&'a [u8]>,
    pub(super) join_version: Option<u32>,
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
    pub(super) archive: Option<super::archive_bootstrap::Descriptor>,
    pub(super) join_version: Option<u32>,
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
            archive: self.archive.clone(),
            join_version: self.join_version,
        }
    }
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
    parse_known(plain, domain, SupportedLayout::PendingOnly)
}

/// Concrete known-layout DATA parsing for the archive constructor. This function is not a
/// support switch: its consumer must inspect the actual seed/root before any old gate or I/O.
pub(super) fn parse_bootstrap<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
) -> Result<Parsed<'a>, AppError> {
    parse_known(plain, domain, SupportedLayout::ArchiveBootstrap)
}

enum SupportedLayout {
    PendingOnly,
    ArchiveBootstrap,
    Executing,
    ArchiveBootstrapExecuting,
}

pub(super) fn parse_executing<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
) -> Result<Parsed<'a>, AppError> {
    parse_known(plain, domain, SupportedLayout::Executing)
}
pub(super) fn parse_bootstrap_executing<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
) -> Result<Parsed<'a>, AppError> {
    parse_known(plain, domain, SupportedLayout::ArchiveBootstrapExecuting)
}

fn parse_known<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
    layout: SupportedLayout,
) -> Result<Parsed<'a>, AppError> {
    use super::catalog_envelope::{self, Format};
    let envelope = catalog_envelope::parse(plain, domain, MAX_CATALOG_PLAIN)?;
    if envelope.format == Format::LegacyEmpty {
        return Ok(Parsed {
            generation: envelope.generation,
            parent: envelope.parent,
            digest: envelope.digest,
            rows: std::array::from_fn(|_| None),
            count: 0,
            archive: None,
            archive_payload: None,
            join_version: None,
        });
    }
    let mut rows = std::array::from_fn(|_| None);
    let mut count = 0;
    let mut archive = None;
    let mut archive_payload = None;
    let mut join_version = None;
    for index in 0..envelope.count {
        let entry = envelope.entry(index)?;
        match entry.id {
            JOIN_ID => {
                (rows, count) = match (&layout, entry.version) {
                    (
                        SupportedLayout::Executing | SupportedLayout::ArchiveBootstrapExecuting,
                        super::pending_join::execution_payload::VERSION,
                    ) => super::pending_join::execution_payload::parse(entry.payload, domain)?,
                    _ => parse_join_entry(entry, domain)?,
                };
                join_version = Some(entry.version);
            }
            super::archive_bootstrap::ARCHIVE_ID
                if matches!(
                    layout,
                    SupportedLayout::ArchiveBootstrap | SupportedLayout::ArchiveBootstrapExecuting
                ) && entry.version == super::archive_bootstrap::ARCHIVE_VERSION =>
            {
                archive = Some(super::archive_bootstrap::Descriptor::parse(
                    entry.payload,
                    domain,
                )?);
                archive_payload = Some(entry.payload);
            }
            _ => return Err(refused()),
        }
    }
    Ok(Parsed {
        generation: envelope.generation,
        parent: envelope.parent,
        digest: envelope.digest,
        rows,
        count,
        archive,
        archive_payload,
        join_version,
    })
}

/// JOIN payload support stays with the actual JOIN consumer. Future execution schemas must
/// add their concrete policy/producer here; a known outer ID is never enough to admit them.
pub(super) fn parse_join_entry<'a>(
    entry: super::catalog_envelope::Entry<'a>,
    domain: &[u8; 32],
) -> Result<([Option<Row<'a>>; MAX_ROWS], usize), AppError> {
    if entry.id != JOIN_ID
        || entry.version != JOIN_VERSION
        || entry.payload.len() > MAX_JOIN_PAYLOAD
    {
        return Err(refused());
    }
    parse_rows(entry.payload, domain)
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
        let row = parse_original_row(input.get_bytes().map_err(|_| refused())?, domain)?;
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

pub(super) fn parse_original_row<'a>(
    bytes: &'a [u8],
    domain: &[u8; 32],
) -> Result<Row<'a>, AppError> {
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
            execution: None,
        },
        immutable,
        encrypted,
        execution_input: None,
    })
}

/// Exact original source-row framing shared by descriptor-v1 and the nested source in v2.
/// This never emits an execution grant; execution DATA remains a separate frame.
pub(super) fn original_row_len(immutable: &[u8], encrypted: &[u8]) -> Result<usize, AppError> {
    if immutable.len() > 2048 || encrypted.len() < 40 || encrypted.len() > MAX_SOURCE {
        return Err(refused());
    }
    (36 + 8 + 1 + 1 + 4 + 4usize)
        .checked_add(immutable.len())
        .and_then(|length| length.checked_add(encrypted.len()))
        .ok_or_else(refused)
}

/// Writes the framed original source directly into prepaid whole-catalog output. The actual
/// producer validates its complete candidate before admission; hashes are negative identity
/// checks, never membership/root/Current authority or a substitute for opening the source.
pub(super) fn encode_original_row(
    output: &mut Encoder,
    metadata: &Metadata,
    immutable: &[u8],
    encrypted: &[u8],
) -> Result<(), AppError> {
    let length = original_row_len(immutable, encrypted)?;
    if metadata.revision == 0
        || metadata.binding != binding_digest(immutable)
        || metadata.ciphertext != *blake3::hash(encrypted).as_bytes()
    {
        return Err(refused());
    }
    output.put_u32(length as u32);
    output.put_bytes(&metadata.source).map_err(|_| refused())?;
    output.put_u64(metadata.revision);
    output.put_u8(match metadata.scheduling {
        Scheduling::Active => 0,
        Scheduling::Paused => 1,
    });
    output.put_u8(1);
    output.put_bytes(immutable).map_err(|_| refused())?;
    output.put_bytes(encrypted).map_err(|_| refused())?;
    Ok(())
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
    match (&current.archive, &next.archive) {
        // The only namespace-set change is a concrete archive declaration over the exact
        // original JOIN contents. Ordinary JOIN updates below preserve the immutable layout.
        (None, Some(_)) => {
            return if current.count == next.count
                && current.rows == next.rows
                && current.join_version == next.join_version
            {
                Ok(())
            } else {
                Err(refused())
            };
        }
        (old, new) if old == new => {}
        _ => return Err(refused()),
    }
    join_payload_successor(current, next)
}

/// Actual JOIN payload transition, kept separate from namespace-set/whole-parent validation.
/// A future execution consumer owns its concrete version/state transition here; this v1
/// implementation permits no downgrade, arm/grant migration or unknown payload version.
fn join_payload_successor(current: &Summary, next: &Summary) -> Result<(), AppError> {
    use super::pending_join::execution_payload;
    if current.join_version == Some(JOIN_VERSION)
        && next.join_version == Some(execution_payload::VERSION)
    {
        return execution_payload::migration(current, next);
    }
    if current.join_version == Some(execution_payload::VERSION)
        && next.join_version == Some(execution_payload::VERSION)
    {
        return execution_payload::successor(current, next);
    }
    if next.join_version != Some(JOIN_VERSION)
        || current
            .join_version
            .is_some_and(|version| version != JOIN_VERSION)
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
        if old.execution.is_some() || new.execution.is_some() {
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
        if added.revision != 1
            || added.scheduling != Scheduling::Active
            || added.execution.is_some()
        {
            return Err(refused());
        }
    }
    Ok(())
}
