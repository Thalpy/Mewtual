/** Unfinished caller intents live only in the vault-sealed continuity record. */
export type PendingSend = {
  token: string;
  server: number;
  channel: string;
  expectedContext: string;
  text: string;
  replyTo: string;
  /** A permanent refusal requires a user decision, including after the vault reopens. */
  retryBlock?: PendingSendRetryBlock;
};
export type PendingSendRetryBlock = "invalid" | "conflict" | "context_changed";
export type PendingSends = Record<string, PendingSend>;
export const MAX_PENDING_SENDS = 32;
export const MAX_PENDING_SEND_BYTES = 256 * 1024;
const bytes = (value: unknown) => new TextEncoder().encode(JSON.stringify(value)).length;

export function sanitizePendingSends(value: unknown): PendingSends {
  const result: PendingSends = {};
  if (!value || typeof value !== "object" || Array.isArray(value)) return result;
  let used = 0;
  for (const [token, raw] of Object.entries(value)) {
    if (Object.keys(result).length >= MAX_PENDING_SENDS) break;
    if (!/^[a-f0-9]{32}$/.test(token) || !raw || typeof raw !== "object" || Array.isArray(raw)) continue;
    const entry = raw as Partial<PendingSend>;
    if (entry.token !== token || !Number.isSafeInteger(entry.server) || entry.server! < 0
      || typeof entry.channel !== "string" || !/^(0|[1-9][0-9]{0,38})$/.test(entry.channel)
      || BigInt(entry.channel) >= (1n << 128n)
      || typeof entry.expectedContext !== "string" || !/^[a-f0-9]{64}$/.test(entry.expectedContext)
      || typeof entry.text !== "string" || !entry.text.trim() || new TextEncoder().encode(entry.text).length > 65536
      || typeof entry.replyTo !== "string" || entry.replyTo.length > 128) continue;
    const intent: PendingSend = { token, server: entry.server!, channel: entry.channel,
      expectedContext: entry.expectedContext, text: entry.text, replyTo: entry.replyTo };
    // Retry disposition is separately bounded to one fixed enum per retained entry. Recording
    // a refusal must not push an existing payload over the hydration budget and lose its token.
    used += bytes(intent);
    if (used > MAX_PENDING_SEND_BYTES) break;
    if (entry.retryBlock !== undefined) {
      intent.retryBlock = ["invalid", "conflict", "context_changed"].includes(entry.retryBlock)
        ? entry.retryBlock : "invalid";
    }
    result[token] = intent;
  }
  return result;
}

/** Refuse capacity pressure; never evict an unresolved send or give it another retry token. */
export function addPendingSend(current: PendingSends, intent: PendingSend): PendingSends {
  const existing = current[intent.token];
  if (existing && JSON.stringify(existing) !== JSON.stringify(intent)) throw new Error("A pending send has conflicting content.");
  const next = { ...current, [intent.token]: intent };
  if (Object.keys(next).length > MAX_PENDING_SENDS || bytes(next) > MAX_PENDING_SEND_BYTES) {
    throw new Error("Pending messages need attention before another message can be sent.");
  }
  return next;
}

export function matchingPendingSend(current: PendingSends, server: number, channel: string,
  text: string, replyTo: string): PendingSend | undefined {
  return Object.values(current).find(entry => entry.server === server && entry.channel === channel
    && entry.text === text && entry.replyTo === replyTo);
}

/** Native CHAT.SEND.REJECTED also wraps temporary storage refusals; classify the inner reason. */
export function pendingSendRetryBlock(message: string): PendingSendRetryBlock | undefined {
  if (/\bCHAT_SEND_CONTEXT_CHANGED\b|The conversation changed before this message could be saved/.test(message)) return "context_changed";
  if (/\bCHAT_SEND_TOKEN_CONFLICT\b/.test(message)) return "conflict";
  if (/\bCHAT_SEND_INVALID\b|\bCHANNEL\.ID\.INVALID\b|Invalid message retry identity|A retry requires both its identity/.test(message)) return "invalid";
  return undefined;
}
