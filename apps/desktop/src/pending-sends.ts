import type { SendMessageResult } from "./message-send.ts";

/** Unfinished caller intents live only in the vault-sealed continuity record. */
type PendingIntentBase = {
  token: string;
  server: number;
  channel: string;
  expectedContext: string;
  /** Once sealed, retry only the exact-token native Stop, including after reload. */
  resolution?: PendingResolution;
  /** A permanent refusal requires a user decision, including after the vault reopens. */
  retryBlock?: PendingSendRetryBlock;
  /** Missing legacy state is ambiguous. Only a fresh, never-submitted request is known absent. */
  acceptance?: "not_accepted" | "ambiguous";
};
/** Missing kind is the byte-compatible v1 send shape. Mutations always have an explicit kind. */
export type PendingSend = PendingIntentBase & (
  | { kind?: "send"; text: string; replyTo: string }
  | { kind: "edit"; messageId: string; target: string; text: string; replyTo: "" }
  | { kind: "delete"; messageId: string; target: string; text: ""; replyTo: "" }
);
export type PendingSendRetryBlock = "invalid" | "conflict" | "context_changed";
export type PendingSends = Record<string, PendingSend>;
export const MAX_PENDING_SENDS = 32;
export const MAX_PENDING_SEND_BYTES = 256 * 1024;
const bytes = (value: unknown) => new TextEncoder().encode(JSON.stringify(value)).length;

export function sanitizePendingSends(value: unknown, version: 1 | 2 = 1): PendingSends {
  const result: PendingSends = {};
  if (!value || typeof value !== "object" || Array.isArray(value)) return result;
  let used = 0;
  for (const [token, raw] of Object.entries(value)) {
    if (Object.keys(result).length >= MAX_PENDING_SENDS) {
      if (version === 2) throw new Error("Saved chat actions exceed pending capacity. The sealed copy has been kept.");
      break;
    }
    if (!/^[a-f0-9]{32}$/.test(token) || !raw || typeof raw !== "object" || Array.isArray(raw)) continue;
    const entry = raw as Record<string, unknown>;
    // An unknown action must never become a post. Refuse the load so the only sealed
    // identity survives, rather than overwriting it with a sanitized empty queue.
    if ((entry.kind !== undefined && (typeof entry.kind !== "string" || !["send", "edit", "delete"].includes(entry.kind)))
      || entry.action !== undefined || (entry.kind === undefined && (entry.messageId !== undefined || entry.target !== undefined))) {
      throw new Error("Saved chat action is unsupported. The sealed copy has been kept.");
    }
    if (version === 1 && (entry.kind === "edit" || entry.kind === "delete" || entry.resolution !== undefined)) {
      throw new Error("Saved chat actions require continuity version 2. The sealed copy has been kept.");
    }
    if (entry.token !== token || typeof entry.server !== "number" || !Number.isSafeInteger(entry.server) || entry.server < 0
      || typeof entry.channel !== "string" || !/^(0|[1-9][0-9]{0,38})$/.test(entry.channel)
      || BigInt(entry.channel) >= (1n << 128n)
      || typeof entry.expectedContext !== "string" || !/^[a-f0-9]{64}$/.test(entry.expectedContext)
      || typeof entry.text !== "string" || (entry.kind !== "delete" && !entry.text.trim()) || new TextEncoder().encode(entry.text).length > 65536
      || typeof entry.replyTo !== "string" || entry.replyTo.length > 128) {
      if (entry.kind === "edit" || entry.kind === "delete" || entry.resolution !== undefined) {
        throw new Error("Saved chat action has invalid custody. The sealed copy has been kept.");
      }
      continue;
    }
    if ((entry.kind === "edit" || entry.kind === "delete") && (typeof entry.messageId !== "string"
      || !entry.messageId || entry.messageId.length > 128 || typeof entry.target !== "string"
      || !/^[a-f0-9]{64}$/.test(entry.target) || entry.replyTo !== ""
      || (entry.kind === "delete" && entry.text !== ""))) {
      throw new Error("Saved chat mutation has an invalid target. The sealed copy has been kept.");
    }
    if (entry.resolution !== undefined && entry.resolution !== "cancel" && entry.resolution !== "recover") {
      throw new Error("Saved chat Stop decision is unsupported. The sealed copy has been kept.");
    }
    const intent = { token, server: entry.server, channel: entry.channel,
      expectedContext: entry.expectedContext, text: entry.text, replyTo: entry.replyTo } as PendingSend;
    if (entry.kind !== undefined) Object.assign(intent, { kind: entry.kind });
    if (entry.kind === "edit" || entry.kind === "delete") Object.assign(intent, { messageId: entry.messageId, target: entry.target });
    // Retry disposition is separately bounded to one fixed enum per retained entry. Recording
    // a refusal must not push an existing payload over the hydration budget and lose its token.
    used += bytes(intent);
    if (used > MAX_PENDING_SEND_BYTES) {
      if (version === 2) throw new Error("Saved chat actions exceed pending storage. The sealed copy has been kept.");
      break;
    }
    if (entry.retryBlock !== undefined) {
      intent.retryBlock = ["invalid", "conflict", "context_changed"].includes(String(entry.retryBlock))
        ? entry.retryBlock as PendingSendRetryBlock : "invalid";
    }
    if (entry.acceptance !== undefined) {
      intent.acceptance = entry.acceptance === "not_accepted" ? "not_accepted" : "ambiguous";
    }
    if (entry.resolution !== undefined) intent.resolution = entry.resolution;
    result[token] = intent;
  }
  return result;
}

export type PendingResolution = "recover" | "cancel";

/** A recovered draft preserves the original payload and provenance; it is never a send queue. */
export function resolvePendingSend(
  pending: PendingSends, recovered: PendingSends, token: string, action: PendingResolution,
): { pending: PendingSends; recovered: PendingSends } {
  const intent = pending[token];
  if (!intent) throw new Error("This pending message has already been resolved.");
  if (action !== "recover" && action !== "cancel") throw new Error("Choose how to resolve this message.");
  // Delete has no useful text copy. Recovery never turns a mutation into a queued post.
  const { resolution: _resolution, ...savedCopy } = intent;
  const nextRecovered = action === "recover" && intent.kind !== "delete" ? addPendingSend(recovered, savedCopy as PendingSend) : recovered;
  const nextPending = { ...pending };
  delete nextPending[token];
  return { pending: nextPending, recovered: nextRecovered };
}

export function pendingAcceptanceWarning(intent: PendingSend): string {
  if (intent.resolution) return "Stop is awaiting a durable confirmation. This exact request will only retry Stop; an already saved change cannot be retracted.";
  if (intent.kind === "edit" || intent.kind === "delete") return intent.acceptance === "not_accepted"
    ? "This change request has not been submitted."
    : `${intent.kind === "edit" ? "This edit" : "This deletion"} may already have been applied. Stopping retries cannot retract an applied change.`;
  return intent.acceptance === "not_accepted"
    ? "This request has not been submitted."
    : "This message may already have been accepted. Stopping retries cannot retract a message. Sending recovered text later may create a duplicate.";
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
  return Object.values(current).find(entry => (!entry.kind || entry.kind === "send") && entry.server === server && entry.channel === channel
    && entry.text === text && entry.replyTo === replyTo);
}

/** Target digest is immutable object identity, not mutable message text or list position. */
export function matchingPendingMutation(current: PendingSends, server: number, channel: string,
  messageId: string, target: string | undefined, kind: "edit" | "delete", text: string): PendingSend | undefined {
  return Object.values(current).find(entry => entry.kind === kind && entry.server === server
    && entry.channel === channel && entry.messageId === messageId && (target === undefined || entry.target === target) && entry.text === text);
}

/** Caller custody comparison excludes retry diagnostics and conservative acceptance markers. */
export function samePendingIntent(a: PendingSend, b: PendingSend): boolean {
  return a.token === b.token && a.server === b.server && a.channel === b.channel
    && (a.kind ?? "send") === (b.kind ?? "send") && a.text === b.text && a.replyTo === b.replyTo
    && a.expectedContext === b.expectedContext && a.resolution === b.resolution
    && ((a.kind !== "edit" && a.kind !== "delete") || ((b.kind === "edit" || b.kind === "delete")
      && a.messageId === b.messageId && a.target === b.target));
}

/** Exact original request for native Stop: no current selection, lookup, or new token. */
export function durableStopOriginal(intent: PendingSend) {
  const common = { kind: intent.kind ?? "send", token: intent.token, expected_context: intent.expectedContext, channel: intent.channel };
  if (intent.kind === "edit" || intent.kind === "delete") return { ...common, message_id: intent.messageId,
    target: intent.target, ...(intent.kind === "edit" ? { text: intent.text } : {}) };
  return { ...common, text: intent.text, reply_to: intent.replyTo };
}

export function pendingActionLabel(intent: PendingSend): string {
  return intent.kind === "edit" ? "Edit message" : intent.kind === "delete" ? "Delete message" : "Send message";
}

/** Only an acknowledged durable native terminal permits releasing caller custody. */
export function durableStopTerminal(result: SendMessageResult): boolean {
  if (result.persistence.status !== "durable") return false;
  if (result.disposition === "stopped-retired") return result.accepted === false;
  return result.disposition === "committed" || result.disposition === "stopped-before-commit"
    || result.disposition === "stopped-after-commit";
}

/** A retired store can fence future work without reconstructing its erased historical outcome. */
export function retiredStopWarning(intent: PendingSend): string {
  const history = intent.kind === "edit" ? "This edit may already have been applied."
    : intent.kind === "delete" ? "This deletion may already have been applied."
    : "This message may already have been sent.";
  return `Future retries are stopped. ${history}`;
}

/**
 * Whether a native refusal proves that this request was never accepted. A different question
 * from `pendingSendRetryBlock` (should automatic retry stop?), and the two differ: the native
 * preparation looks a token's receipt up first and returns it when one exists (receipts are never
 * evicted), so a refusal it makes after that lookup and before writing a new preparation proves no
 * record of the token exists. That is true of the permanent refusals `pendingSendRetryBlock`
 * names, and of two temporary ones it deliberately leaves out so a retained entry keeps retrying:
 * capacity, and a channel's outstanding preparation (`prepare_chat` in
 * `crates/catcoms-sync/src/durable_chat.rs`). Found in review: once Pending messages had resolved
 * the entry, those two were reported "don't send it again" though nothing would retry them.
 */
export function sendRefusedUnaccepted(message: string): boolean {
  return pendingSendRetryBlock(message) !== undefined || /\bCHAT_SEND_CAPACITY\b|\bCHAT_SEND_DOCUMENT_PENDING\b/.test(message);
}

/** Native CHAT.SEND.REJECTED also wraps temporary storage refusals; classify the inner reason. */
export function pendingSendRetryBlock(message: string): PendingSendRetryBlock | undefined {
  if (/\bCHAT_SEND_CONTEXT_CHANGED\b|The conversation changed before this message could be saved/.test(message)) return "context_changed";
  if (/\bCHAT_SEND_TOKEN_CONFLICT\b/.test(message)) return "conflict";
  if (/\bCHAT_SEND_INVALID\b|\bCHANNEL\.ID\.INVALID\b|Invalid message retry identity|A retry requires both its identity/.test(message)) return "invalid";
  return undefined;
}
