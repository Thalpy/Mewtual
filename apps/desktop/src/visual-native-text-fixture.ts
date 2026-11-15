import type { NativeOriginal } from "./native-text-history.ts";

/** Development-only IPC data for the actual Native Text App route. This models
 * local saves, never shared history or peer delivery. A new fixture owns a
 * fresh journal and rows so browser interactions cannot affect another tab. */
export function createVisualNativeTextFixture(empty = false) {
  const context = "a1".repeat(32);
  const server = { backend: "text-v2", server: 1, context, channel: "1", name: "Lantern Room", display_name: "Rowan", is_dm: false };
  const servers = empty ? [] : [server];
  const rows: NativeOriginal[] = empty ? [] : [
    { ordinal: "0", action: "b1".repeat(32), signer: "c1".repeat(32), channel: "1", presented_ms: "1787227200000", status: "pending",
      body: { kind: "create", text: "A message saved on this device.", reply_to: null } },
    { ordinal: "1", action: "b2".repeat(32), signer: "c1".repeat(32), channel: "1", presented_ms: "1787227200001", status: "held",
      body: { kind: "create", text: "A second saved message, with a longer line to check wrapping at the desktop window size.", reply_to: null } },
  ];
  const receipts = new Map<string, { text: string; ordinal: string }>();
  let ui = JSON.stringify({ version: 4, drafts: empty ? {} : { "1:1": "A deterministic composer draft" }, nativeTextIntents: {} });
  return (command: string, args: Record<string, unknown>): { handled: true; value: unknown } | { handled: false } => {
    const result = (value: unknown) => ({ handled: true as const, value });
    if (command === "resume_session") return result(structuredClone({ backend: "text-v2", servers }));
    if (command === "get_ui_state") return result(ui);
    if (command === "save_ui_state") { ui = String(args.json); return result(null); }
    if (command === "found_server") {
      if (servers.length && servers[0].context !== args.requestId) throw new Error("Native Text visual fixture already has a group");
      server.context=String(args.requestId);server.name=String(args.serverName);server.display_name=String(args.displayName);
      if (!servers.length) servers.push(server);
      return result({backend:"text-v2",server:1,context:server.context,channel:"1",is_dm:false});
    }
    if (command === "get_message_page" || command === "send_message") {
      if (args.server !== 1 || args.channel !== "1" || args.expectedContext !== server.context || !servers.length) throw new Error("Unknown Native Text fixture conversation");
      if (command === "get_message_page") {
        const start = Number(args.start), end = args.endExclusive === null ? rows.length : Number(args.endExclusive);
        const next = Math.min(end, start + Number(args.limit));
        return result(structuredClone({ backend: "text-v2", scope: "own-saved-originals", next: String(next), end_exclusive: String(end), items: rows.slice(start, next) }));
      }
      const token = String(args.retryToken), text = String(args.text);
      const prior = receipts.get(token);
      if (prior && prior.text !== text) throw new Error("Native Text fixture retry conflict");
      const ordinal = prior?.ordinal ?? String(rows.length);
      if (!prior) {
        receipts.set(token, { text, ordinal });
        rows.push({ ordinal, action: token, signer: "c1".repeat(32), channel: "1", presented_ms: "1787227200002", status: "pending", body: { kind: "create", text, reply_to: null } });
      }
      return result({ backend: "text-v2", disposition: "saved-local", retry_token: token, message_id: token, ordinal });
    }
    // Accidentally entering a Legacy server read must remain visible during
    // visual validation. Global app/settings commands keep their usual mocks.
    if ("server" in args) throw new Error(`Legacy server command in Native Text fixture: ${command}`);
    return { handled: false };
  };
}
