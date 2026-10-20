// Volatile editor projections. These are never native views, causal evidence, recovery choices,
// or authorization. Keep the native model's raw registers intact; only effective display values
// change until an acknowledged Save replaces the native base.
import { scoreReadinessClear, scoreVoiceSlots, type FlipnoteModel, type ScoreModel, type IndexModel, type FrameSource, type IndexSource } from "./studio-native.ts";
import type { SaveRecord } from "./studio-session.ts";
import type { JamPatch } from "./jam-contract.ts";
import { FLIPNOTE_MAX_FRAMES, FLIPNOTE_FRAME_BYTES_PROMISE, FLIPNOTE_MAX_SFX, FLIPNOTE_MAX_PATCHES, FLIPNOTE_MAX_EXPORTS_VISIBLE, SCORE_MAX_PATCHES, canonicalJson, layerOf, noteKey, parseNoteKey, type ScoreCell } from "./studio-contract.ts";
import { admittedSounds, scoreVoices } from "./score-voices.ts";

export function workingFlipnote(base: FlipnoteModel, records: readonly SaveRecord[], me: string): FlipnoteModel {
  const model: FlipnoteModel = { ...base, frames: base.frames.map(f => ({ ...f })), sound: { ...base.sound,
    sfx: [...base.sound.sfx], patches: { ...base.sound.patches }, exports: [...base.sound.exports] } };
  // Empty operation identity means local display only. Never hand this to native/recovery APIs.
  const local: FrameSource = { author: me, opId: "", nonce: "", ts: 0 };
  const deletedSfx = new Set(base.sound.deletedSfx), deletedExports = new Set(base.sound.deletedExports);
  for (const save of records) {
    if (save.kind === "frame") {
      const prior = model.frames.findIndex(f => f.id === save.frame);
      const frame = { ...(prior >= 0 ? model.frames[prior] : { id: save.frame, author: me, ts: 0, opId: "", conflicts: [], overCap: null, insertions: 1 }),
        cid: `draft:${save.object}:${save.frame}`, bytes: save.pix.length };
      if (prior >= 0) model.frames[prior] = frame;
      else if (save.op === "insert") {
        const after = save.after === null ? -1 : model.frames.findIndex(f => f.id === save.after);
        model.frames.splice(after + 1, 0, frame);
      }
      continue;
    }
    if (save.kind !== "apply" && save.kind !== "overlayApply") continue;
    const op = save.op;
    switch (op.op) {
      case "remove_frame": model.frames = model.frames.filter(f => f.id !== op.frame); model.deletedFrames = [...model.deletedFrames, op.frame]; break;
      case "set_header":
        if (op.field === "title") model.title = String(op.value);
        else if (op.field === "fps") model.fps = Number(op.value);
        else if (op.field === "score") model.sound.score = op.value as string | null;
        break;
      case "set_patch": model.sound.patches[op.patch] = op.descriptor as JamPatch; break;
      case "remove_patch": delete model.sound.patches[op.patch]; break;
      case "set_sfx":
        model.sound.sfx = model.sound.sfx.filter(s => s.id !== op.sfx);
        if (!deletedSfx.has(op.sfx)) model.sound.sfx.push({ id: op.sfx, frame: op.frame, patch: op.patch, note: op.note, durationMs: op.duration_ms, source: local, conflicts: [] });
        break;
      case "remove_sfx": deletedSfx.add(op.sfx); model.sound.sfx = model.sound.sfx.filter(s => s.id !== op.sfx); break;
      case "set_export":
        if (!model.sound.exports.some(e => e.id === op.export) && !deletedExports.has(op.export)) model.sound.exports.push({ id: op.export, cid: op.cid, bytes: op.bytes, expiry: op.expiry, source: local, births: [] });
        break;
      case "remove_export": deletedExports.add(op.export); model.sound.exports = model.sound.exports.filter(e => e.id !== op.export); break;
    }
  }
  model.declaredFrameBytes = model.frames.reduce((sum, f) => sum + f.bytes, 0);
  model.overCapCount = Math.max(base.overCapCount, model.frames.length > FLIPNOTE_MAX_FRAMES || model.declaredFrameBytes > FLIPNOTE_FRAME_BYTES_PROMISE ? 1 : 0);
  const frameIds = new Set(model.frames.map(f => f.id));
  model.sound.deletedSfx = [...deletedSfx]; model.sound.deletedExports = [...deletedExports];
  model.sound.readiness = { conflicts: base.sound.readiness.conflicts,
    sfxOverflow: model.sound.sfx.map(s => s.id).sort().slice(FLIPNOTE_MAX_SFX), patchOverflow: Object.keys(model.sound.patches).sort().slice(FLIPNOTE_MAX_PATCHES),
    exportOverflow: model.sound.exports.map(e => e.id).sort().slice(FLIPNOTE_MAX_EXPORTS_VISIBLE),
    missingFrames: model.sound.sfx.filter(s => !frameIds.has(s.frame)).map(s => s.id),
    missingPatches: model.sound.sfx.filter(s => !model.sound.patches[s.patch]).map(s => s.id) };
  model.sound.ready = !model.sound.readiness.conflicts && Object.entries(model.sound.readiness).every(([k, v]) => k === "conflicts" || !(v as string[]).length);
  return model;
}

export function workingScore(base: ScoreModel, records: readonly SaveRecord[]): ScoreModel {
  const model = { ...base, cells: { ...base.cells }, notes: { ...base.notes }, patches: { ...base.patches } };
  // A queued edit is causally after everything this view holds, so it replaces a slot's every
  // alternative when it lands: one value for a set, none for a clear. `pending` records that per
  // note key; slots it does not name keep their native evidence (including concurrent values).
  const pending = new Map<string, ScoreCell | null>();
  const headersSet = new Set<string>();
  for (const save of records) {
    if (save.kind !== "apply" && save.kind !== "overlayApply") continue;
    const op = save.op;
    switch (op.op) {
      case "set_header":
        if (op.field === "title") model.title = String(op.value);
        else if (op.field === "bpm" || op.field === "bpb" || op.field === "steps") model[op.field] = Number(op.value);
        headersSet.add(op.field);
        break;
      case "set_cell": { const key = noteKey(op.step, op.row, null); model.cells[key] = model.notes[key] = op.value; pending.set(key, op.value); break; }
      case "clear_cell": { const key = noteKey(op.step, op.row, null); delete model.cells[key]; delete model.notes[key]; pending.set(key, null); break; }
      case "set_note": { const key = noteKey(op.step, op.row, layerOf(op.value)); model.notes[key] = op.value; pending.set(key, op.value); break; }
      case "clear_note": { const key = noteKey(op.step, op.row, op.layer); delete model.notes[key]; pending.set(key, null); break; }
      case "set_patch": model.patches[op.patch] = op.descriptor as JamPatch; break;
      case "remove_patch": delete model.patches[op.patch]; break;
    }
  }
  // Readiness and voices come from score-voices.ts, the one implementation Rust readiness mirrors
  // (studio-score-readiness-v1.json pins both). Native readiness is sparse, and so is this: a
  // readback of the very score projected here must fingerprint identically in the editor's media
  // snapshot, or it is mistaken for a source change and stops playback or an audition.
  const slots = scoreVoiceSlots(base.cellSlots, base.layerSlots).filter((slot) => !pending.has(slot.key));
  for (const [key, value] of pending) {
    const parsed = parseNoteKey(key);
    if (parsed && value) slots.push({ key, step: parsed.step, row: parsed.row, layer: parsed.layer, values: [value] });
  }
  model.voices = scoreVoices(slots, model.steps, admittedSounds(Object.keys(model.patches)));
  const cells: ScoreModel["readiness"]["cells"] = {}, layers: ScoreModel["readiness"]["cells"] = {};
  for (const [key, issues] of Object.entries(model.voices.issues)) (key.includes("/") ? layers : cells)[key] = issues;
  // A sound's issues: over the 64-sound cap by id, or (never in practice: an id is its descriptor's
  // hash) more than one distinct descriptor retained under one id.
  const densePatches: ScoreModel["readiness"]["patches"] = {};
  Object.keys(model.patches).sort().forEach((id, i) => {
    const slot = base.patchSlots[id], distinct = slot ? new Set([slot.register.selected, ...slot.register.conflicts].filter((a) => a.value !== null).map((a) => canonicalJson(a.value))).size : 0;
    densePatches[id] = { count: i >= SCORE_MAX_PATCHES, conflict: distinct > 1 };
  });
  const patches = Object.fromEntries(Object.entries(densePatches).filter(([, v]) => Object.values(v).some(Boolean)));
  const layered = Object.keys(base.layerSlots).length > 0 || [...pending.keys()].some((key) => key.includes("/"));
  // A queued set of a header is causally after every value its register holds, so once it lands
  // the header has one value. The projection shows it so: its register becomes that one value (the
  // prior winner's source, or a local display-only source for a header never set), and a header
  // conflict (more than one DISTINCT value, the rule native and the DTO parser use) is computed on
  // that. Projecting the base's flag instead made the readback look like a source change and
  // stopped playback, and kept "another value by" chips up while the edit saved (review L7).
  if (headersSet.size && base.headers) {
    const local: IndexSource = { opId: "", author: "", nonce: "" };
    const headers = { ...base.headers } as Record<string, ScoreModel["headers"]["title"] | ScoreModel["headers"]["bpm"]>;
    for (const field of headersSet) {
      const value = field === "title" ? model.title : model[field as "bpm" | "bpb" | "steps"];
      headers[field] = { selected: { value, source: base.headers[field as "title"]?.selected.source ?? local }, conflicts: [] } as never;
    }
    model.headers = headers as ScoreModel["headers"];
  }
  const distinct = (header: ScoreModel["headers"]["title"] | ScoreModel["headers"]["bpm"]) =>
    !!header && new Set([header.selected, ...header.conflicts].map((alternative) => JSON.stringify(alternative.value))).size > 1;
  const headerConflicts = (["title", "bpm", "bpb", "steps"] as const).some((field) => distinct(model.headers?.[field] ?? null));
  model.readiness = { cells, patches, headerConflicts, ...(layered ? { layers } : {}) };
  model.ready = scoreReadinessClear(model.readiness);
  return model;
}

export function workingIndex(base: IndexModel, records: readonly SaveRecord[]): IndexModel {
  const model = { entries: base.entries.map(e => ({ ...e })), overflow: base.overflow.map(e => ({ ...e })), deleted: [...base.deleted] };
  for (const save of records) {
    if (save.kind !== "applyIndex" && !(save.kind === "overlayApply" && save.object === null)) continue;
    const op = save.op;
    if (!("object" in op)) continue;
    const entry = [...model.entries, ...model.overflow].find(e => e.id === op.object);
    if (!entry) continue;
    if (op.op === "set_title") entry.title = op.title;
    else if (op.op === "set_expiry") entry.expiry = op.expiry === null ? { kind: "never" } : op.expiry === undefined ? { kind: "unrecorded" } : { kind: "at", ms: op.expiry };
    else if (op.op === "tombstone_object") {
      model.entries = model.entries.filter(e => e.id !== op.object); model.overflow = model.overflow.filter(e => e.id !== op.object);
    }
  }
  return model;
}

/** Only a never-submitted tail may be replaced. Do not reorder across dependencies/removals. */
export function workingSlot(save: SaveRecord): string | null {
  if (save.kind !== "apply" && save.kind !== "applyIndex" && save.kind !== "overlayApply") return null;
  const op = save.op;
  switch (op.op) {
    case "set_header": return `header:${op.field}`;
    case "set_cell": return `cell:${op.step}:${op.row}`;
    case "set_note": return `note:${noteKey(op.step, op.row, layerOf(op.value))}`;
    case "set_sfx": return `sfx:${op.sfx}`;
    case "set_patch": return `patch:${op.patch}`;
    case "set_title": return `title:${op.object}`;
    case "set_expiry": return `expiry:${op.object}`;
    default: return null;
  }
}
