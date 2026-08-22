export type TransferPiece = "held" | "active" | "pending" | "offline" | "failed";

/**
 * The seal chunk size, used only to guess a transfer's piece count before the native side says
 * what it actually is. Not the streaming contract: an upload takes its real chunk count and slice
 * size from the UploadTicket that begin_file_upload returns, because two languages holding
 * copies of the same protocol constant is a drift neither language's tests can see.
 */
export const TRANSFER_CHUNK_BYTES = 8 * 1024 * 1024;

/**
 * What the native side says about one upload when it opens: the whole streaming contract, stated
 * per upload so the caller never has to hold its own copy of it.
 */
export type UploadTicket = {
  /** Identifies this upload's work to every later native call. Not the caller's own upload id. */
  token: string;
  /** How many chunks the file will be sealed into; the denominator of its progress. */
  chunkTotal: number;
  /** How many bytes to put in each push_file_chunk call (the last slice may be shorter). */
  sliceBytes: number;
};

/** Build a small, bounded piece strip. Files currently have at most 32 chunks. */
export function transferPieces(
  total: number,
  done: number,
  active: boolean,
  connected: boolean,
  failed: boolean,
  complete: boolean,
): TransferPiece[] {
  const count = Math.max(1, Math.min(64, Math.floor(total) || 1));
  const ready = complete ? count : Math.max(0, Math.min(count, Math.floor(done)));
  return Array.from({ length: count }, (_, i) => {
    if (i < ready) return "held";
    if (failed) return "failed";
    if (active && i === ready) return "active";
    return connected ? "pending" : "offline";
  });
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB"];
  const unit = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** unit;
  return `${value >= 10 || unit === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
}

export function formatRate(bytesPerSecond: number): string {
  return bytesPerSecond > 0 ? `${formatBytes(bytesPerSecond)}/s` : "—";
}

/** Smooth noisy per-chunk samples without inventing a rate before bytes actually arrive. */
export function sampleRate(
  previousRate: number,
  previousBytes: number,
  previousAt: number,
  bytes: number,
  at: number,
): number {
  const elapsed = at - previousAt;
  const delta = bytes - previousBytes;
  if (elapsed <= 0 || delta <= 0) return previousRate;
  const sample = delta * 1000 / elapsed;
  return previousRate > 0 ? previousRate * 0.65 + sample * 0.35 : sample;
}
