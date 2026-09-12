//! Typed art checkpoint baseline and complete recovery projection. Positions in the baseline
//! are normalized to a live chain; complete authored/hidden origins remain in local recovery.

use catcoms_wire::Decoder;

use super::*;
use crate::studio::snapshot::{self as codec, *};
use crate::studio::{blob_size, MAX_FRAME_BYTES};
use crate::{CheckpointSeed, VerifiedCheckpoint, VerifiedReceipt};

type FrameParts = (
    BTreeMap<OpId, Node>,
    BTreeMap<ElementId, FrameRegister<FrameBlob>>,
);

impl SnapshotValue for FrameBlob {
    fn put(&self, e: &mut Writer) -> Result<(), ReplError> {
        blob_size(self.bytes, MAX_FRAME_BYTES)?;
        e.bytes(&self.cid)?;
        e.integer(self.bytes)
    }
    fn get(d: &mut Decoder<'_>) -> Result<Self, ReplError> {
        let cid = array(d)?;
        let bytes = d.get_u64().map_err(malformed)?;
        blob_size(bytes, MAX_FRAME_BYTES)?;
        Ok(Self { cid, bytes })
    }
}
impl SnapshotValue for u8 {
    fn put(&self, e: &mut Writer) -> Result<(), ReplError> {
        if !(1..=24).contains(self) {
            return Err(ReplError::Malformed);
        }
        e.byte(*self)
    }
    fn get(d: &mut Decoder<'_>) -> Result<Self, ReplError> {
        let n = d.get_u8().map_err(malformed)?;
        if !(1..=24).contains(&n) {
            return Err(ReplError::Malformed);
        }
        Ok(n)
    }
}
impl SnapshotValue for FrameInsertion {
    fn put(&self, e: &mut Writer) -> Result<(), ReplError> {
        e.byte(u8::from(self.checkpoint))?;
        e.optional(self.after)?;
        e.optional(self.anchor)?;
        e.optional(self.before)?;
        self.blob.put(e)
    }
    fn get(d: &mut Decoder<'_>) -> Result<Self, ReplError> {
        let checkpoint = match d.get_u8().map_err(malformed)? {
            0 => false,
            1 => true,
            _ => return Err(ReplError::Malformed),
        };
        Ok(Self {
            checkpoint,
            after: optional(d)?,
            anchor: optional(d)?,
            before: optional(d)?,
            blob: FrameBlob::get(d)?,
        })
    }
}
fn put_source(e: &mut Writer, doc: &LogicalDocument, s: &FrameSource) -> Result<(), ReplError> {
    if s.op_id != derived(doc, &s.author, s.nonce) {
        return Err(ReplError::Malformed);
    }
    integer_bound(s.ts)?;
    e.bytes(s.author.as_bytes())?;
    e.bytes(&s.nonce)?;
    e.integer(s.ts)
}
fn get_source(d: &mut Decoder<'_>, doc: &LogicalDocument) -> Result<FrameSource, ReplError> {
    let author = DeviceId::from_bytes(array(d)?);
    let nonce = array(d)?;
    let ts = d.get_u64().map_err(malformed)?;
    integer_bound(ts)?;
    Ok(FrameSource {
        op_id: derived(doc, &author, nonce),
        author,
        nonce,
        ts,
    })
}
fn put_value<T: SnapshotValue>(
    e: &mut Writer,
    doc: &LogicalDocument,
    v: &FrameValue<T>,
) -> Result<(), ReplError> {
    put_source(e, doc, &v.source)?;
    v.value.put(e)
}
fn get_value<T: SnapshotValue>(
    d: &mut Decoder<'_>,
    doc: &LogicalDocument,
) -> Result<FrameValue<T>, ReplError> {
    Ok(FrameValue {
        source: get_source(d, doc)?,
        value: T::get(d)?,
    })
}
fn put_register<T: SnapshotValue>(
    e: &mut Writer,
    doc: &LogicalDocument,
    r: &FrameRegister<T>,
) -> Result<(), ReplError> {
    put_value(e, doc, &r.selected)?;
    e.count(r.conflicts.len())?;
    let mut previous = None;
    for v in &r.conflicts {
        if v.source.op_id == r.selected.source.op_id
            || previous.is_some_and(|p| p >= v.source.op_id)
        {
            return Err(ReplError::Malformed);
        }
        previous = Some(v.source.op_id);
        put_value(e, doc, v)?;
    }
    Ok(())
}
fn get_register<T: SnapshotValue>(
    d: &mut Decoder<'_>,
    doc: &LogicalDocument,
) -> Result<FrameRegister<T>, ReplError> {
    let selected = get_value(d, doc)?;
    let n = count(d, MAX_VALUES)?;
    let mut conflicts = Vec::new();
    for _ in 0..n {
        let v: FrameValue<T> = get_value(d, doc)?;
        if v.source.op_id == selected.source.op_id
            || conflicts
                .last()
                .is_some_and(|p: &FrameValue<T>| p.source.op_id >= v.source.op_id)
        {
            return Err(ReplError::Malformed);
        }
        conflicts.push(v);
    }
    Ok(FrameRegister {
        selected,
        conflicts,
    })
}
fn compact_register<T: Clone>(
    r: &FrameRegister<T>,
    budget: &mut ConflictBudget,
) -> FrameRegister<T> {
    FrameRegister {
        selected: r.selected.clone(),
        conflicts: r.conflicts[..budget.retained(r.conflicts.len())].to_vec(),
    }
}
fn put_optional<T: SnapshotValue>(
    e: &mut Writer,
    doc: &LogicalDocument,
    r: &Option<FrameRegister<T>>,
) -> Result<(), ReplError> {
    e.byte(u8::from(r.is_some()))?;
    if let Some(r) = r {
        put_register(e, doc, r)?;
    }
    Ok(())
}
fn get_optional<T: SnapshotValue>(
    d: &mut Decoder<'_>,
    doc: &LogicalDocument,
) -> Result<Option<FrameRegister<T>>, ReplError> {
    match d.get_u8().map_err(malformed)? {
        0 => Ok(None),
        1 => Ok(Some(get_register(d, doc)?)),
        _ => Err(ReplError::Malformed),
    }
}

impl FlipnoteFrameProjection {
    /// Encode the next receipted art baseline using P1. No history is removed here. Settlement
    /// must first persist the full excluded/overflow/conflict projection in bounded recovery.
    pub fn checkpoint(&self, close: [u8; 32]) -> Result<CheckpointSeed, ReplError> {
        let mut next = self.compact()?;
        next.epoch = self.epoch.checked_add(1).ok_or(ReplError::EpochBound)?;
        codec::build(
            &self.document,
            next.epoch,
            close,
            header(&self.document, self.channel, next.epoch),
            next.encode_snapshot(MAX_CHECKPOINT_BYTES)?,
        )
    }
    pub fn verify_checkpoint(
        receipt: &VerifiedReceipt,
        channel: ElementId,
        bytes: &[u8],
    ) -> Result<VerifiedCheckpoint, ReplError> {
        CheckpointSeed::verify(receipt, bytes, |document, epoch, doc| {
            let mut projection = Self::read(document, channel, epoch, doc)?;
            projection.epoch = epoch.checked_sub(1).ok_or(ReplError::EpochScope)?;
            if projection.checkpoint(receipt.close_record_hash())?.bytes() != bytes {
                return Err(ReplError::Malformed);
            }
            Ok(())
        })
    }
    fn parts(&self) -> Result<FrameParts, ReplError> {
        let object = self
            .document
            .logical_key
            .as_slice()
            .try_into()
            .map_err(|_| ReplError::EpochScope)?;
        if self.document != flipnote_document(&self.document.server_id, object)? {
            return Err(ReplError::EpochScope);
        }
        if self.frames.len() > MAX_VALUES || self.tombstones.len() > MAX_VALUES {
            return Err(ReplError::EpochBound);
        }
        let mut nodes = BTreeMap::new();
        let mut pixels = BTreeMap::new();
        for (id, entry) in &self.frames {
            if entry.insertions.is_empty() || entry.insertions.len() > MAX_VALUES {
                return Err(ReplError::EpochBound);
            }
            let mut previous = None;
            for insertion in &entry.insertions {
                if previous.is_some_and(|p| p >= insertion.source.op_id) {
                    return Err(ReplError::Malformed);
                }
                previous = Some(insertion.source.op_id);
                if nodes.len() >= MAX_VALUES {
                    return Err(ReplError::EpochBound);
                }
                if nodes
                    .insert(
                        insertion.source.op_id,
                        Node {
                            frame: *id,
                            insertion: insertion.clone(),
                        },
                    )
                    .is_some()
                {
                    return Err(ReplError::Malformed);
                }
            }
            pixels.insert(*id, entry.pixels.clone());
        }
        Ok((nodes, pixels))
    }
    fn checked_parts(&self) -> Result<(), ReplError> {
        // Validate borrowed values before the topology rebuild duplicates any public content.
        self.write_snapshot(&mut Writer::measure(
            crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES,
        ))?;
        let (nodes, pixels) = self.parts()?;
        let checked = Self::from_parts(
            &self.document,
            self.channel,
            self.epoch,
            self.title.clone(),
            self.fps.clone(),
            nodes,
            pixels,
            self.tombstones.clone(),
        )?;
        if checked != *self {
            return Err(ReplError::Malformed);
        }
        Ok(())
    }
    /// Original insertion gaps are evidence too: normalization into checkpoint positions must
    /// not bypass recovery merely because every signed operation is inside the close.
    pub(in crate::studio) fn checkpoint_omits_evidence(&self) -> Result<bool, ReplError> {
        Ok(self.compact()? != *self)
    }

    fn compact(&self) -> Result<Self, ReplError> {
        self.checked_parts()?;
        let mut budget = ConflictBudget(CONFLICT_FIELDS);
        let title = self
            .title
            .as_ref()
            .map(|r| compact_register(r, &mut budget));
        let fps = self.fps.as_ref().map(|r| compact_register(r, &mut budget));
        let mut retained = BTreeMap::new();
        // Conflict fields are admitted in header/title, header/fps, stable frame-id order.
        for (id, entry) in &self.frames {
            if self.tombstones.contains_key(id) || self.over_cap.contains_key(id) {
                continue;
            }
            let entry = FrameEntry {
                insertions: entry.insertions[..1 + budget.retained(entry.insertions.len() - 1)]
                    .to_vec(),
                pixels: compact_register(&entry.pixels, &mut budget),
            };
            retained.insert(*id, entry);
        }
        let mut previous = None;
        let mut anchor = None;
        let mut nodes = BTreeMap::new();
        let mut pixels = BTreeMap::new();
        for id in &self.timeline {
            let Some(entry) = retained.remove(id) else {
                continue;
            };
            let winner = entry.insertions[0].source.op_id;
            for mut insertion in entry.insertions {
                insertion.value.checkpoint = true;
                insertion.value.after = previous;
                insertion.value.anchor = anchor;
                insertion.value.before = None;
                nodes.insert(
                    insertion.source.op_id,
                    Node {
                        frame: *id,
                        insertion,
                    },
                );
            }
            pixels.insert(*id, entry.pixels);
            previous = Some(*id);
            anchor = Some(winner);
        }
        Self::from_parts(
            &self.document,
            self.channel,
            self.epoch,
            title,
            fps,
            nodes,
            pixels,
            BTreeMap::new(),
        )
    }

    pub(in crate::studio) fn encode_snapshot(&self, limit: usize) -> Result<Vec<u8>, ReplError> {
        self.write_snapshot(&mut Writer::measure(limit))?;
        self.checked_parts()?;
        let mut e = Writer::new(limit);
        self.write_snapshot(&mut e)?;
        Ok(e.finish())
    }
    fn write_snapshot(&self, e: &mut Writer) -> Result<(), ReplError> {
        payload_header(e, &self.document, self.channel, self.epoch)?;
        put_optional(e, &self.document, &self.title)?;
        put_optional(e, &self.document, &self.fps)?;
        e.count(self.frames.len())?;
        for (id, entry) in &self.frames {
            e.bytes(id)?;
            e.count(entry.insertions.len())?;
            for v in &entry.insertions {
                put_value(e, &self.document, v)?;
            }
            put_register(e, &self.document, &entry.pixels)?;
        }
        e.count(self.tombstones.len())?;
        for (id, values) in &self.tombstones {
            e.bytes(id)?;
            if values.is_empty() {
                return Err(ReplError::Malformed);
            }
            e.count(values.len())?;
            let mut previous = None;
            for s in values {
                if previous.is_some_and(|p| p >= s.op_id) {
                    return Err(ReplError::Malformed);
                }
                previous = Some(s.op_id);
                put_source(e, &self.document, s)?;
            }
        }
        Ok(())
    }
    pub(in crate::studio) fn decode_snapshot(
        document: &LogicalDocument,
        channel: ElementId,
        epoch: u64,
        bytes: &[u8],
    ) -> Result<Self, ReplError> {
        payload_bound(bytes)?;
        let mut d = Decoder::new(bytes);
        check_header(&mut d, document, channel, epoch)?;
        let title = get_optional(&mut d, document)?;
        let fps = get_optional(&mut d, document)?;
        let n = count(&mut d, MAX_VALUES)?;
        let mut nodes = BTreeMap::new();
        let mut pixels = BTreeMap::new();
        for _ in 0..n {
            let id = array(&mut d)?;
            if pixels.last_key_value().is_some_and(|(p, _)| p >= &id) {
                return Err(ReplError::Malformed);
            }
            let n = count(&mut d, MAX_VALUES.saturating_sub(nodes.len()))?;
            if n == 0 {
                return Err(ReplError::Malformed);
            }
            let mut previous = None;
            for _ in 0..n {
                let insertion: FrameValue<FrameInsertion> = get_value(&mut d, document)?;
                if previous.is_some_and(|p| p >= insertion.source.op_id) {
                    return Err(ReplError::Malformed);
                }
                previous = Some(insertion.source.op_id);
                if nodes
                    .insert(
                        insertion.source.op_id,
                        Node {
                            frame: id,
                            insertion,
                        },
                    )
                    .is_some()
                {
                    return Err(ReplError::Malformed);
                }
            }
            pixels.insert(id, get_register(&mut d, document)?);
        }
        let n = count(&mut d, MAX_VALUES)?;
        let mut tombstones = BTreeMap::new();
        for _ in 0..n {
            let id = array(&mut d)?;
            if tombstones.last_key_value().is_some_and(|(p, _)| p >= &id) {
                return Err(ReplError::Malformed);
            }
            let n = count(&mut d, MAX_VALUES)?;
            if n == 0 {
                return Err(ReplError::Malformed);
            }
            let mut values = Vec::new();
            for _ in 0..n {
                let source = get_source(&mut d, document)?;
                if values
                    .last()
                    .is_some_and(|p: &FrameSource| p.op_id >= source.op_id)
                {
                    return Err(ReplError::Malformed);
                }
                values.push(source);
            }
            tombstones.insert(id, values);
        }
        d.finish().map_err(malformed)?;
        Self::from_parts(
            document, channel, epoch, title, fps, nodes, pixels, tombstones,
        )
    }
    pub(super) fn decode_baseline(
        document: &LogicalDocument,
        channel: ElementId,
        epoch: u64,
        bytes: &[u8],
    ) -> Result<Self, ReplError> {
        if epoch == 0 || bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(ReplError::EpochScope);
        }
        let p = Self::decode_snapshot(document, channel, epoch, bytes)?;
        if p.compact()? != p {
            return Err(ReplError::Malformed);
        }
        Ok(p)
    }
    pub(super) fn source_ids(&self) -> BTreeSet<OpId> {
        let mut ids = BTreeSet::new();
        for frame in self.frames.values() {
            ids.extend(frame.insertions.iter().map(|v| v.source.op_id));
            ids.insert(frame.pixels.selected.source.op_id);
            ids.extend(frame.pixels.conflicts.iter().map(|v| v.source.op_id));
        }
        if let Some(r) = &self.title {
            ids.insert(r.selected.source.op_id);
            ids.extend(r.conflicts.iter().map(|v| v.source.op_id));
        }
        if let Some(r) = &self.fps {
            ids.insert(r.selected.source.op_id);
            ids.extend(r.conflicts.iter().map(|v| v.source.op_id));
        }
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::{StudioProjection, StudioRecovery};
    use crate::RecoveryReason;

    #[test]
    fn studio_maximal_art_seed_bounds_conflict_fields_and_preserves_overflow_in_recovery() {
        let channel = [7; 16];
        let logical = flipnote_document(b"maximum-art-fixture", [9; 16]).unwrap();
        let mut nodes = BTreeMap::new();
        let mut pixels = BTreeMap::new();
        // Codec-size fixture built directly from typed nodes, not thousands of writes in one
        // Automerge transaction (which is neither an admissible change nor an admission test).
        // Actual signed edit/ingest/seed reconstruction is exercised by integration_tests.
        for n in 1u128..=1000 {
            let mut values = Vec::new();
            for a in 1..=6 {
                let author = DeviceId::from_bytes([a; 32]);
                let nonce = n.to_be_bytes();
                let source = FrameSource {
                    op_id: derived(&logical, &author, nonce),
                    author,
                    nonce,
                    ts: 1234,
                };
                let blob = FrameBlob {
                    cid: [a; 32],
                    bytes: 100,
                };
                nodes.insert(
                    source.op_id,
                    Node {
                        frame: nonce,
                        insertion: FrameValue {
                            source: source.clone(),
                            value: FrameInsertion {
                                checkpoint: false,
                                after: None,
                                anchor: None,
                                before: None,
                                blob: blob.clone(),
                            },
                        },
                    },
                );
                values.push(FrameValue {
                    source,
                    value: blob,
                });
            }
            values.sort_by_key(|v| v.source.op_id);
            // A second conflict field per frame also exercises the GLOBAL 1024-field cap.
            let selected = values.remove(0);
            pixels.insert(
                n.to_be_bytes(),
                FrameRegister {
                    selected,
                    conflicts: values,
                },
            );
        }
        let original = FlipnoteFrameProjection::from_parts(
            &logical,
            channel,
            0,
            None,
            None,
            nodes,
            pixels,
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(original.frames.len(), 1000);
        assert_eq!(original.over_cap.len(), 1);
        assert!(original.checkpoint_omits_evidence().unwrap());
        let seed = original.checkpoint([7; 32]).unwrap();
        assert!(seed.bytes().len() < MAX_CHECKPOINT_BYTES);
        let mut next = AutoCommit::new();
        next.apply_changes([automerge::Change::from_bytes(seed.bytes().to_vec()).unwrap()])
            .unwrap();
        let compact = FlipnoteFrameProjection::read(&logical, channel, 1, &next).unwrap();
        assert!(!compact.checkpoint_omits_evidence().unwrap());
        assert_eq!(compact.frames.len(), 999);
        assert_eq!(compact.timeline, original.timeline[..999]);
        assert_eq!(
            compact
                .frames
                .values()
                .map(|v| usize::from(v.insertions.len() > 1)
                    + usize::from(!v.pixels.conflicts.is_empty()))
                .sum::<usize>(),
            CONFLICT_FIELDS
        );
        assert!(compact
            .frames
            .values()
            .all(|v| v.insertions.len() <= 4 && v.pixels.conflicts.len() <= 3));
        let projection = StudioProjection::Flipnote(Box::new(original.clone()));
        let snapshot = StudioRecovery::snapshot(
            &projection,
            None,
            RecoveryReason::Rewound,
            [0; 32],
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            StudioRecovery::inspect_vault_references(&snapshot, &logical).unwrap(),
            (1..=6).map(|n| [n; 32]).collect(),
            "all conflicting/overflow alternatives pin"
        );
        assert_eq!(
            *StudioRecovery::from_snapshot(&snapshot, &logical, channel)
                .unwrap()
                .projection(),
            projection
        );
        let bytes = original
            .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
            .unwrap();
        assert_eq!(original.encode_snapshot(bytes.len()).unwrap(), bytes);
        assert!(matches!(
            original.encode_snapshot(bytes.len() - 1),
            Err(ReplError::EpochBound)
        ));
    }
}
