export type TransferPiece = "held" | "active" | "pending" | "offline" | "failed";
export const TRANSFER_CHUNK_BYTES = 8 * 1024 * 1024;
/**
 * How much of an upload crosses the IPC bridge in one message.
 *
 * Deliberately far smaller than TRANSFER_CHUNK_BYTES, which is the size a chunk is *sealed* at
 * and cannot change: an invoke argument becomes a base64 string that both sides serialize whole,
 * so the message size, not the chunk size, is what decides whether the webview stutters. The
 * native side buffers slices until it has a whole chunk, so this only has to divide
 * TRANSFER_CHUNK_BYTES exactly, which is what keeps chunk boundaries uniform.
 */
export const TRANSFER_SLICE_BYTES = 1024 * 1024;

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
