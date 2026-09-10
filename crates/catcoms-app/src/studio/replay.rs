//! Conservative own-intent replay. Operation ids are idempotency keys, NEVER causal order.
//! Recovery holds final register provenance, not every original dependency frontier. Reapply
//! only effects that can be proved safe from that evidence; preserve everything else for a
//! deliberate manual choice rather than overwrite newer shared work.
use super::*;
use catcoms_replication::{studio::StudioRecovery, LocalIntent};
use std::collections::BTreeMap;
use std::collections::{BTreeSet, VecDeque};

/// One linear dependency graph, not repeated hash-order rescans. Missing prerequisites and
/// cycles become explicit manual recovery; no recursion can overflow on a hostile long chain.
pub(crate) fn ordered(
    choices: &BTreeMap<[u8; 32], ReplayChoice>,
) -> (VecDeque<[u8; 32]>, BTreeSet<[u8; 32]>) {
    let mut edges = BTreeMap::<[u8; 32], Vec<[u8; 32]>>::new();
    let mut ready = BTreeSet::new();
    let mut remaining = BTreeSet::new();
    let mut manual = BTreeSet::new();
    for (&id, &choice) in choices {
        match choice {
            ReplayChoice::Ready => {
                ready.insert(id);
                remaining.insert(id);
            }
            ReplayChoice::After(parent) => {
                remaining.insert(id);
                edges.entry(parent).or_default().push(id);
            }
            ReplayChoice::Manual => {
                manual.insert(id);
            }
            _ => {}
        }
    }
    let mut order = VecDeque::new();
    while let Some(id) = ready.pop_first() {
        if !remaining.remove(&id) {
            continue;
        }
        order.push_back(id);
        if let Some(children) = edges.remove(&id) {
            ready.extend(children);
        }
    }
    manual.extend(remaining);
    (order, manual)
}

/// Short-lived evidence inside one worker turn. Never cached across a custody boundary.
pub(crate) struct ReplayEvidence {
    pub current: StudioProjection,
    pub signed: BTreeMap<[u8; 32], LocalIntent>,
    pub history: Vec<StudioRecovery>,
    pub own: BTreeMap<[u8; 32], LocalIntent>,
    pub history_ids: Vec<[u8; 32]>,
}
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    pub(crate) fn studio_replay_evidence(
        &mut self,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        epoch: u128,
    ) -> Result<Option<ReplayEvidence>, AppError> {
        self.sync.with_registry_context(|group, device, _, _| {
            let logical = target.document(&group.group_id()).map_err(invalid)?;
            let Some((current, signed)) = store
                .with_studio_source(id, group, target, device, |state| {
                    if state.doc_id() != epoch || state.phase() != EpochPhase::Open {
                        // A real seal/fault is a per-document hold, not a vault-wide receive
                        // failure. Watch rebinding after installation starts a fresh pass.
                        return Ok(None);
                    }
                    Ok(Some((state.projection()?, state.current_operations()?)))
                })?
                .flatten()
            else {
                return Ok(None);
            };
            let recovery = store.load_epoch_recovery(id, &logical)?;
            let mut history_ids = recovery
                .retained()
                .chain(recovery.staged())
                .map(|s| s.id().map_err(invalid))
                .collect::<Result<Vec<_>, _>>()?;
            history_ids.sort_unstable();
            let history = recovery
                .retained()
                .chain(recovery.staged())
                .map(|s| {
                    StudioRecovery::from_snapshot(s, &logical, target.channel()).map_err(invalid)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let own = store
                .load_epoch_intents(id, &logical)?
                .pending()
                .filter(|(_, i)| i.author == device.device_id())
                .map(|(id, i)| (*id, i.clone()))
                .collect();
            Ok(Some(ReplayEvidence {
                current,
                signed,
                history,
                own,
                history_ids,
            }))
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplayChoice {
    /// Current signed-log exact retry: keep pending for receipt; never reapply its effect.
    Current,
    Ready,
    /// Stable creation dependency. The runtime topologically traverses these edges.
    After([u8; 32]),
    /// Exact historical envelope exists, but applying now would be ambiguous/destructive.
    Manual,
    /// A failed Save can leave a ledger entry without a historical accepted operation.
    NoEvidence,
}

/// Recovery order is not a causal clock. Every version that contains this complete envelope
/// must agree it remains an unconflicted selected effect before automatic authoring is safe.
fn selected(projection: &StudioProjection, intent: &LocalIntent) -> Result<bool, AppError> {
    let id = intent.operation.id(&intent.author);
    macro_rules! reg {
        ($r:expr) => {
            $r.selected.source.op_id == id && $r.conflicts.is_empty()
        };
    }
    Ok(match projection {
        StudioProjection::Flipnote(p) => {
            match FlipnoteOp::decode(&intent.operation.body).map_err(invalid)? {
                FlipnoteOp::SetHeader(FlipnoteHeader::Title(_)) => {
                    p.title.as_ref().is_some_and(|r| reg!(r))
                }
                FlipnoteOp::SetHeader(FlipnoteHeader::Fps(_)) => {
                    p.fps.as_ref().is_some_and(|r| reg!(r))
                }
                FlipnoteOp::InsertFrame { frame, .. } => p
                    .frames
                    .get(&frame)
                    .is_some_and(|e| e.insertions.len() == 1 && e.insertions[0].source.op_id == id),
                FlipnoteOp::ReplaceFrame { frame, .. } => {
                    p.frames.get(&frame).is_some_and(|e| reg!(e.pixels))
                }
                _ => false,
            }
        }
        StudioProjection::Index(p) => {
            match IndexOp::decode(&intent.operation.body).map_err(invalid)? {
                IndexOp::PutObject { object, .. } => p
                    .objects
                    .get(&object)
                    .or_else(|| p.overflow.get(&object))
                    .is_some_and(|e| e.creations.len() == 1 && e.creations[0].source.op_id == id),
                IndexOp::SetTitle { object, .. } => p
                    .objects
                    .get(&object)
                    .or_else(|| p.overflow.get(&object))
                    .is_some_and(|e| reg!(e.title)),
                IndexOp::SetExpiry { object, .. } => p
                    .objects
                    .get(&object)
                    .or_else(|| p.overflow.get(&object))
                    .is_some_and(|e| reg!(e.expiry)),
                _ => false,
            }
        }
    })
}

/// Distinct branches can each select a different own value without carrying the other's
/// envelope. Neither is an automatic winner; hold all such mutable-field candidates.
pub(crate) fn deconflict(
    choices: &mut BTreeMap<[u8; 32], ReplayChoice>,
    own: &BTreeMap<[u8; 32], LocalIntent>,
) -> Result<(), AppError> {
    let mut fields = BTreeMap::<(u8, [u8; 16]), Vec<[u8; 32]>>::new();
    for (id, choice) in choices.iter() {
        if !matches!(choice, ReplayChoice::Ready | ReplayChoice::After(_)) {
            continue;
        }
        let op = &own[id].operation;
        let field = match op.doc_type {
            catcoms_wire::DocType::StudioIndex => {
                match IndexOp::decode(&op.body).map_err(invalid)? {
                    IndexOp::SetTitle { object, .. } => Some((0, object)),
                    IndexOp::SetExpiry { object, .. } => Some((1, object)),
                    _ => None,
                }
            }
            catcoms_wire::DocType::StudioObject => {
                match FlipnoteOp::decode(&op.body).map_err(invalid)? {
                    FlipnoteOp::SetHeader(FlipnoteHeader::Title(_)) => Some((2, [0; 16])),
                    FlipnoteOp::SetHeader(FlipnoteHeader::Fps(_)) => Some((3, [0; 16])),
                    FlipnoteOp::ReplaceFrame { frame, .. } => Some((4, frame)),
                    _ => None,
                }
            }
            _ => return Err(invalid("unsupported replay family")),
        };
        if let Some(field) = field {
            fields.entry(field).or_default().push(*id);
        }
    }
    for ids in fields.values().filter(|ids| ids.len() > 1) {
        for id in ids {
            choices.insert(*id, ReplayChoice::Manual);
        }
    }
    Ok(())
}

pub(crate) fn choose(
    current: &StudioProjection,
    current_ops: &BTreeMap<[u8; 32], LocalIntent>,
    history: &[StudioRecovery],
    intent: &LocalIntent,
) -> Result<ReplayChoice, AppError> {
    let id = intent.operation.id(&intent.author);
    if let Some(held) = current_ops.get(&id) {
        return if held == intent {
            Ok(ReplayChoice::Current)
        } else {
            Err(invalid("replay envelope conflicts with current signed log"))
        };
    }
    let Some(old) = history
        .iter()
        .find(|r| r.operations().get(&id) == Some(intent))
    else {
        return Ok(ReplayChoice::NoEvidence);
    };
    if old.projection().document() != current.document()
        || old.projection().channel() != current.channel()
    {
        return Err(invalid("replay recovery scope mismatch"));
    }
    for version in history
        .iter()
        .filter(|r| r.operations().get(&id) == Some(intent))
    {
        if !selected(version.projection(), intent)? {
            return Ok(ReplayChoice::Manual);
        }
    }
    let ready = ReplayChoice::Ready;
    let manual = ReplayChoice::Manual;
    match (current, old.projection()) {
        (StudioProjection::Flipnote(now), StudioProjection::Flipnote(then)) => {
            let deleted = |frame: &[u8; 16]| {
                now.tombstones.contains_key(frame) || history.iter().any(|h|
                matches!(h.projection(),StudioProjection::Flipnote(p) if p.tombstones.contains_key(frame)))
            };
            match FlipnoteOp::decode(&intent.operation.body).map_err(invalid)? {
                FlipnoteOp::SetHeader(FlipnoteHeader::Title(_)) => {
                    Ok(
                        if then.title.as_ref().is_some_and(|v| {
                            v.selected.source.op_id == id && v.conflicts.is_empty()
                        }) && now.title.is_none()
                        {
                            ready
                        } else {
                            manual
                        },
                    )
                }
                FlipnoteOp::SetHeader(FlipnoteHeader::Fps(_)) => {
                    Ok(
                        if then.fps.as_ref().is_some_and(|v| {
                            v.selected.source.op_id == id && v.conflicts.is_empty()
                        }) && now.fps.is_none()
                        {
                            ready
                        } else {
                            manual
                        },
                    )
                }
                FlipnoteOp::InsertFrame {
                    frame,
                    after,
                    bytes,
                    ..
                } => {
                    if deleted(&frame) || now.frames.contains_key(&frame) {
                        return Ok(manual);
                    }
                    if now.timeline.len() >= types::FLIPNOTE_MAX_FRAMES
                        || now.declared_frame_bytes.saturating_add(bytes)
                            > types::FLIPNOTE_FRAME_BYTES
                    {
                        return Ok(manual);
                    }
                    let Some(entry) = then.frames.get(&frame) else {
                        return Ok(manual);
                    };
                    if entry.insertions.len() != 1 || entry.insertions[0].source.op_id != id {
                        return Ok(manual);
                    }
                    if let Some(previous) = after {
                        if deleted(&previous) {
                            return Ok(manual);
                        }
                        if !now.frames.contains_key(&previous) {
                            return Ok(then
                                .frames
                                .get(&previous)
                                .filter(|e| e.insertions.len() == 1)
                                .map(|e| ReplayChoice::After(e.insertions[0].source.op_id))
                                .unwrap_or(manual));
                        }
                    }
                    Ok(ready)
                }
                FlipnoteOp::ReplaceFrame { frame, bytes, .. } => {
                    if deleted(&frame) {
                        return Ok(manual);
                    }
                    let Some(old) = then.frames.get(&frame) else {
                        return Ok(manual);
                    };
                    if old.insertions.len() != 1
                        || old.pixels.selected.source.op_id != id
                        || !old.pixels.conflicts.is_empty()
                    {
                        return Ok(manual);
                    }
                    let birth = old.insertions[0].source.op_id;
                    let Some(now) = now.frames.get(&frame) else {
                        return Ok(ReplayChoice::After(birth));
                    };
                    let StudioProjection::Flipnote(projection) = current else {
                        unreachable!()
                    };
                    if projection.timeline.len() > types::FLIPNOTE_MAX_FRAMES
                        || projection.over_cap.contains_key(&frame)
                        || projection
                            .declared_frame_bytes
                            .saturating_sub(now.pixels.selected.value.bytes)
                            .saturating_add(bytes)
                            > types::FLIPNOTE_FRAME_BYTES
                    {
                        return Ok(manual);
                    }
                    Ok(
                        if now.insertions.len() == 1
                            && now.insertions[0].source.op_id == birth
                            && now.pixels.selected.source.op_id == birth
                            && now.pixels.conflicts.is_empty()
                        {
                            ready
                        } else {
                            manual
                        },
                    )
                }
                // Deletion and unsupported future families must never be silently replayed.
                _ => Ok(manual),
            }
        }
        (StudioProjection::Index(now), StudioProjection::Index(then)) => {
            let op = IndexOp::decode(&intent.operation.body).map_err(invalid)?;
            let object = match &op {
                IndexOp::PutObject { object, .. }
                | IndexOp::SetTitle { object, .. }
                | IndexOp::SetExpiry { object, .. }
                | IndexOp::TombstoneObject { object } => object,
            };
            if now.tombstones.contains_key(object) || history.iter().any(|h|
                matches!(h.projection(),StudioProjection::Index(p) if p.tombstones.contains_key(object))) {return Ok(manual);}
            let Some(old) = then
                .objects
                .get(object)
                .or_else(|| then.overflow.get(object))
            else {
                return Ok(manual);
            };
            if old.creations.len() != 1 {
                return Ok(manual);
            }
            let birth = old.creations[0].source.op_id;
            let held = now.objects.get(object).or_else(|| now.overflow.get(object));
            match op {
                IndexOp::PutObject { kind, .. } => Ok(
                    if birth == id
                        && held.is_none()
                        && kind == StudioKind::Flipnote
                        && now.objects.len() + now.overflow.len() < types::MAX_INDEX_OBJECTS
                    {
                        ready
                    } else {
                        manual
                    },
                ),
                IndexOp::SetTitle { .. } | IndexOp::SetExpiry { .. } => {
                    let title = matches!(op, IndexOp::SetTitle { .. });
                    let selected = if title {
                        old.title.selected.source.op_id == id && old.title.conflicts.is_empty()
                    } else {
                        old.expiry.selected.source.op_id == id && old.expiry.conflicts.is_empty()
                    };
                    if !selected {
                        return Ok(manual);
                    }
                    let Some(now) = held else {
                        return Ok(ReplayChoice::After(birth));
                    };
                    Ok(
                        if now.creations.len() == 1
                            && now.creations[0].source.op_id == birth
                            && if title {
                                now.title.selected.source.op_id == birth
                                    && now.title.conflicts.is_empty()
                            } else {
                                now.expiry.selected.source.op_id == birth
                                    && now.expiry.conflicts.is_empty()
                            }
                        {
                            ready
                        } else {
                            manual
                        },
                    )
                }
                IndexOp::TombstoneObject { .. } => Ok(manual),
            }
        }
        _ => Err(invalid("replay projection family mismatch")),
    }
}

#[cfg(test)]
mod tests;
