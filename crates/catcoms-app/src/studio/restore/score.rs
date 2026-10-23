//! Score recovery authors fresh ordinary operations; retained old signatures grant no authority.
//! Coordinates/recipe hashes are reusable registers, unlike art's permanent element tombstones.
//! Restore is deliberately additive. Any retained clear, existing slot or alternative needs Copy,
//! whose current-projection fingerprint and ordinary Save path are checked again by the caller.
use super::*;
use types::{
    ScoreCell, ScoreLayerSlot, ScoreProjection, ScoreRegister, ScoreSlot, SCORE_MAX_CELLS,
    SCORE_MAX_PATCHES,
};

/// LEGACY slots only (removal wins): one retained value equal to `value`. Never apply this to a
/// layer slot, whose register may select a clear while a live value still plays.
fn same<T: PartialEq>(slot: &ScoreSlot<T>, value: &Option<T>) -> bool {
    slot.register.conflicts.is_empty() && slot.register.selected.value == *value
}

/// Layer slots (add wins): authoring `value` would change nothing that plays. A value is
/// unchanged when it is exactly the slot's one live note (a `SetNote` at the current frontier
/// would replace every other live value, so a second live value means a real change); a clear is
/// unchanged when nothing there plays.
fn same_note(slot: Option<&ScoreLayerSlot>, value: Option<&ScoreCell>) -> bool {
    let live = slot.map(ScoreLayerSlot::live).unwrap_or_default();
    match value {
        Some(cell) => live == [cell],
        None => live.is_empty(),
    }
}

/// The combined note cap's occupancy (docs/design-score-layers.md section 6): active legacy cells
/// (removal wins) plus live layer slots (add wins). Admission counts the same way.
fn occupied(now: &ScoreProjection) -> usize {
    now.cells.values().filter(|s| s.value().is_some()).count()
        + now.layers.values().filter(|s| !s.cleared()).count()
}

/// Shared by both note families: the hold must fit the CURRENT header length, and a note's
/// sound must be an active recipe now. `None` when the value may be authored.
fn note_hold(
    now: &ScoreProjection,
    step: u16,
    cell: &ScoreCell,
) -> Option<StudioRecoveryDisposition> {
    let end = step
        + match cell {
            ScoreCell::Note { len, .. } => len.unwrap_or(1),
            ScoreCell::Drum { .. } => 1,
        };
    if end > now.effective_steps() {
        return Some(StudioRecoveryDisposition::Conflict);
    }
    if let ScoreCell::Note { patch, .. } = cell {
        if now.patches.get(patch).and_then(ScoreSlot::value).is_none() {
            return Some(StudioRecoveryDisposition::MissingTarget);
        }
    }
    None
}

fn header<T: Clone + PartialEq>(
    now: Option<&ScoreRegister<T>>,
    old: Option<&ScoreRegister<T>>,
    value: [u8; 32],
    mode: StudioRecoveryMode,
    operation: impl FnOnce(T) -> ScoreHeader,
) -> Result<StudioRecoveryPlan, AppError> {
    use StudioRecoveryDisposition as D;
    let reg = old.ok_or_else(|| invalid("missing recovery score header"))?;
    let value = index_value(reg, value)?;
    if now.is_some_and(|r| r.conflicts.is_empty() && r.selected.value == value.value) {
        return Ok(StudioRecoveryPlan::held(D::Unchanged));
    }
    if mode == StudioRecoveryMode::Restore
        && (now.is_some()
            || !reg.conflicts.is_empty()
            || reg.selected.source.op_id != value.source.op_id)
    {
        return Ok(StudioRecoveryPlan::held(D::Conflict));
    }
    StudioRecoveryPlan::ready(
        ScoreOp::SetHeader(operation(value.value.clone())).encode(),
        value.source.author,
        &[value.source.op_id],
    )
}

pub(super) fn plan(
    now: &ScoreProjection,
    old: &ScoreProjection,
    history: &[StudioRecovery],
    item: StudioRecoveryItem,
    mode: StudioRecoveryMode,
) -> Result<StudioRecoveryPlan, AppError> {
    use StudioRecoveryDisposition as D;
    use StudioRecoveryItem as I;
    let held = |d| Ok(StudioRecoveryPlan::held(d));
    match item {
        I::ScoreCell { step, row, value } => {
            let address = (step, row);
            let slot = old
                .cells
                .get(&address)
                .ok_or_else(|| invalid("unknown recovery score cell"))?;
            let value = index_value(&slot.register, value)?;
            let existing = now.cells.get(&address);
            // Even an absent current coordinate cannot prove that an intervening Clear never
            // occurred. Retained history is useful negative evidence, never automatic replay.
            if mode == StudioRecoveryMode::Restore {
                if value.value.is_none()
                    || existing.is_some()
                    || slot.removed()
                    || !slot.register.conflicts.is_empty()
                    || slot.register.selected.source.op_id != value.source.op_id
                {
                    return held(
                        if value.value.is_some() && existing.is_some_and(|s| same(s, &value.value))
                        {
                            D::Unchanged
                        } else {
                            D::Conflict
                        },
                    );
                }
                if history.iter().any(|h| matches!(h.projection(), StudioProjection::Score(p) if p.cells.get(&address).is_some_and(ScoreSlot::removed))) {
                    return held(D::Deleted);
                }
            }
            if existing.is_some_and(|s| same(s, &value.value)) {
                return held(D::Unchanged);
            }
            let operation = match &value.value {
                None => ScoreOp::ClearCell { step, row },
                Some(cell) => {
                    if let Some(d) = note_hold(now, step, cell) {
                        return held(d);
                    }
                    // Full counts legacy cells and live layer slots together.
                    if existing.and_then(ScoreSlot::value).is_none()
                        && occupied(now) >= SCORE_MAX_CELLS
                    {
                        return held(D::Full);
                    }
                    ScoreOp::SetCell {
                        step,
                        row,
                        value: cell.clone(),
                    }
                }
            };
            StudioRecoveryPlan::ready(
                operation.encode(),
                value.source.author,
                &[value.source.op_id],
            )
        }
        // Layered notes are ADD WINS: none of the removal-wins helpers above (`same`,
        // `ScoreSlot::removed`/`value`) apply. A version's slot "shows" `shown()`; it is
        // "deleted" only when `cleared()`.
        I::ScoreNote {
            step,
            row,
            layer,
            value,
        } => {
            let key = (step, row, layer);
            let slot = old
                .layers
                .get(&key)
                .ok_or_else(|| invalid("unknown recovery score note"))?;
            let value = index_value(slot.register(), value)?;
            // The projection already binds a value's layer to its slot; restate it so a body
            // authored here can never file a note under another sound's layer.
            if value
                .value
                .as_ref()
                .is_some_and(|cell| cell.layer() != layer)
            {
                return Err(invalid("recovery note is in another layer"));
            }
            let existing = now.layers.get(&key);
            let unchanged = same_note(existing, value.value.as_ref());
            if mode == StudioRecoveryMode::Restore {
                // Additive only: restore re-creates the note the version SHOWED (one author holds
                // one value per slot, so a version's other concurrent values are not recreated)
                // into an empty slot. A clear, another alternative, or anything already at the
                // slot (a different note, or a present-day clear) needs an explicit Copy.
                let shown = slot.shown_alternative().map(|v| v.source.op_id);
                if value.value.is_none() || existing.is_some() || shown != Some(value.source.op_id)
                {
                    return held(if value.value.is_some() && unchanged {
                        D::Unchanged
                    } else {
                        D::Conflict
                    });
                }
                // Retained destination history is negative evidence: a version in which this
                // slot was cleared blocks resurrecting it.
                if history.iter().any(|h| {
                    matches!(h.projection(), StudioProjection::Score(p)
                        if p.layers.get(&key).is_some_and(ScoreLayerSlot::cleared))
                }) {
                    return held(D::Deleted);
                }
            }
            if unchanged {
                return held(D::Unchanged);
            }
            let operation = match &value.value {
                // A clear removes only what its author observes, i.e. every value at the slot
                // now: Copy of a recovered clear empties the slot.
                None => ScoreOp::ClearNote { step, row, layer },
                Some(cell) => {
                    if let Some(d) = note_hold(now, step, cell) {
                        return held(d);
                    }
                    // A new (absent or cleared) layer slot is growth; replacing a live one is not.
                    if existing.is_none_or(ScoreLayerSlot::cleared)
                        && occupied(now) >= SCORE_MAX_CELLS
                    {
                        return held(D::Full);
                    }
                    ScoreOp::SetNote {
                        step,
                        row,
                        value: cell.clone(),
                    }
                }
            };
            StudioRecoveryPlan::ready(
                operation.encode(),
                value.source.author,
                &[value.source.op_id],
            )
        }
        I::ScorePatch { patch, value } => {
            let slot = old
                .patches
                .get(&patch)
                .ok_or_else(|| invalid("unknown recovery score recipe"))?;
            let value = index_value(&slot.register, value)?;
            let existing = now.patches.get(&patch);
            if mode == StudioRecoveryMode::Restore {
                if value.value.is_none()
                    || existing.is_some()
                    || slot.removed()
                    || !slot.register.conflicts.is_empty()
                    || slot.register.selected.source.op_id != value.source.op_id
                {
                    return held(
                        if value.value.is_some() && existing.is_some_and(|s| same(s, &value.value))
                        {
                            D::Unchanged
                        } else {
                            D::Conflict
                        },
                    );
                }
                if history.iter().any(|h| matches!(h.projection(), StudioProjection::Score(p) if p.patches.get(&patch).is_some_and(ScoreSlot::removed))) { return held(D::Deleted); }
            }
            if existing.is_some_and(|s| same(s, &value.value)) {
                return held(D::Unchanged);
            }
            let operation = match &value.value {
                None => ScoreOp::RemovePatch { patch },
                Some(recipe) => {
                    if existing.and_then(ScoreSlot::value).is_none()
                        && now.patches.values().filter(|s| s.value().is_some()).count()
                            >= SCORE_MAX_PATCHES
                    {
                        return held(D::Full);
                    }
                    ScoreOp::SetPatch {
                        patch,
                        descriptor: recipe.clone(),
                    }
                }
            };
            StudioRecoveryPlan::ready(
                operation.encode(),
                value.source.author,
                &[value.source.op_id],
            )
        }
        I::ScoreHeader { field, value } => match field {
            StudioScoreHeaderField::Title => header(
                now.title.as_ref(),
                old.title.as_ref(),
                value,
                mode,
                ScoreHeader::Title,
            ),
            StudioScoreHeaderField::Bpm => header(
                now.bpm.as_ref(),
                old.bpm.as_ref(),
                value,
                mode,
                ScoreHeader::Bpm,
            ),
            StudioScoreHeaderField::Bpb => header(
                now.bpb.as_ref(),
                old.bpb.as_ref(),
                value,
                mode,
                ScoreHeader::Bpb,
            ),
            StudioScoreHeaderField::Steps => header(
                now.steps.as_ref(),
                old.steps.as_ref(),
                value,
                mode,
                ScoreHeader::Steps,
            ),
        },
        _ => Err(invalid("recovery choice is for a different document type")),
    }
}
