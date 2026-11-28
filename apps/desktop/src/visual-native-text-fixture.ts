import type { NativeAuditRecord, NativeHeldPost, NativeOriginal, NativeOwnRevision, NativeReceived } from "./native-text-history.ts";

/** Development-only IPC data for the actual Native Text App route. This models
 * local saves and a recovered projection. A new fixture owns a
 * fresh journal and rows so browser interactions cannot affect another tab. */
export function createVisualNativeTextFixture(empty = false) {
  const context = "a1".repeat(32);
  const server = { backend: "text-v2", server: 1, context, channel: "1", name: "Lantern Room", display_name: "Rowan", is_dm: false };
  const servers = empty ? [] : [server];
  const archived: NativeReceived | null = empty ? null : {
    action: "d3".repeat(32),
    text: "A message recovered after reconnecting.",
    certainty: "provisional",
  };
  const auditRecords: NativeAuditRecord[] = archived ? [
    { action: archived.action, signer: "d2".repeat(32), admission: "b4".repeat(32), original_digest: "e1".repeat(32),
      presented_ms: "1787227201000", parents: [], body: { kind: "create", text: archived.text, reply_to: null } },
    { action: "d4".repeat(32), signer: "d2".repeat(32), admission: "b4".repeat(32), original_digest: "e2".repeat(32),
      presented_ms: "1787227201001", parents: [archived.action],
      body: { kind: "edit", create: archived.action, text: "A retained edit from the same device." } },
  ] : [];
  const held: NativeHeldPost = {
    caseId: "da".repeat(32), action: "db".repeat(32), signer: "dc".repeat(32),
    admission: "dd".repeat(32), originalDigest: "de".repeat(32), removalHeader: "df".repeat(32),
    removalEpoch: "3",
    body: { kind: "create", text: "A late post held for review.", reply_to: null },
  };
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
    if (command === "get_native_chat_members" && args.server === 1 && args.expectedContext === server.context && servers.length) {
      return result({ canRemove: false, members: [
        { admission: "b3".repeat(32), identity: "c1".repeat(32), fingerprint: "c1c1c1c1", you: true, founder: true },
      ] });
    }
    if (command === "found_server") {
      if (servers.length && servers[0].context !== args.requestId) throw new Error("Native Text visual fixture already has a group");
      server.context=String(args.requestId);server.name=String(args.serverName);server.display_name=String(args.displayName);
      if (!servers.length) servers.push(server);
      return result({backend:"text-v2",server:1,context:server.context,channel:"1",is_dm:false});
    }
    if (["get_message_page", "send_message", "edit_message", "delete_message"].includes(command)) {
      if (args.server !== 1 || args.channel !== "1" || args.expectedContext !== server.context || !servers.length) throw new Error("Unknown Native Text fixture conversation");
      if (command === "get_message_page") {
        if ("heldAfter" in args) {
          if (args.heldAfter !== null || args.limit !== 2) {
            throw new Error("Invalid Native Text fixture held-post request");
          }
          return result({ backend: "text-v2", scope: "selected-held-posts",
            next: null, items: [structuredClone(held)], unavailable: 0 });
        }
        if ("auditAction" in args) {
          if (args.limit !== 8 || args.auditAfter !== null || (args.auditAction !== null && args.auditAction !== archived?.action)) {
            throw new Error("Invalid Native Text fixture audit request");
          }
          return result({ backend: "text-v2", scope: "selected-shared-history-audit", target: args.auditAction,
            next: null, items: structuredClone(args.auditAction === null ? auditRecords.slice(0, 1) : auditRecords) });
        }
        if (typeof args.ownAction === "string") {
          return result({ backend: "text-v2", scope: "own-message-revision", action: args.ownAction, scanning: false, item: revisions.get(args.ownAction) ?? null });
        }
        if ("archiveAfter" in args) {
          if (args.limit !== 8 || (args.archiveAfter !== null && args.archiveAfter !== archived?.action)) {
            throw new Error("Invalid Native Text fixture archive cursor");
          }
          const ids = args.archiveAfter === null && archived ? [archived.action] : [];
          return result({ backend: "text-v2", scope: "received-archive-index", available: true, ids, more: false });
        }
        if ("archivedAction" in args) {
          return result({ backend: "text-v2", scope: "received-archive", item: archived?.action === args.archivedAction ? archived : null });
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
