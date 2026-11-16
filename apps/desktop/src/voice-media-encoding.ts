/** Compact local encoding state, never native or media authority. Keep one canonical
 * original request rather than eleven textual fields after media adoption. Its private
 * template never crosses IPC: every caller receives a fresh body which it may clear. */
import { encodeVoiceMediaOriginal, type VoiceMediaExistingKind, type VoiceMediaOriginalKind } from "./voice-ipc-media.ts";
import type { VoiceSdpOriginal } from "./voice-ipc-sdp-actions.ts";

type Encoded = ReturnType<typeof encodeVoiceMediaOriginal>;
const unavailable = (): never => { throw new Error("original media encoding unavailable"); };
const HEX = "0123456789abcdef";

export class VoiceMediaEncoding {
  #template: Uint8Array<ArrayBuffer> | null;
  #logical: string | null;
  constructor(original: VoiceSdpOriginal) {
    const encoded = encodeVoiceMediaOriginal(original, "resolve");
    try {
      // Numeric nibble selection materializes only characters from our fixed table.
      // Never keep the caller's short row substring and its potentially large backing.
      const characters: string[] = [];
      for (let at = 0; at < 32; at++) {
        const unit = encoded.logical.charCodeAt(at);
        characters.push(HEX[unit <= 57 ? unit - 48 : unit - 87]);
      }
      this.#logical = characters.join("");
      this.#template = encoded.body;
    } catch (error) { encoded.body.fill(0); throw error; }
  } // No constructor-created closure may retain original after this frame returns.

  original(kind: VoiceMediaOriginalKind): Encoded {
    const template = this.#template, logical = this.#logical;
    if (!template || logical === null) return unavailable();
    const mode = kind === "admit" ? 0 : kind === "resolve" ? 1 : kind === "close" ? 2 : kind === "release" ? 3 : -1;
    if (mode < 0) return unavailable();
    const body = template.slice(); body[1] = mode;
    return Object.freeze({ operation: 28, kind: 0, logical, body });
  }
  existing(proof: string, kind: VoiceMediaExistingKind): Encoded {
    const template = this.#template, logical = this.#logical;
    if (!template || logical === null || typeof proof !== "string" || !/^[0-9a-f]{64}$/.test(proof)) return unavailable();
    const mode = kind === "current" ? 4 : kind === "close" ? 5 : kind === "release" ? 6 : kind === "detach" ? 7 : kind === "retirehistory" ? 8 : -1;
    if (mode < 0) return unavailable();
    const body = new Uint8Array(82);
    body[0] = 9; body[1] = mode;
    body.set(template.subarray(2, 50), 2); // SAME canonical action/version/input tag.
    for (let at = 0; at < 32; at++) body[50 + at] = Number.parseInt(proof.slice(at * 2, at * 2 + 2), 16);
    return Object.freeze({ operation: 28, kind: 0, logical, body });
  }
  /** Negative-only final cleanup. An outstanding request owns independent bytes. */
  dispose(): void {
    this.#template?.fill(0); this.#template = null; this.#logical = null;
  }
}

/** Logical compact record:226 payload +80 typed-array/buffer wrappers +96 owned row
 * +48 instance +128 destruction frame =578, rounded to640. This is only this record,
 * not the whole driver's RPC/timer/audio inventory or an extra reservation. */
export const VOICE_MEDIA_ENCODING_BYTES = 640;
