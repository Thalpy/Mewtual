import { sanitizeStatusCursor, type StatusCursors } from "./statusread.ts";
import { MAX_LATE_PAST, type LatePast, type ReadMark } from "./unread.ts";
import { sanitizeFileTrustPolicies, type FileTrustPolicies } from "./file-trust.ts";
import { pendingContinuityVersion, sanitizePendingSends, type PendingSends } from "./pending-sends.ts";
import { decodeStudioDraft } from "./studio-draft.ts";

export type UiContinuity = {
  version: 1 | 2 | 3;
  pendingSends: PendingSends;
  /** Explicitly recovered text, independent of whether its original conversation still exists. */
  recoveredSendDrafts: PendingSends;
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
  /** Per-server automatic-fetch policy; sealed because it names member relationships. */
  fileTrustPolicies: FileTrustPolicies;
  /**
   * Per conversation (same key as `readMarks`), the messages that arrived behind this person's
   * read position and have not yet been in front of them. Sealed for the same reason as the read
   * marks, and persisted at all because a restart is the one moment the markers matter most: the
   * thing that survived a desync must not be the thing a reboot clears.
   */
  latePast: Record<string, LatePast>;
  /**
   * Whether third-party player cards (Spotify, YouTube) may load without being clicked.
   *
   * Device-wide rather than per-server, unlike [`fileTrustPolicies`], because the thing being
   * decided is not about a server or its members. A card discloses this device's address to
   * Google or Spotify; which of this person's servers the link happened to be posted in changes
   * nothing about who learns what, so a per-server answer would be finer-grained without being
   * more meaningful.
   *
   * Sealed for the same reason the read marks are: "load these automatically" is a standing
   * instruction to contact two named companies, and a preference that decides what this device
   * reaches out to does not belong in plaintext webview storage where anything running in the
   * window could flip it.
   */
  embedAutoLoad: boolean;
  /** Orderly lock/close recovery; the decoder rejects the whole queue rather than dropping retries. */
  studioDraft: string | null;
};

const MAX_ENTRIES = 2_000;
const MAX_KEY_CHARS = 256;
export const MAX_DRAFT_CHARS = 32_768;
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
  // Critical versions are sticky: v2 protects mutation/Stop custody from v1, while v3
  // protects enlarged Send retention from the existing v2 reader's silent field drops.
  if (root.version !== undefined && root.version !== 1 && root.version !== 2 && root.version !== 3) {
    throw new Error("Saved UI continuity version is unsupported. The sealed copy has been kept.");
  }
  let version: 1 | 2 | 3 = root.version === 3 ? 3 : root.version === 2 ? 2 : 1;
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
  const fileTrustPolicies = sanitizeFileTrustPolicies(root.fileTrustPolicies);
  const latePast: Record<string, LatePast> = {};
  for (const [key, item] of Object.entries(record(root.latePast)).slice(0, MAX_ENTRIES)) {
    if (key.length > MAX_KEY_CHARS || !Array.isArray(item)) continue;
    const ids: string[] = [];
    const seen = new Set<string>();
    for (const id of item) {
      if (typeof id !== "string" || !id.length || id.length > MAX_ID_CHARS || seen.has(id)) continue;
      seen.add(id);
      ids.push(id);
    }
    // Newest last is the order they were added in, so an over-long list keeps its newest.
    const kept = ids.length > MAX_LATE_PAST ? ids.slice(ids.length - MAX_LATE_PAST) : ids;
    if (kept.length) latePast[key] = kept;
  }
  // Strictly `=== true`, so anything that is not an explicit stored yes reads as no: a missing
  // field, a truthy string a future build wrote, a corrupted record. The permissive direction of
  // this flag starts network requests, so it is the one that has to be asked for exactly.
  const embedAutoLoad = root.embedAutoLoad === true;
  const pendingSends = sanitizePendingSends(root.pendingSends, version);
  const recoveredSendDrafts = sanitizePendingSends(root.recoveredSendDrafts, version);
  if ([...Object.values(pendingSends), ...Object.values(recoveredSendDrafts)].some(intent => pendingContinuityVersion(intent) === 3)) version = 3;
  if (root.studioDraft !== undefined && root.studioDraft !== null && typeof root.studioDraft !== "string") throw new Error("Saved Studio edits have an invalid format. The sealed copy has been kept.");
  const studioDraft = typeof root.studioDraft === "string" ? root.studioDraft : null;
  if (studioDraft !== null) decodeStudioDraft(studioDraft);
  return { version, drafts, readMarks, statusCursors, fileTrustPolicies, latePast, embedAutoLoad, pendingSends, recoveredSendDrafts, studioDraft };
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
      version: current.version,
      drafts: current.drafts,
      readMarks: parsed,
      statusCursors: current.statusCursors,
      fileTrustPolicies: current.fileTrustPolicies,
      latePast: current.latePast,
      embedAutoLoad: current.embedAutoLoad,
      pendingSends: current.pendingSends,
      recoveredSendDrafts: current.recoveredSendDrafts,
      studioDraft: current.studioDraft,
    });
    return { state: migrated, saveBeforeRemoval: true, removeLegacy: true };
  } catch {
    // Preserve malformed legacy data for diagnosis rather than deleting it under a migration claim.
    return { state: current, saveBeforeRemoval: false, removeLegacy: false };
  }
}
