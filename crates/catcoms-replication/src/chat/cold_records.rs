//! Bounded private record-page DATA. No I/O, namespace activation, source selection, historical
//! authority or Current capability follows from these content checks. The actual archive host
//! must authenticate its original manifest/source, replay transitions and pay all live buffers.
use super::cold_manifest::{ArchiveOperation, PageRole};
use super::owner_bodies::{
    AcceptanceRecord, AppendBody, RemovalBody, ReviewDecisionBody, MAX_ACCEPTANCE_RECORD_BYTES,
    MAX_APPEND_BODY_BYTES, MAX_REMOVAL_BODY_BYTES, MAX_REVIEW_BODY_BYTES,
};
use super::owner_facts::OwnerFactId;
use super::owner_header::{OwnerBodyId, SignedOwnerHeader, MAX_HEADER_BYTES};
use super::owner_profile::{
    AdmissionBody, AdmissionOrigin, SignedOwnerProfile, MAX_ADMISSION_BYTES, MAX_PROFILE_BYTES,
};
use super::owner_state::{StateFrame, MAX_STATE_BYTES};
use super::{ChatError, MAX_GROUP_ID_BYTES};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

const DOMAIN: &[u8] = b"catcoms/private-chat-record-batch/v1\0";
const HEADER_DOMAIN: &[u8] = b"catcoms/private-chat-header-row/v1\0";
const ORIGINAL_DOMAIN: &[u8] = b"catcoms/private-chat-original-record/v1\0";
const VERSION: u16 = 1;
pub const MAX_RECORD_BATCH_ROWS: usize = 64;
/// One requested input/output plaintext buffer, not a physical reservation or whole-host fee.
pub const MAX_RECORD_BATCH_BYTES: usize = 65_536;
pub const MAX_HEADER_ROW_BYTES: usize =
    4 + HEADER_DOMAIN.len() + 2 + 4 + MAX_HEADER_BYTES + 4 + MAX_STATE_BYTES;
const ROW_FRAME_BYTES: usize = 1 + 36 + 36 + 4;

/// Closed PRIVATE kind, distinct from the public header's operation kind. A typed semantic key
/// cannot confuse a Profile/Header/body/acceptance that happens to contain the same32 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RecordKind {
    Profile,
    Header,
    Admission,
    Acceptance,
    Append,
    Removal,
    Review,
}
impl RecordKind {
    fn tag(self) -> u8 {
        match self {
            Self::Profile => 1,
            Self::Header => 2,
            Self::Admission => 3,
            Self::Acceptance => 4,
            Self::Append => 5,
            Self::Removal => 6,
            Self::Review => 7,
        }
    }
    fn from_tag(tag: u8) -> Result<Self, ChatError> {
        match tag {
            1 => Ok(Self::Profile),
            2 => Ok(Self::Header),
            3 => Ok(Self::Admission),
            4 => Ok(Self::Acceptance),
            5 => Ok(Self::Append),
            6 => Ok(Self::Removal),
            7 => Ok(Self::Review),
            _ => Err(ChatError::Malformed),
        }
    }
    fn payload_cap(self) -> usize {
        match self {
            Self::Profile => MAX_PROFILE_BYTES,
            Self::Header => MAX_HEADER_ROW_BYTES,
            Self::Admission => MAX_ADMISSION_BYTES,
            Self::Acceptance => MAX_ACCEPTANCE_RECORD_BYTES,
            Self::Append => MAX_APPEND_BODY_BYTES,
            Self::Removal => MAX_REMOVAL_BODY_BYTES,
            Self::Review => MAX_REVIEW_BODY_BYTES,
        }
    }
}

/// DATA-only complete-representation digest, including the actual signature and, for Header,
/// its full state image. Neither a semantic CID nor the separate OriginalActionDigest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OriginalRecordDigest([u8; 32]);
impl OriginalRecordDigest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
fn original(kind: RecordKind, payload: &[u8]) -> OriginalRecordDigest {
    let mut h = blake3::Hasher::new();
    h.update(ORIGINAL_DOMAIN);
    h.update(&[kind.tag()]);
    h.update(payload);
    OriginalRecordDigest(*h.finalize().as_bytes())
}

/// One borrowed composite with a bounded owned Header. Its signature/state consistency does
/// NOT prove a selected parent or that applying the actual body produces this resulting state.
pub struct HeaderRow<'a> {
    header: SignedOwnerHeader,
    image: &'a [u8],
}
impl std::fmt::Debug for HeaderRow<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeaderRow").finish_non_exhaustive()
    }
}
impl<'a> HeaderRow<'a> {
    pub fn decode(raw: &'a [u8]) -> Result<Self, ChatError> {
        if raw.len() > MAX_HEADER_ROW_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(raw);
        if d.get_bytes()? != HEADER_DOMAIN || d.get_u16()? != VERSION {
            return Err(ChatError::Malformed);
        }
        let envelope = d.get_bytes()?;
        let image = d.get_bytes()?;
        d.finish()?;
        // Check BOTH variable caps/framing before either public typed decode/group copy.
        if envelope.len() > MAX_HEADER_BYTES || image.len() > MAX_STATE_BYTES {
            return Err(ChatError::Bound);
        }
        let header = SignedOwnerHeader::decode(envelope)?;
        StateFrame::decode(image)?.verify_header_binding(&header)?;
        Ok(Self { header, image })
    }
    pub fn header(&self) -> &SignedOwnerHeader {
        &self.header
    }
    pub fn image(&self) -> &'a [u8] {
        self.image
    }
}

/// The caller must already hold/pay the image, Header and one composite output plus the nested
/// Header encoder/verification scratch. This helper changes no public wire grammar.
pub fn encode_header_row(
    header: &SignedOwnerHeader,
    image: &[u8],
) -> Result<Zeroizing<Vec<u8>>, ChatError> {
    if image.len() > MAX_STATE_BYTES {
        return Err(ChatError::Bound);
    }
    StateFrame::decode(image)?.verify_header_binding(header)?;
    let envelope = header.encode();
    let mut e = Encoder::with_capacity(MAX_HEADER_ROW_BYTES);
    e.put_bytes(HEADER_DOMAIN)?;
    e.put_u16(VERSION);
    e.put_bytes(&envelope)?;
    e.put_bytes(image)?;
    Ok(Zeroizing::new(e.finish()))
}

/// External expected DATA, never selected-source or positive authority. A manifest descriptor
/// cannot supply the actual Append needed for an AcceptanceRecords page's leaf bijection.
#[derive(Clone, Copy)]
pub struct BatchBinding<'a> {
    pub vault: &'a [u8; 32],
    pub profile: &'a SignedOwnerProfile,
    pub role: PageRole,
    pub operation: &'a ArchiveOperation,
    /// Mandatory for AcceptanceRecords. Other roles validate their actual retained body row.
    pub append: Option<&'a AppendBody>,
}
impl std::fmt::Debug for BatchBinding<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchBinding").finish_non_exhaustive()
    }
}
impl BatchBinding<'_> {
    fn body(self) -> Result<OwnerBodyId, ChatError> {
        match *self.operation {
            ArchiveOperation::Genesis { body }
            | ArchiveOperation::Admission { body }
            | ArchiveOperation::Append { body, .. }
            | ArchiveOperation::Removal { body }
            | ArchiveOperation::Review { body } => Ok(body),
            ArchiveOperation::RetainAction { .. } => Err(ChatError::Scope),
        }
    }
    fn check(self, count: usize) -> Result<(), ChatError> {
        if count == 0 || count > MAX_RECORD_BATCH_ROWS {
            return Err(ChatError::Bound);
        }
        match (self.role, self.operation) {
            (PageRole::GenesisRecords, ArchiveOperation::Genesis { .. }) if count == 3 => {}
            (
                PageRole::HeaderBodyRecords,
                ArchiveOperation::Admission { .. }
                | ArchiveOperation::Append { .. }
                | ArchiveOperation::Removal { .. }
                | ArchiveOperation::Review { .. },
            ) if count == 2 => {}
            (PageRole::AcceptanceRecords, ArchiveOperation::Append { .. }) => {
                self.check_append(self.append.ok_or(ChatError::Dependency)?)?;
            }
            _ => return Err(ChatError::Scope),
        }
        if !self.profile.verify() {
            return Err(ChatError::Signature);
        }
        Ok(())
    }
    fn check_append(self, append: &AppendBody) -> Result<(), ChatError> {
        let spec = append.spec();
        match *self.operation {
            ArchiveOperation::Append {
                body,
                admission,
                before,
                after,
            } if body == append.id()
                && admission == spec.old_tip.context.admission
                && before == spec.old_tip.count
                && after == spec.new_tip.count
                && spec.group == self.profile.spec().group
                && spec.old_tip.context.profile == self.profile.id() =>
            {
                Ok(())
            }
            _ => Err(ChatError::Scope),
        }
    }
}

/// Already ordered borrowed typed inputs. No arbitrary raw payload or caller-chosen semantic
/// key enters the encoder. DATA validity still does not establish actual MLS/protocol authority.
pub enum RecordSource<'a> {
    Profile(&'a SignedOwnerProfile),
    Header {
        header: &'a SignedOwnerHeader,
        image: &'a [u8],
    },
    Admission(&'a AdmissionBody),
    Acceptance(&'a AcceptanceRecord),
    Append(&'a AppendBody),
    Removal(&'a RemovalBody),
    Review(&'a ReviewDecisionBody),
}
impl std::fmt::Debug for RecordSource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Profile(_) => "RecordSource::Profile",
            Self::Header { .. } => "RecordSource::Header",
            Self::Admission(_) => "RecordSource::Admission",
            Self::Acceptance(_) => "RecordSource::Acceptance",
            Self::Append(_) => "RecordSource::Append",
            Self::Removal(_) => "RecordSource::Removal",
            Self::Review(_) => "RecordSource::Review",
        };
        f.debug_struct(name).finish_non_exhaustive()
    }
}
impl RecordSource<'_> {
    fn encode(&self) -> Result<(RecordKind, Zeroizing<Vec<u8>>), ChatError> {
        let (kind, bytes) = match self {
            Self::Profile(p) => (RecordKind::Profile, p.encode()),
            Self::Header { header, image } => {
                return Ok((RecordKind::Header, encode_header_row(header, image)?));
            }
            Self::Admission(b) => (RecordKind::Admission, b.encode()),
            Self::Acceptance(b) => (RecordKind::Acceptance, b.encode()),
            Self::Append(b) => (RecordKind::Append, b.encode()),
            Self::Removal(b) => (RecordKind::Removal, b.encode()),
            Self::Review(b) => (RecordKind::Review, b.encode()),
        };
        Ok((kind, Zeroizing::new(bytes)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RecordKey {
    pub kind: RecordKind,
    pub semantic: [u8; 32],
}
/// Borrowed row DATA. Rows returned by RecordBatch passed that invocation's explicit content
/// binding; direct construction proves nothing. No row is an authority witness: later source/
/// chain/ordinary/read consumers must enforce their original capabilities.
#[derive(Clone, Copy)]
pub struct RecordRow<'a> {
    pub key: RecordKey,
    pub original: OriginalRecordDigest,
    pub payload: &'a [u8],
}
impl std::fmt::Debug for RecordRow<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Private receipt/state/membership bytes must not leak through a diagnostic dump.
        f.debug_struct("RecordRow")
            .field("kind", &self.key.kind)
            .field("bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy)]
struct RowMeta {
    key: RecordKey,
    original: OriginalRecordDigest,
    bytes: usize,
}

/// Full outer/framing preflight retains only a fixed64 array of borrowed rows. No decoded
/// complete-record Vec, sorting, attacker count allocation or implicit source selection occurs.
pub struct RecordBatch<'a> {
    vault: [u8; 32],
    group: &'a [u8],
    profile: OwnerFactId,
    role: PageRole,
    body: OwnerBodyId,
    rows: [Option<RecordRow<'a>>; MAX_RECORD_BATCH_ROWS],
    count: usize,
}
impl std::fmt::Debug for RecordBatch<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordBatch").finish_non_exhaustive()
    }
}
impl<'a> RecordBatch<'a> {
    pub fn decode(raw: &'a [u8], binding: BatchBinding<'_>) -> Result<Self, ChatError> {
        if raw.len() > MAX_RECORD_BATCH_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(raw);
        if d.get_bytes()? != DOMAIN || d.get_u16()? != VERSION {
            return Err(ChatError::Malformed);
        }
        let vault = digest(&mut d)?;
        let group = d.get_bytes()?;
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        let profile = OwnerFactId::from_bytes(digest(&mut d)?);
        let role = PageRole::from_tag(d.get_u8()?)?;
        let body = OwnerBodyId::from_bytes(digest(&mut d)?);
        let count = usize::from(d.get_u16()?);
        if count == 0 || count > MAX_RECORD_BATCH_ROWS {
            return Err(ChatError::Bound);
        }
        let packed = d.get_bytes()?;
        d.finish()?;
        let mut rows = [None; MAX_RECORD_BATCH_ROWS];
        let mut p = Decoder::new(packed);
        for slot in rows.iter_mut().take(count) {
            let kind = RecordKind::from_tag(p.get_u8()?)?;
            let semantic = digest(&mut p)?;
            let original = OriginalRecordDigest(digest(&mut p)?);
            let payload = p.get_bytes()?;
            if payload.is_empty() || payload.len() > kind.payload_cap() {
                return Err(ChatError::Bound);
            }
            *slot = Some(RecordRow {
                key: RecordKey { kind, semantic },
                original,
                payload,
            });
        }
        p.finish()?;
        // No typed decode/group copy occurs until the COMPLETE input's framing/caps pass.
        binding.check(count)?;
        if &vault != binding.vault
            || group != binding.profile.spec().group.as_slice()
            || profile != binding.profile.id()
            || role != binding.role
            || body != binding.body()?
        {
            return Err(ChatError::Scope);
        }
        let batch = Self {
            vault,
            group,
            profile,
            role,
            body,
            rows,
            count,
        };
        for row in batch.rows() {
            let semantic = check_payload(row.key.kind, row.payload, binding.profile)?;
            if semantic != row.key.semantic || original(row.key.kind, row.payload) != row.original {
                return Err(ChatError::Scope);
            }
        }
        // EVERY supplied signed representation was checked before semantic duplicate/order.
        check_order(batch.rows().map(|row| row.key))?;
        batch.check_role(binding)?;
        Ok(batch)
    }
    pub fn vault_domain(&self) -> &[u8; 32] {
        &self.vault
    }
    pub fn group(&self) -> &'a [u8] {
        self.group
    }
    pub fn profile(&self) -> OwnerFactId {
        self.profile
    }
    pub fn role(&self) -> PageRole {
        self.role
    }
    pub fn body(&self) -> OwnerBodyId {
        self.body
    }
    pub fn row_count(&self) -> usize {
        self.count
    }
    pub fn rows(&self) -> impl ExactSizeIterator<Item = RecordRow<'a>> + '_ {
        self.rows[..self.count]
            .iter()
            .map(|row| row.expect("complete preflight"))
    }
    fn one(&self, kind: RecordKind) -> Result<RecordRow<'a>, ChatError> {
        let mut rows = self.rows().filter(|row| row.key.kind == kind);
        let row = rows.next().ok_or(ChatError::Scope)?;
        if rows.next().is_some() {
            return Err(ChatError::Scope);
        }
        Ok(row)
    }
    fn check_role(&self, binding: BatchBinding<'_>) -> Result<(), ChatError> {
        if self.role == PageRole::AcceptanceRecords {
            let append = binding.append.ok_or(ChatError::Dependency)?;
            binding.check_append(append)?;
            let spec = append.spec();
            if self.count != spec.leaves.len() {
                return Err(ChatError::Dependency);
            }
            for row in self.rows() {
                if row.key.kind != RecordKind::Acceptance {
                    return Err(ChatError::Scope);
                }
                let record = AcceptanceRecord::decode(row.payload)?;
                if record.spec().context != spec.old_tip.context {
                    return Err(ChatError::Scope);
                }
                // Strict row CID uniqueness and the body's strict (channel,CID) order plus
                // equal counts make this an exact bijection, not a partial-membership check.
                if spec
                    .leaves
                    .iter()
                    .filter(|leaf| **leaf == record.leaf())
                    .count()
                    != 1
                {
                    return Err(ChatError::Dependency);
                }
            }
            return Ok(());
        }
        let header_row = HeaderRow::decode(self.one(RecordKind::Header)?.payload)?;
        let header = header_row.header();
        match *binding.operation {
            ArchiveOperation::Genesis { .. } => {
                // First pass verified this row's actual representation and exact Profile CID.
                // Use the equivalent already verified external Profile rather than retaining
                // a third decoded group/semantic allocation alongside Header and Admission.
                self.one(RecordKind::Profile)?;
                let body = AdmissionBody::decode(self.one(RecordKind::Admission)?.payload)?;
                StateFrame::decode(header_row.image())?.verify_genesis_binding(
                    binding.profile,
                    &body,
                    header,
                )?;
            }
            ArchiveOperation::Admission { .. } => {
                let body = AdmissionBody::decode(self.one(RecordKind::Admission)?.payload)?;
                if !matches!(body.spec().origin, AdmissionOrigin::Add { .. }) {
                    return Err(ChatError::Scope);
                }
                body.verify_profile_binding(binding.profile, header)?;
            }
            ArchiveOperation::Append { .. } => {
                let body = AppendBody::decode(self.one(RecordKind::Append)?.payload)?;
                binding.check_append(&body)?;
                body.verify_header_binding(header)?;
            }
            ArchiveOperation::Removal { .. } => {
                RemovalBody::decode(self.one(RecordKind::Removal)?.payload)?
                    .verify_header_binding(header)?;
            }
            ArchiveOperation::Review { .. } => {
                ReviewDecisionBody::decode(self.one(RecordKind::Review)?.payload)?
                    .verify_header_binding(header)?;
            }
            ArchiveOperation::RetainAction { .. } => return Err(ChatError::Scope),
        }
        if header.spec().body != self.body
            || (self.role == PageRole::HeaderBodyRecords && header.spec().sequence <= 1)
        {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }
}

/// Encode ordered typed DATA with fixed metadata and one row scratch at a time. Complete
/// aggregate size is checked BEFORE allocating/copying output. Count checks precede iteration.
/// The same immutable sources are re-encoded; no digest/cache can skip their actual signatures.
pub fn encode_record_batch(
    binding: BatchBinding<'_>,
    sources: &[RecordSource<'_>],
) -> Result<Zeroizing<Vec<u8>>, ChatError> {
    binding.check(sources.len())?;
    let mut metas = [None; MAX_RECORD_BATCH_ROWS];
    let mut packed_bytes = 0usize;
    for (source, meta) in sources.iter().zip(metas.iter_mut()) {
        let (kind, payload) = source.encode()?;
        if payload.len() > kind.payload_cap() {
            return Err(ChatError::Bound);
        }
        let semantic = check_payload(kind, &payload, binding.profile)?;
        packed_bytes = packed_bytes
            .checked_add(ROW_FRAME_BYTES)
            .and_then(|n| n.checked_add(payload.len()))
            .ok_or(ChatError::Bound)?;
        if packed_bytes > MAX_RECORD_BATCH_BYTES {
            return Err(ChatError::Bound);
        }
        *meta = Some(RowMeta {
            key: RecordKey { kind, semantic },
            original: original(kind, &payload),
            bytes: payload.len(),
        });
    }
    let prefix =
        4 + DOMAIN.len() + 2 + 36 + 4 + binding.profile.spec().group.len() + 36 + 1 + 36 + 2 + 4;
    let total = prefix.checked_add(packed_bytes).ok_or(ChatError::Bound)?;
    if total > MAX_RECORD_BATCH_BYTES {
        return Err(ChatError::Bound);
    }
    check_order(
        metas
            .iter()
            .take(sources.len())
            .map(|m| m.expect("source preflight").key),
    )?;
    let mut e = Encoder::with_capacity(MAX_RECORD_BATCH_BYTES);
    e.put_bytes(DOMAIN)?;
    e.put_u16(VERSION);
    e.put_bytes(binding.vault)?;
    e.put_bytes(&binding.profile.spec().group)?;
    e.put_bytes(binding.profile.id().as_bytes())?;
    e.put_u8(binding.role.tag());
    e.put_bytes(binding.body()?.as_bytes())?;
    e.put_u16(sources.len() as u16).put_u32(packed_bytes as u32);
    // Write the packed field directly; avoid a second page-sized packed scratch allocation.
    for (source, meta) in sources.iter().zip(metas.iter()) {
        let meta = meta.expect("source preflight");
        let (kind, payload) = source.encode()?;
        if kind != meta.key.kind
            || payload.len() != meta.bytes
            || original(kind, &payload) != meta.original
        {
            return Err(ChatError::Scope);
        }
        e.put_u8(kind.tag());
        e.put_bytes(&meta.key.semantic)?;
        e.put_bytes(meta.original.as_bytes())?;
        e.put_bytes(&payload)?;
    }
    let output = Zeroizing::new(e.finish());
    debug_assert_eq!(output.len(), total);
    RecordBatch::decode(&output, binding)?;
    Ok(output)
}

/// This inventory is logical fixed codec scratch, not an actual host debit. Callers additionally
/// own original inputs, <=64KiB output, <=HeaderRowMax scratch, nested Header/Profile encoding
/// and signature scratch, Append's packed leaves and one retained Header/body/group. Both arrays
/// can coexist during final output validation. AEAD/files/SDK/replay/node/control heaps excluded.
pub const RECORD_BATCH_METADATA_BYTES: usize =
    std::mem::size_of::<[Option<RowMeta>; MAX_RECORD_BATCH_ROWS]>()
        + std::mem::size_of::<[Option<RecordRow<'static>>; MAX_RECORD_BATCH_ROWS]>();

fn check_order(keys: impl IntoIterator<Item = RecordKey>) -> Result<(), ChatError> {
    let mut previous = None;
    for key in keys {
        // Freeze the on-disk order by the explicit closed wire tag, not enum declaration order.
        let canonical = (key.kind.tag(), key.semantic);
        if previous.is_some_and(|old| old >= canonical) {
            return Err(ChatError::Dependency);
        }
        previous = Some(canonical);
    }
    Ok(())
}
fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}
fn scope(
    group: &[u8],
    profile: OwnerFactId,
    expected: &SignedOwnerProfile,
) -> Result<(), ChatError> {
    if group != expected.spec().group.as_slice() || profile != expected.id() {
        Err(ChatError::Scope)
    } else {
        Ok(())
    }
}
fn check_payload(
    kind: RecordKind,
    bytes: &[u8],
    profile: &SignedOwnerProfile,
) -> Result<[u8; 32], ChatError> {
    Ok(match kind {
        RecordKind::Profile => {
            let p = SignedOwnerProfile::decode(bytes)?;
            if !p.verify() {
                return Err(ChatError::Signature);
            }
            scope(&p.spec().group, p.id(), profile)?;
            *p.id().as_bytes()
        }
        RecordKind::Header => {
            let row = HeaderRow::decode(bytes)?;
            let h = row.header();
            scope(&h.spec().group, h.spec().profile, profile)?;
            if h.spec().issuer != profile.spec().founder || h.spec().tenure != profile.tenure() {
                return Err(ChatError::Scope);
            }
            *h.id().as_bytes()
        }
        RecordKind::Admission => {
            let b = AdmissionBody::decode(bytes)?;
            scope(&b.spec().group, b.spec().profile, profile)?;
            *b.id().as_bytes()
        }
        RecordKind::Acceptance => {
            let b = AcceptanceRecord::decode(bytes)?;
            scope(&b.spec().group, b.spec().context.profile, profile)?;
            *b.id().as_bytes()
        }
        RecordKind::Append => {
            let b = AppendBody::decode(bytes)?;
            scope(&b.spec().group, b.spec().old_tip.context.profile, profile)?;
            *b.id().as_bytes()
        }
        RecordKind::Removal => {
            let b = RemovalBody::decode(bytes)?;
            scope(
                &b.spec().group,
                b.spec().frozen_tip.context.profile,
                profile,
            )?;
            *b.id().as_bytes()
        }
        RecordKind::Review => {
            let b = ReviewDecisionBody::decode(bytes)?;
            scope(
                &b.spec().target.spec().group,
                b.spec().target.spec().context.profile,
                profile,
            )?;
            *b.id().as_bytes()
        }
    })
}

#[cfg(test)]
mod tests;
