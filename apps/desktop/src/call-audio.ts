/**
 * What a listener hears from one peer, and how loud.
 *
 * Two problems that turn out to be the same problem. A peer's voice could not be turned up past
 * 100%, because the level drove `HTMLAudioElement.volume`, which the browser hard-caps at unity:
 * a quiet friend could be turned down but never up. And a peer sharing a screen had their voice
 * and their game arrive as two audio tracks that were merged into one element, so there was one
 * slider for both and no way to hear someone over what they were showing.
 *
 * Both are answered by routing each source through its own Web Audio gain: gain is a multiplier
 * with no ceiling of its own, and separate chains are separately adjustable. The rules for
 * WHICH chain a track belongs to, and for what a level means, are here so they can be decided
 * without a browser; the graph itself is built at the call site next to the rest of the audio.
 */

/** The two things a peer can send you sound from. */
export type PeerAudioKind = "voice" | "share";

/** Unity: what you hear is what they sent. */
export const DEFAULT_PEER_LEVEL = 100;

/**
 * The loudest a peer may be made, as a percentage.
 *
 * Above unity this is amplification of an already-compressed Opus stream, so the top of the range
 * is for rescuing someone whose microphone is genuinely too quiet, not for everyday use. 250%
 * (about +8 dB) is enough to make a distant laptop microphone intelligible; past that the noise
 * floor comes up faster than the voice does.
 */
export const MAX_PEER_LEVEL = 250;

/**
 * Read a stored or typed level without letting NaN or an unbounded value reach the graph.
 *
 * Coercion is deliberately narrow. `Number(null)` and `Number("")` are both 0, so a level that
 * was never stored would coerce to SILENCE rather than to unity, and a peer nobody had ever
 * touched a slider for would simply not be audible.
 */
export function normalizePeerLevel(value: unknown, fallback: number = DEFAULT_PEER_LEVEL): number {
  const numeric = typeof value === "number"
    ? value
    : typeof value === "string" && value.trim() !== ""
      ? Number(value)
      : Number.NaN;
  if (!Number.isFinite(numeric)) return fallback;
  return Math.round(Math.min(MAX_PEER_LEVEL, Math.max(0, numeric)));
}

/** Web Audio gain is a linear multiplier: 100% is unity, 0% is silence, 250% is 2.5x. */
export function peerGain(level: unknown): number {
  return normalizePeerLevel(level) / 100;
}

/**
 * Which of a peer's audio streams is their voice and which is their share.
 *
 * A sender announces its shared audio under the same stream id as its shared video (see
 * `syncScreenAudioPeer`), so the stream that carries video is the share and every other stream
 * from that peer is voice. Tracks are NOT guaranteed to arrive in that order: one negotiation can
 * deliver the audio first, and the answer for it has to change once the video turns up. So this
 * remembers what it has been told and reports when an earlier answer has been revised, which is
 * the signal to move that track to the other chain.
 */
export class RemoteAudioRouter {
  /** peer -> the stream id that carried video, once one has. */
  readonly #shareStreams = new Map<string, string>();
  /** peer -> stream id -> the kind last reported for it. */
  readonly #decided = new Map<string, Map<string, PeerAudioKind>>();

  /** The kind for an audio stream, recorded so a later revision can be detected. */
  classify(peer: string, streamId: string): PeerAudioKind {
    const kind: PeerAudioKind = this.#shareStreams.get(peer) === streamId ? "share" : "voice";
    let byStream = this.#decided.get(peer);
    if (!byStream) {
      byStream = new Map();
      this.#decided.set(peer, byStream);
    }
    byStream.set(streamId, kind);
    return kind;
  }

  /**
   * Record that this peer's video arrived on `streamId`.
   *
   * Returns true when that CHANGES an answer already given, meaning audio from this stream is
   * currently on the voice chain and has to be moved. Returns false when it tells us nothing new,
   * so an ordinary share (video first, or a re-offer of a stream we already knew about) does not
   * make the call rebuild a graph that is already right.
   */
  noteVideo(peer: string, streamId: string): boolean {
    const previous = this.#shareStreams.get(peer);
    this.#shareStreams.set(peer, streamId);
    if (previous === streamId) return false;
    return this.#decided.get(peer)?.get(streamId) === "voice";
  }

  /** Forget a peer entirely: they left, or the call did. */
  forget(peer: string): void {
    this.#shareStreams.delete(peer);
    this.#decided.delete(peer);
  }

  /** Forget everyone. */
  clear(): void {
    this.#shareStreams.clear();
    this.#decided.clear();
  }
}

/**
 * The effective gain for one chain, with every reason it might be silent already applied.
 *
 * Silence is expressed as a gain of zero rather than as a muted element because the element is no
 * longer what the listener hears: a graph whose gain is still 2.5 while a muted element sits at
 * the end of it would come back at 2.5 the moment the mute lifted, which is right, but the mute
 * itself has to reach the graph or it reaches nothing.
 */
export function effectivePeerGain(level: unknown, muted: boolean, deafened: boolean): number {
  return muted || deafened ? 0 : peerGain(level);
}
