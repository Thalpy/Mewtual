/** Renderer projection only. Saved manifests, ciphertext, keys, export leases and every external
 * path, file name or file identity stay native. The status is a closed, typed native decision;
 * nothing here infers state from paths or flags. */
export type PrivateDownloadStatus =
  | "receiving" | "incomplete" | "ready" | "paused" | "exhausted" | "blocked" | "removing"
  | "saving" | "save-interrupted" | "save-paused" | "destination-unavailable"
  | "destination-changed" | "saved";
export type PrivateDownload = {
  id: string; cid: string; intent: string; grant: string;
  status: PrivateDownloadStatus;
  /** Encrypted acquisition progress (whole chunks verified in the private copy). */
  verifiedChunks: number;
  /** Durable checkpoint of the partly saved file in Downloads (0 without one). */
  savedChunks: number;
  totalChunks: number;
};
const labels: Record<PrivateDownloadStatus, string> = {
  receiving: "Receiving", incomplete: "Interrupted", ready: "Ready to save",
  paused: "Paused", exhausted: "Retry needed", blocked: "Verification failed", removing: "Removing",
  saving: "Saving", "save-interrupted": "Saving interrupted", "save-paused": "Saving paused",
  "destination-unavailable": "Folder or file unavailable",
  "destination-changed": "Partly saved file changed", saved: "Saved",
};
export function privateDownloadLabel(download: PrivateDownload): string { return labels[download.status]; }

/** Statuses whose detail line counts the partly saved file instead of encrypted progress. */
const exporting = new Set<PrivateDownloadStatus>([
  "saving", "save-interrupted", "save-paused", "destination-unavailable", "destination-changed",
]);
/** One short status line. Plain words and commas: no "·" separators or dashes in app text. */
export function privateDownloadDetail(download: PrivateDownload): string {
  const label = privateDownloadLabel(download);
  if (download.status === "saved" || download.status === "removing") return label;
  return exporting.has(download.status)
    ? `${label}, ${download.savedChunks} of ${download.totalChunks} chunks saved`
    : `${label}, ${download.verifiedChunks} of ${download.totalChunks} chunks received`;
}

export type PrivateDownloadActionKind = "resume" | "pause" | "fresh" | "remove" | "keep";
export type PrivateDownloadAction = {
  kind: PrivateDownloadActionKind;
  label: string;
  /** Secondary styling. */
  ghost: boolean;
  /** Starts a save, so it must wait for any save of the same file to settle. */
  needsIdle: boolean;
};
const resume = (label: string): PrivateDownloadAction => ({ kind: "resume", label, ghost: false, needsIdle: true });
const fresh = (label: string): PrivateDownloadAction => ({ kind: "fresh", label, ghost: false, needsIdle: true });
const pause: PrivateDownloadAction = { kind: "pause", label: "Pause", ghost: false, needsIdle: false };
const remove = (label: string): PrivateDownloadAction => ({ kind: "remove", label, ghost: true, needsIdle: false });
const keep: PrivateDownloadAction = { kind: "keep", label: "Keep partly saved file", ghost: true, needsIdle: false };
/** The actions each typed state offers, in display order. Resume never appears for a changed
 * destination (it cannot succeed there) and nothing re-saves an already saved file implicitly. */
export function privateDownloadActions(download: PrivateDownload): PrivateDownloadAction[] {
  switch (download.status) {
    case "receiving": case "saving": return [pause, remove("Remove")];
    case "ready": return [resume("Save to Downloads"), remove("Remove")];
    case "incomplete": case "paused": case "blocked": return [resume("Resume download"), remove("Remove")];
    case "exhausted": return [resume("Retry download"), remove("Remove")];
    case "save-interrupted": case "save-paused": return [resume("Resume saving"), remove("Remove")];
    case "destination-unavailable": return [resume("Try again"), fresh("Save new copy"), remove("Remove")];
    case "destination-changed": return [fresh("Save new copy"), remove("Remove")];
    case "saved": return [fresh("Save another copy"), remove("Clear")];
    case "removing": return [remove("Remove"), keep];
  }
}
/** Whether `download` currently offers the resume (or fresh-copy) action. */
export function privateDownloadOffers(download: PrivateDownload, kind: "resume" | "fresh"): boolean {
  return privateDownloadActions(download).some((action) => action.kind === kind);
}
/** Saving from the file list only continues states where that is the obvious next step; every
 * other state (running, halted, saved, removing, exhausted) opens Transfers for an explicit choice. */
export function privateDownloadChatSave(download: PrivateDownload): "resume" | "transfers" {
  return download.status === "ready" || download.status === "incomplete" || download.status === "paused" ||
    download.status === "blocked" || download.status === "save-interrupted" || download.status === "save-paused"
    ? "resume" : "transfers";
}

/** Native save progress must belong to the exact active invocation. Preview reads and delayed
 * events from an older same-CID save cannot revive a completed/failed/replacement row. */
export function acceptSaveProgress(transfer: { operation?: string; status: string }, progress: { cancellation: string | null }): boolean {
  return typeof transfer.operation === "string" && progress.cancellation === transfer.operation &&
    transfer.status !== "done" && transfer.status !== "failed";
}
export function checkedPrivateDownloads(value: unknown): PrivateDownload[] {
  if (!Array.isArray(value) || value.length > 32) throw new Error("Download progress could not be read.");
  const seen = new Set<string>();
  return value.map((row) => {
    if (!row || typeof row !== "object" ||
        typeof row.id !== "string" || typeof row.cid !== "string" || typeof row.intent !== "string" || typeof row.grant !== "string" ||
        !/^[0-9a-f]{64}$/.test(row.id) || !/^[0-9a-f]{64}$/.test(row.cid) ||
        seen.has(row.id) || typeof row.status !== "string" || !Object.hasOwn(labels, row.status) ||
        !/^(0|[1-9][0-9]*)$/.test(row.intent) || !/^[1-9][0-9]*$/.test(row.grant) ||
        BigInt(row.intent) > 0xffffffffffffffffn || BigInt(row.grant) > 0xffffffffffffffffn ||
        !Number.isSafeInteger(row.verifiedChunks) || !Number.isSafeInteger(row.totalChunks) ||
        !Number.isSafeInteger(row.savedChunks) ||
        row.totalChunks < 1 || row.totalChunks > 128 || row.verifiedChunks < 0 || row.verifiedChunks > row.totalChunks ||
        row.savedChunks < 0 || row.savedChunks > row.totalChunks) {
      throw new Error("Download progress could not be read.");
    }
    seen.add(row.id);
    // Closed projection: unexpected native properties must not become renderer-held secrets.
    return { id: row.id, cid: row.cid, intent: row.intent, grant: row.grant, status: row.status,
      verifiedChunks: row.verifiedChunks, savedChunks: row.savedChunks, totalChunks: row.totalChunks } as PrivateDownload;
  });
}
