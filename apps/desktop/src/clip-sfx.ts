/**
 * The clip maker's built-in sound effects: short cartoon sounds (pow, whoosh, boing...) made from
 * small synthesis recipes and rendered offline into `AudioBuffer`s on first use.
 *
 * Recipes rather than recordings: nothing has to ship, nothing is fetched, and the same recipe
 * always renders the same sound, because the only noise in them comes from a seeded generator
 * (`seededNoise`), never `Math.random`. (The same sound, not the same bits: the browser's audio
 * processing differs by float rounding between renders, around -140 dB, measured in Edge.)
 *
 * A recipe is a few voices. Each voice is an oscillator or noise, an optional filter, and
 * breakpoint lines for pitch, loudness and filter cutoff: `[seconds, value]` pairs, straight lines
 * between them (exponential for frequencies, which is how pitch is heard). Every recipe ends in
 * silence, so a sound effect never ends on a click, and its voices' levels never add up past 1 at
 * any moment, so it cannot clip on its own (the tests check both).
 */

export type Breakpoints = readonly (readonly [number, number])[];

export type SfxVoice = {
  readonly wave: OscillatorType | "noise";
  /** Pitch in Hz over time (ignored for noise). */
  readonly pitch?: Breakpoints;
  /** Loudness 0..1 over time. Starts and ends at 0. */
  readonly level: Breakpoints;
  readonly filter?: { readonly type: BiquadFilterType; readonly q: number; readonly cutoff: Breakpoints };
  /** Seed for noise. */
  readonly seed?: number;
};

export type SfxRecipe = {
  readonly id: string;
  readonly name: string;
  readonly emoji: string;
  readonly seconds: number;
  readonly voices: readonly SfxVoice[];
};

/** A decaying wobble around `centre`: the boing's spring. */
function wobble(centre: number, depth: number, cycles: number, seconds: number): Breakpoints {
  const points: [number, number][] = [];
  const steps = cycles * 2;
  for (let i = 0; i <= steps; i++) {
    const t = (i / steps) * seconds;
    const decay = 1 - i / steps;
    points.push([t, centre * (1 + (i % 2 ? depth : -depth) * decay)]);
  }
  return points;
}

/** Fade-out tail shared by short percussive sounds. */
const hit = (peak: number, seconds: number): Breakpoints => [[0, 0], [0.004, peak], [seconds * 0.35, peak * 0.4], [seconds, 0]];

export const SFX: readonly SfxRecipe[] = [
  {
    id: "pow", name: "Pow", emoji: "💥", seconds: 0.45,
    voices: [
      { wave: "noise", seed: 1, level: hit(0.45, 0.45), filter: { type: "lowpass", q: 1, cutoff: [[0, 4000], [0.45, 180]] } },
      { wave: "sine", pitch: [[0, 160], [0.3, 45]], level: hit(0.5, 0.35) },
    ],
  },
  {
    id: "whoosh", name: "Whoosh", emoji: "💨", seconds: 0.7,
    voices: [
      { wave: "noise", seed: 2, level: [[0, 0], [0.3, 0.7], [0.7, 0]], filter: { type: "bandpass", q: 2.5, cutoff: [[0, 350], [0.32, 2600], [0.7, 500]] } },
    ],
  },
  {
    id: "boing", name: "Boing", emoji: "🌀", seconds: 0.8,
    voices: [{ wave: "triangle", pitch: [[0, 140], ...wobble(300, 0.35, 7, 0.8).slice(1)], level: [[0, 0], [0.01, 0.8], [0.5, 0.45], [0.8, 0]] }],
  },
  {
    id: "zap", name: "Zap", emoji: "⚡", seconds: 0.3,
    voices: [
      { wave: "sawtooth", pitch: [[0, 1900], [0.3, 110]], level: [[0, 0], [0.005, 0.45], [0.3, 0]], filter: { type: "bandpass", q: 3, cutoff: [[0, 3000], [0.3, 500]] } },
    ],
  },
  {
    id: "pop", name: "Pop", emoji: "🫧", seconds: 0.12,
    voices: [{ wave: "sine", pitch: [[0, 380], [0.06, 1300]], level: [[0, 0], [0.003, 0.9], [0.05, 0.4], [0.12, 0]] }],
  },
  {
    id: "ding", name: "Ding", emoji: "🔔", seconds: 1.4,
    voices: [
      { wave: "sine", pitch: [[0, 1318], [1.4, 1318]], level: [[0, 0], [0.004, 0.55], [0.25, 0.3], [1.4, 0]] },
      { wave: "sine", pitch: [[0, 3290], [1.4, 3290]], level: [[0, 0], [0.004, 0.18], [0.4, 0.04], [1.0, 0]] },
    ],
  },
  {
    id: "slide-up", name: "Slide up", emoji: "⬆️", seconds: 0.65,
    voices: [{ wave: "sine", pitch: [[0, 320], [0.6, 1500]], level: [[0, 0], [0.03, 0.6], [0.55, 0.6], [0.65, 0]] }],
  },
  {
    id: "slide-down", name: "Slide down", emoji: "⬇️", seconds: 0.65,
    voices: [{ wave: "sine", pitch: [[0, 1500], [0.6, 300]], level: [[0, 0], [0.03, 0.6], [0.55, 0.6], [0.65, 0]] }],
  },
  {
    id: "thud", name: "Thud", emoji: "🪨", seconds: 0.4,
    voices: [
      { wave: "sine", pitch: [[0, 95], [0.3, 38]], level: hit(0.7, 0.4) },
      { wave: "noise", seed: 3, level: hit(0.25, 0.18), filter: { type: "lowpass", q: 0.7, cutoff: [[0, 900], [0.18, 120]] } },
    ],
  },
  {
    id: "laser", name: "Laser", emoji: "🔫", seconds: 0.35,
    voices: [{ wave: "square", pitch: [[0, 2600], [0.33, 260]], level: [[0, 0], [0.004, 0.3], [0.35, 0]], filter: { type: "lowpass", q: 4, cutoff: [[0, 6000], [0.35, 1200]] } }],
  },
  {
    id: "sparkle", name: "Sparkle", emoji: "✨", seconds: 0.9,
    voices: [2100, 2650, 3150, 3700, 4400].map((hz, i) => ({
      wave: "sine" as const,
      pitch: [[0, hz], [0.9, hz]] as Breakpoints,
      level: [[0, 0], [i * 0.11, 0], [i * 0.11 + 0.004, 0.32], [i * 0.11 + 0.3, 0], [0.9, 0]] as Breakpoints,
    })),
  },
  {
    id: "honk", name: "Honk", emoji: "📯", seconds: 0.5,
    voices: [
      { wave: "sawtooth", pitch: [[0, 262], [0.5, 255]], level: [[0, 0], [0.02, 0.35], [0.4, 0.35], [0.5, 0]], filter: { type: "lowpass", q: 2, cutoff: [[0, 1400], [0.5, 900]] } },
      { wave: "sawtooth", pitch: [[0, 265], [0.5, 258]], level: [[0, 0], [0.02, 0.35], [0.4, 0.35], [0.5, 0]], filter: { type: "lowpass", q: 2, cutoff: [[0, 1400], [0.5, 900]] } },
    ],
  },
];

export function sfxRecipe(id: string): SfxRecipe | null {
  return SFX.find((r) => r.id === id) ?? null;
}

/**
 * Deterministic white noise in [-1, 1) (mulberry32). The same seed always gives the same samples,
 * so a sound effect sounds identical on every render.
 */
export function seededNoise(seed: number, length: number): Float32Array<ArrayBuffer> {
  const out = new Float32Array(length);
  let state = seed >>> 0;
  for (let i = 0; i < length; i++) {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    out[i] = (((t ^ (t >>> 14)) >>> 0) / 4294967296) * 2 - 1;
  }
  return out;
}

/** Lay breakpoints onto an AudioParam: linear for levels, exponential for frequencies. */
function automate(param: AudioParam, points: Breakpoints, exponential: boolean): void {
  param.setValueAtTime(points[0][1], points[0][0]);
  for (const [t, value] of points.slice(1)) {
    if (exponential) param.exponentialRampToValueAtTime(Math.max(1, value), t);
    else param.linearRampToValueAtTime(value, t);
  }
}

/** Render a recipe at `sampleRate`, mono. Browser-only (OfflineAudioContext). */
export async function renderSfx(
  recipe: SfxRecipe,
  sampleRate: number,
  createContext: (channels: number, frames: number, sampleRate: number) => OfflineAudioContext =
    (channels, length, rate) => new OfflineAudioContext({ numberOfChannels: channels, length, sampleRate: rate }),
): Promise<AudioBuffer> {
  const frames = Math.ceil(recipe.seconds * sampleRate);
  const context = createContext(1, frames, sampleRate);
  for (const voice of recipe.voices) {
    let source: AudioScheduledSourceNode;
    if (voice.wave === "noise") {
      const buffer = context.createBuffer(1, frames, sampleRate);
      buffer.copyToChannel(seededNoise(voice.seed ?? 0, frames), 0);
      const node = context.createBufferSource();
      node.buffer = buffer;
      source = node;
    } else {
      const node = context.createOscillator();
      node.type = voice.wave;
      if (voice.pitch) automate(node.frequency, voice.pitch, true);
      source = node;
    }
    const level = context.createGain();
    automate(level.gain, voice.level, false);
    let tail: AudioNode = source;
    if (voice.filter) {
      const filter = context.createBiquadFilter();
      filter.type = voice.filter.type;
      filter.Q.value = voice.filter.q;
      automate(filter.frequency, voice.filter.cutoff, true);
      tail = tail.connect(filter);
    }
    tail.connect(level).connect(context.destination);
    source.start(0);
    source.stop(recipe.seconds);
  }
  return context.startRendering();
}
