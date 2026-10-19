//! Bounded score materialization. Records are internally consistent provenance claims, not
//! authentication: P1 and the causal validator must bind them to signed member changes.

use std::collections::{BTreeMap, BTreeSet};

use automerge::{AutoCommit, ChangeHash, ReadDoc, ScalarValue, Value as AmValue, ROOT};
use catcoms_crypto::DeviceId;

use super::*;
use crate::checkpoint::am_error;
use crate::epoch::{MAX_EPOCH_BYTES, MAX_EPOCH_OPERATIONS};
use crate::studio::{IndexRegister, IndexSource, IndexValue};
use crate::MAX_CHECKPOINT_BYTES;

pub type ScoreAddress = (u16, u8);
/// Scores use the Index provenance shape: full author, original nonce and derived operation id.
/// There is no invented per-cell timestamp or new author/signature mechanism.
pub type ScoreSource = IndexSource;
pub type ScoreValue<T> = IndexValue<T>;
pub type ScoreRegister<T> = IndexRegister<T>;
pub(super) const MAX_PRIMITIVES: u64 = (MAX_EPOCH_OPERATIONS * 7 + 8192) as u64;
pub(super) const MAX_READ_BYTES: usize = MAX_EPOCH_BYTES + MAX_CHECKPOINT_BYTES;
const MAX_KEY_BYTES: usize = 71;
pub(super) const MAX_RECORD_BYTES: usize = 33 + MAX_DOMAIN_OP_BYTES;

/// Raw Automerge winner and ALL live alternatives. Effective removal is deliberately distinct
/// from that winner: an unobserved concurrent Clear/Remove still dominates a winning Set.
#[derive(Clone, PartialEq, Eq)]
pub struct ScoreSlot<T> {
    pub register: ScoreRegister<Option<T>>,
}
impl<T> ScoreSlot<T> {
    pub fn removed(&self) -> bool {
        self.values().any(|v| v.value.is_none())
    }
    pub fn value(&self) -> Option<&T> {
        if self.removed() {
            None
        } else {
            self.register.selected.value.as_ref()
        }
    }
    pub fn values(&self) -> impl Iterator<Item = &ScoreValue<Option<T>>> {
        std::iter::once(&self.register.selected).chain(self.register.conflicts.iter())
    }
    /// Stable diagnostic representative, never a substitute for retaining all removals.
    pub fn removal_source(&self) -> Option<&ScoreSource> {
        self.values()
            .filter(|v| v.value.is_none())
            .map(|v| &v.source)
            .min_by_key(|s| s.op_id)
    }
}
impl<T> std::fmt::Debug for ScoreSlot<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScoreSlot").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScoreCellIssues {
    pub count: bool,
    pub extent: bool,
    pub recipe: bool,
    pub overlap: bool,
    pub conflict: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScorePatchIssues {
    pub count: bool,
    pub conflict: bool,
}
/// Reasons a resolved export is unavailable. Evidence remains editable/recoverable, not dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct ScoreReadiness {
    pub cells: BTreeMap<ScoreAddress, ScoreCellIssues>,
    pub patches: BTreeMap<ContentId, ScorePatchIssues>,
    pub header_conflicts: bool,
}
impl ScoreReadiness {
    pub fn ready(&self) -> bool {
        self.cells.is_empty() && self.patches.is_empty() && !self.header_conflicts
    }
}
impl std::fmt::Debug for ScoreReadiness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScoreReadiness").finish_non_exhaustive()
    }
}

/// Complete bounded live state. Removed slots and losing alternatives are retained until the
/// existing recovery-first checkpoint path safely retires them. Defaults carry no source claim.
#[derive(Clone, PartialEq, Eq)]
pub struct ScoreProjection {
    pub(super) document: LogicalDocument,
    pub channel: ElementId,
    pub epoch: u64,
    pub title: Option<ScoreRegister<String>>,
    pub bpm: Option<ScoreRegister<u16>>,
    pub bpb: Option<ScoreRegister<u8>>,
    pub steps: Option<ScoreRegister<u16>>,
    pub cells: BTreeMap<ScoreAddress, ScoreSlot<ScoreCell>>,
    pub patches: BTreeMap<ContentId, ScoreSlot<StudioPatch>>,
}
impl std::fmt::Debug for ScoreProjection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScoreProjection").finish_non_exhaustive()
    }
}

/// Same logical StudioObject namespace as Flipnote; trusted target kind selects this reader.
pub fn score_document(server: &[u8], object: ElementId) -> Result<LogicalDocument, ReplError> {
    LogicalDocument::new(
        server.to_vec(),
        catcoms_wire::DocType::StudioObject,
        object.to_vec(),
    )
}

impl ScoreProjection {
    pub fn document(&self) -> &LogicalDocument {
        &self.document
    }
    pub fn effective_bpm(&self) -> u16 {
        self.bpm.as_ref().map_or(120, |r| r.selected.value)
    }
    pub fn effective_bpb(&self) -> u8 {
        self.bpb.as_ref().map_or(4, |r| r.selected.value)
    }
    pub fn effective_steps(&self) -> u16 {
        self.steps.as_ref().map_or(16, |r| r.selected.value)
    }
    pub(super) fn empty(document: LogicalDocument, channel: ElementId, epoch: u64) -> Self {
        Self {
            document,
            channel,
            epoch,
            title: None,
            bpm: None,
            bpb: None,
            steps: None,
            cells: BTreeMap::new(),
            patches: BTreeMap::new(),
        }
    }

    /// Current state under independently supplied logical/channel/epoch scope, never a hint's
    /// self-asserted binding. This does not authenticate records or run blob/publication work.
    pub fn read(
        document: &LogicalDocument,
        channel: ElementId,
        epoch: u64,
        doc: &AutoCommit,
    ) -> Result<Self, ReplError> {
        Self::read_view(
            document,
            channel,
            epoch,
            doc,
            None,
            (MAX_PRIMITIVES, MAX_READ_BYTES),
        )
    }
    pub(super) fn read_at(
        document: &LogicalDocument,
        channel: ElementId,
        epoch: u64,
        doc: &AutoCommit,
        heads: &[ChangeHash],
    ) -> Result<Self, ReplError> {
        if heads.len() > MAX_EPOCH_OPERATIONS {
            return Err(ReplError::EpochBound);
        }
        if heads.iter().any(|h| doc.get_change_by_hash(h).is_none()) {
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
        if *document != score_document(&document.server_id, object)? {
            return Err(ReplError::EpochScope);
        }
        // Check before get_all allocates a malicious concentration of concurrent values.
        let stats = doc.stats();
        if stats.num_ops > primitive_limit {
            return Err(ReplError::EpochBound);
        }
        let view = View { doc, heads };
        let seed = view.all(crate::studio::snapshot::SEED_KEY)?;
        let mut result = if epoch > 0 {
            if seed.len() != 1 {
                return Err(ReplError::Malformed);
            }
            Self::decode_baseline(document, channel, epoch, record_bytes(&seed[0].0)?)?
        } else {
            if !seed.is_empty() {
                return Err(ReplError::Malformed);
            }
            Self::empty(document.clone(), channel, epoch)
        };
        let inherited = result.source_ids();
        let expected = header(document, channel, epoch);
        let mut header_count = 0;
        let mut key_count = 0;
        let mut budget = Budget {
            used: 0,
            limit: byte_limit,
        };
        let mut operations = BTreeMap::new();
        for key in view.keys() {
            key_count += 1;
            if key.len() > MAX_KEY_BYTES {
                return Err(ReplError::EpochBound);
            }
            budget.add(key.len())?;
            let values = view.all(&key)?;
            if values.is_empty() {
                return Err(ReplError::Malformed);
            }
            if key == crate::studio::snapshot::SEED_KEY {
                budget.add(record_bytes(&values[0].0)?.len())?;
                continue;
            }
            for (value, _) in &values {
                budget.value(value)?;
            }
            if let Some(expected) = expected.get(&key) {
                if values.iter().any(|(v, _)| !scalar_eq(v, expected)) {
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
            let winner = view.winner(&key)?.ok_or(ReplError::Malformed)?.1;
            let mut candidates = BTreeMap::new();
            let mut selected = None;
            for (value, am_id) in values {
                let bytes = record_bytes(&value)?;
                let (source, operation) = decode_record(document, bytes)?;
                if operation_key(&operation) != key {
                    return Err(ReplError::Malformed);
                }
                if inherited.contains(&source.op_id) {
                    return Err(ReplError::IntentConflict);
                }
                let digest = *blake3::hash(bytes).as_bytes();
                if operations
                    .insert(source.op_id, digest)
                    .is_some_and(|old| old != digest)
                {
                    return Err(ReplError::IntentConflict);
                }
                if am_id == winner {
                    selected = Some(source.op_id);
                }
                candidates.insert(
                    source.op_id,
                    ScoreValue {
                        source,
                        value: operation,
                    },
                );
            }
            let selected = candidates
                .remove(&selected.ok_or(ReplError::Malformed)?)
                .ok_or(ReplError::Malformed)?;
            let register = ScoreRegister {
                selected,
                conflicts: candidates.into_values().collect(),
            };
            // An explicit post-seed property REPLACES its whole fallback, including seeded
            // removals/conflicts. Merging fallback alternatives here makes removal permanent.
            if let Some(address) = key.strip_prefix("c/") {
                let address = parse_address(address)?;
                result.cells.insert(
                    address,
                    ScoreSlot {
                        register: map_register(register, |op| match op {
                            ScoreOp::SetCell { value, .. } => Ok(Some(value)),
                            ScoreOp::ClearCell { .. } => Ok(None),
                            _ => Err(ReplError::Malformed),
                        })?,
                    },
                );
            } else if let Some(patch) = key.strip_prefix("p/") {
                let patch = fixed_hex(patch)?;
                result.patches.insert(
                    patch,
                    ScoreSlot {
                        register: map_register(register, |op| match op {
                            ScoreOp::SetPatch { descriptor, .. } => Ok(Some(descriptor)),
                            ScoreOp::RemovePatch { .. } => Ok(None),
                            _ => Err(ReplError::Malformed),
                        })?,
                    },
                );
            } else {
                match key.as_str() {
                    "h/title" => {
                        result.title = Some(map_register(register, |op| match op {
                            ScoreOp::SetHeader(ScoreHeader::Title(v)) => Ok(v),
                            _ => Err(ReplError::Malformed),
                        })?)
                    }
                    "h/bpm" => {
                        result.bpm = Some(map_register(register, |op| match op {
                            ScoreOp::SetHeader(ScoreHeader::Bpm(v)) => Ok(v),
                            _ => Err(ReplError::Malformed),
                        })?)
                    }
                    "h/bpb" => {
                        result.bpb = Some(map_register(register, |op| match op {
                            ScoreOp::SetHeader(ScoreHeader::Bpb(v)) => Ok(v),
                            _ => Err(ReplError::Malformed),
                        })?)
                    }
                    "h/steps" => {
                        result.steps = Some(map_register(register, |op| match op {
                            ScoreOp::SetHeader(ScoreHeader::Steps(v)) => Ok(v),
                            _ => Err(ReplError::Malformed),
                        })?)
                    }
                    _ => return Err(ReplError::Malformed),
                }
            }
        }
        let pristine = key_count == 0
            && epoch == 0
            && match heads {
                Some(heads) => heads.is_empty(),
                None => stats.num_ops == 0 && stats.num_changes == 0,
            };
        if header_count != expected.len() && !pristine {
            return Err(ReplError::Malformed);
        }
        Ok(result)
    }

    /// Derive bounded readiness without deleting authored material. Caller-mutable projection
    /// fields are measured/validated before allocating derived flags or walking overlap intervals.
    pub fn readiness(&self) -> Result<ScoreReadiness, ReplError> {
        self.measure_snapshot()?;
        let mut result = ScoreReadiness {
            cells: BTreeMap::new(),
            patches: BTreeMap::new(),
            header_conflicts: self.title.as_ref().is_some_and(|r| !r.conflicts.is_empty())
                || self.bpm.as_ref().is_some_and(|r| !r.conflicts.is_empty())
                || self.bpb.as_ref().is_some_and(|r| !r.conflicts.is_empty())
                || self.steps.as_ref().is_some_and(|r| !r.conflicts.is_empty()),
        };
        let active_patches: BTreeSet<_> = self
            .patches
            .iter()
            .filter(|(_, slot)| slot.value().is_some())
            .map(|(id, _)| *id)
            .collect();
        let admitted_patches: BTreeSet<_> = active_patches
            .iter()
            .take(SCORE_MAX_PATCHES)
            .copied()
            .collect();
        for (id, slot) in &self.patches {
            if slot.value().is_none() {
                continue;
            }
            let issues = ScorePatchIssues {
                count: !admitted_patches.contains(id),
                conflict: !slot.register.conflicts.is_empty(),
            };
            if issues != ScorePatchIssues::default() {
                result.patches.insert(*id, issues);
            }
        }
        let mut active = 0;
        let mut held: BTreeMap<u8, Vec<(u16, ScoreAddress)>> = BTreeMap::new();
        for (address @ (step, row), slot) in &self.cells {
            let Some(cell) = slot.value() else {
                continue;
            };
            active += 1;
            let mut issues = ScoreCellIssues {
                count: active > SCORE_MAX_CELLS,
                extent: *step >= self.effective_steps(),
                conflict: !slot.register.conflicts.is_empty(),
                ..ScoreCellIssues::default()
            };
            if let ScoreCell::Note { patch, len, .. } = cell {
                let end = *step + len.unwrap_or(1); // static validation proved global extent.
                issues.extent |= end > self.effective_steps();
                issues.recipe = !admitted_patches.contains(patch);
                let intervals = held.entry(*row).or_default();
                intervals.retain(|(end, _)| *end > *step);
                for (_, other) in intervals.iter() {
                    result.cells.entry(*other).or_default().overlap = true;
                    issues.overlap = true;
                }
                intervals.push((end, *address));
            }
            if issues != ScoreCellIssues::default() {
                let entry = result.cells.entry(*address).or_default();
                issues.overlap |= entry.overlap;
                *entry = issues;
            }
        }
        Ok(result)
    }
}

pub(super) fn header(
    document: &LogicalDocument,
    channel: ElementId,
    epoch: u64,
) -> BTreeMap<String, ScalarValue> {
    BTreeMap::from([
        ("v".into(), ScalarValue::Uint(1)),
        ("kind".into(), ScalarValue::Str("score".into())),
        (
            "id".into(),
            ScalarValue::Str(hex(&document.logical_key).into()),
        ),
        ("channel".into(), ScalarValue::Str(hex(&channel).into())),
        ("epoch".into(), ScalarValue::Uint(epoch)),
    ])
}
pub(super) fn operation_key(operation: &ScoreOp) -> String {
    match operation {
        ScoreOp::SetCell { step, row, .. } | ScoreOp::ClearCell { step, row } => {
            format!("c/{step}:{row}")
        }
        ScoreOp::SetPatch { patch, .. } | ScoreOp::RemovePatch { patch } => {
            format!("p/{}", hex(patch))
        }
        ScoreOp::SetHeader(header) => format!(
            "h/{}",
            match header {
                ScoreHeader::Title(_) => "title",
                ScoreHeader::Bpm(_) => "bpm",
                ScoreHeader::Bpb(_) => "bpb",
                ScoreHeader::Steps(_) => "steps",
            }
        ),
    }
}
fn parse_address(value: &str) -> Result<ScoreAddress, ReplError> {
    let (step, row) = value.split_once(':').ok_or(ReplError::Malformed)?;
    let step: u16 = step.parse().map_err(|_| ReplError::Malformed)?;
    let row: u8 = row.parse().map_err(|_| ReplError::Malformed)?;
    address(step, row)?;
    if format!("{step}:{row}") != value {
        return Err(ReplError::Malformed);
    }
    Ok((step, row))
}
pub(super) fn record(domain: &DomainOp, author: &DeviceId) -> Result<(String, Vec<u8>), ReplError> {
    let operation = ScoreOp::decode(&domain.body)?;
    let mut bytes = vec![1];
    bytes.extend_from_slice(author.as_bytes());
    bytes.extend_from_slice(&domain.encode()?);
    Ok((operation_key(&operation), bytes))
}
pub(super) fn decode_record(
    document: &LogicalDocument,
    bytes: &[u8],
) -> Result<(ScoreSource, ScoreOp), ReplError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(ReplError::EpochBound);
    }
    if bytes.len() < 33 || bytes[0] != 1 {
        return Err(ReplError::Malformed);
    }
    let author = DeviceId::from_bytes(bytes[1..33].try_into().map_err(|_| ReplError::Malformed)?);
    let domain = DomainOp::decode(&bytes[33..])?;
    let operation = ScoreOp::decode_domain(document, &domain)?;
    Ok((
        ScoreSource {
            op_id: domain.id(&author),
            author,
            nonce: domain.nonce,
        },
        operation,
    ))
}
fn map_register<T, U>(
    register: ScoreRegister<T>,
    mut map: impl FnMut(T) -> Result<U, ReplError>,
) -> Result<ScoreRegister<U>, ReplError> {
    let selected = ScoreValue {
        source: register.selected.source,
        value: map(register.selected.value)?,
    };
    let conflicts = register
        .conflicts
        .into_iter()
        .map(|v| {
            Ok(ScoreValue {
                source: v.source,
                value: map(v.value)?,
            })
        })
        .collect::<Result<_, ReplError>>()?;
    Ok(ScoreRegister {
        selected,
        conflicts,
    })
}
pub(super) fn scalar_eq(value: &AmValue<'_>, expected: &ScalarValue) -> bool {
    matches!(value, AmValue::Scalar(v) if v.as_ref() == expected)
}
pub(super) fn record_bytes<'a>(value: &'a AmValue<'_>) -> Result<&'a [u8], ReplError> {
    match value {
        AmValue::Scalar(v) => match v.as_ref() {
            ScalarValue::Bytes(v) => Ok(v),
            _ => Err(ReplError::Malformed),
        },
        _ => Err(ReplError::Malformed),
    }
}
struct View<'a> {
    doc: &'a AutoCommit,
    heads: Option<&'a [ChangeHash]>,
}
impl View<'_> {
    fn all(&self, key: &str) -> Result<Vec<(AmValue<'_>, automerge::ObjId)>, ReplError> {
        match self.heads {
            Some(h) => self.doc.get_all_at(ROOT, key, h),
            None => self.doc.get_all(ROOT, key),
        }
        .map_err(am_error)
    }
    fn winner(&self, key: &str) -> Result<Option<(AmValue<'_>, automerge::ObjId)>, ReplError> {
        match self.heads {
            Some(h) => self.doc.get_at(ROOT, key, h),
            None => self.doc.get(ROOT, key),
        }
        .map_err(am_error)
    }
    fn keys(&self) -> Box<dyn Iterator<Item = String> + '_> {
        match self.heads {
            Some(h) => Box::new(self.doc.keys_at(ROOT, h)),
            None => Box::new(self.doc.keys(ROOT)),
        }
    }
}
struct Budget {
    used: usize,
    limit: usize,
}
impl Budget {
    fn add(&mut self, n: usize) -> Result<(), ReplError> {
        self.used = self.used.checked_add(n).ok_or(ReplError::EpochBound)?;
        if self.used > self.limit {
            return Err(ReplError::EpochBound);
        }
        Ok(())
    }
    fn value(&mut self, value: &AmValue<'_>) -> Result<(), ReplError> {
        let AmValue::Scalar(value) = value else {
            return Err(ReplError::Malformed);
        };
        let n = match value.as_ref() {
            ScalarValue::Bytes(b) if b.len() <= MAX_RECORD_BYTES => b.len(),
            ScalarValue::Str(s) if s.len() <= MAX_KEY_BYTES => s.len(),
            ScalarValue::Uint(_) => 8,
            ScalarValue::Bytes(_) | ScalarValue::Str(_) => return Err(ReplError::EpochBound),
            _ => return Err(ReplError::Malformed),
        };
        self.add(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::{transaction::Transactable, ActorId};
    #[test]
    fn score_reader_charges_primitives_before_conflicts_and_all_bytes_before_decode() {
        let document = score_document(b"read-bounds", [1; 16]).unwrap();
        let mut base = AutoCommit::new().with_actor(ActorId::from(vec![1; 32]));
        for (key, value) in header(&document, [2; 16], 0) {
            base.put(ROOT, key, value).unwrap();
        }
        base.commit();
        let frontier = base.clone();
        for who in 2..20u8 {
            let mut branch = frontier.clone().with_actor(ActorId::from(vec![who; 32]));
            let domain = DomainOp {
                nonce: [who; 16],
                doc_type: DocType::StudioObject,
                logical_key: vec![1; 16],
                body: ScoreOp::ClearCell { step: 0, row: 0 }.encode().unwrap(),
            };
            let (key, bytes) = record(&domain, &DeviceId::from_bytes([who; 32])).unwrap();
            branch.put(ROOT, key, bytes).unwrap();
            branch.commit();
            base.merge(&mut branch).unwrap();
        }
        assert_eq!(
            ScoreProjection::read(&document, [2; 16], 0, &base)
                .unwrap()
                .cells[&(0, 0)]
                .register
                .conflicts
                .len(),
            17
        );
        assert!(matches!(
            ScoreProjection::read_view(&document, [2; 16], 0, &base, None, (5, MAX_READ_BYTES)),
            Err(ReplError::EpochBound)
        ));
        assert!(matches!(
            ScoreProjection::read_view(&document, [2; 16], 0, &base, None, (MAX_PRIMITIVES, 100)),
            Err(ReplError::EpochBound)
        ));
        assert!(matches!(
            ScoreProjection::read_at(&document, [2; 16], 0, &base, &[ChangeHash([255; 32])]),
            Err(ReplError::EpochScope)
        ));
        // Known historical empty heads are empty past, not receiver-current headers/conflicts.
        assert!(ScoreProjection::read_at(&document, [2; 16], 0, &base, &[])
            .unwrap()
            .cells
            .is_empty());
    }
}
