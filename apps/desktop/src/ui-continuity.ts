export type UiContinuity = {
  version: 1;
  drafts: Record<string, string>;
  readMarks: Record<string, number>;
};

const MAX_ENTRIES = 2_000;
const MAX_KEY_CHARS = 256;
const MAX_DRAFT_CHARS = 32_768;

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

/** Treat native continuity JSON as untrusted even though the bridge also validates its envelope. */
export function sanitizeUiContinuity(value: unknown): UiContinuity {
  const root = record(value);
  const drafts: Record<string, string> = {};
  const readMarks: Record<string, number> = {};
  for (const [key, item] of Object.entries(record(root.drafts)).slice(0, MAX_ENTRIES)) {
    if (key.length <= MAX_KEY_CHARS && typeof item === "string" && item.length <= MAX_DRAFT_CHARS) {
      drafts[key] = item;
    }
  }
  for (const [key, item] of Object.entries(record(root.readMarks)).slice(0, MAX_ENTRIES)) {
    if (key.length <= MAX_KEY_CHARS && typeof item === "number" && Number.isSafeInteger(item) && item >= 0) {
      readMarks[key] = item;
    }
  }
  return { version: 1, drafts, readMarks };
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
    const migrated = sanitizeUiContinuity({ drafts: current.drafts, readMarks: parsed });
    return { state: migrated, saveBeforeRemoval: true, removeLegacy: true };
  } catch {
    // Preserve malformed legacy data for diagnosis rather than deleting it under a migration claim.
    return { state: current, saveBeforeRemoval: false, removeLegacy: false };
  }
}
