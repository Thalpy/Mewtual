//! Closed borrowed Pending-only JOIN1 grammar used by the explicit configured opener/producer.
//! The ordinary Store opener still rejects nonempty catalogs; this codec alone grants no handler.
//! Parsing returns inert fixed metadata only, never admission, Fresh or an active worker lease.
use super::*;
use crate::store::chat_admission::journal_basis::{JournalBasis, MAX_ENCODED_BASIS};

#[cfg(test)]
pub(super) use super::catalog_envelope::catalog_digest;
pub(super) use super::catalog_envelope::TAG_V2;
pub(super) const JOIN_TAG: &str = "catcoms/destination-join-journal/v1";
pub(super) const IMMUTABLE_TAG: &str = "catcoms/destination-join-source-binding/v1";
pub(super) const JOIN_ID: u32 = 1;
pub(super) const JOIN_VERSION: u32 = 1;
pub(super) const MAX_ROWS: usize = 5;
pub(super) const MAX_JOIN_PAYLOAD: usize = 16 * 1024 * 1024;
pub(super) const MAX_CATALOG_PLAIN: usize = super::catalog_envelope::MAX_ENVELOPE_PLAIN;
pub(in crate::store) const MAX_CATALOG_FRAMED: usize = MAX_CATALOG_PLAIN + 40;
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
    pub(super) current_layout: Option<super::archive_current_namespace_codec::CurrentLayoutData>,
    pub(super) current_payload: Option<&'a [u8]>,
    pub(in crate::store) governance:
        Option<super::governance_namespace_codec::capability::GovernanceCapabilityData>,
    pub(super) governance_payload: Option<&'a [u8]>,
    // Namespace5 (ordinary-member outbox), populated only when the concrete configured
    // member-outbox handler selected this grammar. Preserved byte for byte by every JOIN
    // writer exactly like ID2/3/4; it is never inferred, normalized or dropped.
    pub(in crate::store) member_outbox:
        Option<super::member_outbox_namespace_codec::MemberOutboxCapabilityData>,
    pub(super) member_outbox_payload: Option<&'a [u8]>,
    pub(super) native_registry_payload: Option<&'a [u8]>,
    // Captured only after the matching closed reader validates this actual
    // entry. JOIN must conserve its version as well as its descriptor bytes;
    // choosing the writer's historical default would relabel index2 as1.
    pub(super) native_registry_version: Option<u32>,
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
    pub(in crate::store) archive: Option<super::archive_bootstrap::Descriptor>,
    pub(super) join_version: Option<u32>,
    pub(in crate::store) current_layout:
        Option<super::archive_current_namespace_codec::CurrentLayoutData>,
    pub(super) current_payload_digest: Option<[u8; 32]>,
    pub(in crate::store) governance:
        Option<super::governance_namespace_codec::capability::GovernanceCapabilityData>,
    pub(super) governance_payload_digest: Option<[u8; 32]>,
    pub(in crate::store) member_outbox:
        Option<super::member_outbox_namespace_codec::MemberOutboxCapabilityData>,
    pub(super) member_outbox_payload_digest: Option<[u8; 32]>,
    pub(super) native_registry_payload_digest: Option<[u8; 32]>,
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
            current_layout: self.current_layout,
            current_payload_digest: self
                .current_payload
                .map(|bytes| *blake3::hash(bytes).as_bytes()),
            governance: self.governance,
            governance_payload_digest: self
                .governance_payload
                .map(|bytes| *blake3::hash(bytes).as_bytes()),
            member_outbox: self.member_outbox,
            member_outbox_payload_digest: self
                .member_outbox_payload
                .map(|bytes| *blake3::hash(bytes).as_bytes()),
            native_registry_payload_digest: self
                .native_registry_payload
                .map(|bytes| *blake3::hash(bytes).as_bytes()),
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
    parse_known(plain, domain, SupportedLayout::PENDING_ONLY)
}

/// Concrete known-layout DATA parsing for the archive constructor. This function is not a
/// support switch: its consumer must inspect the actual seed/root before any old gate or I/O.
pub(super) fn parse_bootstrap<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
) -> Result<Parsed<'a>, AppError> {
    parse_known(plain, domain, SupportedLayout::ARCHIVE_BOOTSTRAP)
}

/// The closed set of namespaces one concrete configured reader can fully inspect. Each flag
/// is set only by the matching actual handler's policy, never by parsed DATA. An entry whose
/// flag is clear refuses the whole catalog before any old cleaner; there is no generic
/// unknown-namespace pass-through.
#[derive(Clone, Copy)]
struct SupportedLayout {
    /// JOIN execution payload version2 is accepted as DATA.
    executing: bool,
    archive: bool,
    current: bool,
    governance: bool,
    member_outbox: bool,
    native_registry: Option<u32>,
}
impl SupportedLayout {
    const PENDING_ONLY: Self = Self {
        executing: false,
        archive: false,
        current: false,
        governance: false,
        member_outbox: false,
        native_registry: None,
    };
    const ARCHIVE_BOOTSTRAP: Self = Self {
        archive: true,
        ..Self::PENDING_ONLY
    };
    const CURRENT_LAYOUT: Self = Self {
        current: true,
        ..Self::ARCHIVE_BOOTSTRAP
    };
    const GOVERNANCE_LAYOUT: Self = Self {
        governance: true,
        ..Self::CURRENT_LAYOUT
    };
    const fn executing(self, executing: bool) -> Self {
        Self { executing, ..self }
    }
    const fn member_outbox(self, member_outbox: bool) -> Self {
        Self {
            member_outbox,
            ..self
        }
    }
    const fn native_registry(self, native_registry: Option<u32>) -> Self {
        Self {
            native_registry,
            ..self
        }
    }
}

/// Closed grammar used only by the private native physical handler. The descriptor
/// is checked against the same Current3 entry; parsing issues no mounted owner.
pub(super) fn parse_native_registry_layout<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
    executing: bool,
    governance: bool,
) -> Result<Parsed<'a>, AppError> {
    let layout = if governance {
        SupportedLayout::GOVERNANCE_LAYOUT
    } else {
        SupportedLayout::CURRENT_LAYOUT
    };
    parse_known(
        plain,
        domain,
        layout
            .executing(executing)
            .member_outbox(true)
            .native_registry(Some(1)),
    )
}

/// The distinct private v2 handler chooses this closed grammar. A v1 handler
/// cannot opt into it, and knowing this parser does not construct either owner.
pub(super) fn parse_operational_native_registry_layout<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
    executing: bool,
) -> Result<Parsed<'a>, AppError> {
    parse_known(
        plain,
        domain,
        SupportedLayout::CURRENT_LAYOUT
            .executing(executing)
            .member_outbox(true)
            .native_registry(Some(2)),
    )
}

/// Explicit version3 DATA grammar for the owning lifecycle upgrade. Existing
/// version1/2 handlers never select it; a descriptor cannot install a handler.
/// Historical JOIN rows remain part of the catalog, including paused rows.
pub(super) fn parse_lifecycle_native_registry_layout<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
    executing: bool,
) -> Result<Parsed<'a>, AppError> {
    parse_known(
        plain,
        domain,
        SupportedLayout::CURRENT_LAYOUT
            .executing(executing)
            .member_outbox(true)
            .native_registry(Some(3)),
    )
}

/// Only the concrete Current-layout physical inspector may select this grammar. The default,
/// Pending and ID2-only inspectors still reject namespace3 in EITHER fixed source before old
/// cleanup. No parsed label or external flag can install this actual constructor policy.
/// Namespace5 still refuses here; only `parse_configured_current_layout` with the concrete
/// member-outbox handler configured accepts it. Production readers all go through
/// `parse_configured_current_layout`; this fixed grammar remains as a regression oracle for
/// "an opener without the member handler refuses ID5".
#[cfg(test)]
pub(super) fn parse_current_layout<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
    executing: bool,
) -> Result<Parsed<'a>, AppError> {
    parse_known(
        plain,
        domain,
        SupportedLayout::CURRENT_LAYOUT.executing(executing),
    )
}

/// The Current3 opener's grammar, widened to namespace5 exactly when the caller's actual
/// policy carries the concrete member-outbox handler (`member_outbox`). Without that handler
/// an ID5 entry refuses like any unknown namespace.
pub(super) fn parse_configured_current_layout<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
    executing: bool,
    member_outbox: bool,
) -> Result<Parsed<'a>, AppError> {
    parse_known(
        plain,
        domain,
        SupportedLayout::CURRENT_LAYOUT
            .executing(executing)
            .member_outbox(member_outbox),
    )
}

pub(super) fn parse_governance_layout<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
    policy: &super::governance_layout::V2GovernanceLayoutPolicy,
) -> Result<Parsed<'a>, AppError> {
    parse_known(
        plain,
        domain,
        SupportedLayout::GOVERNANCE_LAYOUT
            .executing(policy.current.archive.pending.execution.is_some())
            .member_outbox(policy.current.member_outbox.is_some())
            .native_registry(
                policy
                    .current
                    .native_registry
                    .as_ref()
                    .map(|handler| handler.namespace_version()),
            ),
    )
}

/// DATA grammar for the ORIGINAL side of a namespace5 declaration: a complete known Current3
/// catalog with optional governance4 and any known JOIN version, and NO namespace5 (an
/// existing ID5 refuses here, so a second declaration is impossible). The configured
/// opener's own parser still decides runtime support before this is ever reached.
pub(super) fn parse_member_outbox_declaration_original<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
) -> Result<Parsed<'a>, AppError> {
    parse_known(
        plain,
        domain,
        SupportedLayout::GOVERNANCE_LAYOUT.executing(true),
    )
}

/// DATA grammar for the ORIGINAL side of a governance4 declaration: a complete known Current3
/// catalog with optional namespace5 and any known JOIN version, and NO governance4.
pub(super) fn parse_governance_declaration_original<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
) -> Result<Parsed<'a>, AppError> {
    parse_known(
        plain,
        domain,
        SupportedLayout::CURRENT_LAYOUT
            .executing(true)
            .member_outbox(true),
    )
}

/// Regression-only grammar with EVERY known namespace (governance4 and member5 together),
/// standing in for an opener configured with both concrete handlers. Not a support switch.
#[cfg(test)]
pub(super) fn parse_all_known_for_test<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
    executing: bool,
) -> Result<Parsed<'a>, AppError> {
    parse_known(
        plain,
        domain,
        SupportedLayout::GOVERNANCE_LAYOUT
            .executing(executing)
            .member_outbox(true),
    )
}

pub(super) fn parse_executing<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
) -> Result<Parsed<'a>, AppError> {
    parse_known(plain, domain, SupportedLayout::PENDING_ONLY.executing(true))
}
pub(super) fn parse_bootstrap_executing<'a>(
    plain: &'a [u8],
    domain: &[u8; 32],
) -> Result<Parsed<'a>, AppError> {
    parse_known(
        plain,
        domain,
        SupportedLayout::ARCHIVE_BOOTSTRAP.executing(true),
    )
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
            current_layout: None,
            current_payload: None,
            governance: None,
            governance_payload: None,
            member_outbox: None,
            member_outbox_payload: None,
            native_registry_payload: None,
            native_registry_version: None,
        });
    }
    let mut rows = std::array::from_fn(|_| None);
    let mut count = 0;
    let mut archive = None;
    let mut archive_payload = None;
    let mut join_version = None;
    let mut current_layout = None;
    let mut current_payload = None;
    let mut governance = None;
    let mut governance_payload = None;
    let mut member_outbox = None;
    let mut member_outbox_payload = None;
    let mut native_registry_payload = None;
    let mut native_registry_version = None;
    for index in 0..envelope.count {
        let entry = envelope.entry(index)?;
        match entry.id {
            JOIN_ID => {
                (rows, count) = if layout.executing
                    && entry.version == super::pending_join::execution_payload::VERSION
                {
                    super::pending_join::execution_payload::parse(entry.payload, domain)?
                } else {
                    parse_join_entry(entry, domain)?
                };
                join_version = Some(entry.version);
            }
            super::archive_bootstrap::ARCHIVE_ID
                if layout.archive && entry.version == super::archive_bootstrap::ARCHIVE_VERSION =>
            {
                archive = Some(super::archive_bootstrap::Descriptor::parse(
                    entry.payload,
                    domain,
                )?);
                archive_payload = Some(entry.payload);
            }
            super::archive_current_namespace_codec::NAMESPACE_ID
                if layout.current
                    && entry.version
                        == super::archive_current_namespace_codec::NAMESPACE_VERSION =>
            {
                // Sorted outer IDs guarantee the original ID2 descriptor has been consumed.
                // Namespace3 can never make a new root or accompany ID2's first declaration.
                let archive = archive.as_ref().ok_or_else(refused)?;
                let expected = super::archive_current_namespace_codec::CurrentLayoutData {
                    vault: archive.vault,
                    root: archive.root,
                };
                current_layout = Some(
                    super::archive_current_namespace_codec::CurrentLayoutData::decode_data(
                        entry.payload,
                        expected,
                    )?,
                );
                current_payload = Some(entry.payload);
            }
            super::governance_namespace_codec::NAMESPACE_ID
                if layout.governance
                    && entry.version == super::governance_namespace_codec::NAMESPACE_VERSION =>
            {
                let current = current_layout.ok_or_else(refused)?;
                governance = Some(super::governance_namespace_codec::capability::GovernanceCapabilityData::decode_data(
                    entry.payload, current, current_payload.ok_or_else(refused)?,
                )?);
                governance_payload = Some(entry.payload);
            }
            super::member_outbox_namespace_codec::NAMESPACE_ID
                if layout.member_outbox
                    && entry.version == super::member_outbox_namespace_codec::NAMESPACE_VERSION =>
            {
                // Sorted outer IDs guarantee ID3 was already consumed. Namespace5 binds that
                // SAME catalog's ID3 descriptor; it can never create a root of its own.
                let current = current_layout.ok_or_else(refused)?;
                member_outbox = Some(
                    super::member_outbox_namespace_codec::MemberOutboxCapabilityData::decode_data(
                        entry.payload,
                        current,
                        current_payload.ok_or_else(refused)?,
                    )?,
                );
                member_outbox_payload = Some(entry.payload);
            }
            7 if layout.native_registry == Some(1) && entry.version == 1 => {
                let expected =
                    super::native_registry::capability::NativeCapabilityData::for_current_data(
                        current_layout.ok_or_else(refused)?,
                        current_payload.ok_or_else(refused)?,
                    )?;
                super::native_registry::capability::NativeCapabilityData::decode_exact(
                    entry.payload,
                    expected,
                )?;
                native_registry_payload = Some(entry.payload);
                native_registry_version = Some(entry.version);
            }
            7 if layout.native_registry == Some(2) && entry.version == 2 => {
                super::native_registry::operational::validate_descriptor_data(
                    entry.payload,
                    current_layout.ok_or_else(refused)?,
                    current_payload.ok_or_else(refused)?,
                )?;
                native_registry_payload = Some(entry.payload);
                native_registry_version = Some(entry.version);
            }
            7 if layout.native_registry == Some(3) && entry.version == 3 => {
                super::native_registry::lifecycle::validate_descriptor_data(
                    entry.payload,
                    current_layout.ok_or_else(refused)?,
                    current_payload.ok_or_else(refused)?,
                )?;
                native_registry_payload = Some(entry.payload);
                native_registry_version = Some(entry.version);
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
        current_layout,
        current_payload,
        governance,
        governance_payload,
        member_outbox,
        member_outbox_payload,
        native_registry_payload,
        native_registry_version,
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
    // Only the native declaration's exact whole-frame validator may introduce ID7.
    // JOIN cannot remove, normalize or replace it, including in a validly sealed stage.
    if current.native_registry_payload_digest != next.native_registry_payload_digest {
        return Err(refused());
    }
    // A namespace4 addition is NEVER inferred from Summary fields. The private
    // declaration owns BOTH whole frames and uses its exact-pair validator.
    if current.governance != next.governance
        || current.governance_payload_digest != next.governance_payload_digest
        || (current.governance.is_some()
            && (current.current_layout != next.current_layout
                || current.current_payload_digest != next.current_payload_digest
                || current.archive != next.archive))
    {
        return Err(refused());
    }
    // Namespace5 likewise: its addition is validated only by the member-outbox handler's
    // exact-pair declaration. Every ordinary JOIN successor (pause, resume, add, arm,
    // execution, migration) must keep ID5 and its exact payload, and ID2/ID3 it binds.
    if current.member_outbox != next.member_outbox
        || current.member_outbox_payload_digest != next.member_outbox_payload_digest
        || (current.member_outbox.is_some()
            && (current.current_layout != next.current_layout
                || current.current_payload_digest != next.current_payload_digest
                || current.archive != next.archive))
    {
        return Err(refused());
    }
    if current.generation.checked_add(1) != Some(next.generation)
        || next.parent != Some(current.digest)
        || next.count < current.count
        || next.count > current.count + 1
    {
        return Err(refused());
    }
    match (current.current_layout, next.current_layout) {
        (None, Some(_)) => {
            // Exactly namespace3 is added over the COMPLETE unchanged current catalog. It
            // cannot hide a JOIN arm/grant/pause/add, alter ID2 or synthesize an earlier parent.
            return if current.archive.is_some()
                && current.archive == next.archive
                && current.rows == next.rows
                && current.count == next.count
                && current.join_version == next.join_version
                && current.current_payload_digest.is_none()
                && next.current_payload_digest.is_some()
            {
                Ok(())
            } else {
                Err(refused())
            };
        }
        (old, new)
            if old == new && current.current_payload_digest == next.current_payload_digest => {}
        _ => return Err(refused()),
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
