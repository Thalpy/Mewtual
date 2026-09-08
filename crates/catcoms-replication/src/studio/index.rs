//! Read-only StudioIndex materialization. This is NOT a live P1 admission callback.
//!
//! Like the registry, the physical CRDT is flat: concurrent creation must not select one of
//! several nested map objects and silently hide the others. Immutable insertion candidates
//! have separate `i/<object>/<op_id>` properties. Mutable `t/<object>` and `e/<object>` properties
//! use Automerge registers. `d/<object>/<op_id>` retains epoch-local, remove-wins deletions
//! with their provenance, even when more than one author deletes the same id.
//!
//! A record is `1 || author[32] || DomainOp::encode()`. Recomputing its id and decoding its
//! body checks internal consistency, NOT authorship: a separate delta validator must bind the
//! record to the signed change, validate its causal predecessors, forbid replacing insertions
//! or deleting evidence, and run checkpoint preflight. A checkpoint needs separate receipt
//! verification. `validate_index_change` checks epoch-zero causal mutations, but no production
//! writer, checkpoint builder, actor command or publication path is enabled here.

use std::collections::BTreeMap;

use automerge::{AutoCommit, ReadDoc, ScalarValue, Value, ROOT};
use catcoms_crypto::DeviceId;
use catcoms_wire::DocType;

use super::{fixed_hex, ElementId, IndexOp, StudioExpiry, StudioKind};
use crate::checkpoint::am_error;
use crate::epoch::{MAX_DOMAIN_OP_BYTES, MAX_EPOCH_BYTES, MAX_EPOCH_OPERATIONS};
use crate::registry::hex;
use crate::{DomainOp, LogicalDocument, ReplError, MAX_CHECKPOINT_BYTES};

mod change;
pub use change::validate_index_change;

/// Visible objects in a channel index. Concurrent excess remains explicit recovery evidence.
pub const MAX_INDEX_OBJECTS: usize = 64;
// Defensive reader-work limits, not a substitute for signed-log admission or exact seed size.
// A live IndexOp needs at most four headers, one record and one marker. The extra allowance
// accommodates a future bounded checkpoint's records/conflicts, not an additional epoch of ops.
const MAX_INDEX_PRIMITIVES: u64 = (MAX_EPOCH_OPERATIONS * 6 + 8192) as u64;
const MAX_INDEX_READ_BYTES: usize = MAX_EPOCH_BYTES + MAX_CHECKPOINT_BYTES;
const MAX_RECORD_BYTES: usize = 1 + 32 + MAX_DOMAIN_OP_BYTES;
const MAX_ROOT_KEY_BYTES: usize = 2 + 32 + 1 + 64;

/// Channel ids use their 16-byte big-endian representation; no epoch is part of a logical key.
pub fn studio_index_document(
    server_id: &[u8],
    channel: ElementId,
) -> Result<LogicalDocument, ReplError> {
    LogicalDocument::new(server_id.to_vec(), DocType::StudioIndex, channel.to_vec())
}

/// Provenance asserted by a well-formed record. Trust it only after signed-delta/receipt checks.
#[derive(Clone, PartialEq, Eq)]
pub struct IndexSource {
    /// Derived from the logical channel key, full author and nonce, never a body-supplied id.
    pub op_id: [u8; 32],
    /// Full identity; a short display fingerprint is not an authority or conflict key.
    pub author: DeviceId,
    /// Original domain nonce, retained for later checkpoint/recovery representation.
    pub nonce: [u8; 16],
}

/// One live value plus its origin, including equal concurrent values from different authors.
#[derive(Clone, PartialEq, Eq)]
pub struct IndexValue<T> {
    pub value: T,
    pub source: IndexSource,
}

/// A mutable field's Automerge winner and all other live candidates, in ascending op-id order.
/// Causally superseded edits remain in the open history, not in this current-state projection.
#[derive(Clone, PartialEq, Eq)]
pub struct IndexRegister<T> {
    pub selected: IndexValue<T>,
    pub conflicts: Vec<IndexValue<T>>,
}

/// Immutable `put_object` payload. Competing creations are retained independently of renames.
#[derive(Clone, PartialEq, Eq)]
pub struct IndexCreation {
    pub kind: StudioKind,
    pub title: String,
    pub created_by: DeviceId,
    pub ts: u64,
    pub expiry: StudioExpiry,
}

/// One stable object id's materialized state. `creations[0]` is the smallest-op-id insertion;
/// later entries are insertion conflicts, not additional objects. Kind/creator/time come from
/// that winner. An explicit mutable title/expiry overrides only the corresponding initial value.
#[derive(Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub creations: Vec<IndexValue<IndexCreation>>,
    pub title: IndexRegister<String>,
    pub expiry: IndexRegister<StudioExpiry>,
}

/// Visible state and complete live conflict/deletion/overflow evidence within the reader bounds.
/// This value is neither a persistence receipt nor proof that referenced object documents exist.
#[derive(Clone, PartialEq, Eq)]
pub struct StudioIndexProjection {
    document: LogicalDocument,
    pub epoch: u64,
    /// First 64 nondeleted objects by ascending stable id; never chosen by arrival order.
    pub objects: BTreeMap<ElementId, IndexEntry>,
    /// Every other live object, with all its candidate values. Do not drop these during rotation.
    pub overflow: BTreeMap<ElementId, IndexEntry>,
    /// Content hidden by tombstones, retained here for a future typed recovery snapshot.
    pub deleted_objects: BTreeMap<ElementId, IndexEntry>,
    /// Includes a tombstone even if no insertion is present. No wall clock is consulted.
    pub tombstones: BTreeMap<ElementId, Vec<IndexSource>>,
}

// Titles, identities, ids and expiry metadata are private document content, not log context.
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
    IndexSource,
    IndexCreation,
    IndexEntry,
    StudioIndexProjection
);
impl<T> std::fmt::Debug for IndexValue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexValue").finish_non_exhaustive()
    }
}
impl<T> std::fmt::Debug for IndexRegister<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexRegister").finish_non_exhaustive()
    }
}

impl StudioIndexProjection {
    /// Read an already parsed CRDT under an independently supplied logical scope and epoch.
    /// Every concurrent root value is checked, including losing values and deleted/overflow
    /// entries. Only a genuinely empty epoch zero may lack headers. This reader never mutates
    /// the document or resolves conflicts by deleting evidence.
    pub fn read(
        document: &LogicalDocument,
        epoch: u64,
        doc: &AutoCommit,
    ) -> Result<Self, ReplError> {
        Self::read_bounded(
            document,
            epoch,
            doc,
            MAX_INDEX_PRIMITIVES,
            MAX_INDEX_READ_BYTES,
        )
    }

    // Private test seam: small deterministic fixtures exercise every budget path without
    // constructing multi-megabyte Automerge transactions. Production callers cannot raise or
    // disable either bound; the only public reader fixes both limits above.
    fn read_bounded(
        document: &LogicalDocument,
        epoch: u64,
        doc: &AutoCommit,
        primitive_limit: u64,
        byte_limit: usize,
    ) -> Result<Self, ReplError> {
        let channel = document
            .logical_key
            .as_slice()
            .try_into()
            .map_err(|_| ReplError::EpochScope)?;
        if *document != studio_index_document(&document.server_id, channel)? {
            return Err(ReplError::EpochScope);
        }
        // Bound get_all's result count before it allocates, including a malicious concentration
        // of conflicts at one key. The raw-change parser/ingest still owns pre-Automerge limits.
        let stats = doc.stats();
        if stats.num_ops > primitive_limit {
            return Err(ReplError::EpochBound);
        }
        let expected = header(document, epoch);
        let mut header_count = 0;
        let mut key_count = 0;
        let mut budget = ReadBudget {
            used: 0,
            limit: byte_limit,
        };
        let mut creations: BTreeMap<ElementId, BTreeMap<[u8; 32], IndexValue<IndexCreation>>> =
            BTreeMap::new();
        let mut titles = BTreeMap::new();
        let mut expiries = BTreeMap::new();
        let mut deletions: BTreeMap<ElementId, BTreeMap<[u8; 32], IndexSource>> = BTreeMap::new();
        // An op id excludes the body. Reject conflicting nonce reuse rather than treating two
        // unrelated payloads as one idempotent operation. Keep digests, not a second body copy.
        let mut operations = BTreeMap::new();
        for key in doc.keys(ROOT) {
            key_count += 1;
            if key.len() > MAX_ROOT_KEY_BYTES {
                return Err(ReplError::EpochBound);
            }
            budget.add(key.len())?;
            let values = doc.get_all(ROOT, &key).map_err(am_error)?;
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
            let (object, immutable_id) = if matches!(tag, "i" | "d") {
                let (object, op_id) = suffix.split_once('/').ok_or(ReplError::Malformed)?;
                (fixed_hex::<16>(object)?, Some(fixed_hex::<32>(op_id)?))
            } else if matches!(tag, "t" | "e") {
                (fixed_hex::<16>(suffix)?, None)
            } else {
                return Err(ReplError::Malformed);
            };
            // Ask Automerge for its winner. Neither op-id ordering nor get_all enumeration order
            // implements the mutable-field contract (insertion selection deliberately differs).
            let winner = doc
                .get(ROOT, &key)
                .map_err(am_error)?
                .ok_or(ReplError::Malformed)?
                .1;
            let mut title_values = BTreeMap::new();
            let mut expiry_values = BTreeMap::new();
            let mut selected = None;
            for (value, am_id) in values {
                let bytes = record_bytes(&value)?;
                let (source, operation) = decode_record(document, bytes)?;
                let digest = *blake3::hash(bytes).as_bytes();
                if operations
                    .insert(source.op_id, digest)
                    .is_some_and(|old| old != digest)
                {
                    return Err(ReplError::Malformed);
                }
                if am_id == winner {
                    selected = Some(source.op_id);
                }
                match (tag, operation) {
                    (
                        "i",
                        IndexOp::PutObject {
                            object: target,
                            kind,
                            title,
                            created_by,
                            ts,
                            expiry,
                        },
                    ) if target == object && immutable_id == Some(source.op_id) => {
                        creations.entry(object).or_default().insert(
                            source.op_id,
                            IndexValue {
                                source,
                                value: IndexCreation {
                                    kind,
                                    title,
                                    created_by,
                                    ts,
                                    expiry,
                                },
                            },
                        );
                    }
                    ("d", IndexOp::TombstoneObject { object: target })
                        if target == object && immutable_id == Some(source.op_id) =>
                    {
                        deletions
                            .entry(object)
                            .or_default()
                            .insert(source.op_id, source);
                    }
                    (
                        "t",
                        IndexOp::SetTitle {
                            object: target,
                            title,
                        },
                    ) if target == object => {
                        title_values.insert(
                            source.op_id,
                            IndexValue {
                                value: title,
                                source,
                            },
                        );
                    }
                    (
                        "e",
                        IndexOp::SetExpiry {
                            object: target,
                            expiry,
                        },
                    ) if target == object => {
                        expiry_values.insert(
                            source.op_id,
                            IndexValue {
                                value: expiry,
                                source,
                            },
                        );
                    }
                    _ => return Err(ReplError::Malformed),
                }
            }
            if tag == "t" {
                titles.insert(object, register(title_values, selected)?);
            } else if tag == "e" {
                expiries.insert(object, register(expiry_values, selected)?);
            }
        }
        let pristine = key_count == 0 && epoch == 0 && stats.num_ops == 0 && stats.num_changes == 0;
        if header_count != expected.len() && !pristine {
            return Err(ReplError::Malformed);
        }
        let mut objects = BTreeMap::new();
        let mut overflow = BTreeMap::new();
        let mut deleted_objects = BTreeMap::new();
        for (id, candidates) in creations {
            let candidates: Vec<_> = candidates.into_values().collect();
            let initial = candidates.first().ok_or(ReplError::Malformed)?;
            let title = titles
                .remove(&id)
                .unwrap_or_else(|| fallback(initial.value.title.clone(), initial.source.clone()));
            let expiry = expiries
                .remove(&id)
                .unwrap_or_else(|| fallback(initial.value.expiry, initial.source.clone()));
            let entry = IndexEntry {
                creations: candidates,
                title,
                expiry,
            };
            if deletions.contains_key(&id) {
                deleted_objects.insert(id, entry);
            } else if objects.len() < MAX_INDEX_OBJECTS {
                objects.insert(id, entry);
            } else {
                overflow.insert(id, entry);
            }
        }
        // No invisible orphan scalar payloads. A delta validator must also check existence at
        // the author's causal frontier; a concurrent insertion alone cannot authorize a rename.
        if !titles.is_empty() || !expiries.is_empty() {
            return Err(ReplError::Malformed);
        }
        let tombstones = deletions
            .into_iter()
            .map(|(id, values)| (id, values.into_values().collect()))
            .collect();
        Ok(Self {
            document: document.clone(),
            epoch,
            objects,
            overflow,
            deleted_objects,
            tombstones,
        })
    }

    /// Logical scope of this projection; it grants no group membership or receipt authority.
    pub fn document(&self) -> &LogicalDocument {
        &self.document
    }
}

fn decode_record(
    document: &LogicalDocument,
    bytes: &[u8],
) -> Result<(IndexSource, IndexOp), ReplError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(ReplError::EpochBound);
    }
    if bytes.len() < 33 || bytes[0] != 1 {
        return Err(ReplError::Malformed);
    }
    let author = DeviceId::from_bytes(bytes[1..33].try_into().map_err(|_| ReplError::Malformed)?);
    let domain = DomainOp::decode(&bytes[33..])?;
    let op = IndexOp::decode_domain(document, &domain, &author)?;
    let source = IndexSource {
        op_id: domain.id(&author),
        author,
        nonce: domain.nonce,
    };
    Ok((source, op))
}

fn register<T>(
    mut values: BTreeMap<[u8; 32], IndexValue<T>>,
    selected: Option<[u8; 32]>,
) -> Result<IndexRegister<T>, ReplError> {
    let selected = values
        .remove(&selected.ok_or(ReplError::Malformed)?)
        .ok_or(ReplError::Malformed)?;
    Ok(IndexRegister {
        selected,
        conflicts: values.into_values().collect(),
    })
}

fn fallback<T>(value: T, source: IndexSource) -> IndexRegister<T> {
    IndexRegister {
        selected: IndexValue { value, source },
        conflicts: Vec::new(),
    }
}

fn header(document: &LogicalDocument, epoch: u64) -> BTreeMap<String, ScalarValue> {
    BTreeMap::from([
        ("v".into(), ScalarValue::Uint(1)),
        ("kind".into(), ScalarValue::Str("index".into())),
        (
            "channel".into(),
            ScalarValue::Str(hex(&document.logical_key).into()),
        ),
        ("epoch".into(), ScalarValue::Uint(epoch)),
    ])
}

fn scalar_eq(value: &Value<'_>, expected: &ScalarValue) -> bool {
    matches!(value, Value::Scalar(scalar) if scalar.as_ref() == expected)
}

fn record_bytes<'a>(value: &'a Value<'_>) -> Result<&'a [u8], ReplError> {
    match value {
        Value::Scalar(scalar) => match scalar.as_ref() {
            ScalarValue::Bytes(bytes) => Ok(bytes),
            _ => Err(ReplError::Malformed),
        },
        _ => Err(ReplError::Malformed),
    }
}

struct ReadBudget {
    used: usize,
    limit: usize,
}
impl ReadBudget {
    fn add(&mut self, bytes: usize) -> Result<(), ReplError> {
        self.used = self.used.checked_add(bytes).ok_or(ReplError::EpochBound)?;
        if self.used > self.limit {
            return Err(ReplError::EpochBound);
        }
        Ok(())
    }
    fn value(&mut self, value: &Value<'_>) -> Result<(), ReplError> {
        let Value::Scalar(scalar) = value else {
            return Err(ReplError::Malformed);
        };
        // No arbitrary counters, text objects, nested maps, timestamps or floating point values.
        let len = match scalar.as_ref() {
            ScalarValue::Bytes(bytes) if bytes.len() <= MAX_RECORD_BYTES => bytes.len(),
            ScalarValue::Str(text) if text.len() <= MAX_ROOT_KEY_BYTES => text.len(),
            ScalarValue::Uint(_) => 8,
            ScalarValue::Boolean(_) => 1,
            ScalarValue::Bytes(_) | ScalarValue::Str(_) => return Err(ReplError::EpochBound),
            _ => return Err(ReplError::Malformed),
        };
        self.add(len)
    }
}

#[cfg(test)]
mod tests;
