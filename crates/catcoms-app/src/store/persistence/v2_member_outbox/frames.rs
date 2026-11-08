//! Exact member-outbox file formats. Pure DATA: no file, permission or selection is created.
//!
//! Two encryption families are used, each with its own key domain:
//!
//! * Fixed control files (`head.bin`, `head.stage.v1`, `outbox.wal`, `outbox.wal.stage.v1`)
//!   use a control envelope: a deterministic public preamble, a u32 sealed length, then
//!   `nonce || ciphertext` under a member-outbox-specific derived key. The plaintext repeats
//!   the preamble, which is how the otherwise unauthenticated public preamble is bound.
//! * Immutable objects (action chunks `a-`, entries `e-`, trie nodes `n-`) are private chat
//!   pages ([`PreparedPrivateChatPage`]). That page key is shared with other private chat
//!   pages, so every member page plaintext starts with its own closed role tag and the context
//!   ID. A founder page or another member's page therefore never decodes as one of ours.
//!
//! The preamble is deterministic from (kind, context, binding) so a cold reader can recognise
//! a torn prefix of its OWN fixed pre-commit stage without trusting any byte it cannot rebuild.
use super::data::{
    b32, end, field, finish, input, put, refused, tag, u16_of, u32_of, u64_of, u8_of, MemberHeadId,
    MemberOutboxContextId, MemberRequestDigest, MemberRequestToken, VERSION,
};
use crate::AppError;
use catcoms_crypto::{seal, unseal, SealedBlob};
use catcoms_replication::chat::cold_actions::{
    OriginalActionDigest, MAX_ACTION_CHUNKS, MAX_ACTION_CHUNK_PAGE_BYTES,
};
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::ActionId;
use catcoms_rt::CryptoRngCore;
use catcoms_storage::private_chat_page::{PrivateChatPageId, MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

/// Private page frame overhead (u32 domain length, domain, version, nonce, u32 length, tag).
/// Pinned by a test against the real codec, so a codec change cannot silently widen quotas.
pub(super) const PAGE_FRAME_OVERHEAD: usize = 4 + 36 + 2 + 24 + 4 + 16;
/// Control envelope sealing overhead: nonce plus AEAD tag.
const SEAL_OVERHEAD: usize = 24 + 16;

/// Largest head plaintext body (after the repeated preamble).
const HEAD_BODY_CAP: usize = 256;
/// Largest complete `head.bin` file, preamble included.
pub(super) const HEAD_FILE_CAP: usize = 1024;
/// Largest WAL plaintext. Every transaction is refused BEFORE admission if it would exceed it.
pub(super) const WAL_PLAIN_CAP: usize = 1024 * 1024;
/// Largest complete WAL file.
pub(super) const WAL_FILE_CAP: usize = WAL_PLAIN_CAP + 256;

const CHUNK_DOMAIN: &[u8] = b"catcoms/member-outbox-action-chunk/v1\0";
const ENTRY_DOMAIN: &[u8] = b"catcoms/member-outbox-entry/v1\0";

/// One wrapped action chunk page plaintext: role tag, context, ordinal, index, general chunk.
pub(super) const CHUNK_PLAIN_CAP: usize =
    4 + CHUNK_DOMAIN.len() + 2 + 4 + 32 + 8 + 1 + 4 + MAX_ACTION_CHUNK_PAGE_BYTES;
const _: () = assert!(CHUNK_PLAIN_CAP <= MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES);
pub(super) const ENTRY_PLAIN_CAP: usize =
    4 + ENTRY_DOMAIN.len() + 2 + 4 + 32 + 8 + 6 * (4 + 32) + 4 + 1 + MAX_ACTION_CHUNKS * (4 + 32);
const _: () = assert!(ENTRY_PLAIN_CAP <= MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ControlKind {
    Head,
    Wal,
}
impl ControlKind {
    fn magic(self) -> &'static [u8] {
        match self {
            Self::Head => b"catcoms/member-outbox-head/v1\0",
            Self::Wal => b"catcoms/member-outbox-wal/v1\0",
        }
    }
    fn key_domain(self) -> &'static str {
        match self {
            Self::Head => "catcoms/member-outbox-head-key/v1",
            Self::Wal => "catcoms/member-outbox-wal-key/v1",
        }
    }
    fn plain_cap(self) -> usize {
        match self {
            Self::Head => HEAD_FILE_CAP - 64,
            Self::Wal => WAL_PLAIN_CAP,
        }
    }
    pub(super) fn file_cap(self) -> usize {
        match self {
            Self::Head => HEAD_FILE_CAP,
            Self::Wal => WAL_FILE_CAP,
        }
    }
    /// Exact deterministic public preamble: `u32 len || magic || u16 version || cid || binding`.
    /// For Head the binding is the parent head ID (zero for generation 0); for Wal it is the
    /// parent head the transaction starts from.
    pub(super) fn preamble(self, context: MemberOutboxContextId, binding: &[u8; 32]) -> Vec<u8> {
        let magic = self.magic();
        let mut out = Vec::with_capacity(self.preamble_len());
        out.extend_from_slice(&(magic.len() as u32).to_be_bytes());
        out.extend_from_slice(magic);
        out.extend_from_slice(&VERSION.to_be_bytes());
        out.extend_from_slice(context.as_bytes());
        out.extend_from_slice(binding);
        out
    }
    pub(super) fn preamble_len(self) -> usize {
        4 + self.magic().len() + 2 + 32 + 32
    }
}
const _: () = assert!(HEAD_FILE_CAP - 64 >= 4 + 32 + 2 + 64 + 4 + HEAD_BODY_CAP);

/// Seal a control file. The preamble is both the public prefix and the plaintext's first
/// bytes, so opening can authenticate every public byte it relied on to classify the file.
pub(super) fn seal_control(
    kind: ControlKind,
    key: &[u8; 32],
    context: MemberOutboxContextId,
    binding: &[u8; 32],
    body: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let preamble = kind.preamble(context, binding);
    let plain_len = preamble.len().checked_add(body.len()).ok_or_else(refused)?;
    if body.is_empty() || plain_len > kind.plain_cap() {
        return Err(refused());
    }
    let mut plain = Zeroizing::new(Vec::with_capacity(plain_len));
    plain.extend_from_slice(&preamble);
    plain.extend_from_slice(body);
    let derived = Zeroizing::new(blake3::derive_key(kind.key_domain(), key));
    let sealed = seal(&derived, &plain, rng)?;
    let sealed_len = SEAL_OVERHEAD + plain.len();
    if sealed.ciphertext.len() + 24 != sealed_len {
        return Err(refused());
    }
    let mut file = Zeroizing::new(Vec::with_capacity(preamble.len() + 4 + sealed_len));
    file.extend_from_slice(&preamble);
    file.extend_from_slice(&(sealed_len as u32).to_be_bytes());
    file.extend_from_slice(&sealed.nonce);
    file.extend_from_slice(&sealed.ciphertext);
    if file.len() > kind.file_cap() {
        return Err(refused());
    }
    Ok(file)
}

/// Authenticated control body plus the binding read from its (now authenticated) preamble.
pub(super) struct OpenedControl {
    pub(super) binding: [u8; 32],
    pub(super) body: Zeroizing<Vec<u8>>,
}

/// Open a COMPLETE control file. Every failure is a refusal, never absence.
pub(super) fn open_control(
    kind: ControlKind,
    key: &[u8; 32],
    context: MemberOutboxContextId,
    raw: &[u8],
) -> Result<OpenedControl, AppError> {
    let pre = kind.preamble_len();
    if raw.len() <= pre + 4 + SEAL_OVERHEAD || raw.len() > kind.file_cap() {
        return Err(refused());
    }
    let binding: [u8; 32] = raw[pre - 32..pre].try_into().map_err(|_| refused())?;
    if raw[..pre] != kind.preamble(context, &binding)[..] {
        return Err(refused());
    }
    let declared = u32::from_be_bytes(raw[pre..pre + 4].try_into().map_err(|_| refused())?);
    let sealed = &raw[pre + 4..];
    if declared as usize != sealed.len() {
        return Err(refused());
    }
    let mut nonce = [0; 24];
    nonce.copy_from_slice(&sealed[..24]);
    let blob = SealedBlob {
        nonce,
        ciphertext: sealed[24..].to_vec(),
    };
    let derived = Zeroizing::new(blake3::derive_key(kind.key_domain(), key));
    let plain = Zeroizing::new(unseal(&derived, &blob)?);
    if plain.len() <= pre || plain.len() > kind.plain_cap() || plain[..pre] != raw[..pre] {
        return Err(refused());
    }
    Ok(OpenedControl {
        binding,
        body: Zeroizing::new(plain[pre..].to_vec()),
    })
}

/// Shape of a fixed pre-commit stage file, judged ONLY against the exact preamble a cold
/// reader can rebuild from its own context and currently selected binding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum StageShape {
    /// A strict prefix of the expected preamble (including the empty file).
    PreambleTorn,
    /// The exact expected preamble, then fewer sealed bytes than the file itself declared.
    BodyTorn,
    /// The exact expected preamble and exactly its declared length. Still unauthenticated:
    /// the caller must open it and preserve it if that fails.
    Complete,
    /// Anything else: a different preamble, an impossible length or trailing bytes.
    Unknown,
}

pub(super) fn classify_stage(
    kind: ControlKind,
    context: MemberOutboxContextId,
    binding: &[u8; 32],
    raw: &[u8],
) -> StageShape {
    let expected = kind.preamble(context, binding);
    if raw.len() < expected.len() {
        return if raw[..] == expected[..raw.len()] {
            StageShape::PreambleTorn
        } else {
            StageShape::Unknown
        };
    }
    if raw[..expected.len()] != expected[..] {
        return StageShape::Unknown;
    }
    let rest = &raw[expected.len()..];
    if rest.len() < 4 {
        // The length field itself is torn; only our writer emits this fixed preamble here.
        return StageShape::BodyTorn;
    }
    let declared = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    if declared <= SEAL_OVERHEAD || expected.len() + 4 + declared > kind.file_cap() {
        return StageShape::Unknown;
    }
    match (rest.len() - 4).cmp(&declared) {
        std::cmp::Ordering::Less => StageShape::BodyTorn,
        std::cmp::Ordering::Equal => StageShape::Complete,
        std::cmp::Ordering::Greater => StageShape::Unknown,
    }
}

/// The selected local Head. Counters are persistent physical accounting, not estimates:
/// `live_*` cover every immutable object reachable from this head (entries, chunks and trie
/// nodes); control files are bounded separately by the transaction quota check.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct HeadData {
    pub(super) generation: u64,
    pub(super) parent: [u8; 32],
    pub(super) root: Option<PrivateChatPageId>,
    pub(super) next_ordinal: u64,
    pub(super) front: u64,
    pub(super) live_bytes: u64,
    pub(super) live_objects: u64,
}
impl HeadData {
    pub(super) fn initial() -> Self {
        Self {
            generation: 0,
            parent: [0; 32],
            root: None,
            next_ordinal: 0,
            front: 0,
            live_bytes: 0,
            live_objects: 0,
        }
    }
    /// Unsent publication debt: entries `front..next_ordinal`.
    pub(super) fn pending(&self) -> u64 {
        self.next_ordinal - self.front
    }
    fn encode_body(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(HEAD_BODY_CAP);
        e.put_u64(self.generation);
        e.put_bytes(&self.parent).map_err(|_| refused())?;
        match self.root {
            None => e.put_u8(0),
            Some(root) => e
                .put_u8(1)
                .put_bytes(root.as_bytes())
                .map_err(|_| refused())?,
        };
        e.put_u64(self.next_ordinal)
            .put_u64(self.front)
            .put_u64(self.live_bytes)
            .put_u64(self.live_objects);
        finish(e, HEAD_BODY_CAP)
    }
    fn decode_body(raw: &[u8]) -> Result<Self, AppError> {
        if raw.len() > HEAD_BODY_CAP {
            return Err(refused());
        }
        let mut d = Decoder::new(raw);
        let generation = u64_of(&mut d)?;
        let parent = b32(&mut d)?;
        let root = match u8_of(&mut d)? {
            0 => None,
            1 => Some(PrivateChatPageId::from_bytes(b32(&mut d)?)),
            _ => return Err(refused()),
        };
        let head = Self {
            generation,
            parent,
            root,
            next_ordinal: u64_of(&mut d)?,
            front: u64_of(&mut d)?,
            live_bytes: u64_of(&mut d)?,
            live_objects: u64_of(&mut d)?,
        };
        end(d)?;
        // Structural invariants every head satisfies; anything else is not ours.
        if head.front > head.next_ordinal
            || (head.generation == 0) != (head.parent == [0; 32])
            || (head.root.is_none()) != (head.next_ordinal == 0)
            || (head.live_objects == 0) != (head.live_bytes == 0)
        {
            return Err(refused());
        }
        Ok(head)
    }
    /// Seal into an exact `head.bin` file. Its random nonce makes the file ID unique.
    pub(super) fn seal_file(
        &self,
        key: &[u8; 32],
        context: MemberOutboxContextId,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let body = self.encode_body()?;
        seal_control(ControlKind::Head, key, context, &self.parent, &body, rng)
    }
    pub(super) fn open_file(
        key: &[u8; 32],
        context: MemberOutboxContextId,
        raw: &[u8],
    ) -> Result<Self, AppError> {
        let opened = open_control(ControlKind::Head, key, context, raw)?;
        let head = Self::decode_body(&opened.body)?;
        if head.parent != opened.binding {
            return Err(refused());
        }
        Ok(head)
    }
}

/// The exact selected head: its parsed DATA, exact file bytes and file identity.
#[derive(Clone, Debug)]
pub(super) struct SelectedHead {
    pub(super) data: HeadData,
    pub(super) id: MemberHeadId,
    pub(super) file: Zeroizing<Vec<u8>>,
}
impl SelectedHead {
    pub(super) fn open(
        key: &[u8; 32],
        context: MemberOutboxContextId,
        file: Zeroizing<Vec<u8>>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            data: HeadData::open_file(key, context, &file)?,
            id: MemberHeadId::of_file(&file),
            file,
        })
    }
}

/// Closed roles of immutable objects; each has its own file prefix.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ObjectRole {
    Chunk,
    Entry,
    Node,
}
impl ObjectRole {
    pub(super) fn prefix(self) -> &'static str {
        match self {
            Self::Chunk => "a-",
            Self::Entry => "e-",
            Self::Node => "n-",
        }
    }
    fn code(self) -> u8 {
        match self {
            Self::Chunk => 1,
            Self::Entry => 2,
            Self::Node => 3,
        }
    }
    fn of_code(code: u8) -> Result<Self, AppError> {
        match code {
            1 => Ok(Self::Chunk),
            2 => Ok(Self::Entry),
            3 => Ok(Self::Node),
            _ => Err(refused()),
        }
    }
}

/// One exact immutable output: its role, page identity and complete sealed frame.
#[derive(Clone)]
pub(super) struct OutputData {
    pub(super) role: ObjectRole,
    pub(super) id: PrivateChatPageId,
    pub(super) frame: Zeroizing<Vec<u8>>,
}
impl std::fmt::Debug for OutputData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputData")
            .field("role", &self.role)
            .field("bytes", &self.frame.len())
            .finish_non_exhaustive()
    }
}

/// A node the transaction makes unreachable. Deleted only after the selector CAS.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct RetiredData {
    pub(super) id: PrivateChatPageId,
    pub(super) bytes: u32,
}

/// A settled publication-debt item recorded by a checkpoint.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct SettledData {
    pub(super) ordinal: u64,
    pub(super) entry: PrivateChatPageId,
    pub(super) disposition: DispositionCode,
    /// The current Admission the publisher presented when settling this item.
    pub(super) admission: OwnerFactId,
}

/// The durable disposition of a debt item the front moved over. Audit DATA only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::store) enum DispositionCode {
    /// Its exact original was handed to transport under its own original Admission. This
    /// is local submission, not peer receipt, durability confirmation or certification.
    Submitted,
    /// Not published: the publisher's current Admission differs from the entry's original
    /// one (removal/rejoin). The original stays in the audit for a separately authorized
    /// moderation path; this outbox never republishes it under the new Admission.
    Held,
}
impl DispositionCode {
    pub(super) fn code(self) -> u8 {
        match self {
            Self::Submitted => 1,
            Self::Held => 2,
        }
    }
    pub(super) fn of_code(code: u8) -> Result<Self, AppError> {
        match code {
            1 => Ok(Self::Submitted),
            2 => Ok(Self::Held),
            _ => Err(refused()),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum WalKind {
    Append,
    Checkpoint,
}

/// A complete typed WAL transition: parent head (the preamble binding), the exact candidate
/// head file, every immutable output, every retiree and every settled debt item.
#[derive(Debug)]
pub(super) struct WalData {
    pub(super) kind: WalKind,
    pub(super) parent: MemberHeadId,
    pub(super) candidate: Zeroizing<Vec<u8>>,
    pub(super) outputs: Vec<OutputData>,
    pub(super) retired: Vec<RetiredData>,
    pub(super) settled: Vec<SettledData>,
}
/// Per-transaction hard caps, enforced before admission and again on decode.
pub(super) const MAX_OUTPUTS: usize = 1 + MAX_ACTION_CHUNKS + MAX_NODE_OUTPUTS;
pub(super) const MAX_NODE_OUTPUTS: usize = 160;
pub(super) const MAX_NODE_OUTPUT_BYTES: usize = 256 * 1024;
/// Covers the production worst case: 8 settled inserts x 17 path nodes = 136 replaced nodes.
pub(super) const MAX_RETIRED: usize = 160;
pub(super) const MAX_SETTLED: usize = 8;
const _: () = assert!(
    1 + 4
        + HEAD_FILE_CAP
        + 2
        + (1 + MAX_ACTION_CHUNKS) * (1 + 4 + 32 + 4 + PAGE_FRAME_OVERHEAD + CHUNK_PLAIN_CAP)
        + MAX_NODE_OUTPUTS * (1 + 4 + 32 + 4)
        + MAX_NODE_OUTPUT_BYTES
        + 2
        + MAX_RETIRED * (4 + 32 + 4)
        + 2
        + MAX_SETTLED * (8 + 4 + 32 + 1 + 4 + 32)
        + 4 * 64
        <= WAL_PLAIN_CAP
);

impl WalData {
    fn encode_body(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if self.outputs.len() > MAX_OUTPUTS
            || self.retired.len() > MAX_RETIRED
            || self.settled.len() > MAX_SETTLED
        {
            return Err(refused());
        }
        let mut e = Encoder::new();
        e.put_u8(match self.kind {
            WalKind::Append => 1,
            WalKind::Checkpoint => 2,
        });
        put(&mut e, &self.candidate)?;
        e.put_u16(self.outputs.len() as u16);
        for output in &self.outputs {
            e.put_u8(output.role.code());
            put(&mut e, output.id.as_bytes())?;
            put(&mut e, &output.frame)?;
        }
        e.put_u16(self.retired.len() as u16);
        for retired in &self.retired {
            put(&mut e, retired.id.as_bytes())?;
            e.put_u32(retired.bytes);
        }
        e.put_u16(self.settled.len() as u16);
        for settled in &self.settled {
            e.put_u64(settled.ordinal);
            put(&mut e, settled.entry.as_bytes())?;
            e.put_u8(settled.disposition.code());
            put(&mut e, settled.admission.as_bytes())?;
        }
        finish(e, WAL_PLAIN_CAP - 256)
    }
    fn decode_body(parent: MemberHeadId, raw: &[u8]) -> Result<Self, AppError> {
        let mut d = Decoder::new(raw);
        let kind = match u8_of(&mut d)? {
            1 => WalKind::Append,
            2 => WalKind::Checkpoint,
            _ => return Err(refused()),
        };
        let candidate = Zeroizing::new(field(&mut d, HEAD_FILE_CAP)?.to_vec());
        let count = usize::from(u16_of(&mut d)?);
        if count > MAX_OUTPUTS {
            return Err(refused());
        }
        let mut outputs = Vec::with_capacity(count);
        for _ in 0..count {
            let role = ObjectRole::of_code(u8_of(&mut d)?)?;
            let id = PrivateChatPageId::from_bytes(b32(&mut d)?);
            let frame = Zeroizing::new(
                field(
                    &mut d,
                    MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES + PAGE_FRAME_OVERHEAD,
                )?
                .to_vec(),
            );
            outputs.push(OutputData { role, id, frame });
        }
        let count = usize::from(u16_of(&mut d)?);
        if count > MAX_RETIRED {
            return Err(refused());
        }
        let mut retired = Vec::with_capacity(count);
        for _ in 0..count {
            retired.push(RetiredData {
                id: PrivateChatPageId::from_bytes(b32(&mut d)?),
                bytes: u32_of(&mut d)?,
            });
        }
        let count = usize::from(u16_of(&mut d)?);
        if count > MAX_SETTLED {
            return Err(refused());
        }
        let mut settled = Vec::with_capacity(count);
        for _ in 0..count {
            settled.push(SettledData {
                ordinal: u64_of(&mut d)?,
                entry: PrivateChatPageId::from_bytes(b32(&mut d)?),
                disposition: DispositionCode::of_code(u8_of(&mut d)?)?,
                admission: OwnerFactId::from_bytes(b32(&mut d)?),
            });
        }
        end(d)?;
        Ok(Self {
            kind,
            parent,
            candidate,
            outputs,
            retired,
            settled,
        })
    }
    pub(super) fn seal_file(
        &self,
        key: &[u8; 32],
        context: MemberOutboxContextId,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let body = self.encode_body()?;
        seal_control(
            ControlKind::Wal,
            key,
            context,
            self.parent.as_bytes(),
            &body,
            rng,
        )
    }
    pub(super) fn open_file(
        key: &[u8; 32],
        context: MemberOutboxContextId,
        raw: &[u8],
    ) -> Result<Self, AppError> {
        let opened = open_control(ControlKind::Wal, key, context, raw)?;
        Self::decode_body(MemberHeadId::from_bytes(opened.binding), &opened.body)
    }
}

/// One immutable entry: everything needed to find, authenticate and attribute one original.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct EntryData {
    pub(super) ordinal: u64,
    pub(super) token: MemberRequestToken,
    pub(super) request: MemberRequestDigest,
    pub(super) admission: OwnerFactId,
    pub(super) action: ActionId,
    pub(super) original: OriginalActionDigest,
    pub(super) original_len: u32,
    pub(super) chunks: Vec<PrivateChatPageId>,
}
impl EntryData {
    pub(super) fn encode(
        &self,
        context: MemberOutboxContextId,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if self.chunks.is_empty() || self.chunks.len() > MAX_ACTION_CHUNKS {
            return Err(refused());
        }
        let mut e = Encoder::with_capacity(ENTRY_PLAIN_CAP);
        tag(&mut e, ENTRY_DOMAIN)?;
        put(&mut e, context.as_bytes())?;
        e.put_u64(self.ordinal);
        put(&mut e, self.token.as_bytes())?;
        put(&mut e, self.request.as_bytes())?;
        put(&mut e, self.admission.as_bytes())?;
        put(&mut e, self.action.as_bytes())?;
        put(&mut e, self.original.as_bytes())?;
        // A sixth fixed field keeps the cap formula honest: reserved, must be zero.
        put(&mut e, &[0; 32])?;
        e.put_u32(self.original_len);
        e.put_u8(self.chunks.len() as u8);
        for chunk in &self.chunks {
            put(&mut e, chunk.as_bytes())?;
        }
        finish(e, ENTRY_PLAIN_CAP)
    }
    pub(super) fn decode(context: MemberOutboxContextId, raw: &[u8]) -> Result<Self, AppError> {
        let mut d = input(raw, ENTRY_PLAIN_CAP, ENTRY_DOMAIN)?;
        if b32(&mut d)? != *context.as_bytes() {
            return Err(refused());
        }
        let ordinal = u64_of(&mut d)?;
        let token = MemberRequestToken::from_bytes(b32(&mut d)?);
        let request = MemberRequestDigest::from_bytes(b32(&mut d)?);
        let admission = OwnerFactId::from_bytes(b32(&mut d)?);
        let action = ActionId::from_bytes(b32(&mut d)?);
        let original = OriginalActionDigest::from_bytes(b32(&mut d)?);
        if b32(&mut d)? != [0; 32] {
            return Err(refused());
        }
        let original_len = u32_of(&mut d)?;
        let count = usize::from(u8_of(&mut d)?);
        if count == 0 || count > MAX_ACTION_CHUNKS {
            return Err(refused());
        }
        let mut chunks = Vec::with_capacity(count);
        for _ in 0..count {
            chunks.push(PrivateChatPageId::from_bytes(b32(&mut d)?));
        }
        end(d)?;
        Ok(Self {
            ordinal,
            token,
            request,
            admission,
            action,
            original,
            original_len,
            chunks,
        })
    }
}

/// Wrap one general cold-action chunk with this outbox's role tag, context, ordinal and index.
pub(super) fn encode_chunk_page(
    context: MemberOutboxContextId,
    ordinal: u64,
    index: usize,
    chunk: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if index >= MAX_ACTION_CHUNKS || chunk.is_empty() || chunk.len() > MAX_ACTION_CHUNK_PAGE_BYTES {
        return Err(refused());
    }
    let mut e = Encoder::with_capacity(CHUNK_PLAIN_CAP);
    tag(&mut e, CHUNK_DOMAIN)?;
    put(&mut e, context.as_bytes())?;
    e.put_u64(ordinal);
    e.put_u8(index as u8);
    put(&mut e, chunk)?;
    finish(e, CHUNK_PLAIN_CAP)
}

/// Unwrap a chunk page, returning the borrowed general chunk bytes for `ActionChunk::decode`.
pub(super) fn decode_chunk_page<'a>(
    context: MemberOutboxContextId,
    ordinal: u64,
    index: usize,
    raw: &'a [u8],
) -> Result<&'a [u8], AppError> {
    let mut d = input(raw, CHUNK_PLAIN_CAP, CHUNK_DOMAIN)?;
    if b32(&mut d)? != *context.as_bytes()
        || u64_of(&mut d)? != ordinal
        || usize::from(u8_of(&mut d)?) != index
    {
        return Err(refused());
    }
    let chunk = field(&mut d, MAX_ACTION_CHUNK_PAGE_BYTES)?;
    end(d)?;
    Ok(chunk)
}
