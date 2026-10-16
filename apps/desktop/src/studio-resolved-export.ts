// Conversion from typed native views, not fixture roots. Publication and authority remain separate.
import { exportScorePlayback, exportScoreTake, type ScoreExportInput } from "./studio-export.ts";
import { exportFlipnotePixa } from "./pixa.ts";
import { flipnoteModel, scoreModel, StudioNativeError, type FlipnoteView, type ScoreView } from "./studio-native.ts";
import { FLIPNOTE_FRAME_BYTES_PROMISE, FLIPNOTE_MAX_FRAMES, PIX_MAX_BYTES } from "./studio-contract.ts";

/** Synchronous preflight before any fetch/hash: never let a permissive display model omit data. */
export function preflightFlipnoteMedia(view: FlipnoteView): void {
  const content = view.content, ids = content.timeline;
  const listed = new Set(ids);
  const active = Object.keys(content.frames).filter((id) => !Object.hasOwn(content.tombstones, id));
  if (!ids.length || ids.length > FLIPNOTE_MAX_FRAMES || listed.size !== ids.length || active.length !== ids.length || active.some((id) => !listed.has(id))) throw new StudioNativeError("animation timeline must name every active frame exactly once within its count budget");
  let sum = 0;
  for (const id of ids) {
    const entry = content.frames[id], size = entry?.pixels.selected.value.bytes;
    if (!entry || Object.hasOwn(content.tombstones, id) || !Number.isSafeInteger(size) || size < 1 || size > PIX_MAX_BYTES) throw new StudioNativeError("animation timeline has a missing or invalid exact frame declaration");
    sum += size;
    if (sum > FLIPNOTE_FRAME_BYTES_PROMISE) throw new StudioNativeError("animation frames exceed the bounded package budget");
  }
  if (sum !== content.declaredFrameBytes) throw new StudioNativeError("animation aggregate declaration does not match its exact frames");
}

function scoreInput(object: string, view: ScoreView): ScoreExportInput {
  const model = scoreModel(view.content);
  if (!model.ready || Object.values(model.cellSlots).some((slot) => !slot.removed && slot.register.conflicts.length > 0) ||
    Object.values(model.patchSlots).some((slot) => !slot.removed && slot.register.conflicts.length > 0)) {
    throw new StudioNativeError("resolve score conflicts, missing recipes, overlap or overflow before playback/export");
  }
  return { id: object, bpm: model.bpm, bpb: model.bpb, steps: model.steps, cells: model.cells, patches: model.patches };
}
function group(view: FlipnoteView | ScoreView): string {
  if (!view.group) throw new StudioNativeError("reopen this document to obtain its stable native group before playback/export");
  return view.group;
}

/** A resolved score is an ordinary validated jam take consumed by the same engine/jukebox. */
export async function scoreTakeFromNative(object: string, view: ScoreView, exporter: string) {
  return exportScoreTake(scoreInput(object, view), { group: group(view), exporter });
}

/** Studio playback of the same resolved score, carrying its authored grid end for looping. */
export async function scorePlaybackFromNative(object: string, view: ScoreView, exporter: string) {
  return exportScorePlayback(scoreInput(object, view), { group: group(view), exporter });
}

/**
 * Build only a complete held animation. Callers retain/fence these exact view snapshots across
 * native fetches and hashing. Missing/conflicted/overflowed state is refusal, never a shorter
 * export. Linked scores must come from their explicit Score target in this same group/channel.
 */
export async function flipnotePixaFromNative(
  object: string,
  view: FlipnoteView,
  held: ReadonlyMap<string, Uint8Array>,
  exporter: string,
  linked?: Readonly<{ object: string; view: ScoreView }>,
): Promise<Uint8Array> {
  preflightFlipnoteMedia(view);
  const model = flipnoteModel(view.content), stableGroup = group(view);
  if (!model.frames.length || model.overCapCount || model.titleConflicts.length || model.fpsConflicts.length ||
    model.frames.some((frame) => frame.conflicts.length > 0 || frame.insertions > 1) || !model.sound.ready) {
    throw new StudioNativeError("resolve animation/sound conflicts, missing references or overflow before playback/export");
  }
  let score: ScoreExportInput | undefined;
  if (model.sound.score) {
    if (!linked || linked.object !== model.sound.score || linked.view.channel !== view.channel || group(linked.view) !== stableGroup) {
      throw new StudioNativeError("the linked Score snapshot is missing or belongs to another target/group/channel");
    }
    score = scoreInput(linked.object, linked.view);
  } else if (linked) throw new StudioNativeError("an unrelated Score cannot substitute for an unlinked animation");
  const frames = model.frames.map((frame) => {
    const bytes = held.get(`${frame.cid}:${frame.bytes}`);
    if (!bytes || bytes.length !== frame.bytes) throw new StudioNativeError("all exact animation frame bytes must be held before playback/export");
    return { id: frame.id, bytes };
  });
  return exportFlipnotePixa({ id: object, epoch: BigInt(view.epoch), fps: model.fps, frames, patches: model.sound.patches,
    ...(score ? { score } : {}), sfx: model.sound.sfx.map((sfx) => ({ id: sfx.id, frame: sfx.frame, patch: sfx.patch, note: sfx.note, durationMs: sfx.durationMs })) },
  { group: stableGroup, exporter });
}
