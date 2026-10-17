// Recovery selectors name authenticated source operations, not renderer-supplied replacement
// bodies. The native planner independently resolves the value and checks destination history.
import { studioSoundModel } from "./studio-sound.ts";
import type { FrameSoundContent, FrameSoundOp, RecoveryChoice, RecoveryMode, StudioContent } from "./studio-native.ts";

/** Effective whole-version choices. Raw alternatives remain retained; a whole walk must never
 * silently apply every alternative of one mutable slot in succession. Frames precede this list,
 * then recipes precede dependent markers. Negative choices and link replacement require Copy. */
export function soundRecoveryChoices(content: FrameSoundContent | undefined, mode: RecoveryMode): { choice: RecoveryChoice; label: string }[] {
  const sound = studioSoundModel(content), out: { choice: RecoveryChoice; label: string }[] = [];
  const add = (value: string, label: string) => out.push({ choice: { kind: "soundRecord", value }, label });
  for (const slot of sound.patchSlots) {
    const values = [slot.register.selected, ...slot.register.conflicts];
    const effective = slot.removed ? values.filter(v => v.value.op === "remove_patch").sort((a,b) => a.source.opId < b.source.opId ? -1 : 1)[0] : slot.register.selected;
    if (effective && (mode === "copy" || (!slot.removed && !slot.register.conflicts.length))) add(effective.source.opId, `${slot.removed ? "remove sound recipe" : "sound recipe"} ${slot.id.slice(0, 8)}`);
  }
  for (const marker of sound.sfx) if (mode === "copy" || !marker.conflicts.length) add(marker.source.opId, `sound marker ${marker.id.slice(0, 8)} (${marker.durationMs}ms)`);
  for (const record of sound.exports) if (mode === "copy" || record.births.length === 1) add(record.source.opId, `PIXA reference ${record.id.slice(0, 8)}`);
  if (mode === "copy") {
    if (sound.scoreRegister) add(sound.scoreRegister.selected.source.opId, "linked Score (including unlink)");
    const seen = new Set<string>();
    for (const [, register] of Object.entries(sound.records).sort(([a],[b]) => a < b ? -1 : 1)) {
      const value = register.selected.value;
      if (value.op !== "remove_sfx" && value.op !== "remove_export") continue;
      const id = value.op === "remove_sfx" ? value.sfx : value.export, key = `${value.op}/${id}`;
      if (!seen.has(key)) { seen.add(key); add(register.selected.source.opId, `${value.op === "remove_sfx" ? "delete sound marker" : "delete PIXA reference"} ${id.slice(0, 8)}`); }
    }
  }
  return out;
}

/** Look up one exact retained value, including an explicitly selected conflict alternative. */
export function historicalSoundOperation(content: StudioContent, choice: RecoveryChoice): FrameSoundOp | null {
  if (choice.kind !== "soundRecord" || content.kind !== "flipnote") return null;
  for (const register of Object.values(content.sound?.records ?? {})) {
    for (const value of [register.selected, ...register.conflicts]) if (value.source.opId === choice.value) return value.value;
  }
  return null;
}
