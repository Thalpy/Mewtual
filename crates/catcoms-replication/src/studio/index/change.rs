//! Causal StudioIndex delta validation, deliberately separate from live edit/ingest adapters.
//!
//! Reading the merged projection cannot prove who wrote a record or what a sender observed.
//! Here each allowed mutation is derived from the domain operation and the change's dependency
//! frontier. In particular, receiver-only creations and property predecessors confer no rights.

use std::collections::BTreeSet;

use automerge::legacy::{Key, ObjectId, OpId, OpType};
use automerge::{Change, ObjId};

use super::*;

/// Check that one epoch-zero change implements exactly its canonical StudioIndex operation.
///
/// Intended as the semantic callback of P1's `*_domain_preflight_gated` methods: those methods
/// independently bind the change actor to the full signed author and current member key, check
/// server/physical-document scope, reject reused operation ids, and atomically admit the result.
/// Calling this function alone verifies NONE of those external authentication/lifecycle facts.
/// `before` must be trusted accepted history, not an arbitrary peer-supplied Automerge snapshot.
///
/// The caller must ALSO preflight the exact prospective checkpoint/recovery encoding. This is
/// not a production Studio writer; no such adapter is enabled while that serializer is missing.
/// Checkpoint epochs deliberately refuse until their retired-state representation is specified.
/// Exact sealed retries belong to P1/log dedup, not marker-only changes fabricated by this API.
pub fn validate_index_change(
    document: &LogicalDocument,
    epoch: u64,
    domain: &DomainOp,
    change: &Change,
    before: &AutoCommit,
) -> Result<(), ReplError> {
    let channel = document
        .logical_key
        .as_slice()
        .try_into()
        .map_err(|_| ReplError::EpochScope)?;
    if epoch != 0 || *document != studio_index_document(&document.server_id, channel)? {
        return Err(ReplError::EpochScope);
    }
    let actor: [u8; 32] = change
        .actor_id()
        .to_bytes()
        .try_into()
        .map_err(|_| ReplError::EpochAuthority)?;
    let author = DeviceId::from_bytes(actor);
    let operation = IndexOp::decode_domain(document, domain, &author)?;
    // Check before decoding expanded operations or allocating historical conflict vectors.
    // Trusted accepted history has tighter byte accounting at the outer P1 boundary as well.
    if change.len() > 6
        || before.stats().num_ops > MAX_INDEX_PRIMITIVES
        || change.deps().len() > MAX_EPOCH_OPERATIONS
    {
        return Err(ReplError::EpochBound);
    }
    if change
        .deps()
        .iter()
        .any(|hash| before.get_change_by_hash(hash).is_none())
    {
        return Err(ReplError::EpochScope);
    }
    let fields = header(document, epoch);
    let mut required = BTreeSet::new();
    for (key, expected) in &fields {
        let values = before
            .get_all_at(ROOT, key, change.deps())
            .map_err(am_error)?;
        if values.is_empty() {
            required.insert(key.clone());
        } else if values.iter().any(|(value, _)| !scalar_eq(value, expected)) {
            return Err(ReplError::Malformed);
        }
    }
    // A valid causal root has all headers or is pristine. Partial header initialization is not
    // a supported migration path, and existing equal concurrent headers must not be rewritten.
    if !required.is_empty() && required.len() != fields.len() {
        return Err(ReplError::Malformed);
    }
    let object = match &operation {
        IndexOp::PutObject { object, .. }
        | IndexOp::TombstoneObject { object }
        | IndexOp::SetTitle { object, .. }
        | IndexOp::SetExpiry { object, .. } => object,
    };
    let insertion_prefix = format!("i/{}/", hex(object));
    let deletion_prefix = format!("d/{}/", hex(object));
    let mut created = false;
    let mut deleted = false;
    let mut has_keys = false;
    // Scan at the author's full dependency frontier, never the receiver's current keys. This
    // preserves legitimate concurrent insertions/deletions without admitting a sequential reuse.
    // The primitive bound above also bounds this key walk; no recursively rebuilt fork is needed.
    for key in before.keys_at(ROOT, change.deps()) {
        has_keys = true;
        created |= key.starts_with(&insertion_prefix);
        deleted |= key.starts_with(&deletion_prefix);
    }
    if !required.is_empty() && (has_keys || !change.deps().is_empty()) {
        return Err(ReplError::Malformed);
    }
    match operation {
        IndexOp::PutObject { .. } if created || deleted => return Err(ReplError::Malformed),
        IndexOp::PutObject { .. } => {}
        _ if !created || deleted => return Err(ReplError::Malformed),
        _ => {}
    }
    let id = domain.id(&author);
    let marker = format!("_p1/op/{}", hex(&id));
    if !before
        .get_all_at(ROOT, &marker, change.deps())
        .map_err(am_error)?
        .is_empty()
    {
        return Err(ReplError::IntentConflict);
    }
    let (entry, bytes) = record(domain, &author, &operation)?;
    let mutable = matches!(
        operation,
        IndexOp::SetTitle { .. } | IndexOp::SetExpiry { .. }
    );
    let mut allowed = fields;
    // Only absent headers may occur in the delta. The others remain immutable, including their
    // harmless equal concurrent values. A Put used to clean them up is still an illegal write.
    allowed.retain(|key, _| required.contains(key));
    allowed.insert(entry.clone(), ScalarValue::Bytes(bytes));
    allowed.insert(marker.clone(), ScalarValue::Uint(1));
    required.insert(entry.clone());
    required.insert(marker);
    let mut seen = BTreeSet::new();
    for op in change.decode().operations {
        let (ObjectId::Root, Key::Map(key), OpType::Put(value)) = (op.obj, op.key, op.action)
        else {
            return Err(ReplError::Malformed);
        };
        if op.insert || !seen.insert(key.to_string()) || allowed.get(key.as_str()) != Some(&value) {
            return Err(ReplError::Malformed);
        }
        let prior = before
            .get_all_at(ROOT, key.as_str(), change.deps())
            .map_err(am_error)?;
        if !(prior.is_empty() || mutable && key.as_str() == entry) {
            return Err(ReplError::Malformed);
        }
        let expected: BTreeSet<_> = prior
            .into_iter()
            .map(|(_, id)| match id {
                ObjId::Id(counter, actor, _) => Ok(OpId(counter, actor)),
                _ => Err(ReplError::Malformed),
            })
            .collect::<Result<_, _>>()?;
        let actual: BTreeSet<_> = op.pred.iter().cloned().collect();
        // Global Automerge op ids can target OTHER properties. Exact same-property equality
        // rejects that attack, hidden historical predecessors, omissions, and duplicate preds.
        // A normal register set supersedes all values the author saw, not an arbitrary subset.
        if actual != expected || actual.len() != op.pred.len() {
            return Err(ReplError::Malformed);
        }
    }
    if seen != required {
        return Err(ReplError::Malformed);
    }
    Ok(())
}

// Internal canonical record constructor shared with this module's TEST-ONLY writer. It does
// not write an Automerge document or mint publication authority.
fn record(
    domain: &DomainOp,
    author: &DeviceId,
    operation: &IndexOp,
) -> Result<(String, Vec<u8>), ReplError> {
    let id = hex(&domain.id(author));
    let key = match operation {
        IndexOp::PutObject { object, .. } => format!("i/{}/{id}", hex(object)),
        IndexOp::TombstoneObject { object } => format!("d/{}/{id}", hex(object)),
        IndexOp::SetTitle { object, .. } => format!("t/{}", hex(object)),
        IndexOp::SetExpiry { object, .. } => format!("e/{}", hex(object)),
    };
    let mut bytes = vec![1];
    bytes.extend_from_slice(author.as_bytes());
    bytes.extend_from_slice(&domain.encode()?);
    Ok((key, bytes))
}

#[cfg(test)]
mod tests;
