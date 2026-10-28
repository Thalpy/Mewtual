# Score layers and additive merging (design, 2026-10-20)

Owner decisions (see HANDOVER.md, top): one note per sound per cell; additive merging where adds
win and nothing blocks playback; automatic re-save on an epoch move (done, stage 1, and extended to
`set_note` here); show / ghost / hide layers in the composer. This document is the contract between
the replication crate, the native bridge and the renderer. Anything not stated here keeps its
current behaviour. A design review (2026-10-20) found four HIGH and five MEDIUM issues in the first
draft; each fix is folded in below and marked "(review Hn/Mn)".

## 1. Wire format: two new Score operations

Existing operations (`set_cell`, `clear_cell`, `set_patch`, `remove_patch`, `set_header`) are
unchanged and stay readable and admissible forever. New builds' composer stops authoring
`set_cell`; it still authors `clear_cell` to remove a legacy note.

```
{"op":"set_note","row":R,"step":S,"value":<ScoreCell>}
{"layer":"d"|"<64 lowercase hex>","op":"clear_note","row":R,"step":S}
```

- Bodies are canonical JSON (sorted keys, as every Score body), exact re-encode on decode.
- `value` is exactly the `set_cell` grammar: `{"k":"n","p":<64hex>,"len"?,"vol"?}` or
  `{"k":"d","pad":0..9}`. Same static bounds (step < 256, row < 25, len within the global extent,
  vol 1..100).
- The note's **layer** comes from its value: a note's layer is its sound id (`p`); every drum is in
  the single drum layer `d`. `clear_note` names the layer explicitly.
- Rust: `ScoreOp::SetNote { step, row, value }`, `ScoreOp::ClearNote { step, row, layer }`,
  `pub enum ScoreLayer { Drums, Sound(ContentId) }` (`Ord`, Drums first), `ScoreCell::layer()`.
- Compatibility floor: a build without these operations rejects a body, a checkpoint or a seed that
  carries one as malformed, which pauses that build's Studio sync for the server (the fail-closed
  rule already used for the `vol` field). Every member of a group editing together must run a
  build with layers before anyone authors one. Seeds containing layered state are unreadable to
  an older build, and an older-build tenure owner cannot close an epoch containing it. Fully
  cleared layers can compact away; their operations or recovery archives can remain incompatible
  independently, so an empty current layer projection is not permission to downgrade.
  This is accepted and goes in the release notes.

## 2. Document keys and projection

- Key: `n/{step}:{row}/{layer}` where layer is `d` or the 64-hex sound id. The longest key is
  `n/255:24/` + 64 hex = 73 bytes, so `MAX_KEY_BYTES` becomes 73 (it was exactly `_p1/op/` + 64).
- `read_view` parses the `n/` prefix with the same canonical-address rules as `c/`, and requires
  `operation_key(op) == key` and, for `SetNote`, `value.layer() == key layer`.
- `ScoreProjection.layers: BTreeMap<(u16, u8, ScoreLayer), ScoreLayerSlot>` beside `cells`.
- Merge semantics differ by collection, and doc comments must say which is which:
  - `cells` (legacy `c/`) and `patches` keep **removal wins** (`ScoreSlot::removed`): a slot with
    any clear alternative is removed, and a live legacy slot plays **only its selected value**
    (today's behaviour; legacy compaction keeps a bounded number of alternatives, so playing more
    than the winner would change the sound at the next checkpoint) (review H2).
  - `layers` are **add wins**. `ScoreLayerSlot` is a newtype over the register with **no**
    `value()` or `removed()` (so removal-wins helpers cannot be applied by copy-paste) (review M2):
    - `live()`: the distinct `Some` values among all alternatives;
    - `cleared()`: no live value;
    - `shown()`: the selected value if it is live, else the live alternative with the lowest
      `source.op_id` (bytewise). This one rule is what the composer shows and edits from, what a
      whole-version restore authors, and what compaction promotes to `selected`.
  - A concurrent clear does not remove a concurrent set. A clear only removes the values its author
    saw.
- `source_ids()` and `empty()` include layer slots (replay detection must see layer op ids)
  (review M4).
- The change validator is unchanged in shape: one entry key plus the marker (plus the 5 headers on
  a pristine epoch 0), at most 7 operations, exact predecessors. `check_cell` gains a `SetNote` arm
  with the same extent and effective-sound checks; `ClearNote` is always admissible.
- `references.rs` adds `set_note`/`clear_note` to its static Score name list.
- Admission (`admission.rs` `local_policy`): a new note slot is refused when active legacy cells
  plus live layer slots already reach `SCORE_MAX_CELLS` (4096).

## 3. What plays: voices

One definition, computed identically in Rust readiness and in the renderer
(`apps/desktop/src/score-voices.ts`), pinned by the shared fixture
`crates/catcoms-replication/tests/fixtures/studio-score-readiness-v1.json`:

1. Candidates per slot:
   - a legacy cell slot that is not removed: its **selected** value only;
   - a layer slot: every distinct live value.
2. Effective value (review H1): a note that starts inside the grid (`step < steps`) has
   `len = min(len or 1, steps - step)`; `len` is written only when greater than 1 (an absent `len`
   and `len: 1` are the same note, as the exporter reads them); `vol` stays as authored (absent is
   not 100 to the take format). A note starting at or after the end keeps `len or 1`. A drum is
   itself.
3. Voices: `(step, row, effective value)`. Identical voices collapse to one, across all slots (a
   legacy cell and a layer note holding the same note, or two lengths clipped to one note, are one
   voice). A voice remembers the slot keys it came from (at most a legacy key and a layer key).
4. Order: `(step, row, canonical JSON bytes of the effective value)`.
5. Skip, never refuse, counting each skipped voice once:
   - `count`: voices after the 4096th in that order;
   - `recipe`: a note whose sound is not active or not among the first 64 active sounds by id;
   - a voice starting at or after `steps`.
6. Everything else plays, including overlapping holds.

Readiness flags stay as diagnostics, per slot key (OR over that slot's voices), and nothing gates
on `ready()` any more:

- `count`, `recipe` as above, on the voice's slot keys;
- `extent`: a candidate (raw value) that starts at or after `steps` or whose raw hold runs past it;
- `overlap` (review H3): over every note voice, played or skipped, with its effective end
  (`step + (len or 1)`), grouped by `(row, layer)`. Within a group the voices are in start order;
  a voice overlaps when its start is before the furthest end of the voices before it, or the next
  voice in the group starts before its own end. This is exactly "intersects another voice of the
  group" and is O(n) after the sort. Different sounds overlapping is what layers are for, so they
  are never flagged. The renderer's old pairwise walk took 24 s for 20,480 voices on one row; Rust
  readiness runs on every view (`src-tauri/src/studio.rs` ~772), so neither side may be quadratic.
- `conflict`: a slot with more than one distinct raw value among its non-clear alternatives
  (identical alternatives are not a conflict). Raw, so the flag does not move when `steps` does;
- `headerConflicts`: a header register with more than one distinct value; the Automerge winner is
  the value used, deterministically on every peer;
- `ScorePatchIssues.conflict`: more than one distinct descriptor (never in practice: a sound id is
  the hash of its descriptor), kept for the DTO.

Readiness stays **sparse** (only slots with a flag). The renderer serialises readiness into its
media key; a dense or reordered projection would make every readback look like a source change.

Declared test-contract change (review M5): for layer-free scores the DTO shape is unchanged, but
the legacy flags change meaning: `conflict`, `headerConflicts` and patch `conflict` count distinct
values only; `overlap` only counts holds of the same sound; `count` is over voices. Only a clean
score (one value per slot, no same-sound overlap) has byte-identical readiness. The renderer's
cross-check of `headerConflicts` (`studio-native.ts` `parseScoreContent`) changes in the same build.

## 4. Bridge DTO

`projection_content` keeps today's shape for a score with no layer slots. When layer slots exist
it adds:

```
"layers": { "S:R/L": { "cleared": bool, "register": { "selected": {...}, "conflicts": [...] } } }
"readiness": { ..., "layers": { "S:R/L": { count, extent, recipe, overlap, conflict } } }
```

`L` is `d` or the 64-hex sound id. Register values are `ScoreCell | null` as for cells, with the
same `{ opId, author, nonce }` sources and the same shared evidence bound. The renderer checks
`cleared` against the evidence (no live value) the way it checks `removed` for cells, that every
value is in the key's layer, that the collection is non-empty when present and at most 68,192
slots, and `readiness.headerConflicts` against "more than one distinct header value".

## 5. Snapshot, seed and compaction

- `score/snapshot.rs` gains an optional trailing section after the sounds: a section tag byte
  `0x01` (room for later trailers), a count (1..MAX_VALUES), then entries in strictly ascending
  `(step, row, layer)`: step u64, row u8, layer tag (0 = drums; 1 followed by the 32 raw bytes of
  the sound id), then the register written with `put_slot_value(put_cell)` (recovery snapshots keep
  clears). Absent when there are no layer slots, so a score without layers encodes byte-
  identically; an empty section is refused by the existing re-encode check. Decode enforces
  `value.layer() == slot layer` and rebuilds `SetNote`/`ClearNote` for the per-op-id digest check.
  `payload_channel` stays 2. A golden byte vector pins a seed with the trailer.
- Compaction (review H2, M1):
  - Headers and sounds: unchanged.
  - Legacy cells: unchanged per slot (removed slots dropped, the shared conflict budget applies
    to their alternatives), so a layer-free score compacts byte-identically.
  - Layer slots are **outside** the shared conflict budget, because their alternatives are voices
    that play: drop cleared slots; in a live slot drop the clear alternatives and collapse
    identical values to the copy with the lowest `source.op_id`; `selected` becomes `shown()`; the
    other distinct values follow in ascending `source.op_id`.
  - Cap, in **voice order** (section 3), not collection order: retain exactly the slots and layer
    values that back the first 4096 voices of the pre-compaction projection (a voice backed by a
    legacy and a layer key keeps both). For a layer-free score this is today's `cells.take(4096)`,
    because its voices are its active cells in `(step, row)` order. Worst case about 450 KB,
    inside the 2 MiB seed limit.
  - Dropped clear alternatives set `checkpoint_omits_evidence()`, engaging the recovery-first path.
    Dropping them changes nothing that plays.
  - Implementation note: where several raw values of one slot clip to the same voice (two lengths
    cut to one note at the end), compaction keeps ONE copy, the shown value first, then the lowest
    op id. That holds the bound of one retained layer value per voice; keeping every clipped length
    would let one member push a seed past the 2 MiB limit. The cost: if the score is lengthened
    after that checkpoint, the dropped longer length is gone (recovery still holds it). The output
    is a fixed point, which `decode_baseline` requires (`compact(p) == p`).

## 6. Recovery and copy

- `StudioRecoveryItem::ScoreNote { step, row, layer, value }` and a matching bridge `ChoiceInput`
  (`layer` is `d` or 64 hex; coordinate bounds). Restore authors `SetNote` of the version's
  `shown()` value (one author holds one value per slot, so a version's other concurrent values are
  not recreated), or `ClearNote` for a recovered clear; Deleted when a retained version shows the
  slot cleared; Full counts legacy and layer slots together. Legacy `ScoreCell` recovery items keep
  authoring `SetCell`.
- Renderer: `RecoveryChoice` gains `{ kind: "scoreNote"; step; row; layer; value: Hex64 }`;
  `choicesForContent` enumerates layer slots after legacy cells, choosing `shown()` (or, in copy
  mode for a cleared slot, the clear with the lowest op id); `studio-draft.ts` accepts the choice and
  validates its layer (review M4).

## 7. Renderer

- `studio-contract.ts`: the two operations, `ScoreLayer` (`"d" | Hex64`), `layerOf(cell)`,
  `noteKey(step, row, layer)` = `"S:R/L"`, `parseNoteKey`.
- `studio-native.ts`: parse optional `layers`/`readiness.layers`; the model gains `layerSlots` and
  `notes: Record<string, ScoreCell>` keyed `"S:R"` (legacy selected value) or `"S:R/L"`
  (`layerShownValue`, the `shown()` rule), and `voices` from `score-voices.ts`.
- `studio-working.ts`: `workingScore` applies `set_note`/`clear_note` and takes readiness from
  `score-voices.ts`; `workingSlot` keys a `set_note` as `note:S:R/L`, never `cell:S:R`.
- `studio-export.ts`: input `notes: {step,row,value}[]` (the voices). Overlapping same-pitch notes
  each take the lowest lane where their pitch is free; lane 0 keeps today's nonce, lane `k >= 1`
  uses `framedHash("catcoms-score-voice:v1", rawId(id), u32be(k))` truncated as lane 0's is.
  Events sort by `(ms, lane, order, row, voice index)` and `q` is assigned per take as today, so a
  score that is clean today exports byte-identically and the vectors in `studio-export-v1.json`
  hold. More same-pitch holds at once than `TAKE_MAX_LANES` are skipped and counted.
  `ScorePlayback.skipped` counts the voices section 3 skipped plus those (`withVoiceSkips`).
- `studio-session.ts`: the editor media key includes `voices.play`; `AUTO_REAUTHOR_OPS` includes
  `set_note` (review H4); `clear_note` and `clear_cell` stay manual.
- Playback engine: the Studio's own playback of a document (an archival playback owner, not a
  jukebox deck, a received take or a live jam) may hold up to `JAM_VOICES_GLOBAL -
  JAM_HELD_PER_PEER` (48) notes at once. The 16-per-performer cap is a fairness rule between live
  jam peers; raising it for every archival take would let one shared take starve a call's live
  performers, because held voices are never stolen. 48 leaves 16 held voices for all live
  performers together while a document plays locally (not 16 each; review L2). The
  global 64-voice ceiling and tail stealing are unchanged. A recipient playing the same take as a
  jukebox deck keeps the 16 cap and may drop holds the author heard; frame sounds share the part-0
  owner and can be refused while a score holds 48. Notes beyond the cap are refused by the
  allocator and counted, not hidden: `StudioPlaybackController.refused` counts each attack refused
  `source-held` or `room-held` once per start (releases, `duplicate` and `sequence` are not lost
  notes), only for the playback still current. `StudioAudioState.refused` is the larger of the pass
  under way and the pass before it, so a loop neither sums its passes nor reads 0 after each
  restart, and the composer shows "N not played" while the score plays (review of `e7250d48`, L1).
- Nothing refuses Play, download, share or post on readiness any more; issues are listed to review.

## 8. Composer: layers

- Each sound in the rack (and the drum kit) has a three-state symbol button: show, ghost, hide.
  Symbols only; an invisible `aria-label` names the state for screen readers.
- Show: notes drawn and interactive. Ghost: outlines only, `pointer-events: none`, and other sounds
  can be placed in those cells. Hide: not drawn.
- A sound takes placements only while it is shown; the place tool is disabled for a ghosted or
  hidden chosen sound.
- A click on a cell acts on the chosen sound's note there (or places it); other layers' notes in the
  same cell are untouched. A click on a cell holding only another shown sound's note selects that
  note (its sound becomes the chosen one); selection, move and delete never reach a ghosted or
  hidden layer.
- Legacy notes are drawn in their sound's layer (their `layerOf`), so they ghost and hide with it.
  Notes of a sound that is gone are drawn in a "missing sound" layer that is always shown.
- A slot with several live values (concurrent edits) shows its `shown()` value with a mark that
  says more than one version plays (the `conflict` flag); the same for a legacy conflict.
- Such a layered slot (more than one DIFFERENT live value) is never moved, resized or rewritten
  from the grid: a write or clear of its key replaces every value the member has seen, so the
  versions they cannot see would go silently. The composer refuses the gesture and points to the
  review list, where choosing a version rewrites the slot to that one value. The member's own erase
  on the grid (a gesture of clears alone) is allowed and removes the note in every version. An undo
  or redo is held to the rule like any edit (its clears would remove versions added since), and the
  review list exempts only the one key it acts on. "Several values" is readiness `conflict`, which
  counts a queued choice as the only value. Identical values are one value and edit normally.
  Legacy cells are unaffected: their losing values never play (review of `e7250d48`, design
  question 1, and its re-review M-A, M-B, L-A).
- Visibility is per viewer and per document, held in the composer, never written to the document.
  Hide does not mute.
- Editing a legacy note (move, resize, volume, sound) authors `set_note` first, then `clear_cell`
  (review M3). The composer checks the new note locally before authoring either: its `len` fits
  `steps - step`, its sound is admitted, and the 4096 slot cap has room. A hold that runs past the
  end (the score was shortened under it) plays clipped, but no edit shortens it on its own: a move,
  volume or sound change of it is refused until the member shortens it, and one Shorten press brings
  it to the end, or lengthens the score. The authored length so survives a score that is shortened
  and grown back (review of `e7250d48`, design question 2; this applies to layered notes too).
  Native admits no `set_cell`/`set_note` whose hold runs past the current length (`change.rs`,
  `step + len <= effective_steps`), so an undo or redo that would write such a hold (undoing that
  Shorten, or an erase of an overrun) is refused before anything is queued, with "Lengthen the
  score, then undo again", and keeps its history entry; once the score is long enough it restores
  the original length. Shorten says so when it trims (re-review of `73b68ada`, MEDIUM 2: an
  earlier exception queued writes native refused). A failure of
  the second op leaves both notes, which is additive and visible, never a lost note. `set_note` is
  re-saved on its own after an epoch move; `clear_cell` keeps its card until the member decides.
  Undo restores exactly where a value lived: a legacy cell the composer cleared comes back as that
  legacy cell (`set_cell`). Restoring it into its layer instead would collide with the layered note
  the same gesture wrote there, and undoing an in-place edit of a legacy note would clear both.
- The grid edits one shown note per cell (the chosen sound's first, then note-key order) and
  `scoreSurfaceOps` translates each gesture: other ops first (a grown length lands before its
  notes), then every write, then every clear, keeping only the last action per note key. Writes
  are ordered so each note's new place lands before its old key is overwritten (phrase moves emit
  sets against the direction of travel; `orderWrites` handles the rest), so every prefix of a
  gesture keeps every note. The session links a gesture's records (`scoreGestureId`): discarding a
  refused write removes the gesture's later records; after a restore, which drops the links, the
  restored note edits behind it are held for the member instead (adversarial review M1/M2 and the
  re-review's H1/M1/M2).
- A legacy cell behind the shown layered note of the same sound is cleared with it only when
  playback hears the two as one voice (section 3's identity: an absent `len` is 1, `vol` exactly as
  authored, so `vol: 100` and no `vol` are two voices), compared on the raw length (a longer note
  clipped by a shortened score is still its own note). Anything else is a note in its own right and
  stays. A gesture the host refuses midway is put back newest first and the put-back stops at its
  first refusal, since an older inverse can depend on a newer one; the prefix left keeps every note.
  A drag binds each carried note's key and value at pointer-down and moves nothing if a
  collaborator changed one meanwhile. Every write, undo and review action included, reaches only
  shown sounds; a legacy `set_cell` also reaches the sound the cell holds now, which it replaces
  (review of `e7250d48`, H1, H2, M1, M2, and the re-review of `73b68ada`, HIGH 1 and MEDIUM 1).
- An erase of a slot with several versions, or choosing one of them, replaces them all; its undo
  can write back only the one value that was shown (sequential edits cannot recreate concurrent
  versions), and the composer says so when it happens.

## 9. Tests (minimum)

- Shared readiness/voice fixture run by Rust and TS: identical values, distinct values (legacy:
  winner only; layers: all), overlap same sound vs different sounds, nested holds, add-vs-clear on
  layers, removal-wins on legacy, clipping (including two lengths clipping to one voice), absent vs
  1 length, skipping, the 4096 cap order.
- Score op vectors: `studio-score-ops-v1.json` gains `set_note`/`clear_note` bodies (the 18-case
  count and op-name assertions are a declared contract change). The 73-byte key accepted and a
  74-byte key refused need a hand-built Automerge read/validator test; the vector file holds
  bodies, not keys.
- Rust: codec, change validator (both Automerge orders of add vs clear), snapshot round trip with
  and without the trailer plus a golden trailer vector, byte-identical legacy seed, compaction
  promotion, collapse, the budget exemption, voice-order cap, combined admission cap, recovery
  ScoreNote, bridge DTO, a readiness timing test with 20,480 voices on one row and sound.
- TS: parse/reject cases, working projection, exporter lanes and byte identity, what plays always
  exports, composer layer states and placement rules, re-save of `set_note`.
- Threat model: add wins lets a malicious member keep a note alive by re-adding it with stale
  dependencies, where before they could force a delete. Not new authority; recorded.
