/** Backend selection is returned by authenticated native session restoration,
 * never inferred from a stored draft or an empty conversation list. This is
 * renderer DATA only; native commands authenticate their own original owners. */
export type NativeTextServer = Readonly<{
  backend: "text-v2"; server: number; context: string; channel: string;
  name: string; display_name: string; is_dm: boolean;
}>;
export type RestoredChatSession<Legacy> =
  | { backend: "legacy"; servers: Legacy[] }
  | { backend: "text-v2"; servers: NativeTextServer[] };
export type NativeMembershipNotice = Readonly<{ server: number; context: string; removed: boolean }>;
const fail = (): never => { throw new Error("The chat session could not be restored. Unlock again."); };
const encoder = new TextEncoder();
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return fail();
  return value as Record<string, unknown>;
}
function fields(value: Record<string, unknown>, expected: string[]) {
  if (Object.keys(value).length !== expected.length || expected.some(key => !Object.hasOwn(value, key))) fail();
}

export function decodeNativeMembershipNotice(value: unknown): NativeMembershipNotice | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const notice = value as Record<string, unknown>;
  if (Object.keys(notice).length !== 3 || typeof notice.server !== "number"
    || !Number.isSafeInteger(notice.server) || notice.server <= 0
    || typeof notice.context !== "string" || !/^[0-9a-f]{64}$/.test(notice.context)
    || /^0+$/.test(notice.context) || typeof notice.removed !== "boolean") return null;
  return { server: notice.server, context: notice.context, removed: notice.removed };
}
function label(value: unknown): string {
  if (typeof value !== "string" || !value.length || value.length > 256) return fail();
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(++i);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return fail();
    } else if (code >= 0xdc00 && code <= 0xdfff) return fail();
  }
  if (encoder.encode(value).length > 256) return fail();
  return value;
}
/** Legacy arrays keep their existing contract. Native uses a closed envelope
 * even with zero rows; mixed/native-looking arrays must never downgrade. */
export function decodeRestoredChatSession<Legacy>(value: unknown): RestoredChatSession<Legacy> | null {
  if (value === null) return null; // locked resume, never an unlocked empty list
  if (Array.isArray(value)) {
    if (value.some(row => row && typeof row === "object" && "backend" in row && row.backend !== "legacy")) return fail();
    return { backend: "legacy", servers: value as Legacy[] };
  }
  const root = object(value);
  fields(root, ["backend", "servers"]);
  if (root.backend !== "text-v2" || !Array.isArray(root.servers) || root.servers.length > 2) return fail();
  const seen = new Set<number>();
  const servers = root.servers.map(raw => {
    const row = object(raw);
    fields(row, ["backend", "server", "context", "channel", "name", "display_name", "is_dm"]);
    if (row.backend !== "text-v2" || typeof row.server !== "number" || !Number.isSafeInteger(row.server) || row.server < 0 || seen.has(row.server)) return fail();
    if (typeof row.context !== "string" || !/^[0-9a-f]{64}$/.test(row.context) || /^0+$/.test(row.context)) return fail();
    if (typeof row.channel !== "string" || !/^(0|[1-9][0-9]{0,38})$/.test(row.channel) || BigInt(row.channel) > (1n << 128n) - 1n) return fail();
    if (typeof row.is_dm !== "boolean") return fail();
    seen.add(row.server);
    return Object.freeze({ backend: "text-v2" as const, server: row.server, context: row.context, channel: row.channel,
      name: label(row.name), display_name: label(row.display_name), is_dm: row.is_dm });
  });
  return { backend: "text-v2", servers };
}
