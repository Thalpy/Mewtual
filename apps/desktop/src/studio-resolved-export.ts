// Conversion from typed native views, not fixture roots. Publication and authority remain separate.
import { exportScoreTake, type ScoreExportInput } from "./studio-export.ts";
import { exportFlipnotePixa } from "./pixa.ts";
import { flipnoteModel, scoreModel, StudioNativeError, type FlipnoteView, type ScoreView } from "./studio-native.ts";

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
