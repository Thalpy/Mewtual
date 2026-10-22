/** Renderer projection only. Saved manifests, ciphertext, keys and export leases stay native. */
export type PrivateDownload = {
  id: string; cid: string; intent: string; grant: string;
  status: "receiving" | "incomplete" | "ready" | "paused" | "exhausted" | "blocked" | "removing";
  verifiedChunks: number; totalChunks: number;
};
const labels: Record<PrivateDownload["status"], string> = {
  receiving: "Receiving", incomplete: "Interrupted", ready: "Ready to save",
  paused: "Paused", exhausted: "Retry needed", blocked: "Verification failed", removing: "Removing",
};
export function privateDownloadLabel(download: PrivateDownload): string { return labels[download.status]; }
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
        row.totalChunks < 1 || row.totalChunks > 128 || row.verifiedChunks < 0 || row.verifiedChunks > row.totalChunks) {
      throw new Error("Download progress could not be read.");
    }
    seen.add(row.id);
    // Closed projection: unexpected native properties must not become renderer-held secrets.
    return { id: row.id, cid: row.cid, intent: row.intent, grant: row.grant, status: row.status,
      verifiedChunks: row.verifiedChunks, totalChunks: row.totalChunks } as PrivateDownload;
  });
}
