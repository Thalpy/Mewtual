//! Epoch-zero art mutations, not the complete Studio admission/persistence adapter.

use automerge::legacy::{Key, ObjectId, OpId as AmOpId, OpType};
use automerge::{Change, ObjId};

use super::*;

/// Validate one art/frame change against its domain operation and full dependency frontier.
///
/// P1's `*_domain_preflight_gated` caller must independently authenticate the change actor as
/// the full signed current-member identity and bind server, physical document and epoch gate.
/// `before` is previously authenticated accepted history, NOT a peer-supplied snapshot. This
/// semantic callback alone grants no membership, blob availability or publication authority.
///
/// Supported operations are insert/remove/replace frame and title/fps. Rotated epochs and all
/// sound/score/export operations refuse pending their real typed checkpoint/stateful support.
/// Aggregate editing/cap policy and exact checkpoint/recovery preflight remain separate required
/// callbacks; the mere presence of an over-cap frame does not make it an unknown target here.
/// No live writer is enabled. Exact signed retries are handled by P1's retained-envelope dedup,
/// not by accepting a fabricated new change with an old marker. Timestamps are author assertions.
pub fn validate_frame_change(
    document: &LogicalDocument,
    channel: ElementId,
    epoch: u64,
    domain: &DomainOp,
    change: &Change,
    before: &AutoCommit,
) -> Result<(), ReplError> {
    if epoch != 0 {
        return Err(ReplError::EpochScope);
    }
    let actor: [u8; 32] = change
        .actor_id()
        .to_bytes()
        .try_into()
        .map_err(|_| ReplError::EpochAuthority)?;
    let author = DeviceId::from_bytes(actor);
    let operation = FlipnoteOp::decode_domain(document, domain)?;
    let (entry, mutable) = entry(&operation, &domain.id(&author))?;
    // Bound expansion before decode. Historical reads use the same bounded materializer as
    // current reads, but every key/value/winner is taken at this change's complete deps.
    if change.len() > 9 || before.stats().num_ops > MAX_PRIMITIVES {
        return Err(ReplError::EpochBound);
    }
    let causal = FlipnoteFrameProjection::read_at(document, channel, epoch, before, change.deps())?;
    let (anchor, right) = placement(&causal, &operation)?;
    let marker = format!("_p1/op/{}", hex(&domain.id(&author)));
    if !before
        .get_all_at(ROOT, &marker, change.deps())
        .map_err(am_error)?
        .is_empty()
    {
        return Err(ReplError::IntentConflict);
    }
    let mut allowed_headers = header(document, channel, epoch);
    // read_at has already verified every causal header, or a genuinely empty root. Existing
    // equal concurrent headers are immutable: never rewrite them to clean up AM conflicts.
    if !change.deps().is_empty() {
        allowed_headers.clear();
    }
    let mut required: BTreeSet<_> = allowed_headers.keys().cloned().collect();
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
            let ScalarValue::Bytes(bytes) = &value else {
                return Err(ReplError::Malformed);
            };
            let actual = decode_record(document, bytes)?;
            // ts is author-asserted (validated as JS-safe by decode_record), while author,
            // envelope and both origins are independently derived. Exact bytes also prevent
            // alternate framing or an inner body/nonce from hiding behind a matching op-id key.
            if *bytes != encode_record(domain, &author, actual.source.ts, anchor, right)? {
                return Err(ReplError::Malformed);
            }
        } else if key.as_str() == marker {
            if value != ScalarValue::Uint(1) {
                return Err(ReplError::Malformed);
            }
        } else if allowed_headers.get(key.as_str()) != Some(&value) {
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
                ObjId::Id(counter, actor, _) => Ok(AmOpId(counter, actor)),
                _ => Err(ReplError::Malformed),
            })
            .collect::<Result<_, _>>()?;
        let actual: BTreeSet<_> = op.pred.iter().cloned().collect();
        // Automerge predecessors are GLOBAL op ids. Equality (not subset) at this property
        // rejects cross-key hiding, receiver-future/obsolete ids, omitted conflicts and duplicates.
        if actual != expected || actual.len() != op.pred.len() {
            return Err(ReplError::Malformed);
        }
    }
    if seen != required {
        return Err(ReplError::Malformed);
    }
    Ok(())
}

fn entry(operation: &FlipnoteOp, op_id: &OpId) -> Result<(String, bool), ReplError> {
    Ok(match operation {
        FlipnoteOp::InsertFrame { frame, .. } => {
            (format!("i/{}/{}", hex(frame), hex(op_id)), false)
        }
        FlipnoteOp::RemoveFrame { frame } => (format!("d/{}/{}", hex(frame), hex(op_id)), false),
        FlipnoteOp::ReplaceFrame { frame, .. } => (format!("r/{}", hex(frame)), true),
        FlipnoteOp::SetHeader(FlipnoteHeader::Title(_)) => ("h/title".into(), true),
        FlipnoteOp::SetHeader(FlipnoteHeader::Fps(_)) => ("h/fps".into(), true),
        _ => return Err(ReplError::Malformed),
    })
}

/// Resolve the requested gap from actual causal projection, never from submitted metadata or
/// the capped visible timeline. Hidden insertion nodes still determine the first right sibling.
fn placement(
    projection: &FlipnoteFrameProjection,
    operation: &FlipnoteOp,
) -> Result<(Option<OpId>, Option<OpId>), ReplError> {
    let live = |frame: &ElementId| -> Result<&FrameEntry, ReplError> {
        if projection.tombstones.contains_key(frame) {
            return Err(ReplError::Malformed);
        }
        projection.frames.get(frame).ok_or(ReplError::Malformed)
    };
    match operation {
        FlipnoteOp::InsertFrame { frame, after, .. } => {
            if projection.frames.contains_key(frame) || projection.tombstones.contains_key(frame) {
                return Err(ReplError::Malformed);
            }
            let anchor = after
                .as_ref()
                .map(|after| {
                    live(after)?
                        .insertions
                        .first()
                        .map(|value| value.source.op_id)
                        .ok_or(ReplError::Malformed)
                })
                .transpose()?;
            // Build one bounded id->parent map. A per-node search through every frame would
            // make a long but valid timeline quadratic just to resolve one new insertion.
            let parents: BTreeMap<_, _> = projection
                .frames
                .values()
                .flat_map(|frame| frame.insertions.iter())
                .map(|insertion| (insertion.source.op_id, insertion.value.anchor))
                .collect();
            let right = projection
                .insertion_order
                .iter()
                .find(|id| parents.get(*id) == Some(&anchor))
                .copied();
            Ok((anchor, right))
        }
        FlipnoteOp::RemoveFrame { frame } | FlipnoteOp::ReplaceFrame { frame, .. } => {
            live(frame)?;
            Ok((None, None))
        }
        FlipnoteOp::SetHeader(FlipnoteHeader::Title(_) | FlipnoteHeader::Fps(_)) => {
            Ok((None, None))
        }
        _ => Err(ReplError::Malformed),
    }
}

// Canonical metadata framing. This produces bytes only; it is NOT a mutation/publication API.
fn encode_record(
    domain: &DomainOp,
    author: &DeviceId,
    ts: u64,
    anchor: Option<OpId>,
    right: Option<OpId>,
) -> Result<Vec<u8>, ReplError> {
    integer_bound(ts)?;
    let mut bytes = vec![1];
    bytes.extend_from_slice(author.as_bytes());
    bytes.extend_from_slice(&ts.to_be_bytes());
    for origin in [anchor, right] {
        bytes.push(u8::from(origin.is_some()));
        if let Some(origin) = origin {
            bytes.extend_from_slice(&origin);
        }
    }
    bytes.extend_from_slice(&domain.encode()?);
    Ok(bytes)
}

#[cfg(test)]
mod tests;
