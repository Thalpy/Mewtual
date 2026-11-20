//! What a Score plays, and what is worth reviewing about it (docs/design-score-layers.md
//! section 3). This is the Rust half of ONE definition: `apps/desktop/src/score-voices.ts` is the
//! renderer half, and the shared fixture `tests/fixtures/studio-score-readiness-v1.json` pins the
//! two together. Native sends readiness with every view and the renderer fingerprints it against
//! its own working projection, so any drift between the halves makes a readback of the very score
//! on screen look like a source change and stops its playback. Change both together or neither.
//!
//! Merging is additive: identical notes collapse to one voice, distinct notes in one layer slot
//! all play, overlapping holds all play. Nothing here refuses a score. A voice that cannot play
//! (past the end, over the cap, its sound not admitted) is skipped and counted; a note running
//! past the end is clipped.
//!
//! Work is O(n log n) in the evidence a projection carries (the ordered voice map), never
//! quadratic: one member can stack thousands of concurrent values on one key (empty predecessors
//! at a stale frontier), and native computes this on every view.

use std::collections::{BTreeMap, BTreeSet};

use super::state::*;
use super::*;

/// A note that plays, as the exporter takes it: its EFFECTIVE value (clipped to the grid, `len`
/// written only above 1), never the raw authored one.
#[derive(Clone, PartialEq, Eq)]
pub struct ScoreVoice {
    pub step: u16,
    pub row: u8,
    pub value: ScoreCell,
}
impl std::fmt::Debug for ScoreVoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScoreVoice").finish_non_exhaustive()
    }
}

/// What a score plays: `play` in canonical voice order, distinct, effective values; `skipped`
/// counts each voice that cannot play exactly once (over the cap, sound not admitted, or starting
/// at or after the end), mirroring `ScoreVoices.play`/`skipped` in score-voices.ts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreVoices {
    pub play: Vec<ScoreVoice>,
    pub skipped: usize,
}

/// One distinct voice with the slot keys it came from. At most one legacy key (the cell at the
/// voice's own coordinate) and at most one layer key (a voice's layer is its value's), because a
/// layered value's layer always equals its slot's.
pub(super) struct Voice {
    pub(super) step: u16,
    pub(super) row: u8,
    /// Effective value (see [`effective`]).
    pub(super) value: ScoreCell,
    /// Canonical JSON bytes of `value`: the third component of the voice identity and order.
    pub(super) bytes: Vec<u8>,
    pub(super) legacy: bool,
    pub(super) layer: Option<ScoreLayer>,
    pub(super) played: bool,
}

/// Every voice in canonical order plus the sparse per-slot flags.
pub(super) struct VoiceReport {
    pub(super) voices: Vec<Voice>,
    pub(super) skipped: usize,
    pub(super) cells: BTreeMap<ScoreAddress, ScoreCellIssues>,
    pub(super) layers: BTreeMap<ScoreNoteAddress, ScoreCellIssues>,
}
impl VoiceReport {
    pub(super) fn public(&self) -> ScoreVoices {
        ScoreVoices {
            play: self
                .voices
                .iter()
                .filter(|v| v.played)
                .map(|v| ScoreVoice {
                    step: v.step,
                    row: v.row,
                    value: v.value.clone(),
                })
                .collect(),
            skipped: self.skipped,
        }
    }
}

/// Structural identity of a cell, for de-duplication only. Two cells are the same note exactly
/// when their canonical bytes are equal, and this tuple is equal exactly then (the codec writes
/// every field and nothing else), without serialising. It is NOT the canonical voice order.
pub(super) fn identity(cell: &ScoreCell) -> (u8, ContentId, Option<u16>, Option<u8>) {
    match cell {
        ScoreCell::Note { patch, len, volume } => (0, *patch, *len, *volume),
        ScoreCell::Drum { pad } => (1, [0; 32], None, Some(*pad)),
    }
}

/// The value a voice plays and is identified by. A note that starts inside the grid
/// (`step < steps`) is clipped to its end; `len` is written only above 1, because an absent `len`
/// and `len: 1` are the same note to the exporter. `vol` stays as authored: an absent level is not
/// a level of 100 to the take format. A note starting at or after the end keeps `len or 1`. A drum
/// is itself.
pub(super) fn effective(step: u16, cell: &ScoreCell, steps: u16) -> ScoreCell {
    match cell {
        ScoreCell::Drum { .. } => cell.clone(),
        ScoreCell::Note { patch, len, volume } => {
            let mut len = len.unwrap_or(1);
            if step < steps {
                len = len.min(steps - step);
            }
            ScoreCell::Note {
                patch: *patch,
                len: (len > 1).then_some(len),
                volume: *volume,
            }
        }
    }
}

/// Canonical JSON bytes of a cell (sorted keys, no whitespace), exactly what `ScoreCell::value`
/// serialised through the shared canonical encoder produces and what the renderer's
/// `canonicalJson` produces; a unit test pins the equality. Written by hand, byte by byte,
/// because the voice order needs one per candidate and this runs on every native view.
pub(super) fn canonical_bytes(cell: &ScoreCell) -> Vec<u8> {
    fn decimal(out: &mut Vec<u8>, value: u16) {
        let digits = value.to_string();
        out.extend_from_slice(digits.as_bytes());
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = Vec::with_capacity(112);
    match cell {
        ScoreCell::Drum { pad } => {
            out.extend_from_slice(br#"{"k":"d","pad":"#);
            decimal(&mut out, u16::from(*pad));
        }
        ScoreCell::Note { patch, len, volume } => {
            out.extend_from_slice(br#"{"k":"n","#);
            if let Some(len) = len {
                out.extend_from_slice(br#""len":"#);
                decimal(&mut out, *len);
                out.push(b',');
            }
            out.extend_from_slice(br#""p":""#);
            for byte in patch {
                out.push(HEX[usize::from(byte >> 4)]);
                out.push(HEX[usize::from(byte & 0x0f)]);
            }
            out.push(b'"');
            if let Some(volume) = volume {
                out.extend_from_slice(br#","vol":"#);
                decimal(&mut out, u16::from(*volume));
            }
        }
    }
    out.push(b'}');
    out
}

/// Where a hold ends, in steps. Widened so a caller-mutated (unvalidated) length cannot overflow.
fn hold_end(step: u16, cell: &ScoreCell) -> u32 {
    u32::from(step)
        + match cell {
            ScoreCell::Note { len, .. } => u32::from(len.unwrap_or(1)),
            ScoreCell::Drum { .. } => 1,
        }
}

#[derive(Default)]
struct Flags {
    cells: BTreeMap<ScoreAddress, ScoreCellIssues>,
    layers: BTreeMap<ScoreNoteAddress, ScoreCellIssues>,
}
impl Flags {
    /// Entries are created only to set a flag, which is what keeps readiness sparse.
    fn slot(&mut self, step: u16, row: u8, layer: Option<ScoreLayer>) -> &mut ScoreCellIssues {
        match layer {
            None => self.cells.entry((step, row)).or_default(),
            Some(layer) => self.layers.entry((step, row, layer)).or_default(),
        }
    }
    fn voice(&mut self, voice: &Voice, set: impl Fn(&mut ScoreCellIssues)) {
        if voice.legacy {
            set(self.slot(voice.step, voice.row, None));
        }
        if let Some(layer) = voice.layer {
            set(self.slot(voice.step, voice.row, Some(layer)));
        }
    }
}

/// Every distinct voice of these slots in canonical order, with the per-slot `conflict` and
/// `extent` flags (the flags that depend on raw values rather than on voices).
fn collect(
    cells: &BTreeMap<ScoreAddress, ScoreSlot<ScoreCell>>,
    layers: &BTreeMap<ScoreNoteAddress, ScoreLayerSlot>,
    steps: u16,
) -> (Vec<Voice>, Flags) {
    let mut flags = Flags::default();
    // Keyed by the voice identity (step, row, canonical bytes of the effective value), so identical
    // voices collapse across every slot AND the map's order is the canonical voice order. Rust
    // compares `Vec<u8>` bytewise, as the renderer compares its ASCII canonical strings.
    let mut voices: BTreeMap<(u16, u8, Vec<u8>), Voice> = BTreeMap::new();
    let mut candidate = |flags: &mut Flags,
                         (step, row): ScoreAddress,
                         layer: Option<ScoreLayer>,
                         raw: &ScoreCell| {
        // `extent` is about the RAW candidate: it starts at or after the end, or its authored hold
        // runs past it (and is clipped). Raw, so the flag shows the clipping that happened.
        if step >= steps || hold_end(step, raw) > u32::from(steps) {
            flags.slot(step, row, layer).extent = true;
        }
        let value = effective(step, raw, steps);
        let bytes = canonical_bytes(&value);
        let voice = voices
            .entry((step, row, bytes.clone()))
            .or_insert_with(|| Voice {
                step,
                row,
                value,
                bytes,
                legacy: false,
                layer: None,
                played: false,
            });
        match layer {
            None => voice.legacy = true,
            Some(layer) => voice.layer = Some(layer),
        }
    };
    for (&address, slot) in cells {
        // Removal wins: any clear alternative removes a legacy cell, flags and all. Only the
        // SELECTED value plays: legacy compaction keeps a bounded number of alternatives, so
        // playing more than the winner would change what a score sounds like at its next
        // checkpoint (design review H2).
        let Some(selected) = slot.value() else {
            continue;
        };
        // More than one DISTINCT raw value retained (identical alternatives are one note). Every
        // alternative of a live legacy slot is a value, so "some differs from the selected" is
        // the same predicate in one linear pass.
        if slot.values().any(|v| v.value.as_ref() != Some(selected)) {
            flags.slot(address.0, address.1, None).conflict = true;
        }
        candidate(&mut flags, address, None, selected);
    }
    for (&(step, row, layer), slot) in layers {
        // Add wins: clears are ignored while any value is live, and every live value plays.
        // Duplicates collapse in the voice map; `conflict` asks whether a second DISTINCT raw
        // value exists, which "some live value differs from the first" answers linearly.
        let mut first: Option<&ScoreCell> = None;
        for raw in slot.alternatives().filter_map(|v| v.value.as_ref()) {
            match first {
                None => first = Some(raw),
                Some(first) if first != raw => {
                    flags.slot(step, row, Some(layer)).conflict = true;
                }
                Some(_) => {}
            }
            candidate(&mut flags, (step, row), Some(layer), raw);
        }
    }
    (voices.into_values().collect(), flags)
}

/// The identities of the voices a checkpoint keeps: the first `SCORE_MAX_CELLS` in canonical
/// order, skipped ones included (they are voices; `count` is over voices). Compaction retains
/// exactly the slots and values that back these (docs/design-score-layers.md section 5). For a
/// layer-free score this is today's `cells.take(4096)` over active cells, because its voices are
/// its active cells in `(step, row)` order, one each.
pub(super) fn retained(
    cells: &BTreeMap<ScoreAddress, ScoreSlot<ScoreCell>>,
    layers: &BTreeMap<ScoreNoteAddress, ScoreLayerSlot>,
    steps: u16,
) -> BTreeSet<(u16, u8, Vec<u8>)> {
    collect(cells, layers, steps)
        .0
        .into_iter()
        .take(SCORE_MAX_CELLS)
        .map(|v| (v.step, v.row, v.bytes))
        .collect()
}

/// The whole section 3 definition over raw slots. `steps` is the effective header length and
/// `admitted` the first 64 active sound ids (`ScoreProjection::admitted_sounds`). Pure: callers
/// that hold caller-mutable projections measure them first (`readiness`, `voices`).
pub(super) fn compute(
    cells: &BTreeMap<ScoreAddress, ScoreSlot<ScoreCell>>,
    layers: &BTreeMap<ScoreNoteAddress, ScoreLayerSlot>,
    steps: u16,
    admitted: &BTreeSet<ContentId>,
) -> VoiceReport {
    let (mut voices, mut flags) = collect(cells, layers, steps);
    // Overlap (design review H3): holds on the same row AND of the same sound (different sounds
    // overlapping is what layers are for), over every note voice, played or skipped, with its
    // effective end. Within one (row, layer) lane the voices are already in start order, so a
    // voice intersects another of its lane exactly when it starts before the furthest end reached
    // so far, or the next voice starts before it ends. One pass per lane after the sort.
    let mut lanes: BTreeMap<(u8, ScoreLayer), Vec<usize>> = BTreeMap::new();
    for (index, voice) in voices.iter().enumerate() {
        if matches!(voice.value, ScoreCell::Note { .. }) {
            lanes
                .entry((voice.row, voice.value.layer()))
                .or_default()
                .push(index);
        }
    }
    for lane in lanes.values() {
        let mut reach: Option<u32> = None;
        for (position, &index) in lane.iter().enumerate() {
            let voice = &voices[index];
            let start = u32::from(voice.step);
            let end = hold_end(voice.step, &voice.value);
            let next = lane.get(position + 1).map(|&n| u32::from(voices[n].step));
            if reach.is_some_and(|reach| start < reach) || next.is_some_and(|next| next < end) {
                flags.voice(voice, |f| f.overlap = true);
            }
            reach = Some(reach.map_or(end, |reach| reach.max(end)));
        }
    }
    // Skip what cannot play, each voice once: over the cap (in canonical order), a sound not
    // admitted, a start at or after the end. Everything else plays, already clipped.
    let mut skipped = 0;
    for (index, voice) in voices.iter_mut().enumerate() {
        let over = index >= SCORE_MAX_CELLS;
        let recipe =
            matches!(&voice.value, ScoreCell::Note { patch, .. } if !admitted.contains(patch));
        if over {
            flags.voice(voice, |f| f.count = true);
        }
        if recipe {
            flags.voice(voice, |f| f.recipe = true);
        }
        voice.played = !(over || recipe || voice.step >= steps);
        if !voice.played {
            skipped += 1;
        }
    }
    VoiceReport {
        voices,
        skipped,
        cells: flags.cells,
        layers: flags.layers,
    }
}

#[cfg(test)]
mod tests;
