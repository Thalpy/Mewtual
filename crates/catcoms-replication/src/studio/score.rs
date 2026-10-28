//! Typed score grammar, bounded state and causal validation. These helpers do not themselves
//! authenticate membership, persist intents or publish: the existing P1 gate owns those steps.

use super::*;

mod state;
pub use state::{
    score_document, ScoreAddress, ScoreCellIssues, ScoreLayerSlot, ScoreNoteAddress,
    ScorePatchIssues, ScoreProjection, ScoreReadiness, ScoreRegister, ScoreSlot, ScoreSource,
    ScoreValue,
};
mod voices;
pub use voices::{ScoreVoice, ScoreVoices};
mod snapshot;
pub(in crate::studio) use snapshot::SCORE_PAYLOAD;
mod change;
pub(in crate::studio) use change::prepare;
pub use change::validate_score_change;

/// Maximum sixteenth-step score extent; the current header may select fewer steps.
pub const SCORE_MAX_STEPS: u16 = 256;
/// Chromatic C4 through C6, inclusive (MIDI 60..84).
pub const SCORE_ROWS: u8 = 25;
/// Materialization/export cap, not a count this single-operation codec can enforce.
pub const SCORE_MAX_CELLS: usize = 4096;
/// Score recipe cap; the linked animation must also check its combined score/SFX union.
pub const SCORE_MAX_PATCHES: usize = 64;

/// A cell contains only musical content. Protocol provenance belongs to its enclosing record.
/// Absent length retains the original one-step draft representation; it is not rewritten to 1.
/// Absent volume likewise means full level (100) and is never rewritten to it: `vol` is an
/// additive `jam-score:v1` field (1..=100, percent of the recipe's own level) that a note may
/// carry; a cell without it is byte-identical to one authored before the field existed. A peer
/// older than this field rejects a body or checkpoint that carries it as malformed, and that
/// rejection pauses the old peer's Studio sync for the whole server until an explicit access
/// (the fail-closed admission rule), so every member of a group editing volumes must run this
/// build or newer before anyone authors one (see docs/HANDOVER.md).
#[derive(Clone, PartialEq, Eq)]
pub enum ScoreCell {
    Note {
        patch: ContentId,
        len: Option<u16>,
        volume: Option<u8>,
    },
    Drum {
        pad: u8,
    },
}

/// Inclusive bounds of an authored note volume; 0 would be a silent note, which is a cleared one.
/// The renderer mirrors them as `SCORE_VOLUME_MIN`/`MAX` in `studio-contract.ts`.
pub(crate) const SCORE_VOLUME_MIN: u8 = 1;
pub(crate) const SCORE_VOLUME_MAX: u8 = 100;

impl std::fmt::Debug for ScoreCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScoreCell").finish_non_exhaustive()
    }
}

/// The layer a note lives in (docs/design-score-layers.md section 1): a note's layer is its sound
/// id; every drum is in the single drum layer. Layered notes are keyed `n/{step}:{row}/{layer}`,
/// so one cell holds at most one note per sound plus one drum, and two members placing different
/// sounds in one cell never touch the same register.
///
/// `Ord` puts `Drums` first, then sounds by raw id bytes (equal to lowercase-hex order). This is
/// the projection's map order and the snapshot trailer's entry order; it is NOT the canonical voice
/// order, which compares canonical JSON bytes of the note itself.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScoreLayer {
    Drums,
    Sound(ContentId),
}

impl std::fmt::Debug for ScoreLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Recipe ids are content hashes of private descriptors; keep Debug as redacted as cells.
        f.debug_struct("ScoreLayer").finish_non_exhaustive()
    }
}

impl ScoreLayer {
    /// The key/body spelling: `d`, or the 64 lowercase hex characters of the sound id.
    pub fn key(&self) -> String {
        match self {
            Self::Drums => "d".into(),
            Self::Sound(id) => hex(id),
        }
    }
    /// Exact inverse of [`Self::key`]: anything that is not `d` or 64 LOWERCASE hex is refused,
    /// so one layer has exactly one spelling in a key, a body or a bridge argument.
    pub fn parse(value: &str) -> Result<Self, ReplError> {
        if value == "d" {
            Ok(Self::Drums)
        } else {
            Ok(Self::Sound(fixed_hex(value)?))
        }
    }
}

impl ScoreCell {
    /// The layer this value belongs to. A `set_note`'s key is derived from this, so a note can
    /// never be filed under another sound's layer (the reader re-derives and compares it).
    pub fn layer(&self) -> ScoreLayer {
        match self {
            Self::Note { patch, .. } => ScoreLayer::Sound(*patch),
            Self::Drum { .. } => ScoreLayer::Drums,
        }
    }

    fn value(&self, step: u16) -> Result<Value, ReplError> {
        match self {
            Self::Note { patch, len, volume } => {
                // The caller first bounds step, making this subtraction safe. This is the
                // format maximum, NOT the current score's header length or overlap policy.
                if len.is_some_and(|len| !(1..=SCORE_MAX_STEPS - step).contains(&len)) {
                    return Err(ReplError::Malformed);
                }
                if volume.is_some_and(|v| !(SCORE_VOLUME_MIN..=SCORE_VOLUME_MAX).contains(&v)) {
                    return Err(ReplError::Malformed);
                }
                let mut value = json!({"k":"n", "p":hex(patch)});
                if let Some(len) = len {
                    value["len"] = (*len).into();
                }
                if let Some(volume) = volume {
                    value["vol"] = (*volume).into();
                }
                Ok(value)
            }
            Self::Drum { pad } => {
                if *pad > 9 {
                    return Err(ReplError::Malformed);
                }
                Ok(json!({"k":"d", "pad":pad}))
            }
        }
    }

    fn read(value: &Value) -> Result<Self, ReplError> {
        let cell = object(value)?;
        match text(cell, "k")? {
            "n" => Ok(Self::Note {
                patch: fixed_hex(text(cell, "p")?)?,
                len: cell
                    .get("len")
                    .map(|_| {
                        u16::try_from(unsigned(cell, "len")?).map_err(|_| ReplError::Malformed)
                    })
                    .transpose()?,
                // Range is enforced by the exact re-encode every decode performs (`value`).
                volume: cell
                    .get("vol")
                    .map(|_| u8::try_from(unsigned(cell, "vol")?).map_err(|_| ReplError::Malformed))
                    .transpose()?,
            }),
            "d" => Ok(Self::Drum {
                pad: u8::try_from(unsigned(cell, "pad")?).map_err(|_| ReplError::Malformed)?,
            }),
            _ => Err(ReplError::Malformed),
        }
    }
}

/// Discriminated header values prevent accidental cross-field coercion at the codec boundary.
#[derive(Clone, PartialEq, Eq)]
pub enum ScoreHeader {
    Title(String),
    Bpm(u16),
    Bpb(u8),
    Steps(u16),
}

impl std::fmt::Debug for ScoreHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScoreHeader").finish_non_exhaustive()
    }
}

/// Complete score body grammar from creative design 2.8/2.9. Construction is not validation:
/// encode/decode enforce static ranges; no aggregate cap, patch availability or author authority
/// follows from a successful decode. In particular, overlapping holds require stateful checks.
///
/// Two note families coexist, and they MERGE DIFFERENTLY (docs/design-score-layers.md section 2):
/// - `SetCell`/`ClearCell` (`c/{step}:{row}`, legacy): removal wins and only the register's
///   selected value plays. Still readable and admissible forever; new composers only author
///   `ClearCell`, to remove a legacy note.
/// - `SetNote`/`ClearNote` (`n/{step}:{row}/{layer}`): add wins, and every distinct live value
///   plays. A clear removes only the values its author observed (its Automerge predecessors).
///
/// Compatibility floor: a build without `set_note`/`clear_note` rejects a body, checkpoint or seed
/// carrying one as malformed (the fail-closed rule already used for `vol`), which pauses that
/// build's Studio sync for the server. Every member of a group must run a layer-aware build before
/// anyone authors one.
#[derive(Clone, PartialEq, Eq)]
pub enum ScoreOp {
    SetCell {
        step: u16,
        row: u8,
        value: ScoreCell,
    },
    ClearCell {
        step: u16,
        row: u8,
    },
    /// A layered note. Its layer is `value.layer()`; it has no separate layer field, so a body
    /// cannot name one layer while carrying another sound's note.
    SetNote {
        step: u16,
        row: u8,
        value: ScoreCell,
    },
    /// Clear the layer's note at this cell. Always admissible (cleanup must stay possible), even
    /// for an absent slot or a layer whose sound is gone.
    ClearNote {
        step: u16,
        row: u8,
        layer: ScoreLayer,
    },
    SetPatch {
        patch: ContentId,
        descriptor: StudioPatch,
    },
    RemovePatch {
        patch: ContentId,
    },
    SetHeader(ScoreHeader),
}

impl std::fmt::Debug for ScoreOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScoreOp").finish_non_exhaustive()
    }
}

fn address(step: u16, row: u8) -> Result<(), ReplError> {
    if step >= SCORE_MAX_STEPS || row >= SCORE_ROWS {
        return Err(ReplError::Malformed);
    }
    Ok(())
}

impl ScoreOp {
    /// Canonical Studio body bytes, using the same bounded parser/serializer as art operations.
    pub fn encode(&self) -> Result<Vec<u8>, ReplError> {
        let value = match self {
            Self::SetCell { step, row, value } => {
                address(*step, *row)?;
                json!({"op":"set_cell", "step":step, "row":row, "value":value.value(*step)?})
            }
            Self::ClearCell { step, row } => {
                address(*step, *row)?;
                json!({"op":"clear_cell", "step":step, "row":row})
            }
            // `value` is exactly the set_cell grammar and static bounds (len within the global
            // extent from this step, vol 1..=100, pad 0..=9).
            Self::SetNote { step, row, value } => {
                address(*step, *row)?;
                json!({"op":"set_note", "step":step, "row":row, "value":value.value(*step)?})
            }
            Self::ClearNote { step, row, layer } => {
                address(*step, *row)?;
                json!({"op":"clear_note", "step":step, "row":row, "layer":layer.key()})
            }
            Self::SetPatch { patch, descriptor } => {
                if *patch != descriptor.id() {
                    return Err(ReplError::Malformed);
                }
                json!({"op":"set_patch", "patch":hex(patch), "descriptor":descriptor.value()})
            }
            Self::RemovePatch { patch } => json!({"op":"remove_patch", "patch":hex(patch)}),
            Self::SetHeader(header) => match header {
                ScoreHeader::Title(title) => {
                    title_bound(title)?;
                    json!({"op":"set_header", "field":"title", "value":title})
                }
                ScoreHeader::Bpm(bpm) => {
                    if !(40..=240).contains(bpm) {
                        return Err(ReplError::Malformed);
                    }
                    json!({"op":"set_header", "field":"bpm", "value":bpm})
                }
                ScoreHeader::Bpb(bpb) => {
                    if !(1..=8).contains(bpb) {
                        return Err(ReplError::Malformed);
                    }
                    json!({"op":"set_header", "field":"bpb", "value":bpb})
                }
                ScoreHeader::Steps(steps) => {
                    if !(1..=SCORE_MAX_STEPS).contains(steps) {
                        return Err(ReplError::Malformed);
                    }
                    json!({"op":"set_header", "field":"steps", "value":steps})
                }
            },
        };
        canonical(&value)
    }

    /// Reject unknown/duplicate fields and noncanonical spelling, not just invalid musical values.
    /// Exact reconstruction also rejects provenance smuggled inside a cell or recipe.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        let value = parse(bytes)?;
        let obj = object(&value)?;
        let coordinate = || -> Result<(u16, u8), ReplError> {
            Ok((
                u16::try_from(unsigned(obj, "step")?).map_err(|_| ReplError::Malformed)?,
                u8::try_from(unsigned(obj, "row")?).map_err(|_| ReplError::Malformed)?,
            ))
        };
        let op = match text(obj, "op")? {
            "set_cell" => {
                let (step, row) = coordinate()?;
                Self::SetCell {
                    step,
                    row,
                    value: ScoreCell::read(field(obj, "value")?)?,
                }
            }
            "clear_cell" => {
                let (step, row) = coordinate()?;
                Self::ClearCell { step, row }
            }
            "set_note" => {
                let (step, row) = coordinate()?;
                Self::SetNote {
                    step,
                    row,
                    value: ScoreCell::read(field(obj, "value")?)?,
                }
            }
            "clear_note" => {
                let (step, row) = coordinate()?;
                Self::ClearNote {
                    step,
                    row,
                    layer: ScoreLayer::parse(text(obj, "layer")?)?,
                }
            }
            "set_patch" => Self::SetPatch {
                patch: fixed_hex(text(obj, "patch")?)?,
                descriptor: StudioPatch::new(field(obj, "descriptor")?)?,
            },
            "remove_patch" => Self::RemovePatch {
                patch: fixed_hex(text(obj, "patch")?)?,
            },
            "set_header" => Self::SetHeader(match text(obj, "field")? {
                "title" => ScoreHeader::Title(text(obj, "value")?.into()),
                "bpm" => ScoreHeader::Bpm(
                    u16::try_from(unsigned(obj, "value")?).map_err(|_| ReplError::Malformed)?,
                ),
                "bpb" => ScoreHeader::Bpb(
                    u8::try_from(unsigned(obj, "value")?).map_err(|_| ReplError::Malformed)?,
                ),
                "steps" => ScoreHeader::Steps(
                    u16::try_from(unsigned(obj, "value")?).map_err(|_| ReplError::Malformed)?,
                ),
                _ => return Err(ReplError::Malformed),
            }),
            _ => return Err(ReplError::Malformed),
        };
        exact(bytes, op.encode()?)?;
        Ok(op)
    }

    /// Check a complete envelope against the caller's independently trusted score target.
    /// StudioObject also holds flipnotes, and both kinds share patch/title body shapes. Never
    /// infer the root kind or membership from this parse; P1 and the typed consumer must bind it.
    pub fn decode_domain(document: &LogicalDocument, domain: &DomainOp) -> Result<Self, ReplError> {
        scope(document, domain, DocType::StudioObject)?;
        // Generic P1 logical keys have a wider bound than Studio's fixed 16-byte object ids.
        // Equality alone is insufficient when both caller and envelope share a malformed key.
        if document.logical_key.len() != 16 {
            return Err(ReplError::EpochScope);
        }
        Self::decode(&domain.body)
    }
}

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod tests;
