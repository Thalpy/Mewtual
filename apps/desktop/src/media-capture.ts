/**
 * Monotonic lease for permission prompts that may resolve long after the user stopped sharing.
 * Starting a competing capture or invalidating the session makes every older result stale.
 */
export class MediaCaptureSession {
  #generation = 0;

  begin(): number {
    this.#generation += 1;
    return this.#generation;
  }

  current(): number {
    return this.#generation;
  }

  invalidate(): void {
    this.#generation += 1;
  }

  isCurrent(lease: number): boolean {
    return lease === this.#generation;
  }
}

export type TrackCollection = {
  getTracks(): Array<{ stop(): void }>;
};

/** Stop every returned track when a capture chooser completed for an obsolete session. */
export function acceptCapture<T extends TrackCollection>(
  session: MediaCaptureSession,
  lease: number,
  capture: T,
  stillWanted: boolean,
): T | null {
  if (stillWanted && session.isCurrent(lease)) return capture;
  for (const track of capture.getTracks()) track.stop();
  return null;
}

type SenderLike = { track: { kind: string } | null };

/**
 * Locate a microphone sender without ever borrowing the parked video or mixed screen-audio slot.
 * The explicit sender wins because a muted/parked microphone has no track kind to rediscover.
 */
export function chooseMicrophoneSender<T extends SenderLike>(
  senders: T[],
  current: T | null,
  video: T | null,
  screenAudio: T | null,
): T | null {
  if (current && current !== video && current !== screenAudio) return current;
  return senders.find((sender) => sender !== video && sender !== screenAudio && sender.track?.kind === "audio")
    ?? senders.find((sender) => sender !== video && sender !== screenAudio && !sender.track)
    ?? null;
}
