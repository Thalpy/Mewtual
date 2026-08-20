/** The initial live DOM stays deliberately bounded even when the CRDT history is very large. */
export const CHAT_INITIAL_ROWS = 320;
/** Rows revealed in either direction when the reader reaches a window edge. */
export const CHAT_WINDOW_STEP = 200;

export type ChatWindow = { start: number; end: number };
export type WindowMessage = { id: string };

function clampWindow(window: ChatWindow, total: number): ChatWindow {
  const end = Math.max(0, Math.min(total, Math.trunc(window.end)));
  const start = Math.max(0, Math.min(end, Math.trunc(window.start)));
  return { start, end };
}

/** Start at the recent tail; older rows are retained in state but do not enter the DOM yet. */
export function initialChatWindow(total: number, rows = CHAT_INITIAL_ROWS): ChatWindow {
  const end = Math.max(0, Math.trunc(total));
  return { start: Math.max(0, end - Math.max(1, Math.trunc(rows))), end };
}

/** Reveal one bounded page before the current window. */
export function revealOlder(window: ChatWindow, total: number, rows = CHAT_WINDOW_STEP): ChatWindow {
  const current = clampWindow(window, total);
  return { start: Math.max(0, current.start - Math.max(1, Math.trunc(rows))), end: current.end };
}

/** Reveal one bounded page after the current window. */
export function revealNewer(window: ChatWindow, total: number, rows = CHAT_WINDOW_STEP): ChatWindow {
  const current = clampWindow(window, total);
  return { start: current.start, end: Math.min(total, current.end + Math.max(1, Math.trunc(rows))) };
}

/**
 * Make a search/reply/unread target renderable without mounting the whole prefix before it.
 * Some context is retained above the target so a centred scroll does not land at a hard edge.
 */
export function windowAround(index: number, total: number, rows = CHAT_INITIAL_ROWS): ChatWindow {
  if (total <= 0) return { start: 0, end: 0 };
  const target = Math.max(0, Math.min(total - 1, Math.trunc(index)));
  const size = Math.max(1, Math.trunc(rows));
  const start = Math.max(0, Math.min(target - Math.min(40, Math.floor(size / 4)), total - size));
  return { start, end: Math.min(total, start + size) };
}

/**
 * Reconcile a mounted window after a full validated snapshot arrives from the actor.
 *
 * Stable message ids anchor both edges while the reader is away from the tail, preventing a new
 * post from silently entering the DOM below them. At the tail, keep a bounded recent window. A
 * missing anchor (delete/legacy row) falls back to the previous size rather than trusting indexes
 * that may now refer to different messages.
 */
export function reconcileChatWindow(
  previous: readonly WindowMessage[],
  next: readonly WindowMessage[],
  window: ChatWindow,
  stickToBottom: boolean,
  reset = false,
): ChatWindow {
  if (reset || previous.length === 0 || window.end <= window.start) return initialChatWindow(next.length);
  const current = clampWindow(window, previous.length);
  const size = Math.max(1, current.end - current.start);
  if (stickToBottom) return initialChatWindow(next.length, Math.max(CHAT_INITIAL_ROWS, size));

  const firstId = previous[current.start]?.id;
  const lastId = previous[current.end - 1]?.id;
  const first = firstId ? next.findIndex((message) => message.id === firstId) : -1;
  const last = lastId ? next.findIndex((message) => message.id === lastId) : -1;
  if (first >= 0 && last >= first) return { start: first, end: last + 1 };
  if (last >= 0) return { start: Math.max(0, last + 1 - size), end: last + 1 };
  if (first >= 0) return { start: first, end: Math.min(next.length, first + size) };
  // Both edge rows can legitimately be deleted by moderation or retention in one snapshot. Keep
  // the reader near the same index in that case; jumping to the live tail would be especially
  // disorienting while they were reading older history.
  const start = Math.min(current.start, Math.max(0, next.length - size));
  return { start, end: Math.min(next.length, start + size) };
}

/** A small tolerance prevents fractional layout/zoom values from making the tail flicker. */
export function nearScrollBottom(scrollTop: number, clientHeight: number, scrollHeight: number, tolerance = 96): boolean {
  return scrollHeight - (scrollTop + clientHeight) <= Math.max(0, tolerance);
}

type CacheEntry = {
  text: string;
  edited: number;
  mentionName: string;
  html: string;
};

/**
 * Bounded, memory-only cache for already-sanitized message HTML.
 *
 * It deliberately stores no raw HTML from peers and has no persistence hook. Callers clear it on
 * lock; the bound prevents a maliciously busy channel from retaining an unbounded plaintext copy.
 */
export class SanitizedMessageCache {
  readonly #entries = new Map<string, CacheEntry>();
  readonly capacity: number;

  constructor(capacity = 640) {
    if (!Number.isInteger(capacity) || capacity < 1) throw new Error("cache capacity must be positive");
    this.capacity = capacity;
  }

  render(
    scope: string,
    id: string,
    text: string,
    edited: number,
    mentionName: string,
    sanitize: (text: string, mentionName: string) => string,
  ): string {
    const key = `${scope}\u0000${id}`;
    const hit = this.#entries.get(key);
    if (hit && hit.text === text && hit.edited === edited && hit.mentionName === mentionName) {
      // Map insertion order is the LRU order; refresh it without duplicating the entry.
      this.#entries.delete(key);
      this.#entries.set(key, hit);
      return hit.html;
    }
    const html = sanitize(text, mentionName);
    this.#entries.delete(key);
    this.#entries.set(key, { text, edited, mentionName, html });
    while (this.#entries.size > this.capacity) {
      const oldest = this.#entries.keys().next().value;
      if (oldest === undefined) break;
      this.#entries.delete(oldest);
    }
    return html;
  }

  clear(): void {
    this.#entries.clear();
  }

  get size(): number {
    return this.#entries.size;
  }
}

/** Serialize bursty invalidations into at most one active refresh plus one merged follow-up. */
export class CoalescedAsyncRefresh {
  #running: Promise<void> | null = null;
  #requested = false;
  #animateRequested = false;
  readonly task: (animate: boolean) => Promise<void>;

  constructor(task: (animate: boolean) => Promise<void>) {
    this.task = task;
  }

  request(animate = false): Promise<void> {
    this.#requested = true;
    this.#animateRequested ||= animate;
    if (this.#running) return this.#running;
    this.#running = (async () => {
      while (this.#requested) {
        const animateThisPass = this.#animateRequested;
        this.#requested = false;
        this.#animateRequested = false;
        await this.task(animateThisPass);
      }
    })().finally(() => {
      this.#running = null;
    });
    return this.#running;
  }
}
