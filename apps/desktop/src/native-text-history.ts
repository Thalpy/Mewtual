/** Native own-original pages are not Legacy channel projections. Never turn
 * their context-wide ordinals into group totals, unread counts or receipts. */
export type NativeOriginal = {
  ordinal: string; action: string; signer: string; channel: string; presented_ms: string;
  status: "pending" | "submitted" | "held";
  body: { kind: "create"; text: string; reply_to: string | null } | { kind: "edit"; create: string; text: string }
    | { kind: "delete" | "join"; create: string };
};
export type NativeOriginalPage = { backend: "text-v2"; scope: "own-saved-originals"; next: string; end_exclusive: string; items: NativeOriginal[] };
export type NativeReceivedTarget = { server: number; channel: string; context: string; action: string | null };
export type NativeReceived = { action: string; text: string; certainty: "provisional" | "proven" };
export type NativeOwnRevision = { revision: string; text: string; status: NativeOriginal["status"] };
export type NativeOwnMessage = { scanning: boolean; item: NativeOwnRevision | null };
export type NativeAuditRecord = {
  action: string; signer: string; admission: string; original_digest: string;
  presented_ms: string; parents: string[]; body: NativeOriginal["body"];
};
export type NativeAuditPage = { items: NativeAuditRecord[] | null; next: string | null };
export type NativeHeldPost = {
  caseId: string; action: string; signer: string; admission: string; originalDigest: string;
  removalHeader: string; removalEpoch: string;
  body: Extract<NativeOriginal["body"], { kind: "create" }>;
};
export type NativeHeldPage = { items: NativeHeldPost[] | null; next: string | null; unavailable: number };
const fail = (): never => { throw new Error("Saved messages could not be read. Try again."); };
const object = (value: unknown): Record<string, unknown> => { if (!value || typeof value !== "object" || Array.isArray(value)) return fail(); return value as Record<string, unknown>; };
function fields(value: Record<string, unknown>, expected: string[]) { if (Object.keys(value).length !== expected.length || expected.some(key => !Object.hasOwn(value, key))) fail(); }
function decimal(value: unknown, bits = 64): string { if (typeof value !== "string" || value.length > (bits === 64 ? 20 : 39) || !/^(0|[1-9][0-9]*)$/.test(value) || BigInt(value) > (1n << BigInt(bits)) - 1n) return fail(); return value; }
function hex(value: unknown): string { if (typeof value !== "string" || !/^[a-f0-9]{64}$/.test(value)) return fail(); return value; }
const encoder = new TextEncoder();
function text(value: unknown): string {
  if (typeof value !== "string" || value.length > 65536) return fail();
  for(let i=0;i<value.length;i++){const unit=value.charCodeAt(i);if(unit>=0xd800&&unit<=0xdbff){const next=value.charCodeAt(++i);if(!(next>=0xdc00&&next<=0xdfff))return fail();}else if(unit>=0xdc00&&unit<=0xdfff)return fail();}
  if(encoder.encode(value).length>65536)return fail();
  return value;
}

function decodeBody(raw: unknown): NativeOriginal["body"] {
  const body = object(raw);
  switch (body.kind) {
    case "create": {
      fields(body, ["kind", "text", "reply_to"]);
      const value = text(body.text);
      if (!value) return fail();
      return { kind: "create", text: value, reply_to: body.reply_to === null ? null : hex(body.reply_to) };
    }
    case "edit":
      fields(body, ["kind", "create", "text"]);
      return { kind: "edit", create: hex(body.create), text: text(body.text) };
    case "delete":
    case "join":
      fields(body, ["kind", "create"]);
      return { kind: body.kind, create: hex(body.create) };
    default: return fail();
  }
}

export function decodeNativeReceivedNotice(value: unknown): NativeReceivedTarget {
  const notice = object(value);
  fields(notice, ["server", "channel", "context", "action"]);
  if (typeof notice.server !== "number" || !Number.isSafeInteger(notice.server) || notice.server < 1) return fail();
  return {
    server: notice.server,
    channel: decimal(notice.channel, 128),
    context: hex(notice.context),
    action: notice.action === null ? null : hex(notice.action),
  };
}

export function decodeNativeReceived(
  value: unknown,
  expectedAction: string,
  scope: "received-live" | "received-archive" = "received-live",
): NativeReceived | null {
  const root = object(value);
  fields(root, ["backend", "scope", "item"]);
  if (root.backend !== "text-v2" || root.scope !== scope) return fail();
  if (root.item === null) return null;
  const row = object(root.item);
  fields(row, ["action", "text", "certainty"]);
  const action = hex(row.action);
  if (action !== expectedAction || (row.certainty !== "provisional" && row.certainty !== "proven")) return fail();
  const body = text(row.text);
  if (!body) return fail();
  return { action, text: body, certainty: row.certainty };
}

type NativeHistoryTarget = { server: number; channel: string; context: string };
type NativeInvoke = (command: string, args: Record<string, unknown>) => Promise<unknown>;

export async function readNativeAudit(
  invoke: NativeInvoke,
  target: NativeHistoryTarget,
  action: string | null,
  after: string | null,
  current: () => boolean,
): Promise<NativeAuditPage | null> {
  if (!current()) return null;
  const raw = await invoke("get_message_page", {
    server: target.server, channel: target.channel, expectedContext: target.context,
    auditAction: action, auditAfter: after, limit: 8,
  });
  if (!current()) return null;
  const result = object(raw);
  fields(result, ["backend", "scope", "target", "next", "items"]);
  if (result.backend !== "text-v2" || result.scope !== "selected-shared-history-audit"
    || result.target !== action) return fail();
  const next = result.next === null ? null : hex(result.next);
  if (result.items === null) {
    if (next !== null) return fail();
    return { items: null, next };
  }
  if (!Array.isArray(result.items) || result.items.length > 8) return fail();
  let previous = after;
  const items = result.items.map((raw): NativeAuditRecord => {
    const row = object(raw);
    fields(row, ["action", "signer", "admission", "original_digest", "presented_ms", "parents", "body"]);
    const id = hex(row.action);
    const body = decodeBody(row.body);
    if ((previous !== null && id <= previous)
      || (action === null && body.kind !== "create")
      || (action !== null && action !== (body.kind === "create" ? id : body.create))
      || !Array.isArray(row.parents) || row.parents.length > 64) return fail();
    previous = id;
    return { action: id, body, signer: hex(row.signer), admission: hex(row.admission),
      original_digest: hex(row.original_digest), presented_ms: decimal(row.presented_ms),
      parents: row.parents.map(hex) };
  });
  if (next !== null && (items.length !== 8 || next !== previous)) return fail();
  return { items, next };
}

export async function readNativeHeldPosts(
  invoke: NativeInvoke,
  target: NativeHistoryTarget,
  after: string | null,
  current: () => boolean,
): Promise<NativeHeldPage | null> {
  if (!current()) return null;
  const raw = await invoke("get_message_page", {
    server: target.server, channel: target.channel, expectedContext: target.context,
    heldAfter: after, limit: 2,
  });
  if (!current()) return null;
  const result = object(raw);
  fields(result, ["backend", "scope", "next", "items", "unavailable"]);
  if (result.backend !== "text-v2" || result.scope !== "selected-held-posts"
    || typeof result.unavailable !== "number" || !Number.isInteger(result.unavailable)
    || result.unavailable < 0 || result.unavailable > 512) return fail();
  const next = result.next === null ? null : hex(result.next);
  if (next !== null && next === after) return fail();
  if (result.items === null) {
    if (next !== null || result.unavailable !== 0) return fail();
    return { items: null, next, unavailable: 0 };
  }
  if (!Array.isArray(result.items) || result.items.length > 2) return fail();
  const seen = new Set<string>();
  const items = result.items.map((raw): NativeHeldPost => {
    const row = object(raw);
    fields(row, ["caseId", "action", "signer", "admission", "originalDigest", "removalHeader", "removalEpoch", "body"]);
    const caseId = hex(row.caseId);
    const body = decodeBody(row.body);
    const removalEpoch = decimal(row.removalEpoch);
    if (body.kind !== "create" || removalEpoch === "0" || seen.has(caseId)) return fail();
    seen.add(caseId);
    return {
      caseId, body, removalEpoch, action: hex(row.action), signer: hex(row.signer),
      admission: hex(row.admission), originalDigest: hex(row.originalDigest), removalHeader: hex(row.removalHeader),
    };
  });
  return { items, next, unavailable: result.unavailable };
}

export function decodeNativeOwnMessage(value: unknown, target: string): NativeOwnMessage {
  const result = object(value);
  fields(result, ["backend", "scope", "action", "scanning", "item"]);
  if (result.backend !== "text-v2" || result.scope !== "own-message-revision"
    || hex(result.action) !== target || typeof result.scanning !== "boolean") return fail();
  if (result.item === null) return { scanning: result.scanning, item: null };
  if (result.scanning) return fail();
  const item = object(result.item);
  fields(item, ["revision", "text", "status"]);
  const status = item.status;
  if (status !== "pending" && status !== "submitted" && status !== "held") return fail();
  return { scanning: false, item: { revision: hex(item.revision), text: text(item.text), status } };
}

export async function readNativeOwnMessage(
  invoke: NativeInvoke,
  target: NativeHistoryTarget,
  action: string,
  current: () => boolean,
): Promise<NativeOwnMessage | null> {
  const address = { server: target.server, channel: target.channel, expectedContext: target.context, ownAction: action };
  for (let step = 0; step < 16; step++) {
    if (!current()) return null;
    const raw = await invoke("get_message_page", address);
    if (!current()) return null;
    const result = decodeNativeOwnMessage(raw, action);
    if (!result.scanning) return result;
  }
  return { scanning: true, item: null };
}

/** Fetch one bounded page, resolving each body under its own native guard. */
export function isNativeHistoryBusy(failure: unknown): boolean {
  return (failure instanceof Error ? failure.message : failure) === "Chat history is busy. Try again.";
}

export async function readNativeReceivedArchive(
  invoke: NativeInvoke,
  target: NativeHistoryTarget,
  current: () => boolean,
  after: string | null,
): Promise<{ available: boolean; ids: string[]; more: boolean; items: NativeReceived[]; covered: string[]; incomplete: boolean; blocked: boolean } | null> {
  if (!current()) return null;
  const address = { server: target.server, channel: target.channel, expectedContext: target.context };
  const raw = await invoke("get_message_page", { ...address, archiveAfter: after, limit: 8 });
  if (!current()) return null;
  const root = object(raw);
  fields(root, ["backend", "scope", "available", "ids", "more"]);
  if (root.backend !== "text-v2" || root.scope !== "received-archive-index"
    || typeof root.available !== "boolean" || typeof root.more !== "boolean"
    || !Array.isArray(root.ids) || root.ids.length > 8
    || (!root.available && root.ids.length > 0) || (root.more && root.ids.length !== 8)) return fail();
  const ids = root.ids.map(hex);
  let previous = after;
  for (const id of ids) {
    if (previous !== null && previous >= id) return fail();
    previous = id;
  }
  const items: NativeReceived[] = [];
  const covered: string[] = [];
  let incomplete = false;
  let blocked = false;
  for (const id of ids) {
    if (!current()) return null;
    try {
      const raw = await invoke("get_message_page", { ...address, archivedAction: id });
      if (!current()) return null;
      const item = decodeNativeReceived(raw, id, "received-archive");
      if (item) items.push(item);
      covered.push(id);
    } catch (failure) {
      if (!current()) return null;
      if (isNativeHistoryBusy(failure)) {
        // Keep earlier guarded results, including deletions. The caller
        // leaves the cursor here so an exact retry can finish the page.
        blocked = true;
        break;
      }
      incomplete = true;
    }
  }
  return { available: root.available, ids, more: root.more, items, covered, incomplete, blocked };
}

export function updateNativeReceived(rows: NativeReceived[], action: string, item: NativeReceived | null): NativeReceived[] {
  const index = rows.findIndex(row => row.action === action);
  if (index < 0) return item ? [...rows, item].slice(-64) : rows;
  if (!item) return rows.filter(row => row.action !== action);
  return rows.map((row, at) => at === index ? item : row);
}
export function decodeNativeOriginalPage(value: unknown, start: string, end: string | null, channel: string): NativeOriginalPage {
  const root = object(value); fields(root, ["backend", "scope", "next", "end_exclusive", "items"]);
  const next = decimal(root.next), exclusive = decimal(root.end_exclusive);
  if (root.backend !== "text-v2" || root.scope !== "own-saved-originals" || !Array.isArray(root.items) || root.items.length > 8
    || (end !== null && exclusive !== end) || BigInt(next) < BigInt(start) || BigInt(next) > BigInt(exclusive)
    || (BigInt(start) < BigInt(exclusive) && next === start)) return fail();
  let previous: bigint | null = null;
  const items = root.items.map(raw => {
    const row = object(raw); fields(row, ["ordinal", "action", "signer", "channel", "presented_ms", "status", "body"]);
    const ordinal = decimal(row.ordinal), actualChannel = decimal(row.channel, 128), presented_ms = decimal(row.presented_ms);
    if (actualChannel !== channel || BigInt(ordinal) < BigInt(start) || BigInt(ordinal) >= BigInt(next) || (previous !== null && BigInt(ordinal) <= previous)
      || typeof row.status !== "string" || !["pending", "submitted", "held"].includes(row.status)) return fail();
    previous = BigInt(ordinal);
    const parsed = decodeBody(row.body);
    return { ordinal, action: hex(row.action), signer: hex(row.signer), channel: actualChannel, presented_ms,
      status: row.status as NativeOriginal["status"], body: parsed };
  });
  return { backend: "text-v2", scope: "own-saved-originals", next, end_exclusive: exclusive, items };
}
/** One bounded page for the owner history view. The native own-outbox
 * command supplies only this device's originals for the current channel. */
export async function readNativeOriginalPage(
  invoke: NativeInvoke,
  target: NativeHistoryTarget,
  current: () => boolean,
  cursor: { next: string; end: string | null },
): Promise<NativeOriginalPage | null> {
  if (!current()) return null;
  const raw = await invoke("get_message_page", {
    server: target.server, channel: target.channel, expectedContext: target.context,
    start: cursor.next, endExclusive: cursor.end, limit: 8,
  });
  return current() ? decodeNativeOriginalPage(raw, cursor.next, cursor.end, target.channel) : null;
}

/** At most 16 native requests/128 originals per batch. Empty channel pages keep
 * advancing the original context cursor; a finite first end is reused exactly. */
export async function readNativeOriginals(
  invoke: NativeInvoke,
  target: NativeHistoryTarget, current: () => boolean,
  cursor: { next: string; end: string } | null = null,
): Promise<{ items: NativeOriginal[]; next: string; end: string } | null> {
  let next = cursor?.next ?? "0", end: string | null = cursor?.end ?? null;
  const items: NativeOriginal[] = [];
  for (let page = 0; page < 16; page++) {
    const result = await readNativeOriginalPage(invoke, target, current, { next, end });
    if (!result || !current()) return null;
    next = result.next; end = result.end_exclusive; items.push(...result.items);
    if (next === end) break;
  }
  return { items, next, end: end! };
}
