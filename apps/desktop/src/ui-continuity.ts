import { sanitizeStatusCursor, type StatusCursors } from "./statusread.ts";
import type { ReadMark } from "./unread";

export type UiContinuity = {
  version: 1;
  drafts: Record<string, string>;
  readMarks: Record<string, ReadMark>;
  /**
   * How far this person has read each server's announcements, keyed by server id.
   *
   * Sealed here beside the chat read marks rather than kept in localStorage, for the reason those
   * were moved: what somebody has and has not read is a reading habit, and reading habits do not
   * fall back to plaintext storage. The native envelope check only requires `version`, `drafts` and
   * `readMarks`, so this field rides along without a Rust change.
   */
  statusCursors: StatusCursors;
};

const MAX_ENTRIES = 2_000;
const MAX_KEY_CHARS = 256;
const MAX_DRAFT_CHARS = 32_768;
/** Message ids are fixed-width hex in practice; this only has to stop an unbounded record. */
const MAX_ID_CHARS = 128;

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

/** Treat native continuity JSON as untrusted even though the bridge also validates its envelope. */
export function sanitizeUiContinuity(value: unknown): UiContinuity {
  const root = record(value);
  const drafts: Record<string, string> = {};
  const readMarks: Record<string, ReadMark> = {};
  for (const [key, item] of Object.entries(record(root.drafts)).slice(0, MAX_ENTRIES)) {
    if (key.length <= MAX_KEY_CHARS && typeof item === "string" && item.length <= MAX_DRAFT_CHARS) {
      drafts[key] = item;
    }
  }
  for (const [key, item] of Object.entries(record(root.readMarks)).slice(0, MAX_ENTRIES)) {
    if (key.length > MAX_KEY_CHARS) continue;
    const mark = readMark(item);
    if (mark) readMarks[key] = mark;
  }
  const statusCursors: StatusCursors = {};
  for (const [key, item] of Object.entries(record(root.statusCursors)).slice(0, MAX_ENTRIES)) {
    // A server id is a number on the app's side and a string in JSON. A key that is not the exact
    // decimal spelling of one names no server, so nothing would ever read its cursor back: `""`
    // would otherwise become server 0, which is a real id belonging to somebody else's feed.
    const server = Number(key);
    if (!Number.isSafeInteger(server) || server < 0 || String(server) !== key) continue;
    const cursor = sanitizeStatusCursor(item);
    // A cursor at zero is indistinguishable from having no cursor, so it is not worth sealing.
    if (cursor.ts > 0 || cursor.ids.length) statusCursors[server] = cursor;
  }
  return { version: 1, drafts, readMarks, statusCursors };
}

/**
 * Read one persisted mark, accepting the bare timestamp older builds wrote.
 *
 * A mark used to be just a number. Upgrading in place rather than discarding those keeps every
 * badge people had already cleared cleared: the id is empty until the channel is next read, which
 * is exactly the state the unread scan treats as "fall back to the timestamp".
 */
function readMark(item: unknown): ReadMark | null {
  if (typeof item === "number") {
    return Number.isSafeInteger(item) && item >= 0 ? { ts: item, id: "" } : null;
  }
  if (item === null || typeof item !== "object" || Array.isArray(item)) return null;
  const m = item as Record<string, unknown>;
  const ts = m.ts;
  const id = m.id;
  if (typeof ts !== "number" || !Number.isSafeInteger(ts) || ts < 0) return null;
  if (id !== undefined && (typeof id !== "string" || id.length > MAX_ID_CHARS)) return null;
  return { ts, id: typeof id === "string" ? id : "" };
}

export type LegacyMigration = {
  state: UiContinuity;
  /** Persist `state` in the vault before removing the legacy key. */
  saveBeforeRemoval: boolean;
  /** Remove the old app-owned localStorage key after any required save succeeds. */
  removeLegacy: boolean;
};

/** Plan a one-time plaintext read-mark migration without letting stale legacy data win. */
export function planLegacyReadMarkMigration(
  current: UiContinuity,
  legacyJson: string | null,
): LegacyMigration {
  if (legacyJson === null) return { state: current, saveBeforeRemoval: false, removeLegacy: false };
  if (Object.keys(current.readMarks).length) {
    return { state: current, saveBeforeRemoval: false, removeLegacy: true };
  }
  try {
    const parsed = JSON.parse(legacyJson);
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
      return { state: current, saveBeforeRemoval: false, removeLegacy: false };
    }
    // The legacy key only ever held chat read marks, so everything else in the sealed record is
    // carried across unchanged rather than being dropped by the migration that adopts them.
    const migrated = sanitizeUiContinuity({
      drafts: current.drafts,
      readMarks: parsed,
      statusCursors: current.statusCursors,
    });
    return { state: migrated, saveBeforeRemoval: true, removeLegacy: true };
  } catch {
    // Preserve malformed legacy data for diagnosis rather than deleting it under a migration claim.
    return { state: current, saveBeforeRemoval: false, removeLegacy: false };
  }
}
