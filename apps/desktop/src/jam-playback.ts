import { JAM_KIT, JAM_TAKE_CACHE_MAX, JAM_VOICE_DECLICK_SECONDS, TAKE_MAX_BYTES, TAKE_PLAYBACK_EVENTS_PER_TICK, type JamTake, type JamTakeEvent } from "./jam-contract.ts";
// The renderer owns what a release actually costs; the transport must not keep a second opinion.
// jam-engine does not import this module, so this direction adds no cycle.
import { effectiveReleaseSeconds } from "./jam-engine.ts";
import { legacyJamPatch } from "./jam-patch.ts";

const TAKE_BASE64_MAX_CHARS = Math.ceil(TAKE_MAX_BYTES / 3) * 4;

type JamTakeLoadTask<T> = {
  epoch: number;
  key: string;
  promise: Promise<T | null>;
  run: (isCurrent: () => boolean) => T | Promise<T>;
  cancel: () => void | Promise<void>;
  retiring: boolean;
  resolve: (value: T | null) => void;
  reject: (reason: unknown) => void;
};

/**
 * Run at most one whole-file take load and retain at most one newer replacement.
 *
 * Equal keys resolve to `null` immediately: the one existing consumer reads current global deck
 * state, so duplicate heartbeats retain no waiter. `invalidate()` makes a running continuation
 * stale, asks its native operation to cancel, and retires its active slot only after that request
 * is acknowledged. A late completion cannot disturb the replacement because finalization is
 * identity-checked against the current active item.
 */
export class JamTakeLoadCoordinator<T> {
  private epoch = 0;
  private active: JamTakeLoadTask<T> | null = null;
  private pending: JamTakeLoadTask<T> | null = null;

  submit(
    key: string,
    run: (isCurrent: () => boolean) => T | Promise<T>,
    cancel: () => void | Promise<void>,
  ): Promise<T | null> {
    if (!key) throw new TypeError("take load needs a stable key");
    if (this.active?.epoch === this.epoch && this.active.key === key) return Promise.resolve(null);
    if (this.pending?.epoch === this.epoch && this.pending.key === key) return Promise.resolve(null);

    let resolve!: (value: T | null) => void;
    let reject!: (reason: unknown) => void;
    const promise = new Promise<T | null>((yes, no) => { resolve = yes; reject = no; });
    const item = { epoch: this.epoch, key, promise, run, cancel, retiring: false, resolve, reject };
    if (!this.active) this.execute(item);
    else {
      this.pending?.resolve(null);
      this.pending = item;
    }
    return promise;
  }

  invalidate(): void {
    this.epoch += 1;
    this.pending?.resolve(null);
    this.pending = null;
    const active = this.active;
    if (!active || active.retiring) return;
    active.retiring = true;
    void Promise.resolve()
      .then(active.cancel)
      .then(() => {
        // Cancellation acknowledgement is the authority to release this slot. The native layer
        // independently caps registered operations, so call churn cannot accumulate without bound.
        if (this.active !== active) return;
        this.active = null;
        active.resolve(null);
        this.startPending();
      }, () => {
        // Do not detach an operation whose native cancellation was not acknowledged. Its normal
        // completion can still release the slot, and a later invalidation may retry cancellation.
        if (this.active === active) active.retiring = false;
      });
  }

  private execute(item: JamTakeLoadTask<T>): void {
    this.active = item;
    void Promise.resolve()
      .then(() => item.run(() => item.epoch === this.epoch))
      .then(item.resolve, item.reject)
      .finally(() => {
        if (this.active !== item) return;
        this.active = null;
        this.startPending();
      });
  }

  private startPending(): void {
    const next = this.pending;
    this.pending = null;
    if (next) this.execute(next);
  }
}

export type JamTakePlaybackLease = Readonly<{
  callLease: number;
  server: number;
  channel: string;
  cid: string;
}>;

/** Exact identity for progress emitted by one cancellable take download. */
export type JamTakeProgressLease = Readonly<{
  callLease: number;
  server: number;
  cid: string;
  cancellation: string;
}>;

/** Reject queued native progress from an older call/server/token, even when the CID is reused. */
export function shouldApplyJamTakeProgress(
  active: JamTakeProgressLease | null,
  event: Readonly<{ server: number; cid: string; cancellation: string | null }>,
  currentCallLease: number,
): boolean {
  return active !== null
    && active.callLease === currentCallLease
    && active.server === event.server
    && active.cid === event.cid
    && event.cancellation !== null
    && active.cancellation === event.cancellation;
}

/** Exact scalar call/deck binding used again after every whole-file fetch await. */
export function jamTakePlaybackLeaseCurrent(
  lease: JamTakePlaybackLease,
  current: Readonly<{
    inCall: boolean;
    callLease: number;
    server: number | null;
    channel: string;
    cid: string | null;
  }>,
): boolean {
  return current.inCall && current.callLease === lease.callLease && current.server === lease.server &&
    current.channel === lease.channel && current.cid === lease.cid;
}

/** Refuse an oversized listing before invoking the generic whole-file download command. */
export function mayFetchJamTake(size: number): boolean {
  return Number.isSafeInteger(size) && size >= 0 && size <= TAKE_MAX_BYTES;
}

/** Decode only a transport string whose encoded and decoded sizes fit the take contract. */
export function decodeJamTakeBase64(
  encoded: unknown,
  decode: (value: string) => string = atob,
): string | null {
  if (typeof encoded !== "string" || encoded.length > TAKE_BASE64_MAX_CHARS) return null;
  let binary: string;
  try { binary = decode(encoded); } catch { return null; }
  if (binary.length > TAKE_MAX_BYTES) return null;
  return new TextDecoder().decode(Uint8Array.from(binary, (char) => char.charCodeAt(0)));
}

/** Small per-call LRU of fully validated takes; removed queue CIDs cannot accumulate forever. */
export class JamTakeCache {
  private readonly values = new Map<string, JamTake>();

  get(cid: string): JamTake | undefined {
    const value = this.values.get(cid);
    if (!value) return undefined;
    this.values.delete(cid);
    this.values.set(cid, value);
    return value;
  }

  set(cid: string, take: JamTake): void {
    this.values.delete(cid);
    this.values.set(cid, take);
    while (this.values.size > JAM_TAKE_CACHE_MAX) this.values.delete(this.values.keys().next().value!);
  }

  clear(): void {
    this.values.clear();
  }
}

/**
 * Decide whether a recorded event may reach the synth while the call is deafened.
 *
 * Deafen is the room's hard master gate, including local take previews. New notes and drum hits
 * stay silent, but note-offs still run so held-state sequencing closes on schedule.
 */
export function shouldDispatchTakeEvent(
  event: JamTakeEvent,
  callDeafened: boolean,
): boolean {
  if (!callDeafened) return true;
  return !("d" in event) && event.on === 0;
}

/** Shared-deck playback is remote-controlled even though synthesis happens on this receiver. */
export function takePlaybackIsRemote(deckCid: string | null): boolean {
  return deckCid !== null;
}

/** One note that a seek must bring back, and how long it has already been held when it does. */
export type JamTakeSeekVoice = Readonly<{
  /** Index of the note-on in `take.events`, so its lane, patch and sequence come from the log. */
  index: number;
  /** Milliseconds between that note-on and the seek point; the age the voice starts at. */
  ageMs: number;
}>;

export type JamTakeSeekPlan = Readonly<{
  /** First event index the ordinary scheduler owns: everything at or after the seek point. */
  next: number;
  /** Note-ons before the seek point whose note-off has not happened yet, in log order. */
  sounding: readonly JamTakeSeekVoice[];
}>;

/**
 * What a take sounds like at an arbitrary offset, not just what happens after it.
 *
 * Starting at the first event whose `ms` reaches the offset is right for the schedule and wrong
 * for the sound. A take holding one chord from 0 ms to 10 s, joined at 5 s, has no due event at
 * all until the key-ups arrive, so a listener who joins the jukebox deck mid-track hears silence
 * where a chord is sounding, and then a run of note-offs for voices that were never opened. The
 * longer the note, the longer the silence, which is exactly backwards.
 *
 * So the events before the offset are folded into held state rather than skipped: for each lane,
 * the latest note-on per pitch that no note-off has closed. Those are re-opened at playback start
 * at the age they have already reached, so a note six seconds into a slow swell arrives at the
 * level it should be, not at the start of its attack. Emitting them in log order preserves each
 * lane's `q` order, which the take validator has already proved is strictly increasing, so the
 * engine's own duplicate/gap sequencing is satisfied without any renumbering here.
 *
 * Drums are deliberately NOT reconstructed. A pad is a one-shot: its tail is the end of a sound
 * whose transient has already gone, and firing a whole fresh crash because its 3-second tail
 * happens to cross the seek point would insert an attack the take does not contain.
 */
export function planTakeSeek(take: JamTake, offsetMs: number): JamTakeSeekPlan {
  const baseMs = Number.isFinite(offsetMs) ? Math.max(0, offsetMs) : 0;
  const held = new Map<string, JamTakeSeekVoice>();
  let next = take.events.length;
  for (let index = 0; index < take.events.length; index += 1) {
    const event = take.events[index];
    if (event.ms >= baseMs) { next = index; break; }
    if ("d" in event) continue;
    // The engine keeps at most one held voice per (source, pitch) and closes the latest on a
    // note-off, so the same single-slot rule decides what is still sounding here.
    const key = `${event.lane}:${event.n}`;
    if (event.on === 1) held.set(key, { index, ageMs: baseMs - event.ms });
    else held.delete(key);
  }
  const sounding = [...held.values()].sort((a, b) => a.index - b.index);
  return { next, sounding };
}

/** Exclusive end of one bounded overdue-event scheduler pass. */
export function takeDueBatchEnd(
  events: readonly JamTakeEvent[],
  next: number,
  elapsedMs: number,
): number {
  let end = next;
  const ceiling = Math.min(events.length, next + TAKE_PLAYBACK_EVENTS_PER_TICK);
  while (end < ceiling && events[end].ms <= elapsedMs) end += 1;
  return end;
}

/**
 * Longest permitted post-event tail used by a take.
 *
 * This is deliberately conservative: a patch used early may have finished before the last event,
 * but keeping its bounded release horizon cannot truncate a legal final chord. The transport and
 * source teardown therefore share the same honest upper bound instead of a magic 3.5 seconds.
 */
export function takeReleaseTailMs(take: JamTake): number {
  let tailMs = 0;
  for (const event of take.events) {
    if ("d" in event) {
      tailMs = Math.max(tailMs, JAM_KIT[event.n]?.tailMs ?? 0);
    } else if (event.on === 1) {
      const patch = event.p === undefined ? legacyJamPatch(event.w) : take.patches[event.p];
      if (patch) tailMs = Math.max(tailMs, effectiveReleaseSeconds(patch.e.r) * 1_000);
    }
  }
  // The renderer's own minimum, even for a take of nothing but zero-release notes: the transport
  // must reserve the horizon the synth will actually use, or normal completion tears the graph
  // down mid-fade and sounds exactly like an intentional hard cancel.
  return take.events.length ? Math.max(tailMs, JAM_VOICE_DECLICK_SECONDS * 1_000) : tailMs;
}
