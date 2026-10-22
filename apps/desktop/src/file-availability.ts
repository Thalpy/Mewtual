/** Possession evidence is separate from peer connectivity and shared-index publication. */
export type KeptFile = { cid: string; manifest_version: string; checked: boolean };
export type KeptTransfer = {
  cid: string; manifest_version: string; transfer: string; phase: "paused" | "completed" | "deleting";
  verified_chunks: number; total_chunks: number; running: boolean; user_paused: boolean; blocked: boolean; intent: string; exhausted: boolean; grant: string;
};
export type KeptFiles = {
  supported: boolean;
  allocated_bytes: number;
  limit_bytes: number;
  files: KeptFile[];
  transfers: KeptTransfer[];
  error: string | null;
};

export function keptTransferLabel(transfer: KeptTransfer): string {
  if (transfer.phase === "deleting") return "Removing…";
  // Healthy completed transfers are shown through `files`. A completed transfer left only
  // here has failed storage publication and must remain visible without a verified-copy claim.
  if (transfer.phase === "completed") return "Storage needs checking";
  const state = transfer.blocked ? "Needs checking" : transfer.exhausted ? "Retry required" : transfer.running ? "Saving\u2026" : transfer.user_paused ? "Paused" : "Waiting to resume";
  return `${state} · ${transfer.verified_chunks} of ${transfer.total_chunks} parts saved`;
}

/** A failed release may hide a poisoned completed copy from `files` while its reservation
 * remains in `transfers`. Use the identity union, never phase arithmetic, for the quota count. */
export function keptAllocationCount(view: KeptFiles): number {
  return new Set([...view.files, ...view.transfers].map(copy => copy.cid)).size;
}
export function keptTransferRows(view: KeptFiles): KeptTransfer[] {
  const completed = new Set(view.files.map(copy => copy.cid));
  return view.transfers.filter(copy => !completed.has(copy.cid));
}

export function fileAvailability(held: number, total: number, hasPeers: boolean) {
  if (total > 0 && held >= total) return { cls: "local", icon: "●", label: "Cached here" };
  if (held > 0) return { cls: "partial", icon: "◐", label: `Partial ${held}/${total}` };
  if (hasPeers) return { cls: "remote", icon: "○", label: "Remote copy unconfirmed" };
  return { cls: "offline", icon: "○", label: "No connected provider" };
}

export function keptCopyLabel(file: KeptFile | undefined): string {
  if (!file) return "No kept copy";
  return file.checked ? "Kept here · verified this session" : "Saved copy · needs checking";
}

/** Prevent an old snapshot from resurrecting a released/unchecked copy after a local mutation.
 * The caller supplies its exact unlocked view check at publication time, not at request start. */
export function fileInventoryRequests() {
  let sequence = 0;
  return {
    begin: () => ++sequence,
    invalidate: () => { ++sequence; },
    current: (request: number, unlockedView: boolean) => unlockedView && request === sequence,
  };
}

/** A failed disk mutation may still have committed a release intent. Always reload present-time
 * evidence for the same unlocked view, even when the command reports an I/O failure. */
export async function mutateFileInventory(
  action: () => Promise<unknown>, refresh: () => Promise<void>, current: () => boolean,
): Promise<void> {
  try { await action(); }
  finally { if (current()) await refresh(); }
}
