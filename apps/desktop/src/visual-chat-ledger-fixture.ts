// Deterministic localhost-only renderer data. These DTOs exercise the actual retained-history UI;
// they do not stand in for native authorization, signatures or a complete typed authority archive.
import type { LedgerDetail, LedgerPage, LegacyLedgerRow } from "./chat-admin-ledger.ts";

const revision = "88".repeat(16);
const author = "a4f29c110b7d8365".repeat(4);
const key = "12".repeat(32);
const originalText = ["A first draft, before the edit.", "The revised message.", null];
const rows: LegacyLedgerRow[] = originalText.map((_, index) => {
  const opHash = `0${index + 1}`.repeat(32);
  return { kind: "legacy-change", rowId: `legacy:1:${opHash}`, opHash,
    namespaceGroupId: "44".repeat(32), channel: "1", authorPublicKey: key, authorId: author,
    authorFingerprint: author.slice(0, 8), claimedKeyByteCount: 32, keyShape: "ed25519",
    signedDocType: 1, signedDocId: "1", scope: "matching", signature: "valid",
    groupBinding: "not-signed", authority: "legacy-unproven", changeHash: null,
    graph: "unavailable", presentedMs: null, preview: null };
});
export function visualChatLedgerResponse(command: string, payload: Record<string, unknown>): unknown {
  if (payload.server !== 1) return { status: "unavailable", code: "not-authorized" };
  if (command === "get_admin_chat_ledger") {
    if (payload.cursor !== null || payload.limit !== 32) return { status: "unavailable", code: "invalid-request" };
    const page: LedgerPage = { status: "page", availability: "incomplete", revision,
      coverage: { source: "retained-local-inventory", typed: "not-active", legacy: "incomplete",
        ownerEvidence: "not-active", unattributedChanges: "unknown" }, rows, knownRowCount: rows.length, nextCursor: null };
    return structuredClone(page);
  }
  if (command === "get_admin_chat_ledger_detail") {
    if (payload.expectedRevision !== revision) return { status: "stale", revision };
    const index = rows.findIndex((row) => row.rowId === payload.rowId);
    if (index < 0 || payload.frameOffset !== 0) return { status: "unavailable", code: "invalid-request" };
    const text = originalText[index];
    const detail: LedgerDetail = { status: "detail", availability: "incomplete", revision, row: rows[index],
      // Renderer data only; exact frame/cryptographic reconstruction is tested in Rust.
      original: { encoding: "hex", bytes: "01020304", offset: 0, totalBytes: 4, nextOffset: null },
      typedBody: null, signatureHex: "ab".repeat(64), factIds: [], missingFactIds: [], evidenceTruncated: false,
      legacyChange: { status: "decoded", actor: key, sequence: String(index + 1), startOp: String(index + 1),
        parents: [], mutationsTruncated: false, mutations: [{ operationId: `${index + 1}@${key}`,
          objectId: "_root", key: { kind: "map", value: "text" }, operation: text === null ? "delete" : "set",
          insert: false, value: text === null ? null : { kind: "string", text }, predecessors: [] }] } };
    return structuredClone(detail);
  }
  throw new Error(`Unsupported chat history fixture command: ${command}`);
}
