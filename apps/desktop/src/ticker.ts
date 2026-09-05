// Pure receipt helpers for the title-bar ticker. The active queue has a five-minute lifetime, but
// a consumed/accepted id must not become eligible again merely because that visual row aged out.

/**
 * How many accepted ids are remembered. A receipt only has to outlive the replayed or duplicate
 * backend events for the row it names, which arrive within moments; this many newer acceptances
 * later, the row is long gone from every feed that could re-announce it. Bounding the set is what
 * keeps a long unlocked session from paying for every notification it ever showed.
 */
export const TICKER_RECEIPT_MAX = 4096;

/**
 * Record a first sighting of `id` in place; false for an empty or already-seen id.
 *
 * Insertion order is eviction order: once the set is full, the oldest receipt makes room. The set
 * is mutated rather than copied because a copy per acceptance made the k-th notification cost k.
 */
export function acceptTickerReceipt(receipts: Set<string>, id: string, max = TICKER_RECEIPT_MAX): boolean {
  if (!id || receipts.has(id)) return false;
  receipts.add(id);
  while (receipts.size > Math.max(1, max)) {
    const oldest = receipts.values().next().value;
    if (oldest === undefined) break;
    receipts.delete(oldest);
  }
  return true;
}

/** Message receipts include every routing component so equal ids in different groups cannot alias. */
export function messageTickerId(server: number, channel: string, messageId: string): string {
  return `message:${server}:${channel}:${messageId}`;
}
