// The acoustic pairing channel (multi-device M6): a small 16-FSK modem for carrying a
// pairing blob between two devices in the same room as sound — the audio counterpart of
// showing a QR. Pure DSP over Float32Array in/out, so the codec is unit-testable in Node
// with no WebAudio; the thin mic/speaker glue lives in the component that uses it.
//
// Design: 16 tones (one per nibble) between F0 and F0 + 15·STEP, SYMBOL_MS per symbol,
// framed as [preamble × PREAMBLE_REPS] [len: 2 bytes] [payload] [crc16: 2 bytes], every
// byte high-nibble-first. Decoding scans for the preamble by Goertzel energy, then reads
// symbol windows at the discovered offset. CRC16-CCITT rejects anything mangled — the
// decoder returns null rather than a wrong payload, matching the "degrade to nothing,
// never to garbage" posture used everywhere else.
//
// The payload is capped small (a pairing request, not a grant bundle): at ~89 bps net,
// MAX_AUDIO_PAYLOAD bytes ≈ 23 s of sound. Callers should keep audio for the request leg
// and use paste/QR for bundles.

/** Lowest tone, Hz. Chosen above typical HVAC rumble and below laptop-speaker rolloff. */
export const F0 = 1_600;
/** Tone spacing, Hz. With 45 ms symbols the Goertzel bins are ~22 Hz wide — 150 Hz
 * spacing keeps neighbours well separated even with cheap-speaker harmonics. */
export const STEP = 150;
/** Symbol duration in milliseconds. */
export const SYMBOL_MS = 45;
/** The sync nibble sequence opening every transmission (never a valid length prefix). */
export const PREAMBLE: number[] = [0xa, 0x5, 0xa, 0x5, 0xf, 0x0];
/** How many times the preamble repeats before the frame (helps the scanner lock on). */
export const PREAMBLE_REPS = 2;
/** Hard cap on the framed payload, in bytes. */
export const MAX_AUDIO_PAYLOAD = 256;

const NIBBLE_FREQS: number[] = Array.from({ length: 16 }, (_, i) => F0 + i * STEP);

/** CRC16-CCITT (0x1021, init 0xFFFF) — tiny, and plenty for a 256-byte frame. */
export function crc16(bytes: Uint8Array): number {
  let crc = 0xffff;
  for (const b of bytes) {
    crc ^= b << 8;
    for (let i = 0; i < 8; i++) {
      crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
    }
  }
  return crc;
}

function frameNibbles(payload: Uint8Array): number[] {
  const frame = new Uint8Array(2 + payload.length + 2);
  frame[0] = payload.length >> 8;
  frame[1] = payload.length & 0xff;
  frame.set(payload, 2);
  const crc = crc16(payload);
  frame[frame.length - 2] = crc >> 8;
  frame[frame.length - 1] = crc & 0xff;
  const nibbles: number[] = [];
  for (let r = 0; r < PREAMBLE_REPS; r++) nibbles.push(...PREAMBLE);
  for (const b of frame) nibbles.push(b >> 4, b & 0xf);
  return nibbles;
}

/**
 * Render `payload` as an FSK waveform at `sampleRate`. Each symbol is a pure tone with a
 * 3 ms raised-cosine ramp at both ends (kills the clicks that would smear energy across
 * bins), preceded and followed by a short silence so a recorder never clips the edges.
 */
export function encodeAudio(payload: Uint8Array, sampleRate: number): Float32Array {
  if (payload.length === 0 || payload.length > MAX_AUDIO_PAYLOAD) {
    throw new Error(`audio payload must be 1..=${MAX_AUDIO_PAYLOAD} bytes`);
  }
  const nibbles = frameNibbles(payload);
  const spb = Math.round((SYMBOL_MS / 1000) * sampleRate); // samples per symbol
  const pad = Math.round(0.15 * sampleRate);
  const out = new Float32Array(pad * 2 + nibbles.length * spb);
  const ramp = Math.round(0.003 * sampleRate);
  let at = pad;
  for (const n of nibbles) {
    const f = NIBBLE_FREQS[n];
    const w = (2 * Math.PI * f) / sampleRate;
    for (let i = 0; i < spb; i++) {
      let a = 0.8;
      if (i < ramp) a *= 0.5 - 0.5 * Math.cos((Math.PI * i) / ramp);
      else if (i >= spb - ramp) a *= 0.5 - 0.5 * Math.cos((Math.PI * (spb - 1 - i)) / ramp);
      out[at + i] = a * Math.sin(w * i);
    }
    at += spb;
  }
  return out;
}

/** Goertzel power of `freq` over `samples[from..from+len)`. */
function goertzel(samples: Float32Array, from: number, len: number, freq: number, sampleRate: number): number {
  const k = Math.round((len * freq) / sampleRate);
  const w = (2 * Math.PI * k) / len;
  const coeff = 2 * Math.cos(w);
  let s0 = 0;
  let s1 = 0;
  let s2 = 0;
  for (let i = 0; i < len; i++) {
    s0 = samples[from + i] + coeff * s1 - s2;
    s2 = s1;
    s1 = s0;
  }
  return s1 * s1 + s2 * s2 - coeff * s1 * s2;
}

function bestNibble(samples: Float32Array, from: number, len: number, sampleRate: number): { n: number; snr: number } {
  let best = 0;
  let bestP = -1;
  let total = 0;
  for (let n = 0; n < 16; n++) {
    const p = goertzel(samples, from, len, NIBBLE_FREQS[n], sampleRate);
    total += p;
    if (p > bestP) {
      bestP = p;
      best = n;
    }
  }
  const rest = (total - bestP) / 15;
  return { n: best, snr: rest > 0 ? bestP / rest : Number.POSITIVE_INFINITY };
}

/**
 * Recover a payload from a recording, or `null` if none is confidently present.
 * Scans for the preamble in quarter-symbol steps, fine-tunes the offset on the first
 * preamble symbol, then reads the frame and checks length + CRC.
 */
export function decodeAudio(samples: Float32Array, sampleRate: number): Uint8Array | null {
  const spb = Math.round((SYMBOL_MS / 1000) * sampleRate);
  if (samples.length < spb * (PREAMBLE.length * PREAMBLE_REPS + 8)) return null;
  const hop = Math.max(1, spb >> 2);
  const limit = samples.length - spb * PREAMBLE.length;
  for (let start = 0; start <= limit; start += hop) {
    // Cheap gate: does the first window even look like the first preamble tone?
    const g = bestNibble(samples, start, spb, sampleRate);
    if (g.n !== PREAMBLE[0] || g.snr < 4) continue;
    // Confirm the whole preamble here before committing to a frame read.
    if (!matchesPreamble(samples, start, spb, sampleRate)) continue;
    const frameAt = start + spb * PREAMBLE.length * PREAMBLE_REPS;
    const payload = readFrame(samples, frameAt, spb, sampleRate);
    if (payload) return payload;
  }
  return null;
}

function matchesPreamble(samples: Float32Array, start: number, spb: number, sampleRate: number): boolean {
  for (let r = 0; r < PREAMBLE_REPS; r++) {
    for (let i = 0; i < PREAMBLE.length; i++) {
      const at = start + (r * PREAMBLE.length + i) * spb;
      if (at + spb > samples.length) return false;
      if (bestNibble(samples, at, spb, sampleRate).n !== PREAMBLE[i]) return false;
    }
  }
  return true;
}

function readFrame(samples: Float32Array, at: number, spb: number, sampleRate: number): Uint8Array | null {
  const readByte = (idx: number): number | null => {
    const hiAt = at + idx * 2 * spb;
    const loAt = hiAt + spb;
    if (loAt + spb > samples.length) return null;
    const hi = bestNibble(samples, hiAt, spb, sampleRate);
    const lo = bestNibble(samples, loAt, spb, sampleRate);
    if (hi.snr < 2 || lo.snr < 2) return null;
    return (hi.n << 4) | lo.n;
  };
  const l0 = readByte(0);
  const l1 = readByte(1);
  if (l0 === null || l1 === null) return null;
  const len = (l0 << 8) | l1;
  if (len === 0 || len > MAX_AUDIO_PAYLOAD) return null;
  const payload = new Uint8Array(len);
  for (let i = 0; i < len; i++) {
    const b = readByte(2 + i);
    if (b === null) return null;
    payload[i] = b;
  }
  const c0 = readByte(2 + len);
  const c1 = readByte(3 + len);
  if (c0 === null || c1 === null) return null;
  if (((c0 << 8) | c1) !== crc16(payload)) return null;
  return payload;
}
