/** Native-only frontend request custody. Persistence always precedes dispatch;
 * uncertainty never redraws a token or edits its original payload. Native still
 * independently checks actual authorization and commits the signed original. */
import { retainNativeTextIntent, type NativeTextIntents, type NativeTextFoundIntent, type NativeTextCreateIntent,
  type NativeTextEditIntent, type NativeTextDeleteIntent, type NativeTextMutationIntent,
  type NativeTextReviewIntent, type NativeTextIntent } from "./native-text-intents.ts";
import { decodeRestoredChatSession, type NativeTextServer } from "./native-text-session.ts";
export type NativeTextDriverDependencies = {
  read(): NativeTextIntents;
  replace(intents: NativeTextIntents): void;
  seal(): Promise<void>;
  current(): boolean;
  mayDispatch?(intent: NativeTextIntent): boolean;
  sealed?(intent: NativeTextIntent): void;
  token(): string;
  invoke: (command: "found_server" | "send_message" | "edit_message" | "delete_message" | "decide_native_chat_held_post", args: Record<string, unknown>) => Promise<unknown>;
};
export type NativeFoundSettings = Omit<NativeTextFoundIntent, "kind" | "token">;
export type NativeMutationSettings = Omit<NativeTextEditIntent, "token"> | Omit<NativeTextDeleteIntent, "token">;
export type NativeReviewSettings = Omit<NativeTextReviewIntent, "kind" | "token">;
export type NativeReviewDisposition = "saved" | "superseded";
export type NativeSavedMessage = { disposition: "saved-local"; action: string; ordinal: string; cleanupPending: boolean };
type ActionIntent = NativeTextCreateIntent | NativeTextMutationIntent;
const same = (a: NativeTextIntent, b: NativeTextIntent) => JSON.stringify(retainNativeTextIntent({}, a)[a.token]) === JSON.stringify(retainNativeTextIntent({}, b)[b.token]);
const object = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("The chat reply could not be confirmed. Retry the same request.");
  return value as Record<string, unknown>;
};
const decimal = (value: unknown): value is string => typeof value === "string" && /^(0|[1-9][0-9]{0,19})$/.test(value) && BigInt(value) <= (1n << 64n) - 1n;
const nonzeroId = (value: unknown): value is string =>
  typeof value === "string" && /^[0-9a-f]{64}$/.test(value) && !/^0+$/.test(value);
export function createNativeTextDriver(deps: NativeTextDriverDependencies) {
  const permitted = (intent: NativeTextIntent) => deps.current() && (deps.mayDispatch?.(intent) ?? true);
  async function prepare(intent: NativeTextIntent): Promise<boolean> {
    if (!permitted(intent)) return false;
    deps.replace(retainNativeTextIntent(deps.read(), intent));
    await deps.seal(); // failed seal means ZERO native invocation
    return true;
  }
  function ready(intent: NativeTextIntent): boolean {
    if (!permitted(intent) || !deps.read()[intent.token] || !same(deps.read()[intent.token], intent)) return false;
    deps.sealed?.(intent);
    return permitted(intent) && !!deps.read()[intent.token] && same(deps.read()[intent.token], intent);
  }
  async function complete(intent: NativeTextIntent): Promise<boolean> {
    if (!deps.current() || !deps.read()[intent.token] || !same(deps.read()[intent.token], intent)) return true;
    const next = { ...deps.read() }; delete next[intent.token]; deps.replace(next);
    try { await deps.seal(); return false; }
    catch {
      // A failed cleanup acknowledgement may have followed a successful rename.
      // Re-retaining this immutable retry is safe; never invent a new request.
      if (deps.current() && !deps.read()[intent.token]) deps.replace(retainNativeTextIntent(deps.read(), intent));
      return true;
    }
  }
  async function dispatch(intent: ActionIntent): Promise<NativeSavedMessage | { disposition: "uncertain" } | null> {
    if (!await prepare(intent) || !ready(intent)) return null;
    const common = { server: intent.server, channel: intent.channel, expectedContext: intent.expectedContext, retryToken: intent.token };
    const mutation = intent.kind === "create" ? null : { messageId: intent.messageId, expectedRevision: intent.expectedRevision };
    const result = intent.kind === "create"
      ? await deps.invoke("send_message", { ...common, text: intent.text, replyTo: intent.replyTo ?? null })
      : intent.kind === "edit"
        ? await deps.invoke("edit_message", { ...common, ...mutation, text: intent.text })
        : await deps.invoke("delete_message", { ...common, ...mutation });
    if (!deps.current()) return null;
    const response = object(result);
    if (response.backend !== "text-v2" || response.retry_token !== intent.token) throw new Error("The message reply could not be confirmed. Retry the same message.");
    if (response.disposition === "uncertain") return { disposition: "uncertain" };
    if (response.disposition !== "saved-local" || typeof response.message_id !== "string" || !/^[0-9a-f]{64}$/.test(response.message_id) || !decimal(response.ordinal)) {
      throw new Error("The message reply could not be confirmed. Retry the same message.");
    }
    const cleanupPending = await complete(intent);
    return deps.current() ? { disposition: "saved-local", action: response.message_id, ordinal: response.ordinal, cleanupPending } : null;
  }
  async function reviewDecision(intent: NativeTextReviewIntent): Promise<{ status: NativeReviewDisposition; cleanupPending: boolean } | null> {
    if (!await prepare(intent) || !ready(intent)) return null;
    const { kind: _kind, token, ...settings } = intent;
    const nonce = token.slice(32);
    const result = await deps.invoke("decide_native_chat_held_post", { ...settings, intent: nonce });
    if (!deps.current()) return null;
    const response = object(result);
    const { status } = response;
    const fields = Object.keys(response).length;
    const confirmed = status === "saved"
      ? fields === 6 && nonzeroId(response.decisionBodyId) && nonzeroId(response.streamEntryId)
      : status === "superseded" && fields === 4;
    if ((status !== "saved" && status !== "superseded") || !confirmed || response.intent !== nonce || response.choice !== intent.choice || response.caseId !== intent.caseId) {
      throw new Error("The decision could not be confirmed. Retry the same decision.");
    }
    const cleanupPending = await complete(intent);
    return deps.current() ? { status, cleanupPending } : null;
  }
  return {
    async found(settings: NativeFoundSettings): Promise<{ server: NativeTextServer; cleanupPending: boolean } | null> {
      if (!deps.current()) return null;
      if (settings.serverName.length > 256 || !settings.displayName.trim()) throw new Error("Choose a display name and a group name of at most 256 bytes.");
      // Resolve the native default BEFORE journal admission and dispatch. Retry
      // comparison and native's frozen request always see these SAME settings.
      const resolved = { displayName:settings.displayName, serverName: settings.serverName.trim() ? settings.serverName : settings.displayName,
        advertise:settings.advertise,relay:settings.relay,rendezvous:settings.rendezvous,isDm:settings.isDm };
      const existing = Object.values(deps.read()).find((item): item is NativeTextFoundIntent => item.kind === "found");
      const intent: NativeTextFoundIntent = existing ?? { kind: "found", token: deps.token(), ...resolved };
      if (existing && !same(existing, { kind: "found", token: existing.token, ...resolved })) throw new Error("Finish creating the previous group before changing its settings.");
      if (!await prepare(intent) || !ready(intent)) return null;
      const response = object(await deps.invoke("found_server", { requestId: intent.token, displayName: intent.displayName, serverName: intent.serverName,
        advertise: intent.advertise, relay: intent.relay, rendezvous: intent.rendezvous, isDm: intent.isDm }));
      if (!deps.current()) return null;
      if (response.backend !== "text-v2" || response.context !== intent.token || response.is_dm !== intent.isDm) throw new Error("The group reply could not be confirmed. Retry the same request.");
      const session = decodeRestoredChatSession({ backend: "text-v2", servers: [{ backend: "text-v2", server: response.server, context: response.context,
        channel: response.channel, name: intent.serverName, display_name: intent.displayName, is_dm: response.is_dm }] });
      if (session?.backend !== "text-v2") throw new Error("The group reply could not be confirmed.");
      const cleanupPending = await complete(intent);
      return deps.current() ? { server: session.servers[0], cleanupPending } : null;
    },
    async send(server: number, channel: string, expectedContext: string, text: string, replyTo: string | null = null) {
      if (!deps.current()) return null;
      const existing = Object.values(deps.read()).find((item): item is NativeTextCreateIntent => item.kind === "create" && item.server === server
        && item.channel === channel && item.expectedContext === expectedContext && item.text === text && (item.replyTo ?? null) === replyTo);
      const intent: NativeTextCreateIntent = existing ?? { kind: "create", token: deps.token(), server, channel, expectedContext, text,
        ...(replyTo === null ? {} : { replyTo }) };
      return dispatch(intent);
    },
    async change(settings: NativeMutationSettings) {
      if (!deps.current()) return null;
      const existing = Object.values(deps.read()).find((item): item is NativeTextMutationIntent =>
        item.kind === settings.kind && same(item, { ...settings, token: item.token }));
      return dispatch(existing ?? { ...settings, token: deps.token() });
    },
    async retry(intent: ActionIntent) {
      const original = deps.read()[intent.token];
      if (!deps.current() || !original || !same(original, intent)) return null;
      return dispatch(intent);
    },
    async decideReview(settings: NativeReviewSettings) {
      if (!deps.current()) return null;
      const existing = Object.values(deps.read()).find((item): item is NativeTextReviewIntent => item.kind === "review");
      const intent: NativeTextReviewIntent = { kind: "review", token: existing?.token ?? deps.token(), ...settings };
      if (existing && !same(existing, intent)) throw new Error("Finish the pending decision first.");
      return reviewDecision(intent);
    },
    async retryReview(intent: NativeTextReviewIntent) {
      const original = deps.read()[intent.token];
      if (!deps.current() || !original || !same(original, intent)) return null;
      return reviewDecision(intent);
    },
  };
}
