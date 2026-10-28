/** Visual fixture for the Transfers tab (Transfers UX, PR #51): one private download row in every
 * state the row can show, opted in with `?fixture=chat&transfers=1`. Development aid only, like
 * the rest of the fixture: deterministic data, no paths, and a saved-file check answer per row so
 * the saved variants (present, missing, replaced, could not check) all render. */

type FixtureRow = {
  id: string; cid: string; intent: string; grant: string; status: string;
  verifiedChunks: number; savedChunks: number; totalChunks: number; chosenPlace: boolean;
  receivedBytes: number | null; savedBytes: number | null; totalBytes: number | null;
  savedTo: "downloads" | "other" | null;
};

const MB = 1_000_000;
/** A row with chunk counts derived from 8 MiB chunks of `total` bytes. */
function fixtureRow(
  n: number,
  status: string,
  total: number,
  received: number,
  saved: number,
  extra: Partial<FixtureRow> = {},
): FixtureRow {
  [total, received, saved] = [total, received, saved].map(Math.round);
  const chunk = 8 * 1024 * 1024;
  const totalChunks = Math.max(1, Math.ceil(total / chunk));
  const chunks = (bytes: number) => Math.min(totalChunks, Math.floor(bytes / chunk));
  return {
    id: n.toString(16).padStart(2, "0").repeat(32),
    cid: (n + 0x80).toString(16).padStart(2, "0").repeat(32),
    intent: String(n + 1),
    grant: "1",
    status,
    verifiedChunks: received >= total ? totalChunks : chunks(received),
    savedChunks: saved >= total ? totalChunks : chunks(saved),
    totalChunks,
    chosenPlace: false,
    receivedBytes: received,
    savedBytes: saved,
    totalBytes: total,
    savedTo: status === "saved" ? "downloads" : null,
    ...extra,
  };
}

/** Every row state, with the file names the file list shows for them. */
export const VISUAL_TRANSFER_ROWS: readonly { row: FixtureRow; name: string; check?: string }[] = [
  { row: fixtureRow(1, "incomplete", 124 * MB, 31 * MB, 0), name: "quarterly-figures-final-v3.xlsx" },
  { row: fixtureRow(2, "exhausted", 88 * MB, 61.6 * MB, 0), name: "holiday-photos.zip" },
  { row: fixtureRow(3, "blocked", 52 * MB, 41.6 * MB, 0), name: "firmware-2.4.1.bin" },
  { row: fixtureRow(4, "save-interrupted", 410 * MB, 410 * MB, 152 * MB), name: "lecture-07.mp4" },
  { row: fixtureRow(5, "destination-unavailable", 980 * MB, 980 * MB, 588 * MB), name: "backup-2026-10.tar" },
  { row: fixtureRow(6, "destination-changed", 640 * MB, 640 * MB, 288 * MB), name: "dataset.parquet" },
  { row: fixtureRow(7, "saved", 3.4 * MB, 3.4 * MB, 3.4 * MB), name: "invoice-0931.pdf", check: "missing" },
  { row: fixtureRow(8, "saved", 0.82 * MB, 0.82 * MB, 0.82 * MB, { savedTo: "other" }), name: "team-roster.csv", check: "replaced" },
  { row: fixtureRow(9, "removing", 300 * MB, 300 * MB, 120 * MB), name: "raw-footage-day2.mov" },
  { row: fixtureRow(10, "receiving", 12.4 * MB, 3.1 * MB, 0), name: "report.pdf" },
  { row: fixtureRow(11, "saving", 96 * MB, 96 * MB, 81.6 * MB), name: "design-system-tokens-and-components-export.sketch" },
  { row: fixtureRow(12, "ready", 1.2 * MB, 1.2 * MB, 0, { chosenPlace: true }), name: "minutes-2026-10-23.docx" },
  { row: fixtureRow(13, "paused", 54 * MB, 27 * MB, 0), name: "podcast-ep-112.m4a" },
  { row: fixtureRow(14, "save-paused", 30 * MB, 30 * MB, 3 * MB), name: "site-export.zip" },
  { row: fixtureRow(15, "saved", 12.4 * MB, 12.4 * MB, 12.4 * MB), name: "report-final.pdf", check: "present" },
  { row: fixtureRow(16, "saved", 0.004 * MB, 0.004 * MB, 0.004 * MB, { savedTo: "other" }), name: "notes.md", check: "unknown" },
];

/** PR #56: the saved-files list entries the Transfers fixture shows in its Saved group, one per
 * list-row state, with the check answer each gets and whether the file is still listed. The
 * fixture's clock is fixed, so `savedAt` is a fixed instant. Display data only, never a folder. */
export const VISUAL_SAVED_FILES: readonly {
  file: { id: string; cid: string; name: string; savedAt: number; size: number; savedTo: "downloads" | "other" | null; checkable: boolean };
  check?: "present" | "missing" | "replaced" | "unknown";
}[] = [
  { file: savedEntry(1, "brand-kit-2026.zip", 41.2 * MB, 9, "downloads", true), check: "present" },
  { file: savedEntry(2, "meeting-notes-oct.docx", 0.3 * MB, 8, "downloads", true), check: "missing" },
  { file: savedEntry(3, "budget-draft.xlsx", 1.6 * MB, 7, "other", true), check: "replaced" },
  { file: savedEntry(4, "clip-final.mp4", 212 * MB, 6, "downloads", true), check: "unknown" },
  { file: savedEntry(5, "photo-dump.zip", 860 * MB, 5, "other", false) },
];
function savedEntry(
  n: number, name: string, size: number, daysAgo: number, savedTo: "downloads" | "other" | null, checkable: boolean,
) {
  return {
    id: (n + 0x40).toString(16).padStart(2, "0").repeat(16),
    cid: (n + 0xc0).toString(16).padStart(2, "0").repeat(32),
    name,
    // 23 October 2026 noon UTC minus whole days: a fixed, readable date in every time zone.
    savedAt: Date.UTC(2026, 9, 23, 12) - daysAgo * 86_400_000,
    size: Math.round(size),
    savedTo,
    checkable,
  };
}

/** The Transfers fixture's answers, or `{ handled: false }` for commands it leaves to the chat
 * fixture. `files` is the chat fixture's own file list response, extended with these files. */
export function visualTransfersResponse(
  command: string,
  args: Record<string, unknown>,
  files: () => { has_peers: boolean; files: Array<Record<string, unknown>> },
): { handled: true; value: unknown } | { handled: false } {
  switch (command) {
    case "get_private_downloads":
      return { handled: true, value: VISUAL_TRANSFER_ROWS.map(({ row }) => structuredClone(row)) };
    case "check_saved_download": {
      const entry = VISUAL_TRANSFER_ROWS.find(({ row }) => row.id === args.download);
      return { handled: true, value: entry?.check ?? "present" };
    }
    case "reveal_saved_download":
      return { handled: true, value: null };
    // PR #56: the saved-files list, its bounded check (one answer per checkable entry), and the
    // list actions, which change nothing in the fixture.
    case "get_saved_files":
      return { handled: true, value: { state: "ok", files: VISUAL_SAVED_FILES.map(({ file }) => ({ ...file })) } };
    case "check_saved_files":
      return {
        handled: true,
        value: VISUAL_SAVED_FILES.filter(({ file, check }) => file.checkable && check)
          .map(({ file, check }) => ({ id: file.id, name: file.name, check })),
      };
    case "reveal_saved_file":
    case "forget_saved_file":
    case "reset_saved_files":
      return { handled: true, value: null };
    case "get_files": {
      const list = files();
      const template = list.files[0];
      return {
        handled: true,
        value: {
          ...list,
          files: [
            ...list.files,
            ...VISUAL_TRANSFER_ROWS.map(({ row, name }) => ({
              ...template, name, cid: row.cid, path: "", size: row.totalBytes ?? 0,
              mime: "application/octet-stream", held: row.verifiedChunks, total: row.totalChunks,
            })),
            // The saved files are still listed (so Save again is offered), except the one the
            // fixture shows as no longer shared.
            ...VISUAL_SAVED_FILES.filter(({ file }) => file.name !== "photo-dump.zip").map(({ file }) => ({
              ...template, name: file.name, cid: file.cid, path: "", size: file.size,
              mime: "application/octet-stream", held: 1, total: 1,
            })),
          ],
        },
      };
    }
    default:
      return { handled: false };
  }
}
