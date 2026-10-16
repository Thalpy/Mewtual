//! Sound/export evidence shares the frame record framing and P1 admission boundary.
//!
//! Random SFX ids are mutable values plus tombstones permanent in retained causal history;
//! exports have immutable births. Recipes are reusable mutable slots: any live removal wins,
//! even when Automerge selected a Set. A later
//! Set may consume exactly the removals it observed; an unseen concurrent removal still wins.

use super::*;

pub const FLIPNOTE_MAX_SFX: usize = 4096;
pub const FLIPNOTE_MAX_PATCHES: usize = 64;
pub const FLIPNOTE_MAX_EXPORTS: usize = 8;

/// Complete live sound records, not an audible-only view. Public fields are checked again on
/// checkpoint/recovery encoding; Debug never exposes recipes, ids or private content.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct FrameSoundState {
    pub records: BTreeMap<String, FrameRegister<FlipnoteOp>>,
}
impl std::fmt::Debug for FrameSoundState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameSoundState").finish_non_exhaustive()
    }
}

/// Effective sound readiness. Evidence is retained even when these conditions prevent export.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct FrameSoundReadiness {
    pub sfx_overflow: BTreeSet<ElementId>,
    pub patch_overflow: BTreeSet<ContentId>,
    pub export_overflow: BTreeSet<ElementId>,
    pub missing_frames: BTreeSet<ElementId>,
    pub missing_patches: BTreeSet<ElementId>,
    pub conflicts: bool,
}
impl std::fmt::Debug for FrameSoundReadiness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameSoundReadiness")
            .finish_non_exhaustive()
    }
}

pub(super) fn key(operation: &FlipnoteOp, id: &OpId) -> Option<(String, bool)> {
    Some(match operation {
        FlipnoteOp::SetSfx { sfx, .. } => (format!("s/{}", hex(sfx)), true),
        FlipnoteOp::RemoveSfx { sfx } => (format!("q/{}/{}", hex(sfx), hex(id)), false),
        FlipnoteOp::SetExport { export, .. } => (format!("x/{}/{}", hex(export), hex(id)), false),
        FlipnoteOp::RemoveExport { export } => (format!("z/{}/{}", hex(export), hex(id)), false),
        FlipnoteOp::SetPatch { patch, .. } | FlipnoteOp::RemovePatch { patch } => {
            (format!("p/{}", hex(patch)), true)
        }
        FlipnoteOp::SetHeader(FlipnoteHeader::Score(_)) => ("h/score".into(), true),
        _ => return None,
    })
}

pub(super) fn is_key(key: &str) -> bool {
    key == "h/score"
        || ["s/", "q/", "x/", "z/", "p/"]
            .iter()
            .any(|p| key.starts_with(p))
}

impl FrameSoundState {
    pub(super) fn insert(
        &mut self,
        key: String,
        values: FrameRegister<FlipnoteOp>,
    ) -> Result<(), ReplError> {
        for v in std::iter::once(&values.selected).chain(&values.conflicts) {
            if self::key(&v.value, &v.source.op_id).is_none_or(|(expected, _)| expected != key) {
                return Err(ReplError::Malformed);
            }
        }
        self.records.insert(key, values);
        Ok(())
    }

    /// Selected reusable recipe slots. Raw selected Set is NOT enough: every live alternative
    /// must be inspected for Remove before choosing capacity, export union or causal authority.
    pub fn patches(&self) -> BTreeMap<ContentId, &FrameValue<FlipnoteOp>> {
        self.records
            .values()
            .filter_map(|r| {
                if std::iter::once(&r.selected)
                    .chain(&r.conflicts)
                    .any(|v| matches!(v.value, FlipnoteOp::RemovePatch { .. }))
                {
                    return None;
                }
                match &r.selected.value {
                    FlipnoteOp::SetPatch { patch, .. } => Some((*patch, &r.selected)),
                    _ => None,
                }
            })
            .collect()
    }

    pub fn sfx(&self) -> BTreeMap<ElementId, &FrameValue<FlipnoteOp>> {
        self.elements(false)
    }
    pub fn exports(&self) -> BTreeMap<ElementId, &FrameValue<FlipnoteOp>> {
        self.elements(true)
    }
    fn elements(&self, exports: bool) -> BTreeMap<ElementId, &FrameValue<FlipnoteOp>> {
        let mut removed = BTreeSet::new();
        let mut live = BTreeMap::new();
        for r in self.records.values() {
            for v in std::iter::once(&r.selected) {
                let id = match (&v.value, exports) {
                    (FlipnoteOp::RemoveSfx { sfx }, false) => {
                        removed.insert(*sfx);
                        continue;
                    }
                    (FlipnoteOp::RemoveExport { export }, true) => {
                        removed.insert(*export);
                        continue;
                    }
                    (FlipnoteOp::SetSfx { sfx, .. }, false) => *sfx,
                    (FlipnoteOp::SetExport { export, .. }, true) => *export,
                    _ => continue,
                };
                live.entry(id)
                    .and_modify(|old: &mut &FrameValue<FlipnoteOp>| {
                        if v.source.op_id < old.source.op_id {
                            *old = v;
                        }
                    })
                    .or_insert(v);
            }
        }
        live.retain(|id, _| !removed.contains(id));
        live
    }
    pub fn score(&self) -> Option<&FrameRegister<FlipnoteOp>> {
        self.records.get("h/score")
    }
    pub(super) fn known(&self, id: &ElementId, exports: bool) -> bool {
        self.records.values().any(|r| {
            std::iter::once(&r.selected)
                .chain(&r.conflicts)
                .any(|v| match (&v.value, exports) {
                    (FlipnoteOp::SetSfx { sfx, .. } | FlipnoteOp::RemoveSfx { sfx }, false) => {
                        sfx == id
                    }
                    (
                        FlipnoteOp::SetExport { export, .. } | FlipnoteOp::RemoveExport { export },
                        true,
                    ) => export == id,
                    _ => false,
                })
        })
    }
    pub(super) fn deleted_sfx(&self, id: &ElementId) -> bool {
        self.records
            .values()
            .any(|r| matches!(&r.selected.value, FlipnoteOp::RemoveSfx { sfx } if sfx == id))
    }

    pub(super) fn source_ids(&self) -> impl Iterator<Item = OpId> + '_ {
        self.records
            .values()
            .flat_map(|r| std::iter::once(&r.selected).chain(&r.conflicts))
            .map(|v| v.source.op_id)
    }

    pub(super) fn compact(&self, budget: &mut crate::studio::snapshot::ConflictBudget) -> Self {
        let patches: BTreeSet<_> = self
            .patches()
            .keys()
            .take(FLIPNOTE_MAX_PATCHES)
            .copied()
            .collect();
        let sfx: BTreeSet<_> = self.sfx().keys().take(FLIPNOTE_MAX_SFX).copied().collect();
        let exports: BTreeSet<_> = self
            .exports()
            .keys()
            .take(FLIPNOTE_MAX_EXPORTS)
            .copied()
            .collect();
        let mut records = BTreeMap::new();
        let mut export_candidates: BTreeMap<ElementId, usize> = BTreeMap::new();
        for r in self.records.values() {
            if let FlipnoteOp::SetExport { export, .. } = r.selected.value {
                *export_candidates.entry(export).or_default() += 1;
            }
        }
        let mut export_remaining = BTreeMap::new();
        for (key, r) in &self.records {
            let keep = match &r.selected.value {
                FlipnoteOp::SetPatch { patch, .. } => patches.contains(patch),
                FlipnoteOp::SetSfx { sfx: id, .. } => sfx.contains(id),
                FlipnoteOp::SetExport { export, .. } if exports.contains(export) => {
                    let remaining = export_remaining
                        .entry(*export)
                        .or_insert_with(|| 1 + budget.retained(export_candidates[export] - 1));
                    if *remaining == 0 {
                        false
                    } else {
                        *remaining -= 1;
                        true
                    }
                }
                FlipnoteOp::SetHeader(FlipnoteHeader::Score(_)) => true,
                _ => false,
            };
            if keep {
                records.insert(
                    key.clone(),
                    FrameRegister {
                        selected: r.selected.clone(),
                        conflicts: r.conflicts[..budget.retained(r.conflicts.len())].to_vec(),
                    },
                );
            }
        }
        Self { records }
    }
}

impl FlipnoteFrameProjection {
    /// This checks this document's state only. A linked score's actual typed patches, scope and
    /// readiness must additionally be resolved by the application before export/playback.
    pub fn sound_readiness(&self) -> Result<FrameSoundReadiness, ReplError> {
        self.sound.measure(self.document())?;
        let patches = self.sound.patches();
        let sfx = self.sound.sfx();
        let exports = self.sound.exports();
        let mut result = FrameSoundReadiness {
            patch_overflow: patches.keys().skip(FLIPNOTE_MAX_PATCHES).copied().collect(),
            sfx_overflow: sfx.keys().skip(FLIPNOTE_MAX_SFX).copied().collect(),
            export_overflow: exports.keys().skip(FLIPNOTE_MAX_EXPORTS).copied().collect(),
            ..Default::default()
        };
        let mut births = BTreeSet::new();
        for r in self.sound.records.values() {
            // Removed choices remain recovery evidence, not a reason to refuse unrelated
            // current content. Only effective live slots contribute export readiness.
            let active = match &r.selected.value {
                FlipnoteOp::SetSfx { sfx: id, .. } => sfx.contains_key(id),
                FlipnoteOp::SetPatch { patch, .. } => patches.contains_key(patch),
                FlipnoteOp::SetExport { export, .. } => exports.contains_key(export),
                FlipnoteOp::SetHeader(FlipnoteHeader::Score(_)) => true,
                _ => false,
            };
            if !active {
                continue;
            }
            result.conflicts |= !r.conflicts.is_empty();
            if let FlipnoteOp::SetSfx { sfx, .. } | FlipnoteOp::SetExport { export: sfx, .. } =
                r.selected.value
            {
                if !births.insert((
                    matches!(r.selected.value, FlipnoteOp::SetExport { .. }),
                    sfx,
                )) {
                    result.conflicts = true;
                }
            }
        }
        for (id, v) in sfx {
            if let FlipnoteOp::SetSfx { frame, patch, .. } = &v.value {
                if !self.frames.contains_key(frame)
                    || self.tombstones.contains_key(frame)
                    || self.over_cap.contains_key(frame)
                {
                    result.missing_frames.insert(id);
                }
                if !patches.contains_key(patch) || result.patch_overflow.contains(patch) {
                    result.missing_patches.insert(id);
                }
            }
        }
        Ok(result)
    }
}

mod snapshot;
