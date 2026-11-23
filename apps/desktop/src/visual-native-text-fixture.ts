import type { NativeOriginal, NativeOwnRevision } from "./native-text-history.ts";

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
  const receipts = new Map<string, { request: string; ordinal: string }>();
  const revisions = new Map<string, NativeOwnRevision>();
  for (const row of rows) {
    if (row.body.kind === "create") revisions.set(row.action, { revision: row.action, text: row.body.text, status: row.status });
  }
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
    if (["get_message_page", "send_message", "edit_message", "delete_message"].includes(command)) {
      if (args.server !== 1 || args.channel !== "1" || args.expectedContext !== server.context || !servers.length) throw new Error("Unknown Native Text fixture conversation");
      if (command === "get_message_page") {
        if (typeof args.ownAction === "string") {
          return result({ backend: "text-v2", scope: "own-message-revision", action: args.ownAction, scanning: false, item: revisions.get(args.ownAction) ?? null });
        }
        const start = Number(args.start), end = args.endExclusive === null ? rows.length : Number(args.endExclusive);
        const next = Math.min(end, start + Number(args.limit));
        return result(structuredClone({ backend: "text-v2", scope: "own-saved-originals", next: String(next), end_exclusive: String(end), items: rows.slice(start, next) }));
      }
      const token = String(args.retryToken), text = String(args.text);
      const request = JSON.stringify([command, text, args.messageId, args.expectedRevision, args.replyTo]);
      const prior = receipts.get(token);
      if (prior && prior.request !== request) throw new Error("Native Text fixture retry conflict");
      const ordinal = prior?.ordinal ?? String(rows.length);
      if (!prior) {
        let body: NativeOriginal["body"];
        if (command === "send_message") {
          body = { kind: "create", text, reply_to: typeof args.replyTo === "string" ? args.replyTo : null };
          revisions.set(token, { revision: token, text, status: "pending" });
        } else {
          const create = String(args.messageId), revision = revisions.get(create);
          if (!revision || revision.revision !== args.expectedRevision) throw new Error("The saved message changed.");
          if (command === "edit_message") {
            body = { kind: "edit", create, text };
            revisions.set(create, { revision: token, text, status: "pending" });
          } else {
            body = { kind: "delete", create };
            revisions.delete(create);
          }
        }
        receipts.set(token, { request, ordinal });
        rows.push({ ordinal, action: token, signer: "c1".repeat(32), channel: "1", presented_ms: "1787227200002", status: "pending", body });
      }
      return result({ backend: "text-v2", disposition: "saved-local", retry_token: token, message_id: token, ordinal });
    }
    // Accidentally entering a Legacy server read must remain visible during
    // visual validation. Global app/settings commands keep their usual mocks.
    if ("server" in args) throw new Error(`Legacy server command in Native Text fixture: ${command}`);
    return { handled: false };
  };
}
