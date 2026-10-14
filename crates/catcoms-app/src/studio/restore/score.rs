//! Score recovery authors fresh ordinary operations; retained old signatures grant no authority.
//! Coordinates/recipe hashes are reusable registers, unlike art's permanent element tombstones.
//! Restore is deliberately additive. Any retained clear, existing slot or alternative needs Copy,
//! whose current-projection fingerprint and ordinary Save path are checked again by the caller.
use super::*;
use types::{
    ScoreCell, ScoreProjection, ScoreRegister, ScoreSlot, SCORE_MAX_CELLS, SCORE_MAX_PATCHES,
};

fn same<T: PartialEq>(slot: &ScoreSlot<T>, value: &Option<T>) -> bool {
    slot.register.conflicts.is_empty() && slot.register.selected.value == *value
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
                    let end = step
                        + match cell {
                            ScoreCell::Note { len, .. } => len.unwrap_or(1),
                            ScoreCell::Drum { .. } => 1,
                        };
                    if end > now.effective_steps() {
                        return held(D::Conflict);
                    }
                    if let ScoreCell::Note { patch, .. } = cell {
                        if now.patches.get(patch).and_then(ScoreSlot::value).is_none() {
                            return held(D::MissingTarget);
                        }
                    }
                    if existing.and_then(ScoreSlot::value).is_none()
                        && now.cells.values().filter(|s| s.value().is_some()).count()
                            >= SCORE_MAX_CELLS
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
