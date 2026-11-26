/** Renderer retry DATA only. None of these fields grant native signing authority.
 * Keep exact native tokens/settings across uncertain replies; never reinterpret
 * these as Legacy's shorter tokens or recovered composer drafts. */
export type NativeTextFoundIntent = Readonly<{
  kind: "found"; token: string; displayName: string; serverName: string;
  advertise: string; relay: string; rendezvous: string; isDm: boolean;
}>;
export type NativeTextCreateIntent = Readonly<{
  kind: "create"; token: string; server: number; channel: string;
  expectedContext: string; text: string; replyTo?: string;
}>;
type NativeTextMutationTarget = Readonly<{
  token: string; server: number; channel: string; expectedContext: string;
  messageId: string; expectedRevision: string;
}>;
export type NativeTextEditIntent = NativeTextMutationTarget & Readonly<{ kind: "edit"; text: string }>;
export type NativeTextDeleteIntent = NativeTextMutationTarget & Readonly<{ kind: "delete" }>;
export type NativeTextMutationIntent = NativeTextEditIntent | NativeTextDeleteIntent;
export type NativeTextIntent = NativeTextFoundIntent | NativeTextCreateIntent | NativeTextMutationIntent;
export type NativeTextIntents = Record<string, NativeTextIntent>;
export const MAX_NATIVE_TEXT_INTENTS = 16;
const encoder = new TextEncoder();
const fail = (): never => { throw new Error("Saved chat requests have an unsupported format. The sealed copy has been kept."); };
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return fail();
  return value as Record<string, unknown>;
}
function exact(object: Record<string, unknown>, fields: string[]) {
  if (Object.keys(object).length !== fields.length || fields.some(field => !Object.hasOwn(object, field))) fail();
}
function text(value: unknown, max: number, nonempty = false): string {
  if (typeof value !== "string" || value.length > max || (nonempty && !value.length)) return fail();
  // JSON/UTF-8 must preserve the exact immutable request; lone surrogates would
  // be replaced by different bytes at the Rust boundary.
  for (let i = 0; i < value.length; i++) {
    const unit = value.charCodeAt(i);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(++i);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return fail();
    } else if (unit >= 0xdc00 && unit <= 0xdfff) return fail();
  }
  if (encoder.encode(value).length > max) return fail();
  return value;
}
function id(value: unknown): string {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value) || /^0+$/.test(value)) return fail();
  return value;
}
function channel(value: unknown): string {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]{0,38})$/.test(value) || BigInt(value) > (1n << 128n) - 1n) return fail();
  return value;
}
/** Strict whole-collection decode. No slice/truncation, guessed defaults or
 * salvage: a future/malformed native request must block resaving its envelope. */
export function decodeNativeTextIntents(value: unknown, version: number): NativeTextIntents {
  if (value === undefined) return {};
  if (version !== 4) return fail();
  const source = record(value), entries = Object.entries(source);
  if (entries.length > MAX_NATIVE_TEXT_INTENTS) return fail();
  const result: NativeTextIntents = {};
  for (const [key, raw] of entries) {
    const item = record(raw), token = id(item.token);
    if (id(key) !== token) return fail();
    if (item.kind === "found") {
      exact(item, ["kind", "token", "displayName", "serverName", "advertise", "relay", "rendezvous", "isDm"]);
      if (typeof item.isDm !== "boolean") return fail();
      result[key] = Object.freeze({ kind: "found", token, displayName: text(item.displayName, 256, true),
        serverName: text(item.serverName, 256, true), advertise: text(item.advertise, 4096),
        relay: text(item.relay, 4096), rendezvous: text(item.rendezvous, 4096), isDm: item.isDm });
    } else if (item.kind === "create" || item.kind === "edit" || item.kind === "delete") {
      if (typeof item.server !== "number" || !Number.isSafeInteger(item.server) || item.server < 0) return fail();
      const target = { token, server: item.server, channel: channel(item.channel), expectedContext: id(item.expectedContext) };
      const fields = ["kind", "token", "server", "channel", "expectedContext", "text"];
      if (item.kind === "create") {
        const reply = Object.hasOwn(item, "replyTo") ? { replyTo: id(item.replyTo) } : {};
        exact(item, [...fields, ...Object.keys(reply)]);
        result[key] = Object.freeze({ kind: "create", ...target, text: text(item.text, 8192, true), ...reply });
      } else {
        const mutation = { ...target, messageId: id(item.messageId), expectedRevision: id(item.expectedRevision) };
        const identity = ["messageId", "expectedRevision"];
        if (item.kind === "edit") {
          exact(item, [...fields, ...identity]);
          result[key] = Object.freeze({ kind: "edit", ...mutation, text: text(item.text, 8192, true) });
        } else {
          exact(item, [...fields.slice(0, -1), ...identity]);
          result[key] = Object.freeze({ kind: "delete", ...mutation });
        }
      }
    } else return fail();
  }
  return result;
}
/** Immutable insertion: the same token can only retain the same exact request.
 * Caller must seal the returned collection before any native dispatch. */
export function retainNativeTextIntent(current: NativeTextIntents, intent: NativeTextIntent): NativeTextIntents {
  const validated = decodeNativeTextIntents({ [intent.token]: intent }, 4)[intent.token];
  const validatedCurrent = decodeNativeTextIntents(current, 4);
  const existing = validatedCurrent[intent.token];
  if (existing && JSON.stringify(existing) !== JSON.stringify(validated)) throw new Error("This message token already belongs to a different request.");
  return decodeNativeTextIntents({ ...validatedCurrent, [intent.token]: validated }, 4);
}
