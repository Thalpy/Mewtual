//! Read-only art/frame projection, NOT signed admission or the complete Flipnote consumer.
//!
//! Flat root properties avoid conflicting nested-map roots. `i/<frame>/<op-id>` and
//! `d/<frame>/<op-id>` are immutable insertion/deletion records; `r/<frame>` is a replacement
//! register; `h/title` and `h/fps` are header registers. All other operations/properties reject.
//! Sound/score/export support must not be silently filtered out to produce an incomplete view.
//!
//! Insertions reference insertion OP ids for their resolved left and right origins, not mutable
//! winning frame ids. Losing and deleted insertions remain anchors. See `order` for the stable
//! gap-ordering rule; none of these fields are authenticated by merely reading a projection.
//! A separate epoch-zero causal validator derives origins from the author's dependency frontier
//! and checks mutations. P1 authentication and exact seed preflight are still separate requirements.

use std::collections::{BTreeMap, BTreeSet};

use automerge::{AutoCommit, ChangeHash, ReadDoc, ScalarValue, Value, ROOT};
use catcoms_crypto::DeviceId;
use catcoms_wire::DocType;

use super::{fixed_hex, integer_bound, ContentId, ElementId, FlipnoteHeader, FlipnoteOp};
use crate::checkpoint::am_error;
use crate::epoch::{MAX_DOMAIN_OP_BYTES, MAX_EPOCH_BYTES, MAX_EPOCH_OPERATIONS};
use crate::registry::hex;
use crate::{DomainOp, LogicalDocument, ReplError, MAX_CHECKPOINT_BYTES};

mod change;
pub use change::validate_frame_change;

/// List-length ceiling, separate from the sum of declared selected-frame sizes.
pub const FLIPNOTE_MAX_FRAMES: usize = 999;
/// Maximum playable prefix's sum of declared encoded frame bytes, without fetching blobs.
pub const FLIPNOTE_FRAME_BYTES: u64 = 8 * 1024 * 1024;
// Seven immutable headers + record + marker per change, plus bounded future seed allowance.
// These bound an already-parsed CRDT reader, not raw ingestion or encoded checkpoint size.
const MAX_PRIMITIVES: u64 = (MAX_EPOCH_OPERATIONS * 9 + 8192) as u64;
const MAX_READ_BYTES: usize = MAX_EPOCH_BYTES + MAX_CHECKPOINT_BYTES;
const MAX_KEY_BYTES: usize = 99;
// version + author + timestamp + two optional 32-byte origins + complete domain envelope.
const MAX_RECORD_BYTES: usize = 1 + 32 + 8 + 2 * 33 + MAX_DOMAIN_OP_BYTES;
type OpId = [u8; 32];

/// Logical object identity. The supplied channel is independently checked by the frame reader;
/// a registry hint or equal object id under another server is not channel/receipt authority.
pub fn flipnote_document(
    server_id: &[u8],
    object: ElementId,
) -> Result<LogicalDocument, ReplError> {
    LogicalDocument::new(server_id.to_vec(), DocType::StudioObject, object.to_vec())
}

/// Internally consistent provenance claims, trusted only after signed-delta/receipt validation.
#[derive(Clone, PartialEq, Eq)]
pub struct FrameSource {
    pub op_id: OpId,
    pub author: DeviceId,
    pub nonce: [u8; 16],
    /// Author-asserted JS-safe milliseconds; not a clock reading, ordering key or freshness proof.
    pub ts: u64,
}

/// A value paired with its original full identity and derived operation id.
#[derive(Clone, PartialEq, Eq)]
pub struct FrameValue<T> {
    pub value: T,
    pub source: FrameSource,
}

/// Actual Automerge winner plus all other live alternatives, sorted by derived op id.
#[derive(Clone, PartialEq, Eq)]
pub struct FrameRegister<T> {
    pub selected: FrameValue<T>,
    pub conflicts: Vec<FrameValue<T>>,
}

/// Exact content/length declaration. Reading proves neither availability nor valid PIX bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct FrameBlob {
    pub cid: ContentId,
    pub bytes: u64,
}

/// Immutable insertion evidence, including its original stable-id request and resolved gap.
#[derive(Clone, PartialEq, Eq)]
pub struct FrameInsertion {
    pub after: Option<ElementId>,
    /// Insertion op id of the predecessor; None is the beginning, not the append position.
    pub anchor: Option<OpId>,
    /// First sibling following that position in the author's view; None if there was none.
    pub before: Option<OpId>,
    pub blob: FrameBlob,
}

/// One frame id, regardless of deletion or display cap. The first insertion wins by smallest
/// op id, but every insertion is retained. An explicit replacement register overrides only
/// pixels, never insertion order; obsolete causal replacements remain in history, not here.
#[derive(Clone, PartialEq, Eq)]
pub struct FrameEntry {
    pub insertions: Vec<FrameValue<FrameInsertion>>,
    pub pixels: FrameRegister<FrameBlob>,
}

/// Reasons the selected frame is beyond the playable prefix. No bin-packing around overflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameLimits {
    pub count: bool,
    pub bytes: bool,
}

/// Art-only projection with ALL live insertion/replacement/deletion evidence, including hidden
/// frames. Not a persisted recovery snapshot, admission capability, exporter or retention pass.
#[derive(Clone, PartialEq, Eq)]
pub struct FlipnoteFrameProjection {
    document: LogicalDocument,
    pub channel: ElementId,
    pub epoch: u64,
    /// Absent until explicitly set; a view can use the existing empty-title / 12-fps defaults.
    /// Defaults have no invented author and are not stored as authenticated operations.
    pub title: Option<FrameRegister<String>>,
    pub fps: Option<FrameRegister<u8>>,
    /// Includes deleted frames and all conflicts; reference/recovery consumers must inspect it.
    pub frames: BTreeMap<ElementId, FrameEntry>,
    pub tombstones: BTreeMap<ElementId, Vec<FrameSource>>,
    /// Every nondeleted frame exactly once, including over-cap frames that the UI must flag.
    pub timeline: Vec<ElementId>,
    pub over_cap: BTreeMap<ElementId, FrameLimits>,
    /// Sum across the entire live timeline, even the over-cap suffix. Never computed by fetching.
    pub declared_frame_bytes: u64,
    // Full insertion traversal, including hidden anchors, for future causal placement checks.
    insertion_order: Vec<OpId>,
}

macro_rules! redacted_debug {
    ($($name:ident),+ $(,)?) => {$(
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_debug!(
    FrameSource,
    FrameBlob,
    FrameInsertion,
    FrameEntry,
    FlipnoteFrameProjection
);
impl<T> std::fmt::Debug for FrameValue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameValue").finish_non_exhaustive()
    }
}
impl<T> std::fmt::Debug for FrameRegister<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameRegister").finish_non_exhaustive()
    }
}

impl FlipnoteFrameProjection {
    /// Read current flat state under independently supplied server/object/channel/epoch scope.
    /// Every concurrent value is checked, not only winners. The sole missing-header exception
    /// is pristine epoch zero, with no applied operations or changes. Input must already have
    /// passed bounded Automerge parsing; this reader adds no wire or membership authority.
    pub fn read(
        document: &LogicalDocument,
        channel: ElementId,
        epoch: u64,
        doc: &AutoCommit,
    ) -> Result<Self, ReplError> {
        Self::read_bounded(
            document,
            channel,
            epoch,
            doc,
            MAX_PRIMITIVES,
            MAX_READ_BYTES,
        )
    }

    // Private reduced-budget seam keeps boundary tests small; public callers cannot raise limits.
    fn read_bounded(
        document: &LogicalDocument,
        channel: ElementId,
        epoch: u64,
        doc: &AutoCommit,
        primitive_limit: u64,
        byte_limit: usize,
    ) -> Result<Self, ReplError> {
        Self::read_view(
            document,
            channel,
            epoch,
            doc,
            None,
            (primitive_limit, byte_limit),
        )
    }

    /// Private causal view for delta validation, never a peer-selected shortcut to current state.
    /// All supplied dependencies must exist before historical queries; empty heads mean the
    /// pristine graph even if the receiver already holds unrelated concurrent roots.
    fn read_at(
        document: &LogicalDocument,
        channel: ElementId,
        epoch: u64,
        doc: &AutoCommit,
        heads: &[ChangeHash],
    ) -> Result<Self, ReplError> {
        if heads.len() > MAX_EPOCH_OPERATIONS {
            return Err(ReplError::EpochBound);
        }
        if heads
            .iter()
            .any(|hash| doc.get_change_by_hash(hash).is_none())
        {
            return Err(ReplError::EpochScope);
        }
        Self::read_view(
            document,
            channel,
            epoch,
            doc,
            Some(heads),
            (MAX_PRIMITIVES, MAX_READ_BYTES),
        )
    }

    // One materializer for current and historical state: keys, ALL values and the actual register
    // winner must use the same frontier. Mixing any current read into the historical branch lets
    // receiver-only state alter the position/authority of a perfectly valid concurrent insertion.
    // Automerge's `_at` getters recompute historical clocks: this avoids randomized/replayed
    // forks, not repeated clock work. Fixed reader bounds are not a measured latency guarantee.
    fn read_view(
        document: &LogicalDocument,
        channel: ElementId,
        epoch: u64,
        doc: &AutoCommit,
        heads: Option<&[ChangeHash]>,
        (primitive_limit, byte_limit): (u64, usize),
    ) -> Result<Self, ReplError> {
        let object = document
            .logical_key
            .as_slice()
            .try_into()
            .map_err(|_| ReplError::EpochScope)?;
        if *document != flipnote_document(&document.server_id, object)? {
            return Err(ReplError::EpochScope);
        }
        let stats = doc.stats();
        if stats.num_ops > primitive_limit {
            return Err(ReplError::EpochBound);
        }
        let expected = header(document, channel, epoch);
        let mut budget = Budget {
            used: 0,
            limit: byte_limit,
        };
        let mut header_count = 0;
        let mut key_count = 0;
        let mut nodes = BTreeMap::new();
        let mut replacements = BTreeMap::new();
        let mut deletions: BTreeMap<ElementId, BTreeMap<OpId, FrameSource>> = BTreeMap::new();
        let mut title = None;
        let mut fps = None;
        let mut operations = BTreeMap::new();
        let keys = match heads {
            Some(heads) => doc.keys_at(ROOT, heads),
            None => doc.keys(ROOT),
        };
        for key in keys {
            key_count += 1;
            if key.len() > MAX_KEY_BYTES {
                return Err(ReplError::EpochBound);
            }
            budget.add(key.len())?;
            // stats bounds the result count before get_all allocates. Budget every losing,
            // deleted and over-cap value before decoding/cloning any retained domain content.
            let values = match heads {
                Some(heads) => doc.get_all_at(ROOT, &key, heads),
                None => doc.get_all(ROOT, &key),
            }
            .map_err(am_error)?;
            for (value, _) in &values {
                budget.value(value)?;
            }
            if let Some(value) = expected.get(&key) {
                if values.iter().any(|(v, _)| !scalar_eq(v, value)) {
                    return Err(ReplError::EpochScope);
                }
                header_count += 1;
                continue;
            }
            if let Some(id) = key.strip_prefix("_p1/op/") {
                fixed_hex::<32>(id)?;
                if values
                    .iter()
                    .any(|(v, _)| !scalar_eq(v, &ScalarValue::Uint(1)))
                {
                    return Err(ReplError::Malformed);
                }
                continue;
            }
            let (tag, suffix) = key.split_once('/').ok_or(ReplError::Malformed)?;
            let (frame, immutable_id) = match tag {
                "i" | "d" => {
                    let (id, op_id) = suffix.split_once('/').ok_or(ReplError::Malformed)?;
                    (Some(fixed_hex::<16>(id)?), Some(fixed_hex::<32>(op_id)?))
                }
                "r" => (Some(fixed_hex::<16>(suffix)?), None),
                "h" if matches!(suffix, "title" | "fps") => (None, None),
                _ => return Err(ReplError::Malformed),
            };
            let winner = match heads {
                Some(heads) => doc.get_at(ROOT, &key, heads),
                None => doc.get(ROOT, &key),
            }
            .map_err(am_error)?
            .ok_or(ReplError::Malformed)?
            .1;
            let mut selected = None;
            let mut pixels = BTreeMap::new();
            let mut titles = BTreeMap::new();
            let mut speeds = BTreeMap::new();
            for (value, am_id) in values {
                let bytes = record_bytes(&value)?;
                let record = decode_record(document, bytes)?;
                let id = record.source.op_id;
                // op ids exclude the body AND derived metadata; only byte-identical retries
                // collapse. Same-id equivocation must never become a second insertion/node.
                let digest = *blake3::hash(bytes).as_bytes();
                if operations
                    .insert(id, digest)
                    .is_some_and(|old| old != digest)
                {
                    return Err(ReplError::Malformed);
                }
                if winner == am_id {
                    selected = Some(id);
                }
                let source = record.source;
                match (tag, record.operation) {
                    (
                        "i",
                        FlipnoteOp::InsertFrame {
                            frame: target,
                            after,
                            cid,
                            bytes,
                        },
                    ) if frame == Some(target) && immutable_id == Some(id) => {
                        nodes.insert(
                            id,
                            Node {
                                frame: target,
                                insertion: FrameValue {
                                    source,
                                    value: FrameInsertion {
                                        after,
                                        anchor: record.anchor,
                                        before: record.before,
                                        blob: FrameBlob { cid, bytes },
                                    },
                                },
                            },
                        );
                    }
                    ("d", FlipnoteOp::RemoveFrame { frame: target })
                        if frame == Some(target) && immutable_id == Some(id) =>
                    {
                        deletions.entry(target).or_default().insert(id, source);
                    }
                    (
                        "r",
                        FlipnoteOp::ReplaceFrame {
                            frame: target,
                            cid,
                            bytes,
                        },
                    ) if frame == Some(target) => {
                        pixels.insert(
                            id,
                            FrameValue {
                                source,
                                value: FrameBlob { cid, bytes },
                            },
                        );
                    }
                    ("h", FlipnoteOp::SetHeader(FlipnoteHeader::Title(value)))
                        if suffix == "title" =>
                    {
                        titles.insert(id, FrameValue { source, value });
                    }
                    ("h", FlipnoteOp::SetHeader(FlipnoteHeader::Fps(value))) if suffix == "fps" => {
                        speeds.insert(id, FrameValue { source, value });
                    }
                    _ => return Err(ReplError::Malformed),
                }
            }
            match tag {
                "r" => {
                    replacements.insert(
                        frame.ok_or(ReplError::Malformed)?,
                        register(pixels, selected)?,
                    );
                }
                "h" if suffix == "title" => {
                    title = Some(register(titles, selected)?);
                }
                "h" => {
                    fps = Some(register(speeds, selected)?);
                }
                _ => {}
            }
        }
        let empty_history = match heads {
            Some(heads) => heads.is_empty(),
            None => stats.num_ops == 0 && stats.num_changes == 0,
        };
        let pristine = epoch == 0 && key_count == 0 && empty_history;
        if header_count != expected.len() && !pristine {
            return Err(ReplError::Malformed);
        }
        let insertion_order = order(&nodes)?;
        let mut insertions: BTreeMap<ElementId, Vec<FrameValue<FrameInsertion>>> = BTreeMap::new();
        // BTreeMap iteration supplies smallest-op-id winner and deterministic conflict order.
        for node in nodes.values() {
            insertions
                .entry(node.frame)
                .or_default()
                .push(node.insertion.clone());
        }
        let mut frames = BTreeMap::new();
        for (id, candidates) in insertions {
            let first = candidates.first().ok_or(ReplError::Malformed)?;
            let pixels = replacements.remove(&id).unwrap_or_else(|| FrameRegister {
                selected: FrameValue {
                    source: first.source.clone(),
                    value: first.value.blob.clone(),
                },
                conflicts: Vec::new(),
            });
            frames.insert(
                id,
                FrameEntry {
                    insertions: candidates,
                    pixels,
                },
            );
        }
        // A reader cannot prove causal target existence, but it must not hide orphan payloads.
        if !replacements.is_empty() {
            return Err(ReplError::Malformed);
        }
        let tombstones: BTreeMap<_, Vec<_>> = deletions
            .into_iter()
            .map(|(id, sources)| (id, sources.into_values().collect()))
            .collect();
        let mut timeline = Vec::new();
        for op_id in &insertion_order {
            let node = &nodes[op_id];
            if !tombstones.contains_key(&node.frame)
                && frames[&node.frame].insertions[0].source.op_id == *op_id
            {
                timeline.push(node.frame);
            }
        }
        let mut declared_frame_bytes = 0u64;
        let mut over_cap = BTreeMap::new();
        for (position, id) in timeline.iter().enumerate() {
            declared_frame_bytes = declared_frame_bytes
                .checked_add(frames[id].pixels.selected.value.bytes)
                .ok_or(ReplError::EpochBound)?;
            let limits = FrameLimits {
                count: position >= FLIPNOTE_MAX_FRAMES,
                bytes: declared_frame_bytes > FLIPNOTE_FRAME_BYTES,
            };
            if limits.count || limits.bytes {
                over_cap.insert(*id, limits);
            }
        }
        Ok(Self {
            document: document.clone(),
            channel,
            epoch,
            title,
            fps,
            frames,
            tombstones,
            timeline,
            over_cap,
            declared_frame_bytes,
            insertion_order,
        })
    }

    pub fn document(&self) -> &LogicalDocument {
        &self.document
    }

    /// Full immutable-node traversal, not a live-frame list. Hidden anchors still affect gaps.
    pub fn insertion_order(&self) -> &[OpId] {
        &self.insertion_order
    }
}

struct Node {
    frame: ElementId,
    insertion: FrameValue<FrameInsertion>,
}

/// Right-origin forest inside each left-origin's children. Visit inserts whose `before` names
/// a target BEFORE that target, then visit the target's own anchored children. Same-gap nodes
/// sort by op id. This preserves observed immediate placement: given siblings X,Y, a later C
/// before X yields C,X,Y, not Y,C,X as generic smallest-id topological sorting could produce.
/// All origins must be causally derived by a future delta validator. Current-state checks also
/// reject missing/wrong-parent origins and cycles, using an iterative bounded walk (no recursion).
fn order(nodes: &BTreeMap<OpId, Node>) -> Result<Vec<OpId>, ReplError> {
    let mut gaps: BTreeMap<(Option<OpId>, Option<OpId>), Vec<OpId>> = BTreeMap::new();
    for (id, node) in nodes {
        let value = &node.insertion.value;
        match (value.after, value.anchor) {
            (None, None) => {}
            (Some(frame), Some(anchor)) if nodes.get(&anchor).is_some_and(|p| p.frame == frame) => {
            }
            _ => return Err(ReplError::Malformed),
        }
        if value.anchor == Some(*id) || value.before == Some(*id) {
            return Err(ReplError::Malformed);
        }
        if let Some(before) = value.before {
            if nodes
                .get(&before)
                .is_none_or(|next| next.insertion.value.anchor != value.anchor)
            {
                return Err(ReplError::Malformed);
            }
        }
        gaps.entry((value.anchor, value.before))
            .or_default()
            .push(*id);
    }
    // false schedules a node's before-subtree; true emits it and schedules anchored children.
    let mut stack: Vec<_> = gaps
        .get(&(None, None))
        .into_iter()
        .flatten()
        .rev()
        .map(|id| (*id, false))
        .collect();
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::with_capacity(nodes.len());
    while let Some((id, emit)) = stack.pop() {
        if emit {
            ordered.push(id);
            stack.extend(
                gaps.get(&(Some(id), None))
                    .into_iter()
                    .flatten()
                    .rev()
                    .map(|id| (*id, false)),
            );
        } else {
            if !seen.insert(id) {
                return Err(ReplError::Malformed);
            }
            stack.push((id, true));
            stack.extend(
                gaps.get(&(nodes[&id].insertion.value.anchor, Some(id)))
                    .into_iter()
                    .flatten()
                    .rev()
                    .map(|id| (*id, false)),
            );
        }
    }
    // Cyclic components have no root; they must not silently disappear from the projection.
    if ordered.len() != nodes.len() {
        return Err(ReplError::Malformed);
    }
    Ok(ordered)
}

struct Record {
    source: FrameSource,
    anchor: Option<OpId>,
    before: Option<OpId>,
    operation: FlipnoteOp,
}

/// Fixed framing: 1, author[32], ts u64 BE, anchor (0 or 1+32 bytes), before (same), DomainOp.
/// Timestamps and origins are signed-delta metadata, not new fields in the stable domain body.
fn decode_record(document: &LogicalDocument, bytes: &[u8]) -> Result<Record, ReplError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(ReplError::EpochBound);
    }
    if bytes.len() < 43 || bytes[0] != 1 {
        return Err(ReplError::Malformed);
    }
    let author = DeviceId::from_bytes(bytes[1..33].try_into().map_err(|_| ReplError::Malformed)?);
    let ts = u64::from_be_bytes(bytes[33..41].try_into().map_err(|_| ReplError::Malformed)?);
    integer_bound(ts)?;
    let mut remaining = &bytes[41..];
    let anchor = origin(&mut remaining)?;
    let before = origin(&mut remaining)?;
    let domain = DomainOp::decode(remaining)?;
    let operation = FlipnoteOp::decode_domain(document, &domain)?;
    if !matches!(operation, FlipnoteOp::InsertFrame { .. })
        && (anchor.is_some() || before.is_some())
    {
        return Err(ReplError::Malformed);
    }
    Ok(Record {
        source: FrameSource {
            op_id: domain.id(&author),
            author,
            nonce: domain.nonce,
            ts,
        },
        anchor,
        before,
        operation,
    })
}

fn origin(bytes: &mut &[u8]) -> Result<Option<OpId>, ReplError> {
    let (&flag, tail) = bytes.split_first().ok_or(ReplError::Malformed)?;
    *bytes = tail;
    match flag {
        0 => Ok(None),
        1 if bytes.len() >= 32 => {
            let id = bytes[..32].try_into().map_err(|_| ReplError::Malformed)?;
            *bytes = &bytes[32..];
            Ok(Some(id))
        }
        _ => Err(ReplError::Malformed),
    }
}

fn register<T>(
    mut values: BTreeMap<OpId, FrameValue<T>>,
    selected: Option<OpId>,
) -> Result<FrameRegister<T>, ReplError> {
    let selected = values
        .remove(&selected.ok_or(ReplError::Malformed)?)
        .ok_or(ReplError::Malformed)?;
    Ok(FrameRegister {
        selected,
        conflicts: values.into_values().collect(),
    })
}

fn header(
    document: &LogicalDocument,
    channel: ElementId,
    epoch: u64,
) -> BTreeMap<String, ScalarValue> {
    BTreeMap::from([
        ("v".into(), ScalarValue::Uint(1)),
        ("kind".into(), ScalarValue::Str("flipnote".into())),
        (
            "id".into(),
            ScalarValue::Str(hex(&document.logical_key).into()),
        ),
        ("channel".into(), ScalarValue::Str(hex(&channel).into())),
        ("epoch".into(), ScalarValue::Uint(epoch)),
        ("w".into(), ScalarValue::Uint(192)),
        ("h".into(), ScalarValue::Uint(144)),
    ])
}

fn scalar_eq(value: &Value<'_>, expected: &ScalarValue) -> bool {
    matches!(value, Value::Scalar(v) if v.as_ref() == expected)
}
fn record_bytes<'a>(value: &'a Value<'_>) -> Result<&'a [u8], ReplError> {
    match value {
        Value::Scalar(v) => match v.as_ref() {
            ScalarValue::Bytes(bytes) => Ok(bytes),
            _ => Err(ReplError::Malformed),
        },
        _ => Err(ReplError::Malformed),
    }
}
struct Budget {
    used: usize,
    limit: usize,
}
impl Budget {
    fn add(&mut self, bytes: usize) -> Result<(), ReplError> {
        self.used = self.used.checked_add(bytes).ok_or(ReplError::EpochBound)?;
        if self.used > self.limit {
            return Err(ReplError::EpochBound);
        }
        Ok(())
    }
    fn value(&mut self, value: &Value<'_>) -> Result<(), ReplError> {
        let Value::Scalar(value) = value else {
            return Err(ReplError::Malformed);
        };
        let len = match value.as_ref() {
            ScalarValue::Bytes(bytes) if bytes.len() <= MAX_RECORD_BYTES => bytes.len(),
            ScalarValue::Str(text) if text.len() <= MAX_KEY_BYTES => text.len(),
            ScalarValue::Uint(_) => 8,
            ScalarValue::Bytes(_) | ScalarValue::Str(_) => return Err(ReplError::EpochBound),
            _ => return Err(ReplError::Malformed),
        };
        self.add(len)
    }
}

#[cfg(test)]
mod tests;
