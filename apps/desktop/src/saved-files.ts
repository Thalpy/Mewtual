/** The saved-files list (PR #56, `docs/design-saved-files-list.md`): the renderer's side.
 *
 * Native keeps a sealed per-server list of finished saves and sends only display data: an entry
 * ID (the handle every action names), the file's CID, the saved file's own name, when it was
 * saved, its size, whether it went to the Downloads folder, and whether native can check it. No
 * folder, path or file identity ever reaches here, so nothing in this module can name one.
 *
 * Pure presentation, kept free of Svelte so it is unit-tested: closed validation of native's
 * answers, the Transfers row of an entry (reusing the `.sdl` row design and icons of PR #51), how
 * list rows and private Download rows share the Saved group without duplicating a file, the chat
 * mark, and the "Replace my older saved copy" preference. */
import type { PrivateDownload, SavedCheck } from "./private-downloads.ts";
import type { SavedRowTone, TransferIcon } from "./transfer-rows.ts";

/** One finished save as native projects it. */
export type SavedFile = {
  /** 32 hex digits, random; names the entry for Show in folder and Clear. */
  id: string;
  /** The file's plaintext CID, 64 hex digits. */
  cid: string;
  /** The saved file's own name (for example `report (1).pdf`), never a path. */
  name: string;
  /** Wall-clock milliseconds when the save finished. */
  savedAt: number;
  /** Plaintext bytes. */
  size: number;
  /** Native proved the file went straight into the resolved Downloads folder (`"downloads"`), or
   * somewhere else (`"other"`); `null` when it cannot tell. Never a folder name. */
  savedTo: "downloads" | "other" | null;
  /** Native can check this file (the save was journaled, so it has an identity to compare). */
  checkable: boolean;
};
/** Whether native could read the list: `ok`, or why not. `newer`: a newer Mewtual wrote it (left
 * alone). `damaged`: it can be started anew. `unavailable`: it could not be read right now. Only
 * `ok` carries files; otherwise nothing is recorded until it can be read again. */
export type SavedListState = "ok" | "newer" | "damaged" | "unavailable";
export type SavedFilesList = { state: SavedListState; files: SavedFile[] };
const listStates = new Set<string>(["ok", "newer", "damaged", "unavailable"]);

/** Native's caps, mirrored for validation only (native enforces them). A list holds at most the
 * largest "Remember saved files" setting. */
export const MAX_SAVED_FILES = 4096;
const MAX_NAME_LENGTH = 1024;
const MAX_FILE_BYTES = 1024 * 1024 * 1024;

const unreadable = () => new Error("The list of saved files could not be read.");

/** A file name native could have recorded: non-empty, bounded, one component, no control
 * characters. Anything else is an unreadable answer, never displayed. */
function plausibleName(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= MAX_NAME_LENGTH &&
    !/[\\/\u0000-\u001f\u007f]/.test(value) && value !== "." && value !== "..";
}

/** Closed validation of `get_saved_files`: anything unexpected is an error, and only the known
 * fields survive (unexpected native properties never become renderer-held state). */
export function checkedSavedFiles(value: unknown): SavedFilesList {
  if (!value || typeof value !== "object") throw unreadable();
  const { state, files } = value as { state?: unknown; files?: unknown };
  if (typeof state !== "string" || !listStates.has(state) || !Array.isArray(files) || files.length > MAX_SAVED_FILES) {
    throw unreadable();
  }
  if (state !== "ok" && files.length > 0) throw unreadable();
  // IDs are unique; a CID may repeat (two saves of one file are two copies, design review M1).
  const ids = new Set<string>();
  const checked = files.map((file): SavedFile => {
    if (!file || typeof file !== "object") throw unreadable();
    const row = file as Record<string, unknown>;
    if (typeof row.id !== "string" || !/^[0-9a-f]{32}$/.test(row.id) || ids.has(row.id) ||
        typeof row.cid !== "string" || !/^[0-9a-f]{64}$/.test(row.cid) ||
        !plausibleName(row.name) ||
        !Number.isSafeInteger(row.savedAt) || (row.savedAt as number) < 0 ||
        !Number.isSafeInteger(row.size) || (row.size as number) < 0 || (row.size as number) > MAX_FILE_BYTES ||
        !(row.savedTo === null || row.savedTo === "downloads" || row.savedTo === "other") ||
        typeof row.checkable !== "boolean") {
      throw unreadable();
    }
    ids.add(row.id);
    return { id: row.id, cid: row.cid, name: row.name, savedAt: row.savedAt as number, size: row.size as number,
      savedTo: row.savedTo, checkable: row.checkable };
  });
  return { state: state as SavedListState, files: checked };
}

/** What Transfers says when the list cannot be read, or `null` when it can. */
export function savedListNotice(state: SavedListState): string | null {
  switch (state) {
    case "ok": return null;
    case "newer": return "A newer version of Mewtual wrote the list of saved files on this device, so this version leaves it as it is and lists no saves.";
    case "damaged": return "Mewtual could not read its list of saved files on this device, so it lists no saves. Starting a new list forgets the old one; the saved files stay where they are.";
    case "unavailable": return "Mewtual could not read its list of saved files right now. It tries again next time.";
  }
}

/** One answer of `check_saved_files`: the entry and name it was made for, and what native found. */
export type SavedFileCheck = { id: string; name: string; check: SavedCheck };
const checks = new Set<string>(["present", "missing", "replaced", "unknown"]);
/** Closed validation of `check_saved_files`. */
export function checkedSavedFileChecks(value: unknown): SavedFileCheck[] {
  if (!Array.isArray(value) || value.length > MAX_SAVED_FILES) throw new Error("The saved file checks could not be read.");
  const seen = new Set<string>();
  return value.map((answer) => {
    if (!answer || typeof answer !== "object") throw new Error("The saved file checks could not be read.");
    const row = answer as Record<string, unknown>;
    if (typeof row.id !== "string" || !/^[0-9a-f]{32}$/.test(row.id) || seen.has(row.id) ||
        !plausibleName(row.name) || typeof row.check !== "string" || !checks.has(row.check)) {
      throw new Error("The saved file checks could not be read.");
    }
    seen.add(row.id);
    return { id: row.id, name: row.name, check: row.check as SavedCheck };
  });
}

/** A check belongs to an entry as it was when checked: the same ID and the same file name. After
 * "Replace my older saved copy" moves a file to its old name, the name changes and an older
 * answer no longer applies. */
export function savedFileCheckKey(file: { id: string; name: string }): string {
  return `${file.id}:${file.name}`;
}

/** The list rows to draw: newest first, leaving out exactly the entries a Saved private row (a
 * Published record left by an interrupted tidy-up) stands for. Native names that entry by
 * identity (`savedEntry`), so another copy of the same file keeps its own row. That private row
 * keeps its own actions; clearing it also forgets its entry natively. */
export function savedListRows(list: SavedFilesList, privateRows: readonly PrivateDownload[]): SavedFile[] {
  const covered = new Set(privateRows.filter((row) => row.status === "saved" && row.savedEntry !== null)
    .map((row) => row.savedEntry));
  return list.files.filter((file) => !covered.has(file.id)).sort((a, b) => b.savedAt - a.savedAt);
}

/** CIDs with an entry, for the chat mark and the live list filter. */
export function savedCidSet(list: SavedFilesList): Set<string> {
  return new Set(list.files.map((file) => file.cid));
}

/** The live transfer list shows a finished download once: a download that finished (`done`) and
 * whose file now has an entry is already the Saved row, so the live list leaves it out. Running,
 * waiting or failed transfers stay, and uploads are never affected. */
export function liveTransferShown(
  transfer: { direction: string; cid?: string; status: string },
  savedCids: ReadonlySet<string>,
): boolean {
  return !(transfer.direction === "download" && transfer.status === "done" &&
    transfer.cid !== undefined && savedCids.has(transfer.cid));
}

export type SavedListActionKind = "again" | "forget";
export type SavedListAction = {
  kind: SavedListActionKind;
  label: string;
  /** "Clear report.pdf", for assistive technology. */
  accessibleName: string;
  ghost: boolean;
  disabled: boolean;
  disabledReason: string | null;
};
/** One list row, shaped like a PR #51 Saved row so the same markup and styles apply. */
export type SavedListRowView = {
  id: string;
  cid: string;
  name: string;
  check: SavedCheck | null;
  tone: SavedRowTone;
  icon: TransferIcon;
  word: string;
  why: string | null;
  /** "Saved to Downloads, 26 Oct", without naming a folder native did not prove. */
  where: string;
  /** Offer Show in folder: checkable and not known to be gone. */
  canReveal: boolean;
  hint: string | null;
  actions: SavedListAction[];
  busy: boolean;
};

const BUSY_REASON = "Another action on this file is running";
const SAVING_REASON = "A save of this file is running";

/** The view of one entry. `check` is the latest answer for this entry and name (or `null`),
 * `listed` whether the file is still listed in the server (Save again needs that), `day` formats
 * `savedAt` for the where line (the caller's locale), and `activity` what else runs for it. */
export function savedListRowView(
  file: SavedFile,
  check: SavedCheck | null,
  listed: boolean,
  day: (ms: number) => string,
  activity: { busy: boolean; saving: boolean },
): SavedListRowView {
  const gone = file.checkable && (check === "missing" || check === "replaced");
  const word = check === "missing" && file.checkable ? "Saved file missing"
    : check === "replaced" && file.checkable ? "Saved file replaced" : "Saved";
  const icon: TransferIcon = !file.checkable ? "saved"
    : check === "missing" ? "saved-missing" : check === "replaced" ? "saved-replaced" : "saved";
  const tone: SavedRowTone = gone ? "warn" : !file.checkable || check === "unknown" ? "muted" : "ok";
  const why = !file.checkable ? null
    : check === "missing" ? "The saved file is no longer there. Save again makes a fresh copy."
      : check === "replaced" ? "A different file is at the saved name now. Save again makes a fresh copy." : null;
  const place = file.savedTo === "downloads" ? " to Downloads" : file.savedTo === "other" ? " to another folder" : "";
  const where = `${gone ? "Was saved" : "Saved"}${place}, ${day(file.savedAt)}`;
  const hint = !file.checkable ? "Mewtual cannot check files in that folder"
    : check === "unknown" ? "Could not check the file" : null;
  const reason = (needsIdle: boolean) => activity.busy ? BUSY_REASON : needsIdle && activity.saving ? SAVING_REASON : null;
  const actions: SavedListAction[] = [];
  if (gone && listed) {
    const disabledReason = reason(true);
    actions.push({ kind: "again", label: "Save again", accessibleName: `Save again ${file.name}`, ghost: false,
      disabled: disabledReason !== null, disabledReason });
  }
  const forgetReason = reason(false);
  actions.push({ kind: "forget", label: "Clear", accessibleName: `Clear ${file.name}`, ghost: true,
    disabled: forgetReason !== null, disabledReason: forgetReason });
  return {
    id: file.id, cid: file.cid, name: file.name, check, tone, icon, word, why, where,
    canReveal: file.checkable && !gone, hint, actions, busy: activity.busy,
  };
}

/** The "already saved" mark on a file in chat, the Files list and the file details. It comes from
 * the loaded list only (no network, no disk), so it says what this device recorded, not whether
 * the file is still there. */
export const SAVED_MARK_LABEL = "Saved";
export const SAVED_MARK_TITLE = "You saved this file on this device. Transfers shows where.";

/** "Replace my older saved copy" (off by default), a device preference in `localStorage`. */
export const REPLACE_OLDER_KEY = "catcoms.saves.replaceOlder";
export const REPLACE_OLDER_LABEL = "Replace my older saved copy";
/** The owner's help text, without an absolute "never" (design review M3(g): Unix has a documented
 * same-account window, so the guarantee is stated as what Mewtual checks). */
export const REPLACE_OLDER_HELP =
  "When you save a file again, Mewtual replaces the copy it saved before, if that copy is still unchanged. " +
  "It replaces only a copy it can prove it saved. Applies on this device, to every server.";
/** Only the exact stored word turns it on; anything else (missing, damaged, an old value) is off. */
export function replaceOlderFrom(stored: string | null): boolean {
  return stored === "on";
}

/** "Remember saved files": how many saves one server's list keeps on this device (Server
 * settings, Storage). Native stores it sealed per server and clamps whatever is sent into
 * `min..=max`. `limit` is `null` while the stored setting cannot be read (`state: "unreadable"`),
 * `listed` while the list cannot be read. */
export type SavedFilesLimit = {
  state: "ok" | "unreadable";
  limit: number | null;
  listed: number | null;
  min: number;
  max: number;
  default: number;
};
/** What a choice would do (`applied: false`, a preview) or did. */
export type SavedLimitChange = { limit: number; forgets: number; applied: boolean };

/** The choices offered: powers of two across native's range, the default among them. */
export const SAVED_LIMIT_CHOICES: readonly number[] = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096];
export const SAVED_LIMIT_LABEL = "Remember saved files";
export const SAVED_LIMIT_HELP =
  "How many of your saves Transfers lists for this server on this device. " +
  "Lowering it forgets the oldest entries from the list; the saved files stay where they are.";

const limitUnreadable = () => new Error("The saved files setting could not be read.");
const count = (value: unknown, max: number) => Number.isSafeInteger(value) && (value as number) >= 0 && (value as number) <= max;

/** Closed validation of `get_saved_files_limit`. */
export function checkedSavedFilesLimit(value: unknown): SavedFilesLimit {
  if (!value || typeof value !== "object") throw limitUnreadable();
  const row = value as Record<string, unknown>;
  const { state, limit, listed, min, max } = row;
  const fallback = row.default;
  if ((state !== "ok" && state !== "unreadable") ||
      !count(min, MAX_SAVED_FILES) || !count(max, MAX_SAVED_FILES) || !count(fallback, MAX_SAVED_FILES) ||
      (min as number) < 1 || (min as number) > (fallback as number) || (fallback as number) > (max as number) ||
      !(listed === null || count(listed, MAX_SAVED_FILES)) ||
      (state === "ok" ? !count(limit, max as number) || (limit as number) < (min as number) : limit !== null)) {
    throw limitUnreadable();
  }
  return { state: state as SavedFilesLimit["state"], limit: limit as number | null,
    listed: listed as number | null, min: min as number,
    max: max as number, default: fallback as number };
}

/** Closed validation of `set_saved_files_limit`. */
export function checkedSavedLimitChange(value: unknown): SavedLimitChange {
  if (!value || typeof value !== "object") throw limitUnreadable();
  const { limit, forgets, applied } = value as Record<string, unknown>;
  if (!count(limit, MAX_SAVED_FILES) || (limit as number) < 1 || !count(forgets, MAX_SAVED_FILES) ||
      typeof applied !== "boolean") {
    throw limitUnreadable();
  }
  return { limit: limit as number, forgets: forgets as number, applied };
}

/** "256 files" (the default is marked, so the user can find it again). */
export function savedLimitLabel(limit: number, fallback: number): string {
  return `${limit} files${limit === fallback ? " (default)" : ""}`;
}

/** Said before the user confirms a lower setting: what the list forgets, and that files stay. */
export function savedLimitWarning(forgets: number): string {
  const entries = forgets === 1 ? "the oldest saved file" : `the ${forgets} oldest saved files`;
  return `This forgets ${entries} from the list. The files themselves stay where they are.`;
}

/** The toast after a change. */
export function savedLimitDone(change: SavedLimitChange): string {
  const forgot = change.forgets === 0 ? "" : change.forgets === 1 ? " Forgot the oldest entry."
    : ` Forgot the ${change.forgets} oldest entries.`;
  return `Remembering ${change.limit} saved files on this device.${forgot}`;
}
