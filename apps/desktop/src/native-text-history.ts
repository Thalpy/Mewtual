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

/** Fetch one bounded page, resolving each body under its own native guard. */
export async function readNativeReceivedArchive(
  invoke: NativeInvoke,
  target: NativeHistoryTarget,
  current: () => boolean,
  after: string | null,
): Promise<{ available: boolean; ids: string[]; more: boolean; items: NativeReceived[]; incomplete: boolean } | null> {
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
  let incomplete = false;
  for (const id of ids) {
    if (!current()) return null;
    try {
      const raw = await invoke("get_message_page", { ...address, archivedAction: id });
      if (!current()) return null;
      const item = decodeNativeReceived(raw, id, "received-archive");
      if (item) items.push(item);
    } catch {
      if (!current()) return null;
      incomplete = true;
    }
  }
  return { available: root.available, ids, more: root.more, items, incomplete };
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
    const body = object(row.body); let parsed: NativeOriginal["body"];
    if (body.kind === "create") { fields(body, ["kind", "text", "reply_to"]); const value = text(body.text); if (!value) return fail(); parsed = { kind: "create", text: value, reply_to: body.reply_to === null ? null : hex(body.reply_to) }; }
    else if (body.kind === "edit") { fields(body, ["kind", "create", "text"]); parsed = { kind: "edit", create: hex(body.create), text: text(body.text) }; }
    else if (body.kind === "delete" || body.kind === "join") { fields(body, ["kind", "create"]); parsed = { kind: body.kind, create: hex(body.create) }; }
    else return fail();
    return { ordinal, action: hex(row.action), signer: hex(row.signer), channel: actualChannel, presented_ms,
      status: row.status as NativeOriginal["status"], body: parsed };
  });
  return { backend: "text-v2", scope: "own-saved-originals", next, end_exclusive: exclusive, items };
}
/** At most16 native requests/128 originals per batch. Empty channel pages keep
 * advancing the original context cursor; a finite first end is reused exactly. */
export async function readNativeOriginals(
  invoke: NativeInvoke,
  target: NativeHistoryTarget, current: () => boolean,
  cursor: { next: string; end: string } | null = null,
): Promise<{ items: NativeOriginal[]; next: string; end: string } | null> {
  let next = cursor?.next ?? "0", end: string | null = cursor?.end ?? null;
  const items: NativeOriginal[] = [];
  for (let page = 0; page < 16; page++) {
    if (!current()) return null;
    const raw = await invoke("get_message_page", { server: target.server, channel: target.channel, expectedContext: target.context, start: next, endExclusive: end, limit: 8 });
    if (!current()) return null;
    const result = decodeNativeOriginalPage(raw, next, end, target.channel);
    next = result.next; end = result.end_exclusive; items.push(...result.items);
    if (next === end) break;
  }
  return { items, next, end: end! };
}
