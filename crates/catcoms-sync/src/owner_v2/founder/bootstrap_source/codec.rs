//! Closed framing, distinct key/ID, and cold borrowed DATA inspection. No serde/SDK load.
use super::*;
use catcoms_crypto::{seal, unseal, SealedBlob};
use catcoms_replication::chat::{
    cold_manifest::ManifestPage,
    cold_records::{HeaderRow, RecordBatch, RecordKind},
    owner_state::MAX_STATE_BYTES,
};
use catcoms_storage::private_chat_page::open_private_chat_page;
use catcoms_wire::Decoder;
use std::ops::Range;

const BODY_DOMAIN: &[u8] = b"catcoms/private-chat-bootstrap-source/v1\0";
const FRAME_DOMAIN: &[u8] = b"catcoms/private-chat-bootstrap-source-sealed/v1\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-bootstrap-source-key/v1";
const VERSION: u16 = 1;
const TAG_BYTES: usize = 16;
const FRAME_OVERHEAD: usize = 4 + FRAME_DOMAIN.len() + 2 + 24 + 4 + TAG_BYTES;
const MAX_MANIFEST_FRAME: usize =
    MAX_MANIFEST_PAGE_BYTES + MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES - MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES;
// Four payload framing words are included here. Fixed IDs use the same framed32 convention.
const MAX_BODY_METADATA: usize = 4
    + BODY_DOMAIN.len()
    + 2
    + 36
    + 4
    + MAX_GROUP_ID_BYTES
    + 4 * 36
    + 8
    + 5 * 36
    + 3 * 8
    + 36
    + 4
    + 4 * 4;
pub const MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES: usize = MAX_BODY_METADATA
    + snapshot::MAX_SNAPSHOT
    + MAX_PROFILE_BYTES
    + MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES
    + MAX_MANIFEST_FRAME;
pub const MAX_BOOTSTRAP_SOURCE_FRAME_BYTES: usize =
    MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES + FRAME_OVERHEAD;
/// Requested body + AEAD ciphertext + final frame, excluding the retained original S0, public
/// pages, actual provider/SDK, context/group/crypto and host debt/file copies. NOT a paid lease.
pub const BOOTSTRAP_SOURCE_SEAL_BUFFER_BYTES: usize = MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES
    + (MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES + TAG_BYTES)
    + MAX_BOOTSTRAP_SOURCE_FRAME_BYTES;
/// Raw caller frame + copied cipher + AEAD output CAPACITY. Drop cipher before nested opens.
/// Add retained source/raw/debt copies and bounded nested/public scratch separately; not RSS.
pub const BOOTSTRAP_SOURCE_OPEN_BUFFER_BYTES: usize =
    MAX_BOOTSTRAP_SOURCE_FRAME_BYTES + 2 * (MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES + TAG_BYTES);
/// Separate conservative logical nested-page/public-codec scratch inventory, NOT an archive
/// quota or host reservation. The large source plaintext/raw/read buffers above remain live.
/// Both nested opens are summed although manifest plaintext drops before records open. There
/// is no JSON/SDK expansion; all inspected public codecs have bounded framing/group copies.
// The opener now keeps exact verified genesis DATA; seed construction overlaps one additional
// bounded copy. These named retained copies are additional to the original nested scratch.
pub(super) const VERIFIED_GENESIS_LAYOUT: usize =
    std::mem::size_of::<VerifiedGenesisData>() + MAX_STATE_BYTES + 3 * MAX_GROUP_ID_BYTES;
pub const BOOTSTRAP_SOURCE_NESTED_DATA_INVOICE: usize = 256 * 1024 + 2 * VERIFIED_GENESIS_LAYOUT;
const NAMED_NESTED_LAYOUT: usize = 2 * (MAX_RECORD_BATCH_BYTES + TAG_BYTES)
    + 2 * (MAX_MANIFEST_PAGE_BYTES + TAG_BYTES)
    + RECORD_BATCH_METADATA_BYTES
    + MAX_HEADER_ROW_BYTES + MAX_HEADER_BYTES + MAX_STATE_BYTES
    + 2 * MAX_PROFILE_BYTES + MAX_ADMISSION_BYTES + 8 * MAX_GROUP_ID_BYTES
    + 4101 + 512 + 16 * 1024 // bounded policy/field/signature/canonical-encoding scratch
    + 4 * std::mem::size_of::<BootstrapSourceContextData>()
    + 4 * std::mem::size_of::<BootstrapSourceContextSpec<'static>>()
    + std::mem::size_of::<s0::Fields<'static>>()
    + std::mem::size_of::<OpenedBootstrapSourceData>()
    + std::mem::size_of::<FounderBootstrapPreparation>()
    + std::mem::size_of::<FounderArchivePreparation>()
    + 2 * VERIFIED_GENESIS_LAYOUT; // boxed pre-candidate refusal + retained/cloned genesis
const _: () = assert!(NAMED_NESTED_LAYOUT <= BOOTSTRAP_SOURCE_NESTED_DATA_INVOICE);
const _: () = assert!(MAX_BODY_METADATA == 755);
const _: () = assert!(MAX_PROFILE_BYTES == 494 && MAX_MANIFEST_FRAME == 1110);
const _: () = assert!(MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES == 8_522_125);
const _: () = assert!(MAX_BOOTSTRAP_SOURCE_FRAME_BYTES == 8_522_223);
const _: () = assert!(BOOTSTRAP_SOURCE_SEAL_BUFFER_BYTES == 25_566_489);
const _: () = assert!(BOOTSTRAP_SOURCE_OPEN_BUFFER_BYTES == 25_566_505);

fn bad() -> SyncError {
    SyncError::Malformed
}
fn bytes(out: &mut Vec<u8>, value: &[u8]) {
    // Only bounded original source fields reach this encoder; there is no fallible secret
    // Encoder whose partial ordinary Vec would escape zeroization on an error.
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}
pub(super) fn encode_original_body<T: MeshTransport, R: CryptoRngCore>(
    original: &FounderArchivePreparation,
    node: &ChannelSync<T, R>,
    context: &BootstrapSourceContextData,
) -> Result<Zeroizing<Vec<u8>>, SyncError> {
    let private = original.borrow_private_snapshot_for_covering_save(node)?;
    let public = original.original.borrow_for_covering_save(node)?;
    let archive = original.borrow_for_covering_save(node)?;
    let spec = context.spec_data();
    spec.validate()?;
    if private.snapshot.len() > spec.limits.max_snapshot_bytes()
        || public.profile.len() > MAX_PROFILE_BYTES
        || archive.records_frame.len() > MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES
        || archive.manifest_frame.len() > MAX_MANIFEST_FRAME
    {
        return Err(bad());
    }
    let capacity = MAX_BODY_METADATA
        .checked_sub(MAX_GROUP_ID_BYTES)
        .and_then(|n| n.checked_add(spec.group.len()))
        .and_then(|n| n.checked_add(private.snapshot.len()))
        .and_then(|n| n.checked_add(public.profile.len()))
        .and_then(|n| n.checked_add(archive.records_frame.len()))
        .and_then(|n| n.checked_add(archive.manifest_frame.len()))
        .ok_or_else(bad)?;
    if capacity > MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES {
        return Err(bad());
    }
    let mut out = Zeroizing::new(Vec::with_capacity(capacity));
    bytes(&mut out, BODY_DOMAIN);
    out.extend_from_slice(&VERSION.to_be_bytes());
    bytes(&mut out, &spec.vault);
    bytes(&mut out, spec.group);
    for field in [
        &spec.signer,
        &spec.fresh_attempt,
        &spec.scope_domain,
        &spec.scope_nonce,
    ] {
        bytes(&mut out, field);
    }
    out.extend_from_slice(&spec.planned_revision.to_be_bytes());
    for field in [
        spec.profile.as_bytes(),
        spec.admission.as_bytes(),
        spec.header.as_bytes(),
        spec.records.as_bytes(),
        spec.manifest.as_bytes(),
    ] {
        bytes(&mut out, field);
    }
    for limit in [
        spec.limits.max_snapshot_bytes(),
        spec.limits.max_mls_bytes(),
        spec.limits.max_provider_backing_bytes(),
    ] {
        out.extend_from_slice(&(limit as u64).to_be_bytes());
    }
    bytes(&mut out, &spec.snapshot_digest);
    out.extend_from_slice(&(spec.records_plaintext_bytes as u32).to_be_bytes());
    for payload in [
        private.snapshot,
        public.profile,
        archive.records_frame,
        archive.manifest_frame,
    ] {
        bytes(&mut out, payload);
    }
    if out.len() != capacity {
        return Err(bad());
    }
    Ok(out)
}

pub(super) fn seal_original_body(
    key: &[u8; 32],
    body: &[u8],
    context: BootstrapSourceContextData,
    rng: &mut dyn CryptoRngCore,
) -> Result<PreparedBootstrapSource, SyncError> {
    if body.is_empty() || body.len() > MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES {
        return Err(bad());
    }
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let sealed = seal(&key, body, rng).map_err(|_| bad())?;
    let mut frame = Zeroizing::new(Vec::with_capacity(body.len() + FRAME_OVERHEAD));
    bytes(&mut frame, FRAME_DOMAIN);
    frame.extend_from_slice(&VERSION.to_be_bytes());
    frame.extend_from_slice(&sealed.nonce);
    bytes(&mut frame, &sealed.ciphertext);
    Ok(PreparedBootstrapSource {
        id: BootstrapSourceId::of(&frame),
        frame,
        context,
    })
}

struct ParsedFrame<'a> {
    nonce: [u8; 24],
    ciphertext: &'a [u8],
}
fn parse_frame(frame: &[u8]) -> Result<ParsedFrame<'_>, SyncError> {
    if frame.len() <= FRAME_OVERHEAD || frame.len() > MAX_BOOTSTRAP_SOURCE_FRAME_BYTES {
        return Err(bad());
    }
    let domain_end = 4 + FRAME_DOMAIN.len();
    let version_end = domain_end + 2;
    let nonce_end = version_end + 24;
    let start = nonce_end + 4;
    if frame[..4] != (FRAME_DOMAIN.len() as u32).to_be_bytes()
        || &frame[4..domain_end] != FRAME_DOMAIN
        || frame[domain_end..version_end] != VERSION.to_be_bytes()
    {
        return Err(bad());
    }
    let len = u32::from_be_bytes(frame[nonce_end..start].try_into().map_err(|_| bad())?) as usize;
    if len != frame.len() - start
        || len <= TAG_BYTES
        || len > MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES + TAG_BYTES
    {
        return Err(bad());
    }
    Ok(ParsedFrame {
        nonce: frame[version_end..nonce_end]
            .try_into()
            .map_err(|_| bad())?,
        ciphertext: &frame[start..],
    })
}

#[derive(Clone)]
struct PayloadRanges {
    snapshot: Range<usize>,
    profile: Range<usize>,
    records: Range<usize>,
    manifest: Range<usize>,
}
struct Body<'a> {
    spec: BootstrapSourceContextSpec<'a>,
    snapshot: &'a [u8],
    profile: &'a [u8],
    records: &'a [u8],
    manifest: &'a [u8],
    ranges: PayloadRanges,
}
fn field<'a>(d: &mut Decoder<'a>) -> Result<&'a [u8], SyncError> {
    d.get_bytes().map_err(|_| bad())
}
fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], SyncError> {
    field(d)?.try_into().map_err(|_| bad())
}
fn payload<'a>(
    d: &mut Decoder<'a>,
    total: usize,
    max: usize,
) -> Result<(&'a [u8], Range<usize>), SyncError> {
    let start = total
        .checked_sub(d.remaining())
        .and_then(|n| n.checked_add(4))
        .ok_or_else(bad)?;
    let value = field(d)?;
    if value.is_empty() || value.len() > max {
        return Err(bad());
    }
    Ok((value, start..start + value.len()))
}
/// Complete borrowed framing of ALL fields and payloads precedes any typed Profile/group copy.
fn parse_body(raw: &[u8]) -> Result<Body<'_>, SyncError> {
    if raw.is_empty() || raw.len() > MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES {
        return Err(bad());
    }
    let mut d = Decoder::new(raw);
    if field(&mut d)? != BODY_DOMAIN || d.get_u16().map_err(|_| bad())? != VERSION {
        return Err(bad());
    }
    let vault = digest(&mut d)?;
    let group = field(&mut d)?;
    if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
        return Err(bad());
    }
    let signer = digest(&mut d)?;
    let fresh_attempt = digest(&mut d)?;
    let scope_domain = digest(&mut d)?;
    let scope_nonce = digest(&mut d)?;
    let planned_revision = d.get_u64().map_err(|_| bad())?;
    let profile = OwnerFactId::from_bytes(digest(&mut d)?);
    let admission = OwnerBodyId::from_bytes(digest(&mut d)?);
    let header = OwnerHeaderId::from_bytes(digest(&mut d)?);
    let records_id = PrivateChatPageId::from_bytes(digest(&mut d)?);
    let manifest_id = PrivateChatPageId::from_bytes(digest(&mut d)?);
    let mut limits = [0usize; 3];
    for limit in &mut limits {
        *limit = usize::try_from(d.get_u64().map_err(|_| bad())?).map_err(|_| bad())?;
    }
    let limits = FounderSnapshotLimits::new(limits[0], limits[1], limits[2])?;
    let snapshot_digest = digest(&mut d)?;
    let records_plaintext_bytes = d.get_u32().map_err(|_| bad())? as usize;
    let spec = BootstrapSourceContextSpec {
        vault,
        group,
        signer,
        fresh_attempt,
        scope_domain,
        scope_nonce,
        planned_revision,
        profile,
        admission,
        header,
        records: records_id,
        manifest: manifest_id,
        limits,
        snapshot_digest,
        records_plaintext_bytes,
    };
    spec.validate()?;
    let (snapshot, snapshot_range) = payload(&mut d, raw.len(), limits.max_snapshot_bytes())?;
    let (profile, profile_range) = payload(&mut d, raw.len(), MAX_PROFILE_BYTES)?;
    let (records, records_range) = payload(&mut d, raw.len(), MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES)?;
    let (manifest, manifest_range) = payload(&mut d, raw.len(), MAX_MANIFEST_FRAME)?;
    d.finish().map_err(|_| bad())?;
    Ok(Body {
        spec,
        snapshot,
        profile,
        records,
        manifest,
        ranges: PayloadRanges {
            snapshot: snapshot_range,
            profile: profile_range,
            records: records_range,
            manifest: manifest_range,
        },
    })
}

/// Immutable offline DATA. It contains secret S0 in a zeroizing allocation; Debug exposes no
/// plaintext/context/digest. Borrowed accessors cannot mint Fresh or create a host. The separate
/// opaque-source seed builder requires an actual SDK admission and returns only unbound DATA.
pub struct OpenedBootstrapSourceData {
    plaintext: Zeroizing<Vec<u8>>,
    context: BootstrapSourceContextData,
    ranges: PayloadRanges,
    pub(super) id: BootstrapSourceId,
    pub(super) genesis: VerifiedGenesisData,
}

/// Exact objects/image retained from the real authenticated record open. This is public
/// consistency DATA only; no key, live original, Saved receipt or Current authority is kept.
#[derive(Clone)]
pub(super) struct VerifiedGenesisData {
    pub profile: SignedOwnerProfile,
    pub admission: AdmissionBody,
    pub header: SignedOwnerHeader,
    pub image: Vec<u8>,
    pub frontier: catcoms_replication::chat::acceptance_tree::Frontier,
}
pub struct BootstrapSourceDataView<'a> {
    pub context: &'a BootstrapSourceContextData,
    pub snapshot: &'a [u8],
    pub profile: &'a [u8],
    pub records_frame: &'a [u8],
    pub manifest_frame: &'a [u8],
}
impl OpenedBootstrapSourceData {
    /// Whole sealed source identity DATA, not a durable source capture or writer result.
    pub fn source_id_data(&self) -> BootstrapSourceId {
        self.id
    }
    pub fn view_data(&self) -> BootstrapSourceDataView<'_> {
        BootstrapSourceDataView {
            context: &self.context,
            snapshot: &self.plaintext[self.ranges.snapshot.clone()],
            profile: &self.plaintext[self.ranges.profile.clone()],
            records_frame: &self.plaintext[self.ranges.records.clone()],
            manifest_frame: &self.plaintext[self.ranges.manifest.clone()],
        }
    }
}
impl std::fmt::Debug for OpenedBootstrapSourceData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedBootstrapSourceData")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for BootstrapSourceDataView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapSourceDataView")
            .finish_non_exhaustive()
    }
}

/// Expected full context/whole SourceId MUST be independently retained before seedwrite. This
/// performs no SDK/serde expansion. The separately paid caller owns raw/debt/file buffers too;
/// decrypted limits cannot authorize an allocation that already happened. During AEAD, raw,
/// copied cipher and plaintext capacity overlap; cipher drops BEFORE nested bounded pages.
pub fn open_bootstrap_source_data(
    dbkey: &[u8; 32],
    expected: &BootstrapSourceContextData,
    expected_id: BootstrapSourceId,
    frame: &[u8],
) -> Result<OpenedBootstrapSourceData, SyncError> {
    let parsed = parse_frame(frame)?;
    if BootstrapSourceId::of(frame) != expected_id {
        return Err(bad());
    }
    let plaintext = {
        let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, dbkey));
        let sealed = SealedBlob {
            nonce: parsed.nonce,
            ciphertext: parsed.ciphertext.to_vec(),
        };
        Zeroizing::new(unseal(&key, &sealed).map_err(|_| bad())?)
    };
    let (ranges, genesis) = {
        let body = parse_body(&plaintext)?;
        if body.spec != expected.spec_data()
            || snapshot::digest(body.spec.profile, body.snapshot).as_bytes()
                != &body.spec.snapshot_digest
        {
            return Err(bad());
        }
        s0::preflight(body.snapshot, body.spec)?;
        let genesis = validate_genesis(dbkey, &body)?;
        (body.ranges, genesis)
    };
    Ok(OpenedBootstrapSourceData {
        plaintext,
        context: expected.clone(),
        ranges,
        id: expected_id,
        genesis,
    })
}

fn validate_genesis(key: &[u8; 32], body: &Body<'_>) -> Result<VerifiedGenesisData, SyncError> {
    let spec = body.spec;
    let profile = SignedOwnerProfile::decode(body.profile).map_err(|_| bad())?;
    let mut creation = blake3::Hasher::new();
    creation.update(b"mewtual/chat/founder-creation/v2\0");
    creation.update(&(spec.group.len() as u32).to_be_bytes());
    creation.update(spec.group);
    creation.update(&spec.signer);
    creation.update(&spec.fresh_attempt);
    if !profile.verify()
        || profile.id() != spec.profile
        || profile.spec().group != spec.group
        || profile.spec().founder != spec.signer
        || profile.spec().creation != *creation.finalize().as_bytes()
    {
        return Err(bad());
    }
    let operation = ArchiveOperation::Genesis {
        body: spec.admission,
    };
    {
        let plain = open_private_chat_page(key, spec.manifest, body.manifest).map_err(|_| bad())?;
        if plain.len() > MAX_MANIFEST_PAGE_BYTES {
            return Err(bad());
        }
        let manifest = ManifestPage::decode(&plain).map_err(|_| bad())?;
        if manifest.vault_domain() != &spec.vault
            || manifest.group() != spec.group
            || manifest.profile() != spec.profile
            || manifest.sequence() != 1
            || manifest.previous_manifest().is_some()
            || manifest.original_source_head().is_some()
            || manifest.before_header().is_some()
            || manifest.after_header() != spec.header
            || manifest.operation() != operation
            || manifest.page_count() != 1
            || manifest.page(0).map_err(|_| bad())?
                != (PageDescriptor {
                    role: PageRole::GenesisRecords,
                    page: spec.records,
                    plaintext_bytes: spec.records_plaintext_bytes,
                    entries: 3,
                })
        {
            return Err(bad());
        }
    } // Manifest plaintext/typed metadata drops before record plaintext opens.
    let plain = open_private_chat_page(key, spec.records, body.records).map_err(|_| bad())?;
    if plain.len() != spec.records_plaintext_bytes {
        return Err(bad());
    }
    let batch = RecordBatch::decode(
        &plain,
        BatchBinding {
            vault: &spec.vault,
            profile: &profile,
            role: PageRole::GenesisRecords,
            operation: &operation,
            append: None,
        },
    )
    .map_err(|_| bad())?;
    let mut found_profile = false;
    let mut found_header = None;
    let mut found_admission = None;
    for row in batch.rows() {
        match row.key.kind {
            RecordKind::Profile => {
                if row.payload != body.profile {
                    return Err(bad());
                }
                found_profile = true;
            }
            RecordKind::Header => {
                let header = HeaderRow::decode(row.payload).map_err(|_| bad())?;
                if header.header().id() != spec.header {
                    return Err(bad());
                }
                // Keep the actual representative and EXACT image, not a normalized re-encode.
                found_header = Some((header.header().clone(), header.image().to_vec()));
            }
            RecordKind::Admission => {
                let admission = AdmissionBody::decode(row.payload).map_err(|_| bad())?;
                if admission.id() != spec.admission {
                    return Err(bad());
                }
                found_admission = Some(admission);
            }
            _ => return Err(bad()),
        }
    }
    // RecordBatch checked every actual signed representative, exact founder/body/header/state
    // genesis consistency and unique sorted kinds before this exact original-frame comparison.
    if !found_profile {
        return Err(bad());
    }
    let (header, image) = found_header.ok_or_else(bad)?;
    let admission = found_admission.ok_or_else(bad)?;
    let frontier = catcoms_replication::chat::acceptance_tree::Frontier::empty(
        catcoms_replication::chat::acceptance_tree::TreeContext {
            profile: profile.id(),
            admission: admission.admission_id(),
        },
    );
    Ok(VerifiedGenesisData {
        profile,
        admission,
        header,
        image,
        frontier,
    })
}
