/** Transfers tab: the view of one private download row (Transfers UX, PR #51).
 *
 * Pure presentation over the closed native projection (`PrivateDownload`) and the latest
 * saved-file check: the group a row sorts into, its tone, icon, chip word and one-line
 * explanation, the byte bar, where a saved file went, and the actions with file-specific
 * accessible names, disabled reasons and the inline remove confirmation. Nothing here reads a
 * path; native never sends one. Live progress of a save in flight (the renderer's own
 * `download-progress` record for this file) may replace the row's durable counts, so a save shows
 * once, in its row, instead of again in the live transfer list.
 *
 * Kept free of Svelte so the whole mapping is unit-tested. A later "keep finished saves listed"
 * only needs to feed finished saves in as rows; nothing here assumes saved rows are rare. */
import {
  privateDownloadActions, privateDownloadLabel,
  type PrivateDownload, type PrivateDownloadAction, type PrivateDownloadStatus, type SavedCheck,
} from "./private-downloads.ts";

/** 1000-based byte sizes, as the owner asked ("3.1 MB / 12.4 MB"): bytes below 1000, then one
 * decimal below 100 of a unit and none from 100 up, moving to the next unit before a value would
 * round to 1000. Non-finite or negative input reads as 0 B. */
export function formatDecimalBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const whole = Math.floor(bytes);
  if (whole < 1000) return `${whole} B`;
  const units = ["kB", "MB", "GB", "TB"];
  for (let i = 0; i < units.length; i++) {
    const value = whole / 1000 ** (i + 1);
    const last = i === units.length - 1;
    if (value < 99.95) return `${value.toFixed(1)} ${units[i]}`;
    if (value < 999.5 || last) return `${Math.round(value)} ${units[i]}`;
  }
  return "0 B"; // unreachable: the last unit always returns
}

/** The bar's visible label, "3.1 MB / 12.4 MB" (no percentage: the bar shows the share). */
export function byteLabel(done: number, total: number): string {
  return `${formatDecimalBytes(done)} / ${formatDecimalBytes(total)}`;
}
/** The same numbers for assistive technology, "3.1 MB of 12.4 MB". */
export function byteValueText(done: number, total: number): string {
  return `${formatDecimalBytes(done)} of ${formatDecimalBytes(total)}`;
}
/** "12 of 40 chunks". */
export function chunkLabel(done: number, total: number): string {
  return `${done} of ${total} chunks`;
}

/** One stroke shape of a 16x16 icon: path data, or a rounded rectangle. */
export type IconShape = { d: string } | { rect: { x: number; y: number; width: number; height: number; rx: number } };
/** The transfer state icons (design file `icons.md`), drawn for the wrapper
 * `viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"
 * stroke-linecap="round" stroke-linejoin="round"`. The dots (`h.01`, `v.01`) need the round caps.
 * The folder tabs are cat ears. Data, not markup, so no HTML is ever injected. */
export const TRANSFER_ICONS = {
  receiving: [{ d: "M8 2.5v8" }, { d: "M5 7.5l3 3 3-3" }, { d: "M2.5 10.5v2a1 1 0 0 0 1 1h9a1 1 0 0 0 1-1v-2" }],
  saving: [{ d: "M8 2v5.5" }, { d: "M5.5 5 8 7.5 10.5 5" }, { rect: { x: 1.5, y: 10, width: 13, height: 4, rx: 1 } }, { d: "M11.5 12h.01" }],
  paused: [{ d: "M5.5 3v10" }, { d: "M10.5 3v10" }],
  interrupted: [{ d: "M1.5 8h2.5" }, { d: "M12 8h2.5" }, { d: "M9.5 3.5l-3 4.5h3l-3 4.5" }],
  retry: [{ d: "M13 8a5 5 0 1 1-1.5-3.5" }, { d: "M11.5 2v2.5h-2.5" }],
  "verify-failed": [{ d: "M8 1.5 2.5 3v4.5c0 3.5 2.5 5.5 5.5 7 3-1.5 5.5-3.5 5.5-7V3z" }, { d: "M6 6l4 4M10 6l-4 4" }],
  "folder-unavailable": [{ d: "M1.5 13.5V4.5L3 2l1.5 2.5h1.5L7.5 2 9 4.5h5.5v9z" }, { d: "M2 14L14 2" }],
  "file-changed": [{ d: "M3.5 1.5h6l3 3v10h-9z" }, { d: "M9.5 1.5v3h3" }, { d: "M8 7v2.5" }, { d: "M8 12h.01" }],
  saved: [{ d: "M3.5 1.5h6l3 3v10h-9z" }, { d: "M9.5 1.5v3h3" }, { d: "M5.75 9.75l1.75 1.75 2.75-3.25" }],
  "saved-missing": [
    { d: "M9.5 1.5v3h3zM3.5 4.5v-3h3M3.5 11.5v3h3M12.5 11.5v3h-3M12.5 8v.01M3.5 8v.01" },
    { d: "M6.75 7.25a1.5 1.5 0 0 1 2.8.75c0 1-1.5 1.5-1.5 1.5" }, { d: "M8 12h.01" },
  ],
  "saved-replaced": [{ d: "M3.5 1.5h6l3 3v10h-9z" }, { d: "M9.5 1.5v3h3" }, { d: "M6 7h3M8 6l1 1-1 1" }, { d: "M10 11H6M7 10l-1 1 1 1" }],
  removing: [{ d: "M2.5 4h11" }, { d: "M6 4V2.5h4V4" }, { d: "M4 4v9.5h8V4" }, { d: "M6.5 7v4M9.5 7v4" }],
  "show-in-folder": [{ d: "M1.5 13.5V4.5L3 2l1.5 2.5h1.5L7.5 2 9 4.5h5.5v9z" }, { d: "M6 11l4-4" }, { d: "M7.5 7H10v2.5" }],
  ready: [{ rect: { x: 1.5, y: 2, width: 13, height: 3.5, rx: 1 } }, { d: "M3 5.5v7.5a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V5.5" }, { d: "M6 9.5l1.5 1.5 3-3" }],
  // Compact row controls (design file `compact.md`, section 4).
  play: [{ d: "M5.5 3.5v9l7.5-4.5z" }],
  pause: [{ d: "M5.5 3v10" }, { d: "M10.5 3v10" }],
  "cancel-x": [{ d: "M4 4l8 8M12 4l-8 8" }],
  more: [{ d: "M3.5 8h.01M8 8h.01M12.5 8h.01" }],
  "save-as": [{ d: "M1.5 13.5V4.5L3 2l1.5 2.5h1.5L7.5 2 9 4.5h5.5v9z" }, { d: "M8 7v4M6 9h4" }],
  "save-copy": [{ d: "M5.5 4.5V2.5h5l3 3v6h-2" }, { d: "M2.5 5.5h5l3 3v6h-8z" }, { d: "M7.5 5.5v3h3" }],
  clear: [{ d: "M9.5 2.5 13.5 6.5 7 13H3.5L2 11.5z" }, { d: "M5.5 7.5l3.5 3.5" }, { d: "M8.5 13h5" }],
  keep: [{ d: "M6 2.5h4l-.5 4 2 2.5H4.5l2-2.5z" }, { d: "M8 9v4.5" }],
} as const satisfies Record<string, readonly IconShape[]>;
export type TransferIcon = keyof typeof TRANSFER_ICONS;

export type SavedRowGroup = "needs" | "progress" | "saved";
export type SavedRowTone = "ok" | "warn" | "danger" | "transfer" | "accent" | "muted";
/** The groups in display order, with their headings. */
export const SAVED_ROW_GROUPS: readonly { id: SavedRowGroup; heading: string }[] = [
  { id: "needs", heading: "Needs you" },
  { id: "progress", heading: "In progress" },
  { id: "saved", heading: "Saved" },
];

/** The renderer's live record of a save in flight for this file, from native `download-progress`
 * events of the active invocation. Native emits those only while writing the readable file (the
 * journaled export and the non-resumable writer), never while the private copy is acquired, so
 * `done` and `bytesDone` count the saved side. The renderer passes one only after the first such
 * event, so a row shows "Saving" exactly while bytes are being written. `fresh`: the save starts a
 * new file (Save new copy, Save again, Save another copy, Save as on a row with a journal), so the
 * row's durable saved counts belong to the earlier file and are no floor for this one. `null` when
 * nothing is being written. */
export type SavedRowLive = { done: number; total: number; bytesDone: number; bytesTotal: number; fresh: boolean };

/** What else is going on for this row: an action of this row is pending (`busy`), or a save of
 * this file is running (`saving`). */
export type SavedRowActivity = { busy: boolean; saving: boolean };

export type SavedRowBar = {
  /** Which side the bar counts: bytes in the private copy, or bytes saved to the file. */
  kind: "received" | "saved";
  /** 0..1, from bytes when they are known, else from chunks. */
  fraction: number;
  /** "3.1 MB / 12.4 MB", or `null` when the byte counts are unknown (chunks only). */
  label: string | null;
  /** The progressbar's numbers: bytes when known, else chunks. */
  valueNow: number;
  valueMax: number;
  valueText: string;
  /** "Receiving report.pdf" / "Saving report.pdf". */
  accessibleName: string;
};

export type SavedRowAction = PrivateDownloadAction & {
  /** "Remove report.pdf": the label plus the file name, for assistive technology. */
  accessibleName: string;
  /** Pressing it first asks for confirmation inline (Remove, or Clear when the file is not
   * proven to be there). */
  confirm: boolean;
  disabled: boolean;
  /** Why it is disabled, for its title and description, or `null`. */
  disabledReason: string | null;
};

export type SavedRowView = {
  id: string;
  name: string;
  state: PrivateDownloadStatus;
  /** Saved rows only: the latest check of the saved file, or `null`. */
  check: SavedCheck | null;
  group: SavedRowGroup;
  tone: SavedRowTone;
  icon: TransferIcon;
  /** The chip's word; the icon is decoration. */
  word: string;
  /** The row's plain sentence, or `null`. Shown under the row only when `showWhy`. */
  why: string | null;
  /** Compact rows: the sentence is visible on NEEDS YOU rows only; elsewhere it is the status
   * symbol's tooltip (and hidden text for assistive technology). */
  showWhy: boolean;
  /** The status symbol's tooltip: the sentence on NEEDS YOU rows, a short line elsewhere. */
  statusTitle: string;
  bar: SavedRowBar | null;
  /** "12 of 40 chunks", alongside the bar. */
  chunks: string | null;
  /** Saved rows: where the file went, without naming a folder native did not prove. */
  where: string | null;
  /** Saved rows whose file may still be there: offer Show in folder. */
  canReveal: boolean;
  /** Saved rows whose file could not be checked. */
  hint: string | null;
  actions: SavedRowAction[];
  busy: boolean;
};

/** Sorting, tone and icon of each state. Every tone pairs with an icon and a word. */
const shape: Record<Exclude<PrivateDownloadStatus, "saved">, { group: SavedRowGroup; tone: SavedRowTone; icon: TransferIcon }> = {
  receiving: { group: "progress", tone: "transfer", icon: "receiving" },
  saving: { group: "progress", tone: "transfer", icon: "saving" },
  ready: { group: "progress", tone: "accent", icon: "ready" },
  paused: { group: "progress", tone: "muted", icon: "paused" },
  "save-paused": { group: "progress", tone: "muted", icon: "paused" },
  incomplete: { group: "needs", tone: "warn", icon: "interrupted" },
  exhausted: { group: "needs", tone: "warn", icon: "retry" },
  blocked: { group: "needs", tone: "danger", icon: "verify-failed" },
  "save-interrupted": { group: "needs", tone: "warn", icon: "interrupted" },
  "destination-unavailable": { group: "needs", tone: "warn", icon: "folder-unavailable" },
  "destination-changed": { group: "needs", tone: "danger", icon: "file-changed" },
  removing: { group: "needs", tone: "warn", icon: "removing" },
};
function savedShape(check: SavedCheck | null): { group: SavedRowGroup; tone: SavedRowTone; icon: TransferIcon; word: string } {
  switch (check) {
    case "missing": return { group: "needs", tone: "warn", icon: "saved-missing", word: "Saved file missing" };
    case "replaced": return { group: "needs", tone: "warn", icon: "saved-replaced", word: "Saved file replaced" };
    case "unknown": return { group: "saved", tone: "muted", icon: "saved", word: "Saved" };
    // Not checked yet reads "Saved", as before the check existed.
    default: return { group: "saved", tone: "ok", icon: "saved", word: "Saved" };
  }
}

/** One sentence per state, checked against what native does (see the design doc's Transfers UX
 * section). `busy` matters only for Removing: running now, or stuck from an earlier attempt.
 * `keepLabel` is the row's own Keep label, so the stuck sentence names the button it shows. */
function explanation(state: PrivateDownloadStatus, check: SavedCheck | null, busy: boolean, keepLabel: string): string | null {
  switch (state) {
    case "receiving": case "saving": return null;
    case "ready": return "The private encrypted copy is complete. Choose where to save the readable file.";
    case "paused": return "Paused. Resume download picks up where it left off.";
    case "save-paused": return "Saving is paused. Resume saving picks up from the last saved part.";
    // Also shown for a copy whose parts all arrived but whose final check did not finish.
    case "incomplete": return "The download stopped before it finished. Resume download continues it.";
    case "exhausted": return "It ran out of attempts to fetch the missing parts. Retry download tries again.";
    case "blocked": return "Some received data did not pass its check, so the download stopped. Resume download tries again.";
    case "save-interrupted": return "Saving stopped partway, for example when Mewtual closed or locked. Resume saving picks up from the last saved part.";
    // Compact rows (design `compact.md`): actions are icons with tooltips now, so the sentences
    // name them by verb, not as buttons to choose: Try again is the row's primary icon, Save new
    // copy is in its menu.
    case "destination-unavailable": return "The folder or the partly saved file cannot be reached right now. Reconnect it and try again, or save a new copy.";
    case "destination-changed": return "The partly saved file was changed by something else, so Mewtual stopped adding to it. Save new copy starts a separate file.";
    // Busy covers Remove, Clear and Keep alike, so the running sentence names none of them; the
    // stuck one names "a file it created" because a Clear removes only a temporary second name.
    // Remove (the red x) retries at once; Keep is in the menu, worded as the row's Keep label.
    case "removing": return busy
      ? "Working on it. The other actions wait until this one finishes."
      : `Mewtual could not finish removing a file it created for this save. Remove tries again, or ${keepLabel === "Keep the file" ? "keep the file" : "keep the partly saved file"}.`;
    case "saved":
      if (check === "missing") return "The saved file is no longer there. Save again makes a fresh copy.";
      if (check === "replaced") return "A different file is at the saved name now. Save again makes a fresh copy.";
      return null;
  }
}

/** The status symbol's tooltip on rows outside NEEDS YOU (design `compact.md`, section 1): the
 * row's own sentence where it has one (ready, paused, saving paused), else a short line. */
function statusLine(state: PrivateDownloadStatus, check: SavedCheck | null, why: string | null, word: string): string {
  switch (state) {
    case "receiving": return "Receiving the encrypted copy.";
    case "saving": return "Writing the readable file.";
    case "saved":
      if (check === "present") return "Saved. The file was there when Mewtual last checked.";
      if (check === "unknown") return "Saved. Mewtual could not check the file just now.";
      return "Saved. Mewtual has not checked the file yet.";
    default: return why ?? `${word}.`;
  }
}

/** An action of a saved-files list row (PR #56): Save again (the file is gone from its name) or
 * Save another copy (it may still be there), both the ordinary save and offered only while the
 * file is still shared in the server; or Clear, which forgets the list entry and never touches a
 * file, so it never asks first. */
export type SavedListRowAction = {
  kind: "again" | "forget";
  label: string;
  /** "Clear report.pdf", for assistive technology. */
  accessibleName: string;
  disabled: boolean;
  disabledReason: string | null;
};

/** One control of a compact row: an action drawn as an icon, or Show in folder. `A` is the row's
 * action type: a private download row's [`SavedRowAction`], or a list row's
 * [`SavedListRowAction`]. */
export type CompactControl<A = SavedRowAction> =
  | { kind: "action"; action: A; icon: TransferIcon; look: "go" | "stop" | "" }
  | { kind: "reveal"; icon: "show-in-folder"; look: ""; label: string; accessibleName: string };
/** The compact row's controls (design `compact.md`, section 5.1): the primary icon left of the
 * chip, the red x, and the "more" menu in order. */
export type CompactControls<A = SavedRowAction> = {
  /** The icon left of the chip, or `null` (Removing: the x is its retry). */
  primary: CompactControl<A> | null;
  /** The red x: the row's remove-kind action (Remove or Clear), or `null`. */
  remove: A | null;
  /** The "more" menu in order; empty means the row has no More button. */
  menu: (CompactControl<A> | { kind: "separator" })[];
};

/** Menu icon per action kind. */
const MENU_ICONS: Record<SavedRowAction["kind"], TransferIcon> = {
  saveAs: "save-as", fresh: "save-copy", keep: "keep", resume: "play", pause: "pause", remove: "clear",
};

/** Arrange a row's actions for the compact row. Every action of the view appears exactly once:
 * the remove-kind action as the x, one primary (the first of pause, resume or fresh; Show in
 * folder on a Saved row in the SAVED group), everything else in the menu in the view's order,
 * then Show in folder after a separator when the row offers it and it is not the primary. */
export function compactControls(view: SavedRowView): CompactControls {
  const remove = view.actions.find((action) => action.kind === "remove") ?? null;
  const reveal: CompactControl = {
    kind: "reveal", icon: "show-in-folder", look: "", label: "Show in folder",
    accessibleName: `Show ${view.name} in folder`,
  };
  const others = view.actions.filter((action) => action.kind !== "remove");
  let primary: CompactControl | null = null;
  if (view.state === "saved" && view.group === "saved") {
    primary = view.canReveal ? reveal : null;
  } else if (view.state !== "removing") {
    const first = others.find((action) => action.kind === "pause" || action.kind === "resume" || action.kind === "fresh");
    if (first) primary = { kind: "action", action: first, ...primaryLook(view, first) };
  }
  const menu: CompactControls["menu"] = others
    .filter((action) => primary?.kind !== "action" || action !== primary.action)
    .map((action): CompactControl => ({ kind: "action", action, icon: MENU_ICONS[action.kind], look: "" }));
  if (view.canReveal && primary?.kind !== "reveal") {
    if (menu.length > 0) menu.push({ kind: "separator" });
    menu.push(reveal);
  }
  return { primary, remove, menu };
}

/** Arrange a saved-files list row (PR #56) as a compact row, by the same rules as a Saved private
 * row ([`compactControls`]): a file gone from its name (`gone`) gets Save again as the primary,
 * like a Saved row whose file is missing; otherwise Show in folder is the primary while the file
 * may still be at its name, and Save another copy goes in the menu. Clear (forget the entry) is the
 * red x. Every action appears exactly once; a row native cannot check offers no Show in folder. */
export function compactListControls(view: {
  name: string;
  canReveal: boolean;
  gone: boolean;
  actions: readonly SavedListRowAction[];
}): CompactControls<SavedListRowAction> {
  const remove = view.actions.find((action) => action.kind === "forget") ?? null;
  const again = view.actions.find((action) => action.kind === "again") ?? null;
  let primary: CompactControl<SavedListRowAction> | null = null;
  if (view.gone && again) {
    primary = { kind: "action", action: again, icon: "saving", look: "go" };
  } else if (view.canReveal) {
    primary = { kind: "reveal", icon: "show-in-folder", look: "", label: "Show in folder", accessibleName: `Show ${view.name} in folder` };
  }
  const menu: CompactControls<SavedListRowAction>["menu"] = view.actions
    .filter((action) => action.kind !== "forget" && (primary?.kind !== "action" || action !== primary.action))
    .map((action) => ({ kind: "action", action, icon: "save-copy", look: "" }));
  return { primary, remove, menu };
}

/** The primary icon and look (design `compact.md`, section 5.1, rule 3). */
function primaryLook(view: SavedRowView, action: SavedRowAction): { icon: TransferIcon; look: "go" | "stop" } {
  if (action.kind === "pause") return { icon: "pause", look: "stop" };
  if (action.kind === "fresh") return { icon: view.state === "destination-changed" ? "save-copy" : "saving", look: "go" };
  if (view.state === "exhausted" || view.state === "destination-unavailable") return { icon: "retry", look: "go" };
  if (view.state === "ready") return { icon: "saving", look: "go" };
  return { icon: "play", look: "go" };
}

/** States whose bar counts the private copy; the save-side states count the saved file. */
const receivingSide = new Set<PrivateDownloadStatus>(["receiving", "incomplete", "paused", "exhausted", "blocked"]);
const savingSide = new Set<PrivateDownloadStatus>([
  "saving", "save-interrupted", "save-paused", "destination-unavailable", "destination-changed",
]);

/** `state` is the state the row shows: native's, or "saving" while a save is being written (the
 * list may not have refreshed since it began). */
function bar(row: PrivateDownload, state: PrivateDownloadStatus, name: string, live: SavedRowLive | null): { bar: SavedRowBar | null; chunks: string | null } {
  const received = receivingSide.has(state);
  if (!received && !savingSide.has(state)) return { bar: null, chunks: null };
  const kind = received ? "received" : "saved";
  // Live numbers count the saved side, never below what native already recorded durably for the
  // same file (both only grow during one save); a fresh copy has no such floor.
  const progress = received ? null : live;
  const floor = !progress?.fresh;
  const totalChunks = row.totalChunks;
  const durableChunks = !floor ? 0 : received ? row.verifiedChunks : row.savedChunks;
  const doneChunks = Math.min(totalChunks, Math.max(durableChunks, progress ? Math.floor(progress.done) : 0));
  const totalBytes = row.totalBytes ?? (progress && progress.bytesTotal > 0 ? progress.bytesTotal : null);
  const durableBytes = !floor ? 0 : received ? row.receivedBytes : row.savedBytes;
  const doneBytes = totalBytes === null ? null
    : Math.min(totalBytes, Math.max(durableBytes ?? 0, progress ? Math.max(0, progress.bytesDone) : 0));
  const bytesKnown = doneBytes !== null && totalBytes !== null;
  const fraction = bytesKnown && (totalBytes as number) > 0
    ? (doneBytes as number) / (totalBytes as number)
    : doneChunks / totalChunks;
  return {
    bar: {
      kind,
      fraction: Math.min(1, Math.max(0, fraction)),
      label: bytesKnown ? byteLabel(doneBytes as number, totalBytes as number) : null,
      valueNow: bytesKnown ? doneBytes as number : doneChunks,
      valueMax: bytesKnown ? totalBytes as number : totalChunks,
      valueText: bytesKnown ? byteValueText(doneBytes as number, totalBytes as number) : chunkLabel(doneChunks, totalChunks),
      accessibleName: `${kind === "received" ? "Receiving" : "Saving"} ${name}`,
    },
    chunks: chunkLabel(doneChunks, totalChunks),
  };
}

const BUSY_REASON = "Another action on this file is running";
const SAVING_REASON = "A save of this file is running";

/** The whole view of one row. `name` is the listed file name (or a short fallback); `live` the
 * active save's progress for this file, if any. */
export function savedRowView(
  row: PrivateDownload,
  check: SavedCheck | null,
  name: string,
  live: SavedRowLive | null,
  activity: SavedRowActivity,
): SavedRowView {
  // While bytes are being written the row says "Saving", whatever native last reported (ready,
  // receiving, a halt, or saved for Save another copy): the list refreshes only on journal steps,
  // and the non-resumable writer has none. Removing keeps its own state.
  const state: PrivateDownloadStatus = live && row.status !== "removing" ? "saving" : row.status;
  const saved = state === "saved";
  const look = state === "saved" ? savedShape(check)
    : { ...shape[state], word: privateDownloadLabel({ ...row, status: state }) };
  const gone = saved && (check === "missing" || check === "replaced");
  // Only what native proved: Downloads, or another folder; `null` (not proven either way) names
  // no folder at all.
  const where = !saved || row.savedTo === null ? null
    : `${gone ? "Was saved" : "Saved"} to ${row.savedTo === "downloads" ? "Downloads" : "another folder"}`;
  const { bar: progress, chunks } = bar(row, state, name, live);
  const actions = privateDownloadActions(row, saved ? check : null).map((action): SavedRowAction => {
    const disabledReason = activity.busy ? BUSY_REASON
      : action.needsIdle && activity.saving ? SAVING_REASON : null;
    return {
      ...action,
      accessibleName: `${action.label} ${name}`,
      // Remove deletes saved progress and any partly saved file Mewtual created. Clear on a
      // saved row deletes the private copy and only a temporary second name of the saved file,
      // so it asks first unless the saved file was just found where it was saved. On a Removing
      // row the x retries a removal the user already confirmed, so it does not ask again
      // (compact rows, design `compact.md`).
      confirm: action.kind === "remove" && state !== "removing" && !(saved && check === "present"),
      disabled: disabledReason !== null,
      disabledReason,
    };
  });
  const why = explanation(state, saved ? check : null, activity.busy,
    actions.find((action) => action.kind === "keep")?.label ?? "Keep partly saved file");
  return {
    id: row.id,
    name,
    state,
    check: saved ? check : null,
    group: look.group,
    tone: look.tone,
    icon: look.icon,
    word: look.word,
    why,
    showWhy: look.group === "needs" && why !== null,
    statusTitle: look.group === "needs" && why !== null ? why : statusLine(state, saved ? check : null, why, look.word),
    bar: progress,
    chunks,
    where,
    canReveal: saved && !gone,
    hint: saved && check === "unknown" ? "Could not check the file" : null,
    actions,
    busy: activity.busy,
  };
}

/** The rows by group, each in the order given (the list's own order). */
export function groupSavedRows(views: readonly SavedRowView[]): Record<SavedRowGroup, SavedRowView[]> {
  const groups: Record<SavedRowGroup, SavedRowView[]> = { needs: [], progress: [], saved: [] };
  for (const view of views) groups[view.group].push(view);
  return groups;
}

/** The confirmation pair's words for a row's Remove or Clear action. */
export function confirmWords(action: SavedRowAction, name: string): { confirm: string; confirmName: string; cancelName: string; group: string } {
  const clear = action.label === "Clear";
  return {
    confirm: clear ? "Confirm clear" : "Confirm remove",
    confirmName: `${clear ? "Confirm clear" : "Confirm remove"} ${name}`,
    cancelName: `Cancel ${clear ? "clearing" : "removing"} ${name}`,
    group: clear ? "Confirm clearing" : "Confirm removal",
  };
}

/** State transitions to announce politely, one sentence per row whose chip word changed since
 * the previous rendering ("report.pdf: Saved"). Chunk ticks never change the word, so a running
 * download is not read out repeatedly. The first rendering (no previous words) announces nothing.
 * Returns the announcement and the words to remember. */
export function savedRowTransitions(
  previous: ReadonlyMap<string, string> | null,
  views: readonly SavedRowView[],
): { announcement: string; words: Map<string, string> } {
  const words = new Map(views.map((view) => [view.id, view.word]));
  if (previous === null) return { announcement: "", words };
  const changed = views
    .filter((view) => previous.has(view.id) && previous.get(view.id) !== view.word)
    .map((view) => `${view.name}: ${view.word}.`);
  return { announcement: changed.join(" "), words };
}
