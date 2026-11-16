import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  ChatLedgerReader, parseLedgerPage, parseLedgerDetail,
  type LedgerPage, type LedgerDetail, type LegacyLedgerRow,
} from "./chat-admin-ledger.ts";

const hash = (digit: string) => digit.repeat(64);

test("the actual native DTO fixture crosses the renderer boundary without losing integer precision", () => {
  // Rust's serializer test pins this same artifact. Neither side can silently rename a field,
  // broaden an authority claim or round a u64 counter while testing only its own fixtures.
  const fixture = JSON.parse(readFileSync(new URL("../src-tauri/security/admin-chat-ledger-dto-fixture.json", import.meta.url), "utf8"));
  assert.deepEqual(parseLedgerPage(fixture.page), fixture.page);
  const detail = parseLedgerDetail(fixture.detail);
  assert.deepEqual(detail, fixture.detail);
  assert.equal(detail.status, "detail");
  if (detail.status === "detail") {
    assert.equal(detail.legacyChange?.sequence, "18446744073709551615");
    assert.equal(detail.legacyChange?.mutations[0].value?.text, "18446744073709551615");
  }
});
function row(digit = "3"): LegacyLedgerRow {
  return { kind: "legacy-change", rowId: `legacy:1:${hash(digit)}`, opHash: hash(digit),
    namespaceGroupId: hash("4"), channel: "1", authorPublicKey: hash("1"), authorId: hash("2"),
    authorFingerprint: "22222222", claimedKeyByteCount: 32, keyShape: "ed25519",
    signedDocType: 1, signedDocId: "1", scope: "matching", signature: "valid",
    groupBinding: "not-signed", authority: "legacy-unproven", changeHash: null,
    graph: "unavailable", presentedMs: null, preview: { text: "<script>original</script>", truncated: false } };
}
function page(rows = [row()]): LedgerPage {
  return { status: "page", availability: "incomplete", revision: "view-1", rows,
    coverage: { source: "retained-local-inventory", typed: "not-active", legacy: "incomplete",
      ownerEvidence: "not-active", unattributedChanges: "unknown" }, knownRowCount: rows.length, nextCursor: null };
}
function detail(): LedgerDetail {
  return { status: "detail", availability: "incomplete", revision: "view-1", row: row(),
    original: { encoding: "hex", bytes: "abcd", offset: 0, totalBytes: 2, nextOffset: null },
    typedBody: null, signatureHex: "aa".repeat(64), factIds: [], missingFactIds: [], evidenceTruncated: false,
    legacyChange: { status: "decoded", actor: hash("1"), sequence: "1", startOp: "18446744073709551615",
      parents: [], mutationsTruncated: false, mutations: [{ operationId: `18446744073709551615@${hash("1")}`,
        objectId: "_root", key: { kind: "map", value: "text" }, operation: "set", insert: false,
        value: { kind: "string", text: "<img src=x onerror=alert(1)>original" }, predecessors: [] }] } };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

test("retained legacy original decoding keeps exact large counters and untrusted text as strings", () => {
  const input = detail(), parsed = parseLedgerDetail(input);
  assert.equal(parsed.status, "detail");
  if (parsed.status !== "detail") return;
  assert.equal(parsed.legacyChange?.startOp, "18446744073709551615");
  assert.equal(parsed.legacyChange?.mutations[0].value?.text, "<img src=x onerror=alert(1)>original");
  input.row.preview!.text = "later mutation";
  assert.equal(parsed.row.preview?.text, "<script>original</script>");
});

test("malformed or foreign-scope originals remain inspectable without invented authorship", () => {
  const foreign = row(); foreign.signedDocType = 2; foreign.signedDocId = "99"; foreign.scope = "mismatched";
  assert.equal(parseLedgerPage(page([foreign])).status, "page");
  const bad = row(); bad.claimedKeyByteCount = 2_000_000; bad.keyShape = "invalid-length";
  bad.authorPublicKey = bad.authorId = bad.authorFingerprint = null; bad.signature = "invalid";
  assert.equal(parseLedgerPage(page([bad])).status, "page");
  bad.signature = "valid"; assert.throws(() => parseLedgerPage(page([bad])));
});

test("closed schemas reject false completeness, new authority variants and contradictory namespaces", () => {
  for (const edit of [
    (p: any) => { p.extra = "unreviewed"; },
    (p: any) => { p.availability = "complete"; },
    (p: any) => { p.rows[0].authority = "member-verified"; },
    (p: any) => { p.rows[0].kind = "typed-action"; },
    (p: any) => { p.coverage.typed = "complete"; },
    (p: any) => { p.rows[0].signedDocId = "01"; },
    (p: any) => { p.rows[0].signedDocId = "2"; },
    (p: any) => { p.knownRowCount = 0x100000000; },
  ]) { const p = page(); edit(p); assert.throws(() => parseLedgerPage(p)); }
  assert.throws(() => parseLedgerPage(page([row("4"), row("3")])));
  assert.throws(() => parseLedgerPage(page([row(), row()])));
  assert.throws(() => parseLedgerPage(page(Array.from({ length: 33 }, () => row()))));
});

test("chunk grammar refuses gaps, no-progress replies, unsafe counts and oversized bytes", () => {
  for (const edit of [
    (d: any) => { d.original.nextOffset = 2; },
    (d: any) => { d.original.bytes = ""; d.original.nextOffset = 0; },
    (d: any) => { d.original.offset = -1; },
    (d: any) => { d.original.offset = Number.MAX_SAFE_INTEGER + 1; },
    (d: any) => { d.original.bytes = "a"; },
    (d: any) => { d.original.bytes = "AA"; },
    (d: any) => { d.original.bytes = "aa".repeat(65537); },
    (d: any) => { d.legacyChange.sequence = 1; },
    (d: any) => { d.legacyChange.startOp = "18446744073709551616"; },
  ]) { const d = detail(); edit(d); assert.throws(() => parseLedgerDetail(d)); }
  const end = detail(); end.original = { encoding: "hex", bytes: "", offset: 2, totalBytes: 2, nextOffset: null };
  assert.equal(parseLedgerDetail(end).status, "detail");
});

test("reader accepts reordered JSON keys but rejects changed scope in exact detail", async () => {
  const response = detail();
  response.row = Object.fromEntries(Object.entries(response.row).reverse()) as LegacyLedgerRow;
  const reader = new ChatLedgerReader({ page: async () => page(), detail: async () => response }, () => {});
  await reader.load(); assert.equal(await reader.open(row().rowId), "loaded");
  response.row.namespaceGroupId = hash("5");
  assert.equal(await reader.open(row().rowId), "refused");
  assert.equal(reader.state.detail, null); assert.equal(reader.state.stale, true);
});

test("a retired mounted reader drops delayed page and detail bodies", async () => {
  const pending = deferred<unknown>();
  const reader = new ChatLedgerReader({ page: () => pending.promise, detail: async () => detail() }, () => {});
  const loading = reader.load(); reader.dispose(); pending.resolve(page()); await loading;
  assert.equal(reader.state.page, null); assert.equal(reader.state.detail, null);
  const late = deferred<unknown>();
  const second = new ChatLedgerReader({ page: async () => page(), detail: () => late.promise }, () => {});
  await second.load(); const opening = second.open(row().rowId); second.dispose(); late.resolve(detail()); await opening;
  assert.equal(second.state.detail, null); assert.equal(second.state.page, null);
});

for (const denialFirst of [true, false]) test(`older authorization loss fences newer positive replies, denial first=${denialFirst}`, async () => {
  const old = deferred<unknown>(), fresh = deferred<unknown>(); let calls = 0;
  const reader = new ChatLedgerReader({ page: () => ++calls === 1 ? old.promise : fresh.promise,
    detail: async () => detail() }, () => {});
  const a = reader.load(), b = reader.load();
  if (denialFirst) { old.resolve({ status: "unavailable", code: "not-authorized" }); await a; fresh.resolve(page()); await b; }
  else { fresh.resolve(page()); await b; old.resolve({ status: "unavailable", code: "not-authorized" }); await a; }
  assert.equal(reader.state.page, null); assert.equal(reader.state.detail, null);
  assert.equal(await reader.load(), "ignored");
});

test("page traversal owns one bounded page and refuses mixed revision/count/order", async () => {
  const first = page(); first.knownRowCount = 2; first.nextCursor = "opaque-next";
  const next = page([row("4")]); next.knownRowCount = 2;
  const reader = new ChatLedgerReader({ page: async (cursor) => cursor === null ? first : next, detail: async () => detail() }, () => {});
  await reader.load(); assert.equal(await reader.load(first.nextCursor), "loaded");
  assert.deepEqual(reader.state.page?.rows.map((r) => r.opHash), [hash("4")]);
  assert.equal(reader.state.page?.knownRowCount, 2);
  next.revision = "another-view";
  assert.equal(await reader.load("next-again"), "refused"); assert.equal(reader.state.stale, true);
});

test("busy next-chunk retry preserves only its original previous chunk and exact anchor", async () => {
  let busy = true;
  const first = detail(); first.original = { encoding: "hex", bytes: "abcd", offset: 0, totalBytes: 4, nextOffset: 2 };
  const next = detail(); next.original = { encoding: "hex", bytes: "ef01", offset: 2, totalBytes: 4, nextOffset: null };
  const reader = new ChatLedgerReader({ page: async () => page(), detail: async (_, __, offset) => offset === 0 ? first
    : busy ? { status: "unavailable", code: "busy" } : next }, () => {});
  await reader.load(); await reader.open(row().rowId);
  assert.equal(await reader.open(row().rowId, 2), "busy");
  assert.equal(reader.state.detail?.original.offset, 0); assert.equal(reader.state.stale, false);
  busy = false; assert.equal(await reader.open(row().rowId, 2), "loaded");
  assert.equal(reader.state.detail?.original.bytes, "ef01");
  assert.equal(await reader.open(row().rowId, 1), "ignored");
});

test("ordinary message IDs never become detail lookups", async () => {
  let calls = 0;
  const reader = new ChatLedgerReader({ page: async () => page(), detail: async () => { calls++; return detail(); } }, () => {});
  await reader.load(); assert.equal(await reader.open("a-visible-message-id"), "ignored"); assert.equal(calls, 0);
});

test("decoded projection applies the aggregate UTF8 budget across individually small fields", () => {
  const d = detail(), first = d.legacyChange!.mutations[0];
  first.value = { kind: "string", text: "🙂".repeat(8192) }; //32KiB UTF8,16KiB code units
  const second = structuredClone(first); second.operationId = `2@${hash("1")}`;
  d.legacyChange!.mutations.push(second);
  assert.throws(() => parseLedgerDetail(d)); //IDs/metadata make the combined projection >64KiB
  d.legacyChange!.mutations.pop();
  assert.equal(parseLedgerDetail(d).status, "detail");
});
