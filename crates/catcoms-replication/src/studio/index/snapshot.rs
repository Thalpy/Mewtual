//! StudioIndex's compact seed and complete local recovery payload. The seed is a baseline,
//! not synthetic author operations: subsequent normal root registers override its fallback.

use catcoms_wire::Decoder;

use super::*;
use crate::studio::snapshot::{self as codec, *};
use crate::{CheckpointSeed, VerifiedCheckpoint, VerifiedReceipt};

impl SnapshotValue for IndexCreation {
    fn put(&self, e: &mut Writer) -> Result<(), ReplError> {
        e.byte(match self.kind {
            StudioKind::Flipnote => 0,
            StudioKind::Score => 1,
        })?;
        self.title.put(e)?;
        e.bytes(self.created_by.as_bytes())?;
        super::super::integer_bound(self.ts)?;
        e.integer(self.ts)?;
        self.expiry.put(e)
    }
    fn get(d: &mut Decoder<'_>) -> Result<Self, ReplError> {
        let kind = match d.get_u8().map_err(malformed)? {
            0 => StudioKind::Flipnote,
            1 => StudioKind::Score,
            _ => return Err(ReplError::Malformed),
        };
        let title = String::get(d)?;
        let created_by = DeviceId::from_bytes(array(d)?);
        let ts = d.get_u64().map_err(malformed)?;
        super::super::integer_bound(ts)?;
        Ok(Self {
            kind,
            title,
            created_by,
            ts,
            expiry: StudioExpiry::get(d)?,
        })
    }
}

fn put_source(
    e: &mut Writer,
    document: &LogicalDocument,
    source: &IndexSource,
) -> Result<(), ReplError> {
    if source.op_id != derived(document, &source.author, source.nonce) {
        return Err(ReplError::Malformed);
    }
    e.bytes(source.author.as_bytes())?;
    e.bytes(&source.nonce)
}
fn get_source(d: &mut Decoder<'_>, document: &LogicalDocument) -> Result<IndexSource, ReplError> {
    let author = DeviceId::from_bytes(array(d)?);
    let nonce = array(d)?;
    Ok(IndexSource {
        op_id: derived(document, &author, nonce),
        author,
        nonce,
    })
}
fn put_value<T: SnapshotValue>(
    e: &mut Writer,
    doc: &LogicalDocument,
    v: &IndexValue<T>,
) -> Result<(), ReplError> {
    put_source(e, doc, &v.source)?;
    v.value.put(e)
}
fn get_value<T: SnapshotValue>(
    d: &mut Decoder<'_>,
    doc: &LogicalDocument,
) -> Result<IndexValue<T>, ReplError> {
    Ok(IndexValue {
        source: get_source(d, doc)?,
        value: T::get(d)?,
    })
}
fn put_register<T: SnapshotValue>(
    e: &mut Writer,
    doc: &LogicalDocument,
    r: &IndexRegister<T>,
) -> Result<(), ReplError> {
    put_value(e, doc, &r.selected)?;
    e.count(r.conflicts.len())?;
    let mut previous = None;
    for value in &r.conflicts {
        if value.source.op_id == r.selected.source.op_id
            || previous.is_some_and(|p| p >= value.source.op_id)
        {
            return Err(ReplError::Malformed);
        }
        previous = Some(value.source.op_id);
        put_value(e, doc, value)?;
    }
    Ok(())
}
fn get_register<T: SnapshotValue>(
    d: &mut Decoder<'_>,
    doc: &LogicalDocument,
) -> Result<IndexRegister<T>, ReplError> {
    let selected = get_value(d, doc)?;
    let n = count(d, MAX_VALUES)?;
    let mut conflicts = Vec::new();
    for _ in 0..n {
        let value: IndexValue<T> = get_value(d, doc)?;
        if value.source.op_id == selected.source.op_id
            || conflicts
                .last()
                .is_some_and(|p: &IndexValue<T>| p.source.op_id >= value.source.op_id)
        {
            return Err(ReplError::Malformed);
        }
        conflicts.push(value);
    }
    Ok(IndexRegister {
        selected,
        conflicts,
    })
}
fn put_objects(
    e: &mut Writer,
    doc: &LogicalDocument,
    map: &BTreeMap<ElementId, IndexEntry>,
) -> Result<(), ReplError> {
    e.count(map.len())?;
    for (id, entry) in map {
        e.bytes(id)?;
        if entry.creations.is_empty() {
            return Err(ReplError::Malformed);
        }
        e.count(entry.creations.len())?;
        let mut previous = None;
        for creation in &entry.creations {
            if creation.value.created_by != creation.source.author
                || previous.is_some_and(|p| p >= creation.source.op_id)
            {
                return Err(ReplError::Malformed);
            }
            previous = Some(creation.source.op_id);
            put_value(e, doc, creation)?;
        }
        put_register(e, doc, &entry.title)?;
        put_register(e, doc, &entry.expiry)?;
    }
    Ok(())
}
fn get_objects(
    d: &mut Decoder<'_>,
    doc: &LogicalDocument,
    left: &mut usize,
) -> Result<BTreeMap<ElementId, IndexEntry>, ReplError> {
    let n = count(d, *left)?;
    *left -= n;
    let mut map = BTreeMap::new();
    for _ in 0..n {
        let id = array(d)?;
        if map.last_key_value().is_some_and(|(p, _)| p >= &id) {
            return Err(ReplError::Malformed);
        }
        let n = count(d, MAX_VALUES)?;
        if n == 0 {
            return Err(ReplError::Malformed);
        }
        let mut creations = Vec::new();
        for _ in 0..n {
            let v: IndexValue<IndexCreation> = get_value(d, doc)?;
            if v.source.author != v.value.created_by
                || creations
                    .last()
                    .is_some_and(|p: &IndexValue<IndexCreation>| p.source.op_id >= v.source.op_id)
            {
                return Err(ReplError::Malformed);
            }
            creations.push(v);
        }
        map.insert(
            id,
            IndexEntry {
                creations,
                title: get_register(d, doc)?,
                expiry: get_register(d, doc)?,
            },
        );
    }
    Ok(map)
}

impl StudioIndexProjection {
    /// Deterministic seed using the existing P1 builder. Only the 64 visible objects and bounded
    /// conflicts carry forward. The caller MUST retain excluded/deleted/overflow evidence through
    /// the existing recovery-first settlement before replacing anything. This method persists nothing.
    pub fn checkpoint(&self, close: [u8; 32]) -> Result<CheckpointSeed, ReplError> {
        let mut next = self.compact()?;
        next.epoch = self.epoch.checked_add(1).ok_or(ReplError::EpochBound)?;
        codec::build(
            &self.document,
            next.epoch,
            close,
            header(&self.document, next.epoch),
            next.encode_snapshot(MAX_CHECKPOINT_BYTES)?,
        )
    }

    pub fn verify_checkpoint(
        receipt: &VerifiedReceipt,
        bytes: &[u8],
    ) -> Result<VerifiedCheckpoint, ReplError> {
        CheckpointSeed::verify(receipt, bytes, |document, epoch, doc| {
            let mut projection = Self::read(document, epoch, doc)?;
            projection.epoch = epoch.checked_sub(1).ok_or(ReplError::EpochScope)?;
            if projection.checkpoint(receipt.close_record_hash())?.bytes() != bytes {
                return Err(ReplError::Malformed);
            }
            Ok(())
        })
    }

    fn compact(&self) -> Result<Self, ReplError> {
        self.write_snapshot(&mut Writer::measure(
            crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES,
        ))?;
        let mut budget = ConflictBudget(CONFLICT_FIELDS);
        let mut objects = BTreeMap::new();
        for (id, entry) in &self.objects {
            if entry.creations.is_empty() {
                return Err(ReplError::Malformed);
            }
            let value = IndexEntry {
                creations: entry.creations[..1 + budget.retained(entry.creations.len() - 1)]
                    .to_vec(),
                title: IndexRegister {
                    selected: entry.title.selected.clone(),
                    conflicts: entry.title.conflicts
                        [..budget.retained(entry.title.conflicts.len())]
                        .to_vec(),
                },
                expiry: IndexRegister {
                    selected: entry.expiry.selected.clone(),
                    conflicts: entry.expiry.conflicts
                        [..budget.retained(entry.expiry.conflicts.len())]
                        .to_vec(),
                },
            };
            objects.insert(*id, value);
        }
        Ok(Self {
            document: self.document.clone(),
            epoch: self.epoch,
            objects,
            overflow: BTreeMap::new(),
            deleted_objects: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        })
    }

    fn validate_partition(&self) -> Result<(), ReplError> {
        let channel = self
            .document
            .logical_key
            .as_slice()
            .try_into()
            .map_err(|_| ReplError::EpochScope)?;
        if self.document != studio_index_document(&self.document.server_id, channel)? {
            return Err(ReplError::EpochScope);
        }
        if self.objects.len() > MAX_INDEX_OBJECTS
            || self.objects.len() + self.overflow.len() + self.deleted_objects.len() > MAX_VALUES
        {
            return Err(ReplError::EpochBound);
        }
        if !self.overflow.is_empty()
            && (self.objects.len() != MAX_INDEX_OBJECTS
                || self.objects.last_key_value().map(|(k, _)| k)
                    >= self.overflow.first_key_value().map(|(k, _)| k))
        {
            return Err(ReplError::Malformed);
        }
        if self
            .objects
            .keys()
            .chain(self.overflow.keys())
            .any(|id| self.tombstones.contains_key(id) || self.deleted_objects.contains_key(id))
            || self
                .deleted_objects
                .keys()
                .any(|id| !self.tombstones.contains_key(id))
        {
            return Err(ReplError::Malformed);
        }
        Ok(())
    }

    /// Complete typed projection, including conflicts beyond the checkpoint limits. The sealed
    /// recovery envelope owns this payload; it is never parsed as a network mutation or receipt.
    pub(in crate::studio) fn encode_snapshot(&self, limit: usize) -> Result<Vec<u8>, ReplError> {
        let mut e = Writer::new(limit);
        self.write_snapshot(&mut e)?;
        Ok(e.finish())
    }

    fn write_snapshot(&self, e: &mut Writer) -> Result<(), ReplError> {
        self.validate_partition()?;
        payload_header(
            e,
            &self.document,
            self.document
                .logical_key
                .as_slice()
                .try_into()
                .map_err(|_| ReplError::EpochScope)?,
            self.epoch,
        )?;
        for map in [&self.objects, &self.overflow, &self.deleted_objects] {
            put_objects(e, &self.document, map)?;
        }
        e.count(self.tombstones.len())?;
        for (id, sources) in &self.tombstones {
            e.bytes(id)?;
            if sources.is_empty() {
                return Err(ReplError::Malformed);
            }
            e.count(sources.len())?;
            let mut previous = None;
            for source in sources {
                if previous.is_some_and(|p| p >= source.op_id) {
                    return Err(ReplError::Malformed);
                }
                previous = Some(source.op_id);
                put_source(e, &self.document, source)?;
            }
        }
        Ok(())
    }

    pub(in crate::studio) fn decode_snapshot(
        document: &LogicalDocument,
        epoch: u64,
        bytes: &[u8],
    ) -> Result<Self, ReplError> {
        payload_bound(bytes)?;
        let mut d = Decoder::new(bytes);
        check_header(
            &mut d,
            document,
            document
                .logical_key
                .as_slice()
                .try_into()
                .map_err(|_| ReplError::EpochScope)?,
            epoch,
        )?;
        let mut left = MAX_VALUES;
        let objects = get_objects(&mut d, document, &mut left)?;
        let overflow = get_objects(&mut d, document, &mut left)?;
        let deleted_objects = get_objects(&mut d, document, &mut left)?;
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
                    .is_some_and(|p: &IndexSource| p.op_id >= source.op_id)
                {
                    return Err(ReplError::Malformed);
                }
                values.push(source);
            }
            tombstones.insert(id, values);
        }
        d.finish().map_err(malformed)?;
        let result = Self {
            document: document.clone(),
            epoch,
            objects,
            overflow,
            deleted_objects,
            tombstones,
        };
        result.validate_partition()?;
        Ok(result)
    }

    pub(super) fn decode_baseline(
        document: &LogicalDocument,
        epoch: u64,
        bytes: &[u8],
    ) -> Result<Self, ReplError> {
        if epoch == 0 || bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(ReplError::EpochScope);
        }
        let result = Self::decode_snapshot(document, epoch, bytes)?;
        if result.compact()? != result {
            return Err(ReplError::Malformed);
        }
        Ok(result)
    }

    pub(super) fn source_ids(&self) -> std::collections::BTreeSet<[u8; 32]> {
        self.objects
            .values()
            .flat_map(|entry| {
                entry
                    .creations
                    .iter()
                    .map(|v| v.source.op_id)
                    .chain(std::iter::once(entry.title.selected.source.op_id))
                    .chain(entry.title.conflicts.iter().map(|v| v.source.op_id))
                    .chain(std::iter::once(entry.expiry.selected.source.op_id))
                    .chain(entry.expiry.conflicts.iter().map(|v| v.source.op_id))
            })
            .collect()
    }
}
