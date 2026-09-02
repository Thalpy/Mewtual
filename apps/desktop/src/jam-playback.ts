import { JAM_KIT, JAM_RELEASE_CAP_MS, JAM_TAKE_CACHE_MAX, TAKE_MAX_BYTES, TAKE_PLAYBACK_EVENTS_PER_TICK, type JamTake, type JamTakeEvent } from "./jam-contract.ts";
import { legacyJamPatch } from "./jam-patch.ts";

const TAKE_BASE64_MAX_CHARS = Math.ceil(TAKE_MAX_BYTES / 3) * 4;

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
      if (patch) tailMs = Math.max(tailMs, Math.min(patch.e.r, JAM_RELEASE_CAP_MS));
    }
  }
  return tailMs;
}
