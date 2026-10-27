//! Exact score expansion at the signed change's complete causal frontier. The receiver's newer
//! recipe/header state must neither authorize an invalid sender edit nor reject a valid one.

use super::state::*;
use super::*;
use crate::checkpoint::am_error;
use automerge::legacy::{Key, ObjectId, OpId as AmOpId, OpType};
use automerge::{AutoCommit, Change, ObjId, ReadDoc, ScalarValue, ROOT};
use std::collections::BTreeSet;

/// Validate semantics, canonical expansion and same-property predecessors. P1 must independently
/// authenticate the full actor identity and current membership, and bind physical scope/epoch.
/// `before` is accepted authenticated history, never an arbitrary peer-supplied snapshot.
pub fn validate_score_change(
    document: &LogicalDocument,
    channel: ElementId,
    epoch: u64,
    domain: &DomainOp,
    change: &Change,
    before: &AutoCommit,
) -> Result<(), ReplError> {
    let actor: [u8; 32] = change
        .actor_id()
        .to_bytes()
        .try_into()
        .map_err(|_| ReplError::EpochAuthority)?;
    let author = DeviceId::from_bytes(actor);
    let operation = ScoreOp::decode_domain(document, domain)?;
    if change.len() > 7 || before.stats().num_ops > MAX_PRIMITIVES {
        return Err(ReplError::EpochBound);
    }
    let causal = ScoreProjection::read_at(document, channel, epoch, before, change.deps())?;
    if epoch > 0 {
        let seed = before
            .get_all_at(ROOT, crate::studio::snapshot::SEED_KEY, change.deps())
            .map_err(am_error)?;
        if seed.len() != 1 {
            return Err(ReplError::Malformed);
        }
        let baseline =
            ScoreProjection::decode_baseline(document, channel, epoch, record_bytes(&seed[0].0)?)?;
        if baseline.source_ids().contains(&domain.id(&author)) {
            return Err(ReplError::IntentConflict);
        }
    }
    check_cell(&causal, &operation)?;
    let (entry, bytes) = record(domain, &author)?;
    let marker = format!("_p1/op/{}", hex(&domain.id(&author)));
    if !before
        .get_all_at(ROOT, &marker, change.deps())
        .map_err(am_error)?
        .is_empty()
    {
        return Err(ReplError::IntentConflict);
    }
    let mut headers = header(document, channel, epoch);
    // read_at has already established immutable headers or a genuinely pristine epoch-zero
    // root. Never rewrite equal concurrent headers merely to remove their AM alternatives.
    if !change.deps().is_empty() {
        headers.clear();
    }
    let mut required: BTreeSet<_> = headers.keys().cloned().collect();
    required.insert(entry.clone());
    required.insert(marker.clone());
    let mut seen = BTreeSet::new();
    for op in change.decode().operations {
        let (ObjectId::Root, Key::Map(key), OpType::Put(value)) = (op.obj, op.key, op.action)
        else {
            return Err(ReplError::Malformed);
        };
        if op.insert || !seen.insert(key.to_string()) {
            return Err(ReplError::Malformed);
        }
        if key.as_str() == entry {
            if !matches!(&value, ScalarValue::Bytes(actual) if *actual == bytes) {
                return Err(ReplError::Malformed);
            }
        } else if key.as_str() == marker {
            if value != ScalarValue::Uint(1) {
                return Err(ReplError::Malformed);
            }
        } else if headers.get(key.as_str()) != Some(&value) {
            return Err(ReplError::Malformed);
        }
        let prior = before
            .get_all_at(ROOT, key.as_str(), change.deps())
            .map_err(am_error)?;
        if !prior.is_empty() && key.as_str() != entry {
            return Err(ReplError::Malformed);
        }
        let expected: BTreeSet<_> = prior
            .into_iter()
            .map(|(_, id)| match id {
                ObjId::Id(counter, actor, _) => Ok(AmOpId(counter, actor)),
                _ => Err(ReplError::Malformed),
            })
            .collect::<Result<_, _>>()?;
        let actual: BTreeSet<_> = op.pred.iter().cloned().collect();
        // AM predecessors are global ids, not a per-key list. Exact equality prevents hiding
        // another key, omitting an observed removal, or importing a receiver-future operation.
        if actual != expected || actual.len() != op.pred.len() {
            return Err(ReplError::Malformed);
        }
    }
    if seen != required {
        return Err(ReplError::Malformed);
    }
    Ok(())
}

fn check_cell(projection: &ScoreProjection, operation: &ScoreOp) -> Result<(), ReplError> {
    if let ScoreOp::SetCell { step, value, .. } = operation {
        let end = *step
            + match value {
                ScoreCell::Note { len, .. } => len.unwrap_or(1),
                ScoreCell::Drum { .. } => 1,
            };
        if end > projection.effective_steps() {
            return Err(ReplError::Malformed);
        }
        if let ScoreCell::Note { patch, .. } = value {
            if projection
                .patches
                .get(patch)
                .and_then(ScoreSlot::value)
                .is_none()
            {
                return Err(ReplError::Malformed);
            }
        }
    }
    // Clear/Remove deliberately accept absent or overflow slots: cleanup must remain possible.
    // Concurrent overlap/cap/header conflicts are projection readiness, not receiver admission.
    Ok(())
}

pub(in crate::studio) fn prepare(
    projection: &ScoreProjection,
    domain: &DomainOp,
    author: &DeviceId,
) -> Result<crate::studio::admission::PreparedEdit, ReplError> {
    let operation = ScoreOp::decode_domain(projection.document(), domain)?;
    check_cell(projection, &operation)?;
    Ok(crate::studio::admission::PreparedEdit {
        headers: header(projection.document(), projection.channel, projection.epoch),
        entry: record(domain, author)?,
    })
}

#[cfg(test)]
mod tests;
