//! Per-item domain-level recovery planning. Historical versions never become an Automerge
//! branch or an authority grant. The caller authenticates every input and routes Ready bodies
//! through ordinary Studio Save; whole-version Restore is a sequence of re-previewed steps.
use super::*;
use types::{ElementId, FrameRegister, FrameValue, IndexRegister, IndexValue, StudioRecovery};
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioRecoveryMode {
    Restore,
    /// Explicit confirmation to replace a mutable field or apply a fork's deletion.
    Copy,
}

/// Stable ids and original value provenance, never a timeline position or a caller's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioRecoveryItem {
    Frame { id: ElementId, value: [u8; 32] },
    FrameDeletion { id: ElementId },
    Title { value: [u8; 32] },
    Fps { value: [u8; 32] },
    Object { id: ElementId },
    ObjectTitle { id: ElementId, value: [u8; 32] },
    ObjectExpiry { id: ElementId, value: [u8; 32] },
    ObjectDeletion { id: ElementId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioRecoveryDisposition {
    Ready,
    Unchanged,
    Conflict,
    Deleted,
    Full,
    MissingTarget,
}

/// Which documents this plan is allowed to span.
///
/// Recovery is always `SameDocument`: a historical version of a document can only be restored into
/// that document. Copy adds `CrossDocument`, which drops **only** the logical-key equality and keeps
/// every capacity, conflict, tombstone and over-cap check, because a Flipnote frame is the same kind
/// of thing whichever Flipnote it came from while a frame's *capacity* is a property of where it is
/// going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanScope {
    SameDocument,
    CrossDocument,
}

/// A proposal, not a save result. Body and provenance are private vault content.
pub struct StudioRecoveryPlan {
    pub disposition: StudioRecoveryDisposition,
    pub body: Option<Vec<u8>>,
    pub original_author: Option<crate::DeviceId>,
    /// The operation ids in `historical` that this proposal actually consumed, derived here rather
    /// than supplied by a caller.
    ///
    /// It reports what was resolved and **nothing more**. It is not a preservation claim: copy is
    /// projection-level, so a superseded entry, a conflict alternative, the original authorship of
    /// an accepted envelope and the accepted ordering all have no representation in what this
    /// produces. No count of these ever establishes that a branch was preserved; only an archive
    /// does that. An earlier revision had the caller pass a `source_entry` in, which let a request
    /// claim to have consumed work it never resolved.
    pub source_ops: Vec<[u8; 32]>,
}
impl std::fmt::Debug for StudioRecoveryPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioRecoveryPlan")
            .field("disposition", &self.disposition)
            .finish_non_exhaustive()
    }
}
impl StudioRecoveryPlan {
    fn held(disposition: StudioRecoveryDisposition) -> Self {
        Self {
            disposition,
            body: None,
            original_author: None,
            source_ops: Vec::new(),
        }
    }
    /// `source_ops` is taken as a slice at every call site so the ids come from the values the arm
    /// just resolved, in that arm, rather than being reconstructed afterwards from the item.
    ///
    /// **Deduplicated here, in order.** An `IndexRegister` with no explicit edit falls back to the
    /// creating operation's own source, so a `PutObject` whose title and expiry were never
    /// separately set resolves three registers that are all the birth op. Reporting that id three
    /// times would say the proposal consumed three operations when it consumed one, and
    /// `source_ops` claims to name exactly what was resolved. Order is preserved rather than
    /// sorted, because the order is the order the arm read them in and that is information.
    fn ready(
        body: Result<Vec<u8>, catcoms_replication::ReplError>,
        author: crate::DeviceId,
        source_ops: &[[u8; 32]],
    ) -> Result<Self, AppError> {
        let mut seen = Vec::with_capacity(source_ops.len());
        for id in source_ops {
            if !seen.contains(id) {
                seen.push(*id);
            }
        }
        Ok(Self {
            disposition: StudioRecoveryDisposition::Ready,
            body: Some(body.map_err(invalid)?),
            original_author: Some(author),
            source_ops: seen,
        })
    }
}

fn frame_value<T>(reg: &FrameRegister<T>, id: [u8; 32]) -> Result<&FrameValue<T>, AppError> {
    std::iter::once(&reg.selected)
        .chain(&reg.conflicts)
        .find(|v| v.source.op_id == id)
        .ok_or_else(|| invalid("recovery value is not in this version"))
}
fn index_value<T>(reg: &IndexRegister<T>, id: [u8; 32]) -> Result<&IndexValue<T>, AppError> {
    std::iter::once(&reg.selected)
        .chain(&reg.conflicts)
        .find(|v| v.source.op_id == id)
        .ok_or_else(|| invalid("recovery value is not in this version"))
}

/// The complete checked recovery set matters: a deletion in another retained/staged version
/// can block resurrection even when it was compacted out of the current checkpoint.
fn deleted(current: &StudioProjection, history: &[StudioRecovery], id: ElementId) -> bool {
    std::iter::once(current)
        .chain(history.iter().map(StudioRecovery::projection))
        .any(|p| match p {
            StudioProjection::Index(p) => p.tombstones.contains_key(&id),
            StudioProjection::Flipnote(p) => p.tombstones.contains_key(&id),
        })
}

pub(crate) fn plan(
    current: &StudioProjection,
    historical: &StudioProjection,
    history: &[StudioRecovery],
    item: StudioRecoveryItem,
    mode: StudioRecoveryMode,
    restorer: crate::DeviceId,
    scope: PlanScope,
) -> Result<StudioRecoveryPlan, AppError> {
    use StudioRecoveryDisposition as D;
    use StudioRecoveryItem as I;
    let held = |d| Ok(StudioRecoveryPlan::held(d));
    // The channel never relaxes, in either scope. A cross-document copy still crosses only within
    // one channel's key material, and a different channel naming the same object id is a different
    // destination (C-0), not a permissible source.
    if current.channel() != historical.channel() {
        return Err(invalid("recovery document scope differs"));
    }
    // `historical` is the one thing a cross-document copy is allowed to have come from elsewhere,
    // and **only its logical key may differ**.
    //
    // Relaxing to "same channel" alone would be wrong and the existing scope test is what shows it:
    // `channel()` is the 16-byte channel id and does not name a server, so two unrelated groups can
    // both hold channel `[1; 16]`. Requiring `server_id` and `doc_type` to match as well keeps a
    // foreign group's document from ever being a copy source, which is the whole point of C-0
    // treating the complete `LogicalDocument` as the identity.
    //
    // Everything checked against `current` below - capacity, tombstones, over-cap, conflicts - is a
    // property of the destination and is unaffected by this.
    let compatible = match scope {
        PlanScope::SameDocument => current.document() == historical.document(),
        PlanScope::CrossDocument => {
            current.document().server_id == historical.document().server_id
                && current.document().doc_type == historical.document().doc_type
        }
    };
    if !compatible {
        return Err(invalid("recovery document scope differs"));
    }
    // `history` is always the DESTINATION's retained versions, in both scopes: it exists to prove a
    // deletion elsewhere in the destination's own history blocks a resurrection here. A cross-
    // document copy that relaxed this would consult the source's tombstones about the destination.
    if history.iter().any(|r| {
        r.projection().document() != current.document()
            || r.projection().channel() != current.channel()
    }) {
        return Err(invalid("recovery document scope differs"));
    }
    match (current, historical, item) {
        (
            StudioProjection::Flipnote(now),
            StudioProjection::Flipnote(old),
            I::Frame { id, value },
        ) => {
            let frame = old
                .frames
                .get(&id)
                .ok_or_else(|| invalid("unknown recovery frame"))?;
            let pixels = frame_value(&frame.pixels, value)?;
            if deleted(current, history, id) || old.tombstones.contains_key(&id) {
                return held(D::Deleted);
            }
            if let Some(existing) = now.frames.get(&id) {
                if existing.pixels.selected.value == pixels.value
                    && existing.pixels.conflicts.is_empty()
                {
                    return held(D::Unchanged);
                }
                if mode == StudioRecoveryMode::Restore {
                    return held(D::Conflict);
                }
                let bytes = now
                    .declared_frame_bytes
                    .saturating_sub(existing.pixels.selected.value.bytes)
                    .checked_add(pixels.value.bytes)
                    .ok_or_else(|| invalid("frame byte overflow"))?;
                if now.over_cap.contains_key(&id)
                    || now.timeline.len() > types::FLIPNOTE_MAX_FRAMES
                    || bytes > types::FLIPNOTE_FRAME_BYTES
                {
                    return held(D::Full);
                }
                return StudioRecoveryPlan::ready(
                    FlipnoteOp::ReplaceFrame {
                        frame: id,
                        cid: pixels.value.cid,
                        bytes: pixels.value.bytes,
                    }
                    .encode(),
                    pixels.source.author,
                    &[pixels.source.op_id],
                );
            }
            // Ordinary Restore takes the snapshot's actual selected pixels, not its birth blob.
            // A different alternative is an explicit Copy choice even for an absent frame.
            if mode == StudioRecoveryMode::Restore && value != frame.pixels.selected.source.op_id {
                return held(D::Conflict);
            }
            if now.timeline.len() >= types::FLIPNOTE_MAX_FRAMES
                || now
                    .declared_frame_bytes
                    .checked_add(pixels.value.bytes)
                    .is_none_or(|n| n > types::FLIPNOTE_FRAME_BYTES)
            {
                return held(D::Full);
            }
            let recorded = frame
                .insertions
                .first()
                .ok_or_else(|| invalid("missing frame insertion"))?
                .value
                .after;
            let after = recorded
                .filter(|id| now.timeline.contains(id))
                .or_else(|| now.timeline.last().copied());
            StudioRecoveryPlan::ready(
                FlipnoteOp::InsertFrame {
                    frame: id,
                    after,
                    cid: pixels.value.cid,
                    bytes: pixels.value.bytes,
                }
                .encode(),
                pixels.source.author,
                // The insertion that supplied `after` is positional and is not consumed: the
                // proposal takes the resolved pixels, not that frame's place in the source.
                &[pixels.source.op_id],
            )
        }
        (
            StudioProjection::Flipnote(now),
            StudioProjection::Flipnote(old),
            I::FrameDeletion { id },
        ) => {
            let deletion = old
                .tombstones
                .get(&id)
                .and_then(|d| d.first())
                .ok_or_else(|| invalid("frame was not deleted on this fork"))?;
            if !now.timeline.contains(&id) {
                return held(D::Unchanged);
            }
            if mode == StudioRecoveryMode::Restore {
                return held(D::Conflict);
            }
            StudioRecoveryPlan::ready(
                FlipnoteOp::RemoveFrame { frame: id }.encode(),
                deletion.author,
                &[deletion.op_id],
            )
        }
        (StudioProjection::Flipnote(now), StudioProjection::Flipnote(old), I::Title { value }) => {
            let value = frame_value(
                old.title
                    .as_ref()
                    .ok_or_else(|| invalid("missing recovery title"))?,
                value,
            )?;
            if now
                .title
                .as_ref()
                .is_some_and(|r| r.selected.value == value.value && r.conflicts.is_empty())
            {
                return held(D::Unchanged);
            }
            if mode == StudioRecoveryMode::Restore {
                return held(D::Conflict);
            }
            StudioRecoveryPlan::ready(
                FlipnoteOp::SetHeader(FlipnoteHeader::Title(value.value.clone())).encode(),
                value.source.author,
                &[value.source.op_id],
            )
        }
        (StudioProjection::Flipnote(now), StudioProjection::Flipnote(old), I::Fps { value }) => {
            let value = frame_value(
                old.fps
                    .as_ref()
                    .ok_or_else(|| invalid("missing recovery fps"))?,
                value,
            )?;
            if now
                .fps
                .as_ref()
                .is_some_and(|r| r.selected.value == value.value && r.conflicts.is_empty())
            {
                return held(D::Unchanged);
            }
            if mode == StudioRecoveryMode::Restore {
                return held(D::Conflict);
            }
            StudioRecoveryPlan::ready(
                FlipnoteOp::SetHeader(FlipnoteHeader::Fps(value.value)).encode(),
                value.source.author,
                &[value.source.op_id],
            )
        }
        (StudioProjection::Index(now), StudioProjection::Index(old), I::Object { id }) => {
            let entry = old
                .objects
                .get(&id)
                .or_else(|| old.overflow.get(&id))
                .ok_or_else(|| invalid("unknown live recovery object"))?;
            let birth = entry
                .creations
                .first()
                .ok_or_else(|| invalid("missing object creation"))?;
            if deleted(current, history, id) {
                return held(D::Deleted);
            }
            if let Some(existing) = now.objects.get(&id).or_else(|| now.overflow.get(&id)) {
                // Immutable creation is never replaced. Mutable title/expiry have separate choices.
                return held(if existing == entry {
                    D::Unchanged
                } else {
                    D::Conflict
                });
            }
            if now.objects.len() + now.overflow.len() >= types::MAX_INDEX_OBJECTS {
                return held(D::Full);
            }
            if birth.value.kind != StudioKind::Flipnote {
                return Err(invalid("score recovery is not available yet"));
            }
            StudioRecoveryPlan::ready(
                IndexOp::PutObject {
                    object: id,
                    kind: birth.value.kind,
                    title: entry.title.selected.value.clone(),
                    created_by: restorer,
                    ts: birth.value.ts,
                    expiry: entry.expiry.selected.value,
                }
                .encode(),
                birth.source.author,
                // Three values, so three ids. A PutObject resolves the creation, the selected title
                // and the selected expiry, and reporting only the creation would understate what
                // this proposal read out of the source.
                &[
                    birth.source.op_id,
                    entry.title.selected.source.op_id,
                    entry.expiry.selected.source.op_id,
                ],
            )
        }
        (
            StudioProjection::Index(now),
            StudioProjection::Index(old),
            I::ObjectTitle { id, value },
        ) => {
            let entry = old
                .objects
                .get(&id)
                .or_else(|| old.overflow.get(&id))
                .ok_or_else(|| invalid("unknown live recovery object"))?;
            let value = index_value(&entry.title, value)?;
            if deleted(current, history, id) {
                return held(D::Deleted);
            }
            let Some(existing) = now.objects.get(&id).or_else(|| now.overflow.get(&id)) else {
                return held(D::MissingTarget);
            };
            if existing.title.selected.value == value.value && existing.title.conflicts.is_empty() {
                return held(D::Unchanged);
            }
            if mode == StudioRecoveryMode::Restore {
                return held(D::Conflict);
            }
            StudioRecoveryPlan::ready(
                IndexOp::SetTitle {
                    object: id,
                    title: value.value.clone(),
                }
                .encode(),
                value.source.author,
                &[value.source.op_id],
            )
        }
        (
            StudioProjection::Index(now),
            StudioProjection::Index(old),
            I::ObjectExpiry { id, value },
        ) => {
            let entry = old
                .objects
                .get(&id)
                .or_else(|| old.overflow.get(&id))
                .ok_or_else(|| invalid("unknown live recovery object"))?;
            let value = index_value(&entry.expiry, value)?;
            if deleted(current, history, id) {
                return held(D::Deleted);
            }
            let Some(existing) = now.objects.get(&id).or_else(|| now.overflow.get(&id)) else {
                return held(D::MissingTarget);
            };
            if existing.expiry.selected.value == value.value && existing.expiry.conflicts.is_empty()
            {
                return held(D::Unchanged);
            }
            if mode == StudioRecoveryMode::Restore {
                return held(D::Conflict);
            }
            StudioRecoveryPlan::ready(
                IndexOp::SetExpiry {
                    object: id,
                    expiry: value.value,
                }
                .encode(),
                value.source.author,
                &[value.source.op_id],
            )
        }
        (StudioProjection::Index(now), StudioProjection::Index(old), I::ObjectDeletion { id }) => {
            let deletion = old
                .tombstones
                .get(&id)
                .and_then(|d| d.first())
                .ok_or_else(|| invalid("object was not deleted on this fork"))?;
            if !now.objects.contains_key(&id) && !now.overflow.contains_key(&id) {
                return held(D::Unchanged);
            }
            if mode == StudioRecoveryMode::Restore {
                return held(D::Conflict);
            }
            StudioRecoveryPlan::ready(
                IndexOp::TombstoneObject { object: id }.encode(),
                deletion.author,
                &[deletion.op_id],
            )
        }
        _ => Err(invalid("recovery choice is for a different document type")),
    }
}
