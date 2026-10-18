// One-note audition for the Flipnote Studio's sound tools ("hear it" in the Sound tab, the
// audition keyboard in the soundfont popup).
//
// There is deliberately NO second synth path for this. An audition is a one-event jam-take:v1
// played through the same bounded archival controller every Score and flipnote plays through,
// so it inherits every gate that path has: an explicit gesture, the Studio playback scope,
// Deafen, cancellation on navigation, and the receiver-side release cap. The take is built here,
// validated by the shared take validator, and handed to `StudioAudioActions.start("score", ...)`.
//
// The take is never shared, recorded or published: it exists for one press. Its `call` and lane
// nonce are therefore fixed labels rather than hashes of anything, which keeps the builder
// synchronous and free of the export context's hashing. `group` and `exporter` still come from
// the real document view, because the validator demands them and a future reviewer should not
// find a take whose provenance fields are invented.
import { PATCH_OSC_WAVES, type JamPatch, type JamTake } from "./jam-contract.ts";
import { validateJamPatch } from "./jam-patch.ts";
import { validateJamTake } from "./jam-recorder.ts";
import type { ScorePlayback, StudioExportContext } from "./studio-export.ts";

/** The `call` label of every audition take. Not a hash: nothing is being identified. */
export const AUDITION_CALL = "catcoms-studio-audition:v1";
/** One lane, one fixed non-zero nonce. The take is local and single-lane; nothing keys on it. */
export const AUDITION_LANE_NONCE = "0000000000000001";
/** Authored hold bounds, the same integer-millisecond range a saved marker may carry. */
export const AUDITION_MIN_MS = 1;
export const AUDITION_MAX_MS = 600_000;

/**
 * Build the playback input for one note of `patch` held for `durationMs`.
 *
 * Throws on anything the saved marker contract would also refuse: a non-integer or out-of-range
 * MIDI note, a hold outside 1..600000 ms, a recipe the validator rejects, or a context without
 * the group/exporter the take validator requires. The caller shows the message as a notice.
 */
export function auditionPlayback(ctx: StudioExportContext, patch: JamPatch, note: number, durationMs: number): ScorePlayback {
  if (!Number.isInteger(note) || note < 0 || note > 127) throw new Error("the audition note is outside MIDI 0..127");
  if (!Number.isInteger(durationMs) || durationMs < AUDITION_MIN_MS || durationMs > AUDITION_MAX_MS) throw new Error("the audition hold is outside 1 ms..600 s");
  if (typeof ctx.group !== "string" || !ctx.group) throw new Error("the document has no group yet; reopen it before auditioning");
  if (typeof ctx.exporter !== "string" || !ctx.exporter) throw new Error("no device identity for the audition take");
  const checked = validateJamPatch(patch);
  if (!checked.ok) throw new Error(checked.error);
  // The legacy wave follows the first oscillator exactly as the Score export does, so an old
  // renderer that ignores `p` still plays something recognisable rather than a default.
  const w = PATCH_OSC_WAVES[checked.patch.o[0].w];
  const take: JamTake = {
    v: 1,
    group: ctx.group,
    call: AUDITION_CALL,
    met: { bpm: 120, bpb: 4 },
    parts: [ctx.exporter],
    lanes: [{ src: 0, sn: AUDITION_LANE_NONCE }],
    patches: [checked.patch],
    events: [
      { ms: 0, lane: 0, n: note, on: 1, w, p: 0, q: 0 },
      { ms: durationMs, lane: 0, n: note, on: 0, q: 1 },
    ],
  };
  const valid = validateJamTake(take);
  if (!valid.ok) throw new Error(valid.error);
  // The grid end is the authored hold: the controller adds the recipe's release tail itself.
  return { take: valid.take, gridMs: durationMs };
}
