// Effective musical state is a view over the native evidence, never a replacement document.
import type { FrameSoundContent, FrameSoundOp, FrameSource, Register, Valued } from "./studio-native.ts";
import type { JamPatch } from "./jam-contract.ts";

export type SfxView = { id: string; frame: string; patch: string; note: number; durationMs: number; source: FrameSource; conflicts: Valued<FrameSoundOp, FrameSource>[] };
export type SoundExportView = { id: string; cid: string; bytes: number; expiry?: number | null; source: FrameSource; births: Valued<FrameSoundOp, FrameSource>[] };
export type SoundPatchView = { id: string; removed: boolean; descriptor: JamPatch | null; register: Register<FrameSoundOp, FrameSource> };
export type StudioSoundModel = {
  records: FrameSoundContent["records"];
  sfx: SfxView[]; deletedSfx: string[];
  patchSlots: SoundPatchView[]; patches: Record<string, JamPatch>;
  exports: SoundExportView[]; deletedExports: string[];
  score: string | null; scoreRegister: Register<FrameSoundOp, FrameSource> | null;
  readiness: FrameSoundContent["readiness"]; ready: boolean;
};

const noIssues = () => ({ sfxOverflow: [], patchOverflow: [], exportOverflow: [], missingFrames: [], missingPatches: [], conflicts: false });

/** Permanent SFX/export tombstones and reusable remove-wins recipe slots deliberately differ. */
export function studioSoundModel(content: FrameSoundContent | undefined): StudioSoundModel {
  const records = content?.records ?? {}, readiness = content?.readiness ?? noIssues();
  const deletedSfx = new Set<string>(), deletedExports = new Set<string>();
  const liveSfx = new Map<string, SfxView>(), liveExports = new Map<string, SoundExportView>();
  const patchSlots: SoundPatchView[] = [], patches: Record<string, JamPatch> = {};
  let score: string | null = null, scoreRegister: Register<FrameSoundOp, FrameSource> | null = null;
  for (const [key, register] of Object.entries(records)) {
    const selected = register.selected, value = selected.value;
    // Every deletion key is immutable and includes its own source operation id. A mutable setter
    // cannot supersede it. Raw removal evidence remains available after filtering effective lists.
    if (value.op === "remove_sfx") deletedSfx.add(value.sfx);
    else if (value.op === "remove_export") deletedExports.add(value.export);
    else if (value.op === "set_sfx") liveSfx.set(value.sfx, { id: value.sfx, frame: value.frame, patch: value.patch,
      note: value.note, durationMs: value.duration_ms, source: selected.source, conflicts: register.conflicts });
    else if (value.op === "set_export") {
      const existing = liveExports.get(value.export);
      const birth = { id: value.export, cid: value.cid, bytes: value.bytes, ...(value.expiry === undefined ? {} : { expiry: value.expiry }), source: selected.source };
      if (!existing) liveExports.set(value.export, { ...birth, births: [selected, ...register.conflicts] });
      else {
        existing.births.push(selected, ...register.conflicts);
        // Export births are immutable; insertion source id, not map order or AM register winner,
        // deterministically chooses among two independently accepted births of one stable id.
        if (selected.source.opId < existing.source.opId) liveExports.set(value.export, { ...birth, births: existing.births });
      }
    } else if (value.op === "set_patch" || value.op === "remove_patch") {
      const removed = [selected, ...register.conflicts].some((alternative) => alternative.value.op === "remove_patch");
      const descriptor = !removed && value.op === "set_patch" ? value.descriptor as JamPatch : null;
      patchSlots.push({ id: value.patch, removed, descriptor, register });
      if (descriptor) patches[value.patch] = descriptor;
    } else if (key === "h/score" && value.op === "set_header") {
      score = value.value; scoreRegister = register;
    }
  }
  const byId = <T extends { id: string }>(a: T, b: T) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  return { records,
    sfx: [...liveSfx.values()].filter((value) => !deletedSfx.has(value.id)).sort(byId), deletedSfx: [...deletedSfx].sort(),
    exports: [...liveExports.values()].filter((value) => !deletedExports.has(value.id)).sort(byId), deletedExports: [...deletedExports].sort(),
    patchSlots: patchSlots.sort(byId), patches, score, scoreRegister, readiness,
    ready: !readiness.conflicts && !readiness.sfxOverflow.length && !readiness.patchOverflow.length && !readiness.exportOverflow.length &&
      !readiness.missingFrames.length && !readiness.missingPatches.length,
  };
}
