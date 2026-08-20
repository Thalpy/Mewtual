// The title-bar ticker's original, asset-free three-note cue. Keeping the score and Web Audio
// scheduling here makes the sound deterministic and unit-testable without coupling it to Svelte.

export type NewsChimeNote = Readonly<{
  frequency: number;
  offset: number;
  duration: number;
  peak: number;
}>;

// Three matching square-wave pips, each settling quickly from ~1.08 kHz to ~930 Hz. The spacing,
// pitch fall, and strong third harmonic mirror the supplied reference's audible shape while this
// remains a tiny generated cue rather than a redistributed game recording.
export const NEWS_CHIME_NOTES: readonly NewsChimeNote[] = [
  { frequency: 932.33, offset: 0, duration: 0.082, peak: 0.042 },
  { frequency: 932.33, offset: 0.1, duration: 0.082, peak: 0.042 },
  { frequency: 932.33, offset: 0.2, duration: 0.082, peak: 0.042 },
];

export function scheduleNewsChime(ctx: AudioContext, at = ctx.currentTime): void {
  for (const note of NEWS_CHIME_NOTES) {
    const start = at + note.offset;
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = "square";
    osc.frequency.setValueAtTime(note.frequency * 1.16, start);
    osc.frequency.exponentialRampToValueAtTime(note.frequency, start + 0.038);
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(note.peak, start + 0.003);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + note.duration);
    osc.connect(gain).connect(ctx.destination);
    osc.start(start);
    osc.stop(start + note.duration + 0.01);
  }
}
