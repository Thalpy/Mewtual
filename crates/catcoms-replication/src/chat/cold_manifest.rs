//! Bounded private per-operation manifest DATA. No paths, I/O, namespace, physical reservation,
//! selected source or authority is created here. Parsed descriptors do not prove their pages
//! exist or contain real records. The actual archive host must check every inner context/record,
//! cover the complete candidate under original head CAS, and reserve mandatory control capacity.
use super::acceptance_tree::{append_nodes, MAX_APPEND_LEAVES};
use super::cold_actions::{OriginalActionDigest, ACTION_CHUNK_BYTES, MAX_ACTION_CHUNK_PAGE_BYTES};
use super::cold_nodes::{MAX_NODE_PAGE_BYTES, MAX_NODE_PAGE_HASHES};
use super::owner_facts::OwnerFactId;
use super::owner_header::{OwnerBodyId, OwnerHeaderId};
use super::{ActionId, ChatError, MAX_ACTION_BYTES, MAX_GROUP_ID_BYTES};
use catcoms_storage::private_chat_page::{PrivateChatPageId, MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

const DOMAIN: &[u8] = b"catcoms/private-chat-manifest/v1\0";
const VERSION: u16 = 1;
const DESCRIPTOR_BYTES: usize = 1 + 32 + 4 + 2;
/// One operation writes at most3 content pages and this separate manifest. No historical list
/// is encoded here, so this does not impose a lifetime action/admission/history count limit.
pub const MAX_MANIFEST_CONTENT_PAGES: usize = 3;
/// One requested plaintext buffer plus117-byte packed descriptor scratch. This is NOT a whole
/// host/AEAD/journal/read-buffer fee; all original and output overlap remains caller-paid.
pub const MAX_MANIFEST_PAGE_BYTES: usize = 1024;
// Largest shape has all three optional digests, a maximum-group Append payload and3 rows.
// Freeze this beside the grammar so adding a field cannot silently grow Encoder beyond its
// one requested output capacity. This is format arithmetic, not a physical host fee.
const MAX_ENCODED_BYTES: usize = 4
    + DOMAIN.len()
    + 2
    + 36
    + 4
    + MAX_GROUP_ID_BYTES
    + 36
    + 8
    + 3 * 37
    + 36
    + (1 + 36 + 36 + 16)
    + 1
    + 4
    + MAX_MANIFEST_CONTENT_PAGES * DESCRIPTOR_BYTES;
const _: () = assert!(MAX_ENCODED_BYTES <= MAX_MANIFEST_PAGE_BYTES);

/// Label of the future complete private PRE-OPERATION canonical head. Not a manifest PageId,
/// public header ID, actual saved head, original writer or permission to publish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveHeadId([u8; 32]);
impl ArchiveHeadId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Closed descriptor role. Exact inner row/type/body bindings are still required after read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageRole {
    OriginalActionChunk,
    AcceptanceRecords,
    CompleteNodes,
    HeaderBodyRecords,
    GenesisRecords,
}
impl PageRole {
    pub(crate) fn tag(self) -> u8 {
        match self {
            Self::OriginalActionChunk => 1,
            Self::AcceptanceRecords => 2,
            Self::CompleteNodes => 3,
            Self::HeaderBodyRecords => 4,
            Self::GenesisRecords => 5,
        }
    }
    pub(crate) fn from_tag(tag: u8) -> Result<Self, ChatError> {
        match tag {
            1 => Ok(Self::OriginalActionChunk),
            2 => Ok(Self::AcceptanceRecords),
            3 => Ok(Self::CompleteNodes),
            4 => Ok(Self::HeaderBodyRecords),
            5 => Ok(Self::GenesisRecords),
            _ => Err(ChatError::Malformed),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageDescriptor {
    pub role: PageRole,
    pub page: PrivateChatPageId,
    pub plaintext_bytes: usize,
    /// Action chunks each describe one FRAGMENT of the same original action, not unique actions.
    pub entries: u16,
}

/// Operation labels/geometry only. A body digest does not prove the real body or its signature,
/// selected header, inclusion, authority, prior acceptance absence or actual MLS event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveOperation {
    Genesis {
        body: OwnerBodyId,
    },
    RetainAction {
        action: ActionId,
        original: OriginalActionDigest,
        total_bytes: u32,
    },
    Admission {
        body: OwnerBodyId,
    },
    Append {
        body: OwnerBodyId,
        admission: OwnerFactId,
        before: u64,
        after: u64,
    },
    Removal {
        body: OwnerBodyId,
    },
    Review {
        body: OwnerBodyId,
    },
}

/// Borrowed encoder inputs, not original source custody. The actual host must acquire these
/// labels under its original writer; parsing/relabeling a matching scalar cannot replace it.
pub struct ManifestSpec<'a> {
    pub vault: [u8; 32],
    pub group: &'a [u8],
    pub profile: OwnerFactId,
    pub sequence: u64,
    pub previous: Option<PrivateChatPageId>,
    pub source_head: Option<ArchiveHeadId>,
    pub before_header: Option<OwnerHeaderId>,
    pub after_header: OwnerHeaderId,
    pub operation: ArchiveOperation,
    pub pages: &'a [PageDescriptor],
}

/// Complete strict borrowed syntax. Keeps no owned group, page Vec, runtime or read permission.
pub struct ManifestPage<'a> {
    vault: [u8; 32],
    group: &'a [u8],
    profile: OwnerFactId,
    sequence: u64,
    previous: Option<PrivateChatPageId>,
    source_head: Option<ArchiveHeadId>,
    before_header: Option<OwnerHeaderId>,
    after_header: OwnerHeaderId,
    operation: ArchiveOperation,
    packed: &'a [u8],
}
impl std::fmt::Debug for ManifestPage<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManifestPage")
            .field("sequence", &self.sequence)
            .field("content_pages", &self.page_count())
            .finish_non_exhaustive()
    }
}
impl<'a> ManifestPage<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_MANIFEST_PAGE_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes()? != DOMAIN || d.get_u16()? != VERSION {
            return Err(ChatError::Malformed);
        }
        let vault = digest(&mut d)?;
        let group = d.get_bytes()?;
        let profile = OwnerFactId::from_bytes(digest(&mut d)?);
        let sequence = d.get_u64()?;
        let previous = optional_digest(&mut d)?.map(PrivateChatPageId::from_bytes);
        let source_head = optional_digest(&mut d)?.map(ArchiveHeadId::from_bytes);
        let before_header = optional_digest(&mut d)?.map(OwnerHeaderId::from_bytes);
        let after_header = OwnerHeaderId::from_bytes(digest(&mut d)?);
        let operation = decode_operation(&mut d)?;
        let count = usize::from(d.get_u8()?);
        let packed = d.get_bytes()?;
        d.finish()?;
        if count > MAX_MANIFEST_CONTENT_PAGES || packed.len() != count * DESCRIPTOR_BYTES {
            return Err(ChatError::Bound);
        }
        let page = Self {
            vault,
            group,
            profile,
            sequence,
            previous,
            source_head,
            before_header,
            after_header,
            operation,
            packed,
        };
        check_common(
            group,
            sequence,
            previous.is_some(),
            source_head.is_some(),
            before_header,
            after_header,
            operation,
            count,
        )?;
        validate_descriptors(operation, group.len(), count, |index| page.page(index))?;
        Ok(page)
    }
    pub fn vault_domain(&self) -> &[u8; 32] {
        &self.vault
    }
    pub fn group(&self) -> &[u8] {
        self.group
    }
    pub fn profile(&self) -> OwnerFactId {
        self.profile
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn previous_manifest(&self) -> Option<PrivateChatPageId> {
        self.previous
    }
    pub fn original_source_head(&self) -> Option<ArchiveHeadId> {
        self.source_head
    }
    pub fn before_header(&self) -> Option<OwnerHeaderId> {
        self.before_header
    }
    pub fn after_header(&self) -> OwnerHeaderId {
        self.after_header
    }
    pub fn operation(&self) -> ArchiveOperation {
        self.operation
    }
    pub fn page_count(&self) -> usize {
        self.packed.len() / DESCRIPTOR_BYTES
    }
    /// Constant bounded geometric lookup, not permission to read or trust the described page.
    pub fn page(&self, index: usize) -> Result<PageDescriptor, ChatError> {
        if index >= self.page_count() {
            return Err(ChatError::Bound);
        }
        let raw = &self.packed[index * DESCRIPTOR_BYTES..(index + 1) * DESCRIPTOR_BYTES];
        Ok(PageDescriptor {
            role: PageRole::from_tag(raw[0])?,
            page: PrivateChatPageId::from_bytes(
                raw[1..33].try_into().map_err(|_| ChatError::Malformed)?,
            ),
            plaintext_bytes: u32::from_be_bytes(
                raw[33..37].try_into().map_err(|_| ChatError::Malformed)?,
            ) as usize,
            entries: u16::from_be_bytes(raw[37..39].try_into().map_err(|_| ChatError::Malformed)?),
        })
    }
    /// DATA consistency against EXTERNALLY selected original labels. The real reader must first
    /// open `previous` under `expected_previous`, and the writer must retain actual original head
    /// custody. Self-supplied scalar/hash agreement cannot establish either selected source.
    pub fn check_data_successor(
        &self,
        expected_previous: PrivateChatPageId,
        expected_source_head: ArchiveHeadId,
        previous: &ManifestPage<'_>,
    ) -> Result<(), ChatError> {
        if self.vault != previous.vault
            || self.group != previous.group
            || self.profile != previous.profile
            || self.previous != Some(expected_previous)
            || self.source_head != Some(expected_source_head)
            || self.before_header != Some(previous.after_header)
        {
            return Err(ChatError::Scope);
        }
        if previous.sequence.checked_add(1) != Some(self.sequence) {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }
}

/// Validate ALL inputs before the single output allocation. Packed page descriptors use a fixed
///117-byte stack scratch, not a second dynamic page list. No inner content/signature is checked.
pub fn encode_manifest(spec: &ManifestSpec<'_>) -> Result<Zeroizing<Vec<u8>>, ChatError> {
    check_common(
        spec.group,
        spec.sequence,
        spec.previous.is_some(),
        spec.source_head.is_some(),
        spec.before_header,
        spec.after_header,
        spec.operation,
        spec.pages.len(),
    )?;
    validate_descriptors(
        spec.operation,
        spec.group.len(),
        spec.pages.len(),
        |index| spec.pages.get(index).copied().ok_or(ChatError::Bound),
    )?;
    let mut packed = [0u8; MAX_MANIFEST_CONTENT_PAGES * DESCRIPTOR_BYTES];
    for (index, page) in spec.pages.iter().enumerate() {
        let offset = index * DESCRIPTOR_BYTES;
        packed[offset] = page.role.tag();
        packed[offset + 1..offset + 33].copy_from_slice(page.page.as_bytes());
        packed[offset + 33..offset + 37]
            .copy_from_slice(&(page.plaintext_bytes as u32).to_be_bytes());
        packed[offset + 37..offset + 39].copy_from_slice(&page.entries.to_be_bytes());
    }
    let mut e = Encoder::with_capacity(MAX_MANIFEST_PAGE_BYTES);
    e.put_bytes(DOMAIN)?;
    e.put_u16(VERSION);
    e.put_bytes(&spec.vault)?;
    e.put_bytes(spec.group)?;
    e.put_bytes(spec.profile.as_bytes())?;
    e.put_u64(spec.sequence);
    put_optional(
        &mut e,
        spec.previous.as_ref().map(PrivateChatPageId::as_bytes),
    )?;
    put_optional(
        &mut e,
        spec.source_head.as_ref().map(ArchiveHeadId::as_bytes),
    )?;
    put_optional(
        &mut e,
        spec.before_header.as_ref().map(OwnerHeaderId::as_bytes),
    )?;
    e.put_bytes(spec.after_header.as_bytes())?;
    encode_operation(&mut e, spec.operation)?;
    e.put_u8(spec.pages.len() as u8);
    e.put_bytes(&packed[..spec.pages.len() * DESCRIPTOR_BYTES])?;
    Ok(Zeroizing::new(e.finish()))
}

#[allow(clippy::too_many_arguments)] // One fixed DATA shape, deliberately no owned intermediate.
fn check_common(
    group: &[u8],
    sequence: u64,
    previous: bool,
    source_head: bool,
    before: Option<OwnerHeaderId>,
    after: OwnerHeaderId,
    operation: ArchiveOperation,
    count: usize,
) -> Result<(), ChatError> {
    if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES || count > MAX_MANIFEST_CONTENT_PAGES {
        return Err(ChatError::Bound);
    }
    if matches!(operation, ArchiveOperation::Genesis { .. }) {
        if sequence != 1 || previous || source_head || before.is_some() {
            return Err(ChatError::Dependency);
        }
    } else {
        if sequence <= 1 || !previous || !source_head || before.is_none() {
            return Err(ChatError::Dependency);
        }
        let retaining = matches!(operation, ArchiveOperation::RetainAction { .. });
        if retaining != (before == Some(after)) {
            return Err(ChatError::Dependency);
        }
    }
    Ok(())
}

fn validate_descriptors(
    operation: ArchiveOperation,
    group_bytes: usize,
    count: usize,
    get: impl Fn(usize) -> Result<PageDescriptor, ChatError>,
) -> Result<(), ChatError> {
    let expected_count = match operation {
        ArchiveOperation::RetainAction { total_bytes, .. } => {
            if total_bytes == 0 || total_bytes as usize > MAX_ACTION_BYTES {
                return Err(ChatError::Bound);
            }
            (total_bytes as usize).div_ceil(ACTION_CHUNK_BYTES)
        }
        ArchiveOperation::Append { before, after, .. } => {
            append_geometry(before, after)?;
            3
        }
        _ => 1,
    };
    if count != expected_count {
        return Err(ChatError::Bound);
    }
    for index in 0..count {
        let page = get(index)?;
        if page.plaintext_bytes == 0 || page.plaintext_bytes > MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES {
            return Err(ChatError::Bound);
        }
        let (role, entries, exact_bytes) = match operation {
            ArchiveOperation::Genesis { .. } => (PageRole::GenesisRecords, 3, None),
            ArchiveOperation::RetainAction { total_bytes, .. } => {
                let payload =
                    (total_bytes as usize - index * ACTION_CHUNK_BYTES).min(ACTION_CHUNK_BYTES);
                let bytes = MAX_ACTION_CHUNK_PAGE_BYTES - ACTION_CHUNK_BYTES - MAX_GROUP_ID_BYTES
                    + group_bytes
                    + payload;
                (PageRole::OriginalActionChunk, 1, Some(bytes))
            }
            ArchiveOperation::Append { before, after, .. } => {
                let (leaves, nodes) = append_geometry(before, after)?;
                match index {
                    0 => (PageRole::AcceptanceRecords, leaves, None),
                    1 => (
                        PageRole::CompleteNodes,
                        nodes,
                        Some(
                            MAX_NODE_PAGE_BYTES - MAX_GROUP_ID_BYTES - MAX_NODE_PAGE_HASHES * 32
                                + group_bytes
                                + nodes * 32,
                        ),
                    ),
                    _ => (PageRole::HeaderBodyRecords, 2, None),
                }
            }
            _ => (PageRole::HeaderBodyRecords, 2, None),
        };
        if page.role != role
            || usize::from(page.entries) != entries
            || exact_bytes.is_some_and(|bytes| page.plaintext_bytes != bytes)
        {
            return Err(ChatError::Dependency);
        }
        for earlier in 0..index {
            if get(earlier)?.page == page.page {
                return Err(ChatError::Dependency);
            }
        }
    }
    Ok(())
}

fn append_geometry(before: u64, after: u64) -> Result<(usize, usize), ChatError> {
    let leaves = after.checked_sub(before).ok_or(ChatError::Dependency)?;
    if leaves == 0 || leaves > MAX_APPEND_LEAVES as u64 {
        return Err(ChatError::Bound);
    }
    let nodes = append_nodes(before, leaves as usize)
        .map_err(|_| ChatError::Dependency)?
        .total();
    Ok((leaves as usize, nodes))
}

fn encode_operation(e: &mut Encoder, op: ArchiveOperation) -> Result<(), ChatError> {
    match op {
        ArchiveOperation::Genesis { body } => {
            e.put_u8(1);
            e.put_bytes(body.as_bytes())?;
        }
        ArchiveOperation::RetainAction {
            action,
            original,
            total_bytes,
        } => {
            e.put_u8(2);
            e.put_bytes(action.as_bytes())?;
            e.put_bytes(original.as_bytes())?;
            e.put_u32(total_bytes);
        }
        ArchiveOperation::Admission { body } => {
            e.put_u8(3);
            e.put_bytes(body.as_bytes())?;
        }
        ArchiveOperation::Append {
            body,
            admission,
            before,
            after,
        } => {
            e.put_u8(4);
            e.put_bytes(body.as_bytes())?;
            e.put_bytes(admission.as_bytes())?;
            e.put_u64(before).put_u64(after);
        }
        ArchiveOperation::Removal { body } => {
            e.put_u8(5);
            e.put_bytes(body.as_bytes())?;
        }
        ArchiveOperation::Review { body } => {
            e.put_u8(6);
            e.put_bytes(body.as_bytes())?;
        }
    }
    Ok(())
}
fn decode_operation(d: &mut Decoder<'_>) -> Result<ArchiveOperation, ChatError> {
    Ok(match d.get_u8()? {
        1 => ArchiveOperation::Genesis {
            body: OwnerBodyId::from_bytes(digest(d)?),
        },
        2 => ArchiveOperation::RetainAction {
            action: ActionId::from_bytes(digest(d)?),
            original: OriginalActionDigest::from_bytes(digest(d)?),
            total_bytes: d.get_u32()?,
        },
        3 => ArchiveOperation::Admission {
            body: OwnerBodyId::from_bytes(digest(d)?),
        },
        4 => ArchiveOperation::Append {
            body: OwnerBodyId::from_bytes(digest(d)?),
            admission: OwnerFactId::from_bytes(digest(d)?),
            before: d.get_u64()?,
            after: d.get_u64()?,
        },
        5 => ArchiveOperation::Removal {
            body: OwnerBodyId::from_bytes(digest(d)?),
        },
        6 => ArchiveOperation::Review {
            body: OwnerBodyId::from_bytes(digest(d)?),
        },
        _ => return Err(ChatError::Malformed),
    })
}
fn put_optional(e: &mut Encoder, bytes: Option<&[u8; 32]>) -> Result<(), ChatError> {
    e.put_u8(u8::from(bytes.is_some()));
    if let Some(bytes) = bytes {
        e.put_bytes(bytes)?;
    }
    Ok(())
}
fn optional_digest(d: &mut Decoder<'_>) -> Result<Option<[u8; 32]>, ChatError> {
    match d.get_u8()? {
        0 => Ok(None),
        1 => Ok(Some(digest(d)?)),
        _ => Err(ChatError::Malformed),
    }
}
fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}

#[cfg(test)]
mod tests;
