// Pure receipt helpers for the title-bar ticker. The active queue has a five-minute lifetime, but
// a consumed/accepted id must not become eligible again merely because that visual row aged out.

/** Return a new receipt set for a first sighting, or null for an empty/duplicate id. */
export function acceptTickerReceipt(receipts: ReadonlySet<string>, id: string): Set<string> | null {
  if (!id || receipts.has(id)) return null;
  return new Set(receipts).add(id);
}

/** Message receipts include every routing component so equal ids in different groups cannot alias. */
export function messageTickerId(server: number, channel: string, messageId: string): string {
  return `message:${server}:${channel}:${messageId}`;
}
