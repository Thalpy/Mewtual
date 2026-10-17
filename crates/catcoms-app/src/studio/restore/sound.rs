//! Sound recovery selects exact authenticated historical records, never caller-supplied bodies.
//! Restore adds effective, unconflicted source content only. Explicit Copy may replace reusable
//! slots or apply removals, but neither mode can resurrect a permanently deleted SFX/export id.
use super::*;
use types::{
    FlipnoteFrameProjection, FLIPNOTE_MAX_EXPORTS, FLIPNOTE_MAX_PATCHES, FLIPNOTE_MAX_SFX,
};

fn values(p: &FlipnoteFrameProjection) -> impl Iterator<Item = &FrameValue<FlipnoteOp>> {
    p.sound
        .records
        .values()
        .flat_map(|r| std::iter::once(&r.selected).chain(&r.conflicts))
}

/// Consult every retained destination version, including staged/compacted-out negative evidence.
fn negative(
    now: &FlipnoteFrameProjection,
    history: &[StudioRecovery],
    matches: impl Fn(&FlipnoteOp) -> bool,
) -> bool {
    values(now).any(|v| matches(&v.value)) || history.iter().any(|h| {
        matches!(h.projection(), StudioProjection::Flipnote(p) if values(p).any(|v| matches(&v.value)))
    })
}

fn identical(reg: Option<&FrameRegister<FlipnoteOp>>, operation: &FlipnoteOp) -> bool {
    reg.is_some_and(|r| r.conflicts.is_empty() && &r.selected.value == operation)
}

pub(super) fn plan(
    now: &FlipnoteFrameProjection,
    old: &FlipnoteFrameProjection,
    history: &[StudioRecovery],
    source_id: [u8; 32],
    mode: StudioRecoveryMode,
) -> Result<StudioRecoveryPlan, AppError> {
    use StudioRecoveryDisposition as D;
    let held = |d| Ok(StudioRecoveryPlan::held(d));
    let (key, register, value) = old
        .sound
        .records
        .iter()
        .find_map(|(key, reg)| {
            std::iter::once(&reg.selected)
                .chain(&reg.conflicts)
                .find(|v| v.source.op_id == source_id)
                .map(|v| (key, reg, v))
        })
        .ok_or_else(|| invalid("sound recovery value is not in this version"))?;
    let operation = &value.value;
    let selected = register.selected.source.op_id == source_id && register.conflicts.is_empty();
    let existing = now.sound.records.get(key);
    match operation {
        FlipnoteOp::SetPatch { patch, .. } => {
            let key = format!("p/{}", hex::encode(patch));
            let existing = now.sound.records.get(&key);
            if mode == StudioRecoveryMode::Restore {
                // A raw Set winner is not effective while any live Remove survives beside it.
                if !selected
                    || !old
                        .sound
                        .patches()
                        .keys()
                        .take(FLIPNOTE_MAX_PATCHES)
                        .any(|p| p == patch)
                {
                    return held(D::Conflict);
                }
            }
            // A recipe explicitly re-added after an older removal is already present. Reporting
            // this exact no-op as Deleted would misstate current truth; no resurrection is made.
            if identical(existing, operation) {
                return held(D::Unchanged);
            }
            if mode == StudioRecoveryMode::Restore
                && negative(
                    now,
                    history,
                    |op| matches!(op, FlipnoteOp::RemovePatch { patch: p } if p == patch),
                )
            {
                return held(D::Deleted);
            }
            if mode == StudioRecoveryMode::Restore && existing.is_some() {
                return held(D::Conflict);
            }
            if !now.sound.patches().contains_key(patch)
                && now.sound.patches().len() >= FLIPNOTE_MAX_PATCHES
            {
                return held(D::Full);
            }
        }
        FlipnoteOp::RemovePatch { .. } => {
            if mode == StudioRecoveryMode::Restore {
                return held(D::Conflict);
            }
            if identical(existing, operation) {
                return held(D::Unchanged);
            }
        }
        FlipnoteOp::SetSfx {
            sfx, frame, patch, ..
        } => {
            if negative(
                now,
                history,
                |op| matches!(op, FlipnoteOp::RemoveSfx { sfx: id } if id == sfx),
            ) {
                return held(D::Deleted);
            }
            if mode == StudioRecoveryMode::Restore
                && (!selected
                    || !old
                        .sound
                        .sfx()
                        .iter()
                        .take(FLIPNOTE_MAX_SFX)
                        .any(|(id, v)| id == sfx && v.source.op_id == source_id))
            {
                return held(D::Conflict);
            }
            if identical(existing, operation) {
                return held(D::Unchanged);
            }
            if mode == StudioRecoveryMode::Restore && existing.is_some() {
                return held(D::Conflict);
            }
            if !now.timeline.contains(frame)
                || !now.frames.contains_key(frame)
                || now.tombstones.contains_key(frame)
                || now.over_cap.contains_key(frame)
                || !now
                    .sound
                    .patches()
                    .keys()
                    .take(FLIPNOTE_MAX_PATCHES)
                    .any(|p| p == patch)
            {
                return held(D::MissingTarget);
            }
            if !now.sound.sfx().contains_key(sfx) && now.sound.sfx().len() >= FLIPNOTE_MAX_SFX {
                return held(D::Full);
            }
        }
        FlipnoteOp::RemoveSfx { sfx } => {
            if mode == StudioRecoveryMode::Restore {
                return held(D::Conflict);
            }
            if negative(
                now,
                &[],
                |op| matches!(op, FlipnoteOp::RemoveSfx { sfx: id } if id == sfx),
            ) {
                return held(D::Unchanged);
            }
            if !now.sound.sfx().contains_key(sfx) {
                return held(D::MissingTarget);
            }
        }
        FlipnoteOp::SetExport { export, .. } => {
            if negative(
                now,
                history,
                |op| matches!(op, FlipnoteOp::RemoveExport { export: id } if id == export),
            ) {
                return held(D::Deleted);
            }
            let births = |p: &FlipnoteFrameProjection| {
                values(p).filter(|v| matches!(&v.value, FlipnoteOp::SetExport { export: id, .. } if id == export)).count()
            };
            if mode == StudioRecoveryMode::Restore
                && (!selected
                    || births(old) != 1
                    || !old
                        .sound
                        .exports()
                        .iter()
                        .take(FLIPNOTE_MAX_EXPORTS)
                        .any(|(id, v)| id == export && v.source.op_id == source_id))
            {
                return held(D::Conflict);
            }
            // Export births are immutable. Copy cannot replace a different declaration at the
            // same id or hide another concurrent birth behind a new operation.
            if births(now) > 0 {
                return held(
                    if births(now) == 1 && values(now).any(|v| &v.value == operation) {
                        D::Unchanged
                    } else {
                        D::Conflict
                    },
                );
            }
            if now.sound.exports().len() >= FLIPNOTE_MAX_EXPORTS {
                return held(D::Full);
            }
        }
        FlipnoteOp::RemoveExport { export } => {
            if mode == StudioRecoveryMode::Restore {
                return held(D::Conflict);
            }
            if negative(
                now,
                &[],
                |op| matches!(op, FlipnoteOp::RemoveExport { export: id } if id == export),
            ) {
                return held(D::Unchanged);
            }
            if !now.sound.exports().contains_key(export) {
                return held(D::MissingTarget);
            }
        }
        FlipnoteOp::SetHeader(FlipnoteHeader::Score(score)) => {
            if identical(existing, operation) {
                return held(D::Unchanged);
            }
            if mode == StudioRecoveryMode::Restore {
                if score.is_none() || !selected || existing.is_some() {
                    return held(D::Conflict);
                }
                if negative(now, history, |op| {
                    matches!(op, FlipnoteOp::SetHeader(FlipnoteHeader::Score(None)))
                }) {
                    return held(D::Deleted);
                }
            }
            // Same-group/channel typed Score existence is a custody probe at preview and Apply,
            // not something this detached projection-only planner can establish.
        }
        _ => {
            return Err(invalid(
                "sound recovery record has an unsupported operation",
            ))
        }
    }
    StudioRecoveryPlan::ready(
        operation.encode(),
        value.source.author,
        &[value.source.op_id],
    )
}
