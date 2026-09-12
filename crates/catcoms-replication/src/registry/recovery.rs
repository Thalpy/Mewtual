//! Typed local recovery evidence for registry keys (not Studio's random element ids).
//! This payload is authenticated by the vault, never accepted as a signed network operation.

use super::*;
use crate::epoch::{MAX_EPOCH_OPERATIONS, MAX_RECOVERY_SNAPSHOT_BYTES};
use crate::registry_epoch::RegistrySettlementPlan;
use crate::{LocalIntent, RecoveryReason, RecoverySnapshot};
use catcoms_wire::{Decoder, Encoder};

// A seed contributes at most 2048 keys, and each accepted user op can introduce at most one.
const MAX_KEYS: usize = MAX_REGISTRY_POINTERS + MAX_EPOCH_OPERATIONS;

/// Validated, read-only typed projection and excluded author-attributed operations. A decoded
/// record is historical local evidence, NOT current receipt authority or permission to replay
/// as another device. Registry pointer numbers remain unverified discovery hints on Restore.
pub struct RegistryRecovery {
    projection: RegistryProjection,
    receipt_hash: [u8; 32],
    excluded: BTreeMap<[u8; 32], LocalIntent>,
}

impl std::fmt::Debug for RegistryRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryRecovery")
            .field("epoch", &self.projection.epoch)
            .field("pointers", &self.projection.pointers.len())
            .field("overflow", &self.projection.overflow.len())
            .field("excluded_operations", &self.excluded.len())
            .finish_non_exhaustive()
    }
}

impl RegistryRecovery {
    /// Complete source version, with pointer-key tombstones and overflow retained separately.
    /// Registry collections have no predecessor ordering or random element-id metadata.
    pub fn projection(&self) -> &RegistryProjection {
        &self.projection
    }

    /// For Excluded, the held receipt selecting the closure. For Rewound, the SOURCE opening
    /// receipt (all-zero only for epoch zero), not the destination. Never signature authority.
    pub fn receipt_hash(&self) -> [u8; 32] {
        self.receipt_hash
    }

    /// Excluded accepted operations (ALL source operations for Rewound), in derived-id order.
    /// Only their original authors may replay
    /// their own durable intents. Other members can later Restore using their OWN new operations.
    pub fn excluded_operations(&self) -> &BTreeMap<[u8; 32], LocalIntent> {
        &self.excluded
    }

    pub(crate) fn snapshot_for_plan(
        plan: &RegistrySettlementPlan,
    ) -> Result<Option<RecoverySnapshot>, ReplError> {
        let projection = plan.source_projection();
        if plan.excluded_operations().is_empty()
            && projection.overflow.is_empty()
            && projection.tombstones.is_empty()
        {
            return Ok(None);
        }
        let applied_ops = plan
            .included_operation_ids()
            .iter()
            .copied()
            .chain(plan.excluded_operations().keys().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut snapshot = RecoverySnapshot {
            doc_type: DocType::DocRegistry,
            logical_key: plan.receipt().document.logical_key.clone(),
            epoch: projection.epoch,
            base_close_record_hash: plan.source_base_close(),
            reason: RecoveryReason::Excluded,
            projection: Vec::new(),
            // Pointer keys are stored losslessly in the typed payload; never truncate/hash them
            // into Studio random element ids or invent authors for inherited seed-only values.
            tombstones: Vec::new(),
            elements: Vec::new(),
            conflicts: Vec::new(),
            applied_ops,
        };
        let overhead = snapshot.encode()?.len();
        let typed = Self {
            projection: projection.clone(),
            receipt_hash: plan.receipt().hash(),
            excluded: plan.excluded_operations().clone(),
        };
        // Preflight the exact OUTER size as well as each field before allocating the payload.
        snapshot.projection = typed.encode(MAX_RECOVERY_SNAPSHOT_BYTES - overhead)?;
        snapshot.encode()?;
        Ok(Some(snapshot))
    }

    /// Whole-version evidence when the selected checkpoint's closure is not locally held.
    /// Destination-independent bytes are essential: fresh owner selections must not consume
    /// recovery slots or restart the seven-day eviction warning for the same frozen source.
    pub(crate) fn snapshot_for_rewind(
        projection: RegistryProjection,
        opening: Option<&crate::Receipt>,
        operations: BTreeMap<[u8; 32], LocalIntent>,
    ) -> Result<Option<RecoverySnapshot>, ReplError> {
        if operations.is_empty()
            && projection.pointers.is_empty()
            && projection.overflow.is_empty()
            && projection.tombstones.is_empty()
        {
            return Ok(None);
        }
        let mut snapshot = RecoverySnapshot {
            doc_type: DocType::DocRegistry,
            logical_key: projection.document.logical_key.clone(),
            epoch: projection.epoch,
            base_close_record_hash: opening.map(|receipt| receipt.close_record_hash),
            reason: RecoveryReason::Rewound,
            projection: Vec::new(),
            tombstones: Vec::new(),
            elements: Vec::new(),
            conflicts: Vec::new(),
            applied_ops: operations.keys().copied().collect(),
        };
        let overhead = snapshot.encode()?.len();
        let typed = Self {
            projection,
            receipt_hash: opening.map_or([0; 32], crate::Receipt::hash),
            excluded: operations,
        };
        snapshot.projection = typed.encode(MAX_RECOVERY_SNAPSHOT_BYTES - overhead)?;
        snapshot.encode()?;
        Ok(Some(snapshot))
    }

    /// Validate a generic snapshot's registry specialization BEFORE exposing it to Restore or
    /// Export. Expected scope comes from the authenticated vault record, not this payload.
    /// Bounds, canonical ordering, key disjointness, full author ids and domain semantics are
    /// checked; this does not re-verify discarded signatures or prove the snapshot is current.
    pub fn from_snapshot(
        snapshot: &RecoverySnapshot,
        expected: &LogicalDocument,
        bucket: u8,
    ) -> Result<Self, ReplError> {
        if expected.server_id.is_empty()
            || expected.server_id.len() > 256
            || *expected != registry_document(&expected.server_id, bucket)?
            || snapshot.doc_type != DocType::DocRegistry
            || snapshot.logical_key != expected.logical_key
            || snapshot.epoch > MAX_REGISTRY_EPOCH
            || (snapshot.epoch == MAX_REGISTRY_EPOCH && snapshot.reason != RecoveryReason::Rewound)
            || (snapshot.epoch == 0) != snapshot.base_close_record_hash.is_none()
            || !matches!(
                snapshot.reason,
                RecoveryReason::Excluded | RecoveryReason::Rewound
            )
            || !snapshot.elements.is_empty()
            || !snapshot.tombstones.is_empty()
            || !snapshot.conflicts.is_empty()
        {
            return Err(ReplError::EpochScope);
        }
        snapshot.encode()?; // generic aggregate/count/ordering preflight before typed allocation
        let mut d = Decoder::new(&snapshot.projection);
        if d.get_u8().map_err(malformed)? != 1
            || d.get_bytes().map_err(malformed)? != expected.server_id
            || d.get_u8().map_err(malformed)? != bucket
        {
            return Err(ReplError::EpochScope);
        }
        let receipt_hash = d
            .get_bytes()
            .map_err(malformed)?
            .try_into()
            .map_err(malformed)?;
        let mut keys_left = MAX_KEYS;
        let pointers = read_pointers(&mut d, bucket, MAX_REGISTRY_POINTERS, &mut keys_left)?;
        let overflow = read_pointers(&mut d, bucket, MAX_KEYS, &mut keys_left)?;
        let count = read_count(&mut d, keys_left)?;
        let mut tombstones = BTreeSet::new();
        for _ in 0..count {
            let key = read_key(&mut d, bucket)?;
            if tombstones.last().is_some_and(|previous| previous >= &key) {
                return Err(ReplError::Malformed);
            }
            tombstones.insert(key);
        }
        if overflow.keys().any(|key| pointers.contains_key(key))
            || tombstones
                .iter()
                .any(|key| pointers.contains_key(key) || overflow.contains_key(key))
            || (!overflow.is_empty() && pointers.len() != MAX_REGISTRY_POINTERS)
        {
            return Err(ReplError::Malformed);
        }
        let count = read_count(&mut d, MAX_EPOCH_OPERATIONS)?;
        let mut excluded = BTreeMap::new();
        for _ in 0..count {
            let author = DeviceId::from_bytes(
                d.get_bytes()
                    .map_err(malformed)?
                    .try_into()
                    .map_err(malformed)?,
            );
            let operation = DomainOp::decode(d.get_bytes().map_err(malformed)?)?;
            validate_domain(expected, bucket, &operation)?;
            let id = operation.id(&author);
            if excluded
                .last_key_value()
                .is_some_and(|(previous, _)| previous >= &id)
                || snapshot.applied_ops.binary_search(&id).is_err()
            {
                return Err(ReplError::Malformed);
            }
            excluded.insert(id, LocalIntent { author, operation });
        }
        d.finish().map_err(malformed)?;
        if (excluded.is_empty()
            && overflow.is_empty()
            && tombstones.is_empty()
            && (snapshot.reason != RecoveryReason::Rewound || pointers.is_empty()))
            || (snapshot.reason == RecoveryReason::Rewound
                && (excluded.keys().copied().collect::<Vec<_>>() != snapshot.applied_ops
                    || ((snapshot.epoch == 0) != (receipt_hash == [0; 32]))))
        {
            return Err(ReplError::Malformed);
        }
        let typed = Self {
            projection: RegistryProjection {
                document: expected.clone(),
                bucket,
                epoch: snapshot.epoch,
                pointers,
                overflow,
                tombstones,
            },
            receipt_hash,
            excluded,
        };
        if typed.encode(MAX_RECOVERY_SNAPSHOT_BYTES)? != snapshot.projection {
            return Err(ReplError::Malformed);
        }
        Ok(typed)
    }

    fn encode(&self, limit: usize) -> Result<Vec<u8>, ReplError> {
        let projection = &self.projection;
        if projection.pointers.len() > MAX_REGISTRY_POINTERS
            || projection.pointers.len() + projection.overflow.len() + projection.tombstones.len()
                > MAX_KEYS
            || self.excluded.len() > MAX_EPOCH_OPERATIONS
        {
            return Err(ReplError::EpochBound);
        }
        let mut size = 1 + 4 + projection.document.server_id.len() + 1 + 36 + 4 * 4;
        for (key, _) in projection.pointers.iter().chain(&projection.overflow) {
            size += 2 + 4 + key.key.len() + 8;
        }
        for key in &projection.tombstones {
            size += 2 + 4 + key.key.len();
        }
        for intent in self.excluded.values() {
            // Bodies are registry-bounded before DomainOp::encode, which otherwise accepts a
            // larger generic envelope. Only one small temporary envelope is allocated at a time.
            validate_domain(&projection.document, projection.bucket, &intent.operation)?;
            size += 36 + 4 + intent.operation.encode()?.len();
        }
        if size > limit {
            return Err(ReplError::EpochBound);
        }
        let mut e = Encoder::with_capacity(size);
        e.put_u8(1);
        e.put_bytes(&projection.document.server_id)
            .map_err(malformed)?;
        e.put_u8(projection.bucket);
        e.put_bytes(&self.receipt_hash).map_err(malformed)?;
        for pointers in [&projection.pointers, &projection.overflow] {
            e.put_u32(pointers.len() as u32);
            for (key, epoch) in pointers {
                put_key(&mut e, key)?;
                e.put_u64(*epoch);
            }
        }
        e.put_u32(projection.tombstones.len() as u32);
        for key in &projection.tombstones {
            put_key(&mut e, key)?;
        }
        e.put_u32(self.excluded.len() as u32);
        for intent in self.excluded.values() {
            e.put_bytes(intent.author.as_bytes()).map_err(malformed)?;
            e.put_bytes(&intent.operation.encode()?)
                .map_err(malformed)?;
        }
        debug_assert_eq!(e.len(), size);
        Ok(e.finish())
    }
}

fn put_key(e: &mut Encoder, key: &PointerKey) -> Result<(), ReplError> {
    e.put_u16(key.doc_type.tag());
    e.put_bytes(&key.key).map_err(malformed)?;
    Ok(())
}

fn read_key(d: &mut Decoder<'_>, bucket: u8) -> Result<PointerKey, ReplError> {
    let tag = DocType::from_tag(d.get_u16().map_err(malformed)?).ok_or(ReplError::Malformed)?;
    let bytes = d.get_bytes().map_err(malformed)?;
    if bytes.len() > 192 {
        return Err(ReplError::EpochBound);
    }
    let key = PointerKey::new(tag, bytes.to_vec())?;
    if key.bucket() != bucket {
        return Err(ReplError::EpochScope);
    }
    Ok(key)
}

fn read_count(d: &mut Decoder<'_>, max: usize) -> Result<usize, ReplError> {
    let count = d.get_u32().map_err(malformed)? as usize;
    if count > max {
        return Err(ReplError::EpochBound);
    }
    Ok(count)
}

fn read_pointers(
    d: &mut Decoder<'_>,
    bucket: u8,
    max: usize,
    keys_left: &mut usize,
) -> Result<BTreeMap<PointerKey, u64>, ReplError> {
    let count = read_count(d, max.min(*keys_left))?;
    *keys_left -= count;
    let mut pointers = BTreeMap::new();
    for _ in 0..count {
        let key = read_key(d, bucket)?;
        let epoch = d.get_u64().map_err(malformed)?;
        if pointers
            .last_key_value()
            .is_some_and(|(previous, _)| previous >= &key)
        {
            return Err(ReplError::Malformed);
        }
        pointers.insert(key, epoch);
    }
    Ok(pointers)
}

fn malformed(_: impl std::fmt::Display) -> ReplError {
    ReplError::Malformed
}

#[cfg(test)]
mod tests;
