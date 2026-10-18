// Volatile editor projections. These are never native views, causal evidence, recovery choices,
// or authorization. Keep the native model's raw registers intact; only effective display values
// change until an acknowledged Save replaces the native base.
import type { FlipnoteModel, ScoreModel, IndexModel, FrameSource } from "./studio-native.ts";
import type { SaveRecord } from "./studio-session.ts";
import type { JamPatch } from "./jam-contract.ts";
import { FLIPNOTE_MAX_FRAMES, FLIPNOTE_FRAME_BYTES_PROMISE, FLIPNOTE_MAX_SFX, FLIPNOTE_MAX_PATCHES, FLIPNOTE_MAX_EXPORTS_VISIBLE } from "./studio-contract.ts";

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
  const model = { ...base, cells: { ...base.cells }, patches: { ...base.patches } };
  for (const save of records) {
    if (save.kind !== "apply" && save.kind !== "overlayApply") continue;
    const op = save.op;
    switch (op.op) {
      case "set_header":
        if (op.field === "title") model.title = String(op.value);
        else if (op.field === "bpm" || op.field === "bpb" || op.field === "steps") model[op.field] = Number(op.value);
        break;
      case "set_cell": model.cells[`${op.step}:${op.row}`] = op.value; break;
      case "clear_cell": delete model.cells[`${op.step}:${op.row}`]; break;
      case "set_patch": model.patches[op.patch] = op.descriptor as JamPatch; break;
      case "remove_patch": delete model.patches[op.patch]; break;
    }
  }
  // Raw slots/headers above remain native evidence. Readiness for a local preview is computed
  // from its effective cells/recipes; the export validator independently checks the full input.
  const dense: ScoreModel["readiness"]["cells"] = {}, densePatches: ScoreModel["readiness"]["patches"] = {};
  const notes = new Map<number, { end: number; key: string }>();
  const ordered = Object.entries(model.cells).sort(([a], [b]) => Number(a.split(":")[0]) - Number(b.split(":")[0]) || a.localeCompare(b));
  ordered.forEach(([key, cell], i) => {
    const [step, row] = key.split(":").map(Number), end = step + (cell.k === "n" ? cell.len ?? 1 : 1), prior = notes.get(row);
    const overlap = cell.k === "n" && !!prior && step < prior.end;
    dense[key] = { count: i >= 4096, extent: end > model.steps, recipe: cell.k === "n" && !model.patches[cell.p], overlap,
      conflict: !!base.cellSlots[key]?.register.conflicts.length };
    if (overlap) dense[prior!.key].overlap = true;
    if (cell.k === "n" && (!prior || end > prior.end)) notes.set(row, { end, key });
  });
  Object.keys(model.patches).sort().forEach((id, i) => { densePatches[id] = { count: i >= 64, conflict: !!base.patchSlots[id]?.register.conflicts.length }; });
  // Native readiness is sparse: a slot with nothing wrong has no entry. The working projection
  // takes the same shape, so a readback of the very score it projected fingerprints identically
  // in the editor's media snapshot and is not mistaken for a source change (which would stop
  // playback or an audition that was started from the working projection).
  const cells = Object.fromEntries(Object.entries(dense).filter(([, v]) => Object.values(v).some(Boolean)));
  const patches = Object.fromEntries(Object.entries(densePatches).filter(([, v]) => Object.values(v).some(Boolean)));
  model.readiness = { cells, patches, headerConflicts: base.readiness.headerConflicts };
  model.ready = !model.readiness.headerConflicts && !Object.values(cells).some(v => Object.values(v).some(Boolean)) && !Object.values(patches).some(v => Object.values(v).some(Boolean));
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
    case "set_sfx": return `sfx:${op.sfx}`;
    case "set_patch": return `patch:${op.patch}`;
    case "set_title": return `title:${op.object}`;
    case "set_expiry": return `expiry:${op.object}`;
    default: return null;
  }
}
