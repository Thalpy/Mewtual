//! Explicit score-family payloads preserve existing Index/art bytes. Seeds are bounded fallback
//! values, not historical operations; all omitted evidence belongs in recovery before replacement.
use super::state::*;
use super::*;
use crate::studio::snapshot::{self as codec, *};
use crate::{CheckpointSeed, VerifiedCheckpoint, VerifiedReceipt, MAX_CHECKPOINT_BYTES};
use catcoms_wire::Decoder;
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::studio) const SCORE_PAYLOAD: u8 = 2;
fn put_source(e: &mut Writer, doc: &LogicalDocument, s: &ScoreSource) -> Result<(), ReplError> {
    if s.op_id != derived(doc, &s.author, s.nonce) {
        return Err(ReplError::Malformed);
    }
    e.bytes(s.author.as_bytes())?;
    e.bytes(&s.nonce)
}
fn get_source(d: &mut Decoder<'_>, doc: &LogicalDocument) -> Result<ScoreSource, ReplError> {
    let author = catcoms_crypto::DeviceId::from_bytes(array(d)?);
    let nonce = array(d)?;
    Ok(ScoreSource {
        op_id: derived(doc, &author, nonce),
        author,
        nonce,
    })
}
fn put_register<T>(
    e: &mut Writer,
    doc: &LogicalDocument,
    r: &ScoreRegister<T>,
    put: &impl Fn(&T, &mut Writer) -> Result<(), ReplError>,
) -> Result<(), ReplError> {
    put_source(e, doc, &r.selected.source)?;
    put(&r.selected.value, e)?;
    e.count(r.conflicts.len())?;
    let mut previous = None;
    for value in &r.conflicts {
        if value.source.op_id == r.selected.source.op_id
            || previous.is_some_and(|id| id >= value.source.op_id)
        {
            return Err(ReplError::Malformed);
        }
        previous = Some(value.source.op_id);
        put_source(e, doc, &value.source)?;
        put(&value.value, e)?;
    }
    Ok(())
}
fn get_register<T>(
    d: &mut Decoder<'_>,
    doc: &LogicalDocument,
    get: &impl Fn(&mut Decoder<'_>) -> Result<T, ReplError>,
) -> Result<ScoreRegister<T>, ReplError> {
    let selected = ScoreValue {
        source: get_source(d, doc)?,
        value: get(d)?,
    };
    let n = count(d, MAX_VALUES)?;
    let mut conflicts = Vec::new();
    for _ in 0..n {
        let value = ScoreValue {
            source: get_source(d, doc)?,
            value: get(d)?,
        };
        if value.source.op_id == selected.source.op_id
            || conflicts
                .last()
                .is_some_and(|p: &ScoreValue<T>| p.source.op_id >= value.source.op_id)
        {
            return Err(ReplError::Malformed);
        }
        conflicts.push(value);
    }
    Ok(ScoreRegister {
        selected,
        conflicts,
    })
}
fn put_optional<T>(
    e: &mut Writer,
    doc: &LogicalDocument,
    r: &Option<ScoreRegister<T>>,
    put: &impl Fn(&T, &mut Writer) -> Result<(), ReplError>,
) -> Result<(), ReplError> {
    e.byte(u8::from(r.is_some()))?;
    if let Some(r) = r {
        put_register(e, doc, r, put)?;
    }
    Ok(())
}
fn get_optional<T>(
    d: &mut Decoder<'_>,
    doc: &LogicalDocument,
    get: &impl Fn(&mut Decoder<'_>) -> Result<T, ReplError>,
) -> Result<Option<ScoreRegister<T>>, ReplError> {
    match d.get_u8().map_err(malformed)? {
        0 => Ok(None),
        1 => Ok(Some(get_register(d, doc, get)?)),
        _ => Err(ReplError::Malformed),
    }
}
fn put_cell(cell: &ScoreCell, e: &mut Writer) -> Result<(), ReplError> {
    match cell {
        ScoreCell::Note { patch, len } => {
            e.byte(0)?;
            e.bytes(patch)?;
            e.byte(u8::from(len.is_some()))?;
            if let Some(len) = len {
                e.integer(u64::from(*len))?;
            }
            Ok(())
        }
        ScoreCell::Drum { pad } => {
            e.byte(1)?;
            e.byte(*pad)
        }
    }
}
fn get_cell(d: &mut Decoder<'_>) -> Result<ScoreCell, ReplError> {
    match d.get_u8().map_err(malformed)? {
        0 => {
            let patch = array(d)?;
            let len = match d.get_u8().map_err(malformed)? {
                0 => None,
                1 => Some(get_u16(d)?),
                _ => return Err(ReplError::Malformed),
            };
            Ok(ScoreCell::Note { patch, len })
        }
        1 => Ok(ScoreCell::Drum {
            pad: d.get_u8().map_err(malformed)?,
        }),
        _ => Err(ReplError::Malformed),
    }
}
fn put_slot_value<T>(
    v: &Option<T>,
    e: &mut Writer,
    put: &impl Fn(&T, &mut Writer) -> Result<(), ReplError>,
) -> Result<(), ReplError> {
    e.byte(u8::from(v.is_some()))?;
    if let Some(v) = v {
        put(v, e)?;
    }
    Ok(())
}
fn get_slot_value<T>(
    d: &mut Decoder<'_>,
    get: &impl Fn(&mut Decoder<'_>) -> Result<T, ReplError>,
) -> Result<Option<T>, ReplError> {
    match d.get_u8().map_err(malformed)? {
        0 => Ok(None),
        1 => Ok(Some(get(d)?)),
        _ => Err(ReplError::Malformed),
    }
}
fn put_patch(p: &StudioPatch, e: &mut Writer) -> Result<(), ReplError> {
    e.bytes(&canonical(p.value())?)
}
fn get_patch(d: &mut Decoder<'_>) -> Result<StudioPatch, ReplError> {
    let bytes = d.get_bytes().map_err(malformed)?;
    let value = parse(bytes)?;
    let patch = StudioPatch::new(&value)?;
    if canonical(patch.value())? != bytes {
        return Err(ReplError::Malformed);
    }
    Ok(patch)
}
fn get_u16(d: &mut Decoder<'_>) -> Result<u16, ReplError> {
    u16::try_from(d.get_u64().map_err(malformed)?).map_err(|_| ReplError::Malformed)
}

impl ScoreProjection {
    pub(super) fn measure_snapshot(&self) -> Result<(), ReplError> {
        self.write_snapshot(&mut Writer::measure(
            crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES,
        ))
    }
    /// Computation only: the owner/store must preserve omitted evidence durably before replacing.
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
            let mut p = Self::read(document, channel, epoch, doc)?;
            p.epoch = epoch.checked_sub(1).ok_or(ReplError::EpochScope)?;
            if p.checkpoint(receipt.close_record_hash())?.bytes() != bytes {
                return Err(ReplError::Malformed);
            }
            Ok(())
        })
    }
    pub(in crate::studio) fn checkpoint_omits_evidence(&self) -> Result<bool, ReplError> {
        Ok(self.compact()? != *self)
    }
    fn compact(&self) -> Result<Self, ReplError> {
        self.measure_snapshot()?; // Charge/validate before cloning caller-mutable content.
        let mut next = self.clone();
        // Remove the WHOLE slot. Carrying a losing Set without its removal resurrects it.
        next.cells.retain(|_, s| !s.removed());
        next.patches.retain(|_, s| !s.removed());
        next.cells = next.cells.into_iter().take(SCORE_MAX_CELLS).collect();
        next.patches = next.patches.into_iter().take(SCORE_MAX_PATCHES).collect();
        let mut budget = ConflictBudget(CONFLICT_FIELDS);
        fn trim<T>(r: &mut ScoreRegister<T>, budget: &mut ConflictBudget) {
            r.conflicts.truncate(budget.retained(r.conflicts.len()));
        }
        if let Some(r) = &mut next.title {
            trim(r, &mut budget);
        }
        if let Some(r) = &mut next.bpm {
            trim(r, &mut budget);
        }
        if let Some(r) = &mut next.bpb {
            trim(r, &mut budget);
        }
        if let Some(r) = &mut next.steps {
            trim(r, &mut budget);
        }
        for slot in next.cells.values_mut() {
            trim(&mut slot.register, &mut budget);
        }
        for slot in next.patches.values_mut() {
            trim(&mut slot.register, &mut budget);
        }
        Ok(next)
    }
    pub(in crate::studio) fn encode_snapshot(&self, limit: usize) -> Result<Vec<u8>, ReplError> {
        let mut e = Writer::new(limit);
        self.write_snapshot(&mut e)?;
        Ok(e.finish())
    }
    fn write_snapshot(&self, e: &mut Writer) -> Result<(), ReplError> {
        let object = self
            .document
            .logical_key
            .as_slice()
            .try_into()
            .map_err(|_| ReplError::EpochScope)?;
        if self.document != score_document(&self.document.server_id, object)? {
            return Err(ReplError::EpochScope);
        }
        e.byte(SCORE_PAYLOAD)?;
        payload_header(e, &self.document, self.channel, self.epoch)?;
        let mut ids = BTreeMap::new();
        let mut validate = |source: &ScoreSource, op: ScoreOp| -> Result<(), ReplError> {
            if source.op_id != derived(&self.document, &source.author, source.nonce) {
                return Err(ReplError::Malformed);
            }
            let digest = *blake3::hash(&op.encode()?).as_bytes();
            if ids
                .insert(source.op_id, digest)
                .is_some_and(|old| old != digest)
            {
                return Err(ReplError::IntentConflict);
            }
            Ok(())
        };
        fn each<T>(
            r: &ScoreRegister<T>,
            mut f: impl FnMut(&ScoreValue<T>) -> Result<(), ReplError>,
        ) -> Result<(), ReplError> {
            if r.conflicts.len() > MAX_VALUES {
                return Err(ReplError::EpochBound);
            }
            for v in std::iter::once(&r.selected).chain(&r.conflicts) {
                f(v)?;
            }
            Ok(())
        }
        // Charge each collection BEFORE hashing/cloning its values for provenance checks. A
        // caller-mutated vector of huge titles must hit the aggregate rail after bounded work.
        put_optional(e, &self.document, &self.title, &|v, e| v.put(e))?;
        put_optional(e, &self.document, &self.bpm, &|v, e| {
            e.integer(u64::from(*v))
        })?;
        put_optional(e, &self.document, &self.bpb, &|v, e| e.byte(*v))?;
        put_optional(e, &self.document, &self.steps, &|v, e| {
            e.integer(u64::from(*v))
        })?;
        if let Some(r) = &self.title {
            each(r, |v| {
                title_bound(&v.value)?;
                validate(
                    &v.source,
                    ScoreOp::SetHeader(ScoreHeader::Title(v.value.clone())),
                )
            })?;
        }
        if let Some(r) = &self.bpm {
            each(r, |v| {
                validate(&v.source, ScoreOp::SetHeader(ScoreHeader::Bpm(v.value)))
            })?;
        }
        if let Some(r) = &self.bpb {
            each(r, |v| {
                validate(&v.source, ScoreOp::SetHeader(ScoreHeader::Bpb(v.value)))
            })?;
        }
        if let Some(r) = &self.steps {
            each(r, |v| {
                validate(&v.source, ScoreOp::SetHeader(ScoreHeader::Steps(v.value)))
            })?;
        }
        if self.cells.len() > usize::from(SCORE_MAX_STEPS) * usize::from(SCORE_ROWS) {
            return Err(ReplError::EpochBound);
        }
        e.count(self.cells.len())?;
        for ((step, row), slot) in &self.cells {
            address(*step, *row)?;
            e.integer(u64::from(*step))?;
            e.byte(*row)?;
            put_register(e, &self.document, &slot.register, &|v, e| {
                put_slot_value(v, e, &put_cell)
            })?;
            each(&slot.register, |v| {
                validate(
                    &v.source,
                    match &v.value {
                        Some(cell) => ScoreOp::SetCell {
                            step: *step,
                            row: *row,
                            value: cell.clone(),
                        },
                        None => ScoreOp::ClearCell {
                            step: *step,
                            row: *row,
                        },
                    },
                )
            })?;
        }
        e.count(self.patches.len())?;
        for (patch, slot) in &self.patches {
            e.bytes(patch)?;
            put_register(e, &self.document, &slot.register, &|v, e| {
                put_slot_value(v, e, &put_patch)
            })?;
            each(&slot.register, |v| {
                validate(
                    &v.source,
                    match &v.value {
                        Some(descriptor) => ScoreOp::SetPatch {
                            patch: *patch,
                            descriptor: descriptor.clone(),
                        },
                        None => ScoreOp::RemovePatch { patch: *patch },
                    },
                )
            })?;
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
        if d.get_u8().map_err(malformed)? != SCORE_PAYLOAD {
            return Err(ReplError::Malformed);
        }
        check_header(&mut d, document, channel, epoch)?;
        let mut p = Self::empty(document.clone(), channel, epoch);
        p.title = get_optional(&mut d, document, &String::get)?;
        p.bpm = get_optional(&mut d, document, &get_u16)?;
        p.bpb = get_optional(&mut d, document, &|d| d.get_u8().map_err(malformed))?;
        p.steps = get_optional(&mut d, document, &get_u16)?;
        let n = count(
            &mut d,
            usize::from(SCORE_MAX_STEPS) * usize::from(SCORE_ROWS),
        )?;
        for _ in 0..n {
            let key = (get_u16(&mut d)?, d.get_u8().map_err(malformed)?);
            address(key.0, key.1)?;
            if p.cells.last_key_value().is_some_and(|(old, _)| old >= &key) {
                return Err(ReplError::Malformed);
            }
            let register = get_register(&mut d, document, &|d| get_slot_value(d, &get_cell))?;
            p.cells.insert(key, ScoreSlot { register });
        }
        let n = count(&mut d, MAX_VALUES)?;
        for _ in 0..n {
            let key = array(&mut d)?;
            if p.patches
                .last_key_value()
                .is_some_and(|(old, _)| old >= &key)
            {
                return Err(ReplError::Malformed);
            }
            let register = get_register(&mut d, document, &|d| get_slot_value(d, &get_patch))?;
            p.patches.insert(key, ScoreSlot { register });
        }
        d.finish().map_err(malformed)?;
        if p.encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)? != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(p)
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
    pub(super) fn source_ids(&self) -> BTreeSet<[u8; 32]> {
        let mut ids = BTreeSet::new();
        fn extend<T>(ids: &mut BTreeSet<[u8; 32]>, r: &ScoreRegister<T>) {
            ids.insert(r.selected.source.op_id);
            ids.extend(r.conflicts.iter().map(|v| v.source.op_id));
        }
        if let Some(r) = &self.title {
            extend(&mut ids, r);
        }
        if let Some(r) = &self.bpm {
            extend(&mut ids, r);
        }
        if let Some(r) = &self.bpb {
            extend(&mut ids, r);
        }
        if let Some(r) = &self.steps {
            extend(&mut ids, r);
        }
        for s in self.cells.values() {
            extend(&mut ids, &s.register);
        }
        for s in self.patches.values() {
            extend(&mut ids, &s.register);
        }
        ids
    }
}
