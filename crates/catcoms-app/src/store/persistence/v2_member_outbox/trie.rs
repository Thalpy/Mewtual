//! Authenticated copy-on-write hash trie: the paged receipt index and publication-debt map.
//!
//! One trie holds four row kinds, each under its own full 33-byte key (kind + 32 raw bytes):
//!
//! * `Token`: request token -> receipt (ordinal, entry, request digest, ActionId, original).
//! * `Ordinal`: entry ordinal -> entry page. This is the paged publication debt: the head's
//!   `front..next_ordinal` range names the unsent ordinals.
//! * `Action`: semantic ActionId -> ordinal and original-byte digest. It refuses a second save
//!   of an already stored action under another token, even when its signature bytes differ.
//! * `Disposition`: ordinal -> what a publication checkpoint decided (audit after debt clears).
//!
//! A row's position is `blake3(domain || context || kind || raw)`, consumed one nibble per
//! level. Fanout is 16, a leaf holds at most [`TrieLimits::leaf_rows`] rows and the depth is
//! bounded; an overfull leaf splits recursively. Rows are never removed. Nodes are immutable
//! private pages; an insert copies its path and the replaced nodes become retirees that the
//! transaction deletes only after its selector CAS (see `transaction.rs`).
//!
//! Every decoded node is checked against the position it was reached from (depth and nibble
//! prefix), its rows' own computed positions, strict row order and the context. Full keys are
//! always compared: two keys with equal position prefixes are distinct rows.
use super::data::{
    b32, end, field, finish, input, put, refused, tag, u16_of, u64_of, u8_of,
    MemberOutboxContextId, MemberRequestDigest, MemberRequestToken,
};
use super::frames::{
    DispositionCode, EntryData, RetiredData, SettledData, MAX_NODE_OUTPUTS, MAX_NODE_OUTPUT_BYTES,
    MAX_RETIRED, PAGE_FRAME_OVERHEAD,
};
use crate::AppError;
use catcoms_replication::chat::cold_actions::OriginalActionDigest;
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::ActionId;
use catcoms_storage::private_chat_page::PrivateChatPageId;
use catcoms_wire::Encoder;
use std::cmp::Ordering;
use zeroize::Zeroizing;

pub(super) const FANOUT: usize = 16;
const NODE_DOMAIN: &[u8] = b"catcoms/member-outbox-node/v1\0";
const PATH_DOMAIN: &[u8] = b"catcoms/member-outbox-trie-path/v1\0";
/// Decode-time ceilings. They equal the production limits; smaller test limits still decode.
const DECODE_LEAF_ROWS: usize = 32;
const DECODE_MAX_DEPTH: usize = 16;
/// Largest node plaintext: fixed header, then 32 rows of at most 189 bytes.
pub(super) const NODE_PLAIN_CAP: usize = 8 * 1024;
const _: () = assert!(
    4 + NODE_DOMAIN.len() + 2 + 36 + 1 + 4 + DECODE_MAX_DEPTH + 1 + 2 + DECODE_LEAF_ROWS * 189
        <= NODE_PLAIN_CAP
);
const _: () = assert!(NODE_PLAIN_CAP + PAGE_FRAME_OVERHEAD <= MAX_NODE_OUTPUT_BYTES);

/// Physical trie shape. Production uses [`PRODUCTION_LIMITS`]; a test may shrink both to
/// force splits and depth exhaustion. Any tree written under smaller limits stays readable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct TrieLimits {
    pub(super) leaf_rows: usize,
    pub(super) max_depth: usize,
}
pub(super) const PRODUCTION_LIMITS: TrieLimits = TrieLimits {
    leaf_rows: DECODE_LEAF_ROWS,
    max_depth: DECODE_MAX_DEPTH,
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum RowKind {
    Token,
    Ordinal,
    Action,
    Disposition,
}
impl RowKind {
    fn code(self) -> u8 {
        match self {
            Self::Token => 1,
            Self::Ordinal => 2,
            Self::Action => 3,
            Self::Disposition => 4,
        }
    }
    fn of_code(code: u8) -> Result<Self, AppError> {
        match code {
            1 => Ok(Self::Token),
            2 => Ok(Self::Ordinal),
            3 => Ok(Self::Action),
            4 => Ok(Self::Disposition),
            _ => Err(refused()),
        }
    }
}

/// A full row key. Ordinal and Disposition keys carry the u64 big-endian in their last bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct RowKey {
    kind: RowKind,
    raw: [u8; 32],
}
impl RowKey {
    pub(super) fn token(token: MemberRequestToken) -> Self {
        Self {
            kind: RowKind::Token,
            raw: *token.as_bytes(),
        }
    }
    pub(super) fn ordinal(ordinal: u64) -> Self {
        Self {
            kind: RowKind::Ordinal,
            raw: ordinal_raw(ordinal),
        }
    }
    pub(super) fn action(action: ActionId) -> Self {
        Self {
            kind: RowKind::Action,
            raw: *action.as_bytes(),
        }
    }
    pub(super) fn disposition(ordinal: u64) -> Self {
        Self {
            kind: RowKind::Disposition,
            raw: ordinal_raw(ordinal),
        }
    }
    fn path(&self, context: MemberOutboxContextId) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(PATH_DOMAIN);
        h.update(context.as_bytes());
        h.update(&[self.kind.code()]);
        h.update(&self.raw);
        *h.finalize().as_bytes()
    }
}
fn ordinal_raw(ordinal: u64) -> [u8; 32] {
    let mut raw = [0; 32];
    raw[24..].copy_from_slice(&ordinal.to_be_bytes());
    raw
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct TokenRowData {
    pub(super) ordinal: u64,
    pub(super) entry: PrivateChatPageId,
    pub(super) request: MemberRequestDigest,
    pub(super) action: ActionId,
    pub(super) original: OriginalActionDigest,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct OrdinalRowData {
    pub(super) entry: PrivateChatPageId,
    pub(super) action: ActionId,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct ActionRowData {
    pub(super) ordinal: u64,
    pub(super) original: OriginalActionDigest,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct DispositionRowData {
    pub(super) code: DispositionCode,
    /// Generation of the head that recorded this disposition.
    pub(super) generation: u64,
    /// The current Admission the publisher presented when settling.
    pub(super) admission: OwnerFactId,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RowValue {
    Token(TokenRowData),
    Ordinal(OrdinalRowData),
    Action(ActionRowData),
    Disposition(DispositionRowData),
}
impl RowValue {
    fn kind(&self) -> RowKind {
        match self {
            Self::Token(_) => RowKind::Token,
            Self::Ordinal(_) => RowKind::Ordinal,
            Self::Action(_) => RowKind::Action,
            Self::Disposition(_) => RowKind::Disposition,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct Row {
    key: RowKey,
    path: [u8; 32],
    value: RowValue,
}
impl Row {
    fn new(context: MemberOutboxContextId, key: RowKey, value: RowValue) -> Result<Self, AppError> {
        if key.kind != value.kind() {
            return Err(refused());
        }
        Ok(Self {
            path: key.path(context),
            key,
            value,
        })
    }
    /// Total order: position first, then the FULL key. Equal positions never merge rows.
    fn order(&self, other: &Self) -> Ordering {
        (self.path, self.key.kind.code(), self.key.raw).cmp(&(
            other.path,
            other.key.kind.code(),
            other.key.raw,
        ))
    }
    fn encode(&self, e: &mut Encoder) -> Result<(), AppError> {
        e.put_u8(self.key.kind.code());
        put(e, &self.key.raw)?;
        match &self.value {
            RowValue::Token(v) => {
                put(e, v.entry.as_bytes())?;
                put(e, v.request.as_bytes())?;
                put(e, v.action.as_bytes())?;
                put(e, v.original.as_bytes())?;
                e.put_u64(v.ordinal);
            }
            RowValue::Ordinal(v) => {
                put(e, v.entry.as_bytes())?;
                put(e, v.action.as_bytes())?;
            }
            RowValue::Action(v) => {
                put(e, v.original.as_bytes())?;
                e.put_u64(v.ordinal);
            }
            RowValue::Disposition(v) => {
                e.put_u8(v.code.code());
                e.put_u64(v.generation);
                put(e, v.admission.as_bytes())?;
            }
        }
        Ok(())
    }
    fn decode(
        context: MemberOutboxContextId,
        d: &mut catcoms_wire::Decoder<'_>,
    ) -> Result<Self, AppError> {
        let kind = RowKind::of_code(u8_of(d)?)?;
        let raw = b32(d)?;
        let value = match kind {
            RowKind::Token => {
                let entry = PrivateChatPageId::from_bytes(b32(d)?);
                let request = MemberRequestDigest::from_bytes(b32(d)?);
                let action = ActionId::from_bytes(b32(d)?);
                let original = OriginalActionDigest::from_bytes(b32(d)?);
                RowValue::Token(TokenRowData {
                    ordinal: u64_of(d)?,
                    entry,
                    request,
                    action,
                    original,
                })
            }
            RowKind::Ordinal => RowValue::Ordinal(OrdinalRowData {
                entry: PrivateChatPageId::from_bytes(b32(d)?),
                action: ActionId::from_bytes(b32(d)?),
            }),
            RowKind::Action => {
                let original = OriginalActionDigest::from_bytes(b32(d)?);
                RowValue::Action(ActionRowData {
                    ordinal: u64_of(d)?,
                    original,
                })
            }
            RowKind::Disposition => RowValue::Disposition(DispositionRowData {
                code: DispositionCode::of_code(u8_of(d)?)?,
                generation: u64_of(d)?,
                admission: OwnerFactId::from_bytes(b32(d)?),
            }),
        };
        // Ordinal-shaped keys must really be ordinals; anything else is not our encoding.
        if matches!(kind, RowKind::Ordinal | RowKind::Disposition) && raw[..24] != [0; 24] {
            return Err(refused());
        }
        Self::new(context, RowKey { kind, raw }, value)
    }
}

fn nibble(path: &[u8; 32], depth: usize) -> u8 {
    let byte = path[depth / 2];
    if depth.is_multiple_of(2) {
        byte >> 4
    } else {
        byte & 15
    }
}

/// A node as decoded: either sorted rows or up to 16 child page IDs (boxed: 528 bytes).
enum DecodedNode {
    Leaf(Vec<Row>),
    Branch(Box<[Option<PrivateChatPageId>; FANOUT]>),
}

fn encode_node(
    context: MemberOutboxContextId,
    depth: usize,
    prefix: &[u8],
    node: &DecodedNodeRef<'_>,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if prefix.len() != depth || depth > DECODE_MAX_DEPTH {
        return Err(refused());
    }
    let mut e = Encoder::with_capacity(NODE_PLAIN_CAP);
    tag(&mut e, NODE_DOMAIN)?;
    put(&mut e, context.as_bytes())?;
    e.put_u8(depth as u8);
    put(&mut e, prefix)?;
    match node {
        DecodedNodeRef::Leaf(rows) => {
            if rows.is_empty() || rows.len() > DECODE_LEAF_ROWS {
                return Err(refused());
            }
            e.put_u8(1);
            e.put_u16(rows.len() as u16);
            for row in *rows {
                row.encode(&mut e)?;
            }
        }
        DecodedNodeRef::Branch(children) => {
            let mut bitmap = 0u16;
            for (n, child) in children.iter().enumerate() {
                if child.is_some() {
                    bitmap |= 1 << n;
                }
            }
            if bitmap == 0 || depth >= DECODE_MAX_DEPTH {
                return Err(refused());
            }
            e.put_u8(2);
            e.put_u16(bitmap);
            for child in children.iter().flatten() {
                put(&mut e, child.as_bytes())?;
            }
        }
    }
    finish(e, NODE_PLAIN_CAP)
}
enum DecodedNodeRef<'a> {
    Leaf(&'a [Row]),
    Branch(&'a [Option<PrivateChatPageId>; FANOUT]),
}

/// Decode one node. With `expected`, it must sit exactly at that depth on that key's path.
fn decode_node(
    context: MemberOutboxContextId,
    raw: &[u8],
    expected: Option<(usize, &[u8; 32])>,
) -> Result<(usize, Vec<u8>, DecodedNode), AppError> {
    let mut d = input(raw, NODE_PLAIN_CAP, NODE_DOMAIN)?;
    if b32(&mut d)? != *context.as_bytes() {
        return Err(refused());
    }
    let depth = usize::from(u8_of(&mut d)?);
    let prefix = field(&mut d, DECODE_MAX_DEPTH)?.to_vec();
    if depth > DECODE_MAX_DEPTH || prefix.len() != depth || prefix.iter().any(|n| *n >= 16) {
        return Err(refused());
    }
    if let Some((want, path)) = expected {
        if depth != want || (0..depth).any(|i| prefix[i] != nibble(path, i)) {
            return Err(refused());
        }
    }
    let node = match u8_of(&mut d)? {
        1 => {
            let count = usize::from(u16_of(&mut d)?);
            if count == 0 || count > DECODE_LEAF_ROWS {
                return Err(refused());
            }
            let mut rows: Vec<Row> = Vec::with_capacity(count);
            for _ in 0..count {
                let row = Row::decode(context, &mut d)?;
                if (0..depth).any(|i| nibble(&row.path, i) != prefix[i])
                    || rows
                        .last()
                        .is_some_and(|last| last.order(&row) != Ordering::Less)
                {
                    return Err(refused());
                }
                rows.push(row);
            }
            DecodedNode::Leaf(rows)
        }
        2 => {
            let bitmap = u16_of(&mut d)?;
            if bitmap == 0 || depth >= DECODE_MAX_DEPTH {
                return Err(refused());
            }
            let mut children = Box::new([None; FANOUT]);
            for (n, child) in children.iter_mut().enumerate() {
                if bitmap & (1 << n) != 0 {
                    *child = Some(PrivateChatPageId::from_bytes(b32(&mut d)?));
                }
            }
            DecodedNode::Branch(children)
        }
        _ => return Err(refused()),
    };
    end(d)?;
    Ok((depth, prefix, node))
}

/// Reads one exact node page (by its authenticated ID) and returns its opened plaintext plus
/// the complete frame length, which is the physical byte cost a retirement refunds.
pub(super) trait NodeStore {
    fn read_node(&mut self, id: PrivateChatPageId)
        -> Result<(Zeroizing<Vec<u8>>, usize), AppError>;
}

/// Receives each new node plaintext, children before parents, and returns its page ID.
/// Preparation seals fresh pages; cold recovery instead verifies the WAL's exact pages.
pub(super) trait NodeSink {
    fn emit(&mut self, plain: Zeroizing<Vec<u8>>) -> Result<PrivateChatPageId, PlanError>;
}

/// Why a plan did not produce a transition. Nothing has been written in any of these cases.
#[derive(Debug)]
pub(super) enum PlanError {
    /// The full key already exists. Rows are never overwritten.
    Duplicate,
    /// Depth, collision, retiree or output bounds would be exceeded. Refused before admission;
    /// no row is dropped and no existing row moves.
    Exhausted,
    Failed(Box<AppError>),
}
impl From<AppError> for PlanError {
    fn from(error: AppError) -> Self {
        Self::Failed(Box::new(error))
    }
}

/// In-memory working tree. `Stored` subtrees are untouched and keep their page IDs.
enum Work {
    Stored(PrivateChatPageId),
    Leaf(Vec<Row>),
    Branch(Box<[Option<Work>; FANOUT]>),
}

/// One copy-on-write insertion plan over the selected root.
pub(super) struct TriePlan<'s> {
    context: MemberOutboxContextId,
    limits: TrieLimits,
    root: Option<Work>,
    store: &'s mut dyn NodeStore,
    retired: Vec<RetiredData>,
}
impl<'s> TriePlan<'s> {
    pub(super) fn new(
        context: MemberOutboxContextId,
        limits: TrieLimits,
        root: Option<PrivateChatPageId>,
        store: &'s mut dyn NodeStore,
    ) -> Self {
        Self {
            context,
            limits,
            root: root.map(Work::Stored),
            store,
            retired: Vec::new(),
        }
    }

    pub(super) fn insert(&mut self, key: RowKey, value: RowValue) -> Result<(), PlanError> {
        let row = Row::new(self.context, key, value)?;
        let mut cx = Loader {
            context: self.context,
            limits: self.limits,
            store: &mut *self.store,
            retired: &mut self.retired,
        };
        match self.root.as_mut() {
            None => {
                self.root = Some(Work::Leaf(vec![row]));
                Ok(())
            }
            Some(work) => insert_at(work, 0, row, &mut cx),
        }
    }

    /// Emit every new node (children first) and return the new root plus the exact retirees.
    pub(super) fn finish(
        self,
        sink: &mut dyn NodeSink,
    ) -> Result<(Option<PrivateChatPageId>, Vec<RetiredData>), PlanError> {
        let root = match self.root {
            None => None,
            Some(work) => {
                let mut prefix = Vec::with_capacity(DECODE_MAX_DEPTH);
                Some(emit(self.context, work, 0, &mut prefix, sink)?)
            }
        };
        Ok((root, self.retired))
    }
}

struct Loader<'a> {
    context: MemberOutboxContextId,
    limits: TrieLimits,
    store: &'a mut dyn NodeStore,
    retired: &'a mut Vec<RetiredData>,
}
impl Loader<'_> {
    /// Load a stored node on `path` at `depth` and record it as replaced. Each stored node is
    /// converted at most once per plan, so a retiree is listed exactly once.
    fn load(
        &mut self,
        id: PrivateChatPageId,
        depth: usize,
        path: &[u8; 32],
    ) -> Result<Work, PlanError> {
        if self.retired.len() >= MAX_RETIRED {
            return Err(PlanError::Exhausted);
        }
        let (plain, bytes) = self.store.read_node(id)?;
        let (_, _, node) = decode_node(self.context, &plain, Some((depth, path)))?;
        self.retired.push(RetiredData {
            id,
            bytes: u32::try_from(bytes).map_err(|_| refused())?,
        });
        Ok(match node {
            DecodedNode::Leaf(rows) => Work::Leaf(rows),
            DecodedNode::Branch(children) => {
                Work::Branch(Box::new((*children).map(|child| child.map(Work::Stored))))
            }
        })
    }
}

fn insert_at(
    slot: &mut Work,
    depth: usize,
    row: Row,
    cx: &mut Loader<'_>,
) -> Result<(), PlanError> {
    if let Work::Stored(id) = *slot {
        *slot = cx.load(id, depth, &row.path)?;
    }
    match slot {
        Work::Stored(_) => Err(PlanError::from(refused())),
        Work::Leaf(rows) => {
            match rows.binary_search_by(|probe| probe.order(&row)) {
                Ok(_) => return Err(PlanError::Duplicate),
                Err(at) => rows.insert(at, row),
            }
            if rows.len() > cx.limits.leaf_rows {
                let taken = std::mem::take(rows);
                *slot = split(taken, depth, cx.limits)?;
            }
            Ok(())
        }
        Work::Branch(children) => {
            if depth >= cx.limits.max_depth {
                // A branch can only exist above the deepest level; anything else is corrupt.
                return Err(PlanError::from(refused()));
            }
            let n = usize::from(nibble(&row.path, depth));
            match children[n].as_mut() {
                Some(child) => insert_at(child, depth + 1, row, cx),
                None => {
                    children[n] = Some(Work::Leaf(vec![row]));
                    Ok(())
                }
            }
        }
    }
}

/// Split an overfull leaf at `depth`. A leaf already at the deepest level cannot split: the
/// plan is refused as exhausted and no row is dropped.
fn split(rows: Vec<Row>, depth: usize, limits: TrieLimits) -> Result<Work, PlanError> {
    if depth >= limits.max_depth {
        return Err(PlanError::Exhausted);
    }
    let mut buckets: [Vec<Row>; FANOUT] = std::array::from_fn(|_| Vec::new());
    for row in rows {
        buckets[usize::from(nibble(&row.path, depth))].push(row);
    }
    let mut children: Box<[Option<Work>; FANOUT]> = Box::new(std::array::from_fn(|_| None));
    for (n, bucket) in buckets.into_iter().enumerate() {
        if bucket.is_empty() {
            continue;
        }
        children[n] = Some(if bucket.len() > limits.leaf_rows {
            split(bucket, depth + 1, limits)?
        } else {
            Work::Leaf(bucket)
        });
    }
    Ok(Work::Branch(children))
}

fn emit(
    context: MemberOutboxContextId,
    work: Work,
    depth: usize,
    prefix: &mut Vec<u8>,
    sink: &mut dyn NodeSink,
) -> Result<PrivateChatPageId, PlanError> {
    match work {
        Work::Stored(id) => Ok(id),
        Work::Leaf(rows) => {
            let plain = encode_node(context, depth, prefix, &DecodedNodeRef::Leaf(&rows))?;
            sink.emit(plain)
        }
        Work::Branch(children) => {
            let mut ids = [None; FANOUT];
            let children: [Option<Work>; FANOUT] = *children;
            for (n, child) in children.into_iter().enumerate() {
                if let Some(child) = child {
                    prefix.push(n as u8);
                    let id = emit(context, child, depth + 1, prefix, sink);
                    prefix.pop();
                    ids[n] = Some(id?);
                }
            }
            let plain = encode_node(context, depth, prefix, &DecodedNodeRef::Branch(&ids))?;
            sink.emit(plain)
        }
    }
}

/// The three rows one saved entry adds, in a fixed order. Preparation and cold recovery both
/// call this, so a recovered WAL is checked against exactly the plan that produced it.
pub(super) fn insert_entry_rows(
    plan: &mut TriePlan<'_>,
    entry: &EntryData,
    entry_id: PrivateChatPageId,
) -> Result<(), PlanError> {
    plan.insert(
        RowKey::token(entry.token),
        RowValue::Token(TokenRowData {
            ordinal: entry.ordinal,
            entry: entry_id,
            request: entry.request,
            action: entry.action,
            original: entry.original,
        }),
    )?;
    plan.insert(
        RowKey::ordinal(entry.ordinal),
        RowValue::Ordinal(OrdinalRowData {
            entry: entry_id,
            action: entry.action,
        }),
    )?;
    plan.insert(
        RowKey::action(entry.action),
        RowValue::Action(ActionRowData {
            ordinal: entry.ordinal,
            original: entry.original,
        }),
    )
}

/// One durable disposition row per settled debt item, recorded at `generation`.
pub(super) fn insert_settled_rows(
    plan: &mut TriePlan<'_>,
    settled: &[SettledData],
    generation: u64,
) -> Result<(), PlanError> {
    for item in settled {
        plan.insert(
            RowKey::disposition(item.ordinal),
            RowValue::Disposition(DispositionRowData {
                code: item.disposition,
                generation,
                admission: item.admission,
            }),
        )?;
    }
    Ok(())
}

/// Exact read-only lookup of one full key from a selected root.
pub(super) fn lookup(
    context: MemberOutboxContextId,
    root: Option<PrivateChatPageId>,
    key: RowKey,
    store: &mut dyn NodeStore,
) -> Result<Option<RowValue>, AppError> {
    let path = key.path(context);
    let Some(mut current) = root else {
        return Ok(None);
    };
    for depth in 0..=DECODE_MAX_DEPTH {
        let (plain, _) = store.read_node(current)?;
        match decode_node(context, &plain, Some((depth, &path)))?.2 {
            DecodedNode::Leaf(rows) => {
                return Ok(rows
                    .into_iter()
                    .find(|row| row.key == key)
                    .map(|row| row.value));
            }
            DecodedNode::Branch(children) => match children[usize::from(nibble(&path, depth))] {
                None => return Ok(None),
                Some(next) => current = next,
            },
        }
    }
    Err(refused())
}

/// Cold cleanup's reachability proof for one retiree: decode the retiree to learn its own
/// position, then walk the NEW root down that position. A retiree is deletable only when the
/// new tree holds a different node there (or no node). Positions are unique: every load
/// checks depth and prefix, so a node ID is only ever reachable at its encoded position.
pub(super) fn retiree_unreachable(
    context: MemberOutboxContextId,
    new_root: Option<PrivateChatPageId>,
    retiree: PrivateChatPageId,
    retiree_plain: &[u8],
    store: &mut dyn NodeStore,
) -> Result<bool, AppError> {
    let (depth, prefix, _) = decode_node(context, retiree_plain, None)?;
    let Some(mut current) = new_root else {
        return Ok(true);
    };
    for level in 0..depth {
        if current == retiree {
            return Ok(false);
        }
        let (plain, _) = store.read_node(current)?;
        let (_, _, node) = decode_node(context, &plain, None)?;
        match node {
            DecodedNode::Leaf(_) => return Ok(true),
            DecodedNode::Branch(children) => match children[usize::from(prefix[level])] {
                None => return Ok(true),
                Some(next) => current = next,
            },
        }
    }
    Ok(current != retiree)
}

/// Every node ID reachable from `root`. Test-only: production never walks the whole tree.
#[cfg(test)]
pub(super) fn reachable_for_test(
    context: MemberOutboxContextId,
    root: Option<PrivateChatPageId>,
    store: &mut dyn NodeStore,
) -> Result<Vec<PrivateChatPageId>, AppError> {
    let mut out = Vec::new();
    let mut stack = Vec::new();
    if let Some(root) = root {
        stack.push(root);
    }
    while let Some(id) = stack.pop() {
        out.push(id);
        let (plain, _) = store.read_node(id)?;
        if let (_, _, DecodedNode::Branch(children)) = decode_node(context, &plain, None)? {
            stack.extend((*children).into_iter().flatten());
        }
    }
    Ok(out)
}

/// Sealing sink used by preparation: fresh pages under the RNG seam, once, then retained.
pub(super) struct SealSink<'a> {
    pub(super) key: &'a [u8; 32],
    pub(super) rng: &'a mut dyn catcoms_rt::CryptoRngCore,
    pub(super) outputs: Vec<super::frames::OutputData>,
    pub(super) bytes: usize,
}
impl NodeSink for SealSink<'_> {
    fn emit(&mut self, plain: Zeroizing<Vec<u8>>) -> Result<PrivateChatPageId, PlanError> {
        if self.outputs.len() >= MAX_NODE_OUTPUTS {
            return Err(PlanError::Exhausted);
        }
        let page = catcoms_storage::private_chat_page::PreparedPrivateChatPage::prepare(
            self.key, &plain, self.rng,
        )
        .map_err(AppError::from)?;
        let bytes = self.bytes + page.as_bytes().len();
        if bytes > MAX_NODE_OUTPUT_BYTES {
            return Err(PlanError::Exhausted);
        }
        self.bytes = bytes;
        let id = page.id();
        self.outputs.push(super::frames::OutputData {
            role: super::frames::ObjectRole::Node,
            id,
            frame: Zeroizing::new(page.as_bytes().to_vec()),
        });
        Ok(id)
    }
}

/// Verifying sink used by cold recovery: each emitted plaintext must equal, byte for byte,
/// the next node page the authenticated WAL lists. Nothing is sealed or written here.
pub(super) struct VerifySink<'a> {
    pub(super) key: &'a [u8; 32],
    pub(super) expected: std::slice::Iter<'a, super::frames::OutputData>,
}
impl NodeSink for VerifySink<'_> {
    fn emit(&mut self, plain: Zeroizing<Vec<u8>>) -> Result<PrivateChatPageId, PlanError> {
        let output = self.expected.next().ok_or_else(refused)?;
        if output.role != super::frames::ObjectRole::Node {
            return Err(PlanError::from(refused()));
        }
        let opened = catcoms_storage::private_chat_page::open_private_chat_page(
            self.key,
            output.id,
            &output.frame,
        )
        .map_err(AppError::from)?;
        if opened.as_slice() != plain.as_slice() {
            return Err(PlanError::from(refused()));
        }
        Ok(output.id)
    }
}
