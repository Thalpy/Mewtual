import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { flushSync, mount, unmount } from "svelte";
import ChatAdminLedger from "./ChatAdminLedger.svelte";
import type { LedgerPage, LedgerDetail, LedgerReads } from "./chat-admin-ledger.ts";
import { identityLabel, sharedFingerprints } from "./call-identity.ts";

function fixture(): { page: LedgerPage; detail: LedgerDetail } {
  const key = "11".repeat(32), author = "22".repeat(32), hash = "33".repeat(32);
  const row = { kind: "legacy-change" as const, rowId: `legacy:1:${hash}`, opHash: hash,
    namespaceGroupId: "44".repeat(32), channel: "1", authorPublicKey: key, authorId: author,
    authorFingerprint: author.slice(0, 8), claimedKeyByteCount: 32, keyShape: "ed25519" as const,
    signedDocType: 1, signedDocId: "1", scope: "matching" as const, signature: "valid" as const,
    groupBinding: "not-signed" as const, authority: "legacy-unproven" as const,
    changeHash: null, graph: "unavailable" as const, presentedMs: null,
    preview: { text: "<img src=x onerror=alert(1)> original post", truncated: false } };
  const page: LedgerPage = { status: "page", availability: "incomplete", revision: "revision-1", rows: [row],
    coverage: { source: "retained-local-inventory", typed: "not-active", legacy: "incomplete",
      ownerEvidence: "not-active", unattributedChanges: "unknown" }, knownRowCount: 1, nextCursor: null };
  const detail: LedgerDetail = { status: "detail", availability: "incomplete", revision: page.revision, row,
    original: { encoding: "hex", bytes: "abcd", offset: 0, totalBytes: 2, nextOffset: null }, typedBody: null,
    signatureHex: "aa".repeat(64), factIds: [], missingFactIds: [], evidenceTruncated: false,
    legacyChange: { status: "decoded", actor: key, sequence: "1", startOp: "1", parents: [], mutationsTruncated: false,
      mutations: [{ operationId: `1@${key}`, objectId: "_root", key: { kind: "map", value: "text" }, operation: "set",
        insert: false, value: { kind: "string", text: "<script>previous deleted text</script>" }, predecessors: [] }] } };
  return { page, detail };
}
async function settle() { for (let i = 0; i < 12; i++) { await Promise.resolve(); flushSync(); } }
function button(target: HTMLElement, text: string) {
  const found = [...target.querySelectorAll("button")].find((node) => node.textContent?.includes(text));
  assert.ok(found, `missing button ${text}`); return found;
}

test("compiled history component renders original untrusted content as inert text and opens exact evidence", async () => {
  const f = fixture(), calls: unknown[][] = [], target = document.createElement("div"); document.body.append(target);
  const reads: LedgerReads = { page: async () => f.page, detail: async (...args) => { calls.push(args); return f.detail; } };
  const instance = mount(ChatAdminLedger, { target, props: { reads, nameOfIdentity: () => "Former member", channelName: () => "general" } });
  try {
    await settle();
    assert.match(target.textContent ?? "", /Former member/);
    assert.match(target.textContent ?? "", /Some records or author details may be missing/);
    assert.equal(target.querySelector("img, script"), null);
    button(target, "original post").click(); await settle();
    assert.deepEqual(calls, [[f.page.rows[0].rowId, f.page.revision, 0]]);
    assert.match(target.textContent ?? "", /<script>previous deleted text<\/script>/);
    assert.equal(target.querySelector("img, script"), null);
    assert.match(target.textContent ?? "", /Not proven by this original/);
    button(target, "Close details").click(); await settle();
    assert.equal(target.querySelector(".history-detail"), null);
  } finally { await unmount(instance); target.remove(); }
});

test("compiled component retires a pending original when its authorized mount disappears", async () => {
  const f = fixture(), target = document.createElement("div"); document.body.append(target);
  let resolve!: (value: unknown) => void;
  const pending = new Promise<unknown>((done) => { resolve = done; });
  const instance = mount(ChatAdminLedger, { target, props: {
    reads: { page: async () => f.page, detail: () => pending }, nameOfIdentity: () => "Member", channelName: () => "general",
  } });
  await settle(); button(target, "original post").click(); await settle();
  await unmount(instance); resolve(f.detail); await settle();
  assert.equal(target.textContent, ""); target.remove();
});

test("unverified originals cannot impersonate a known member name in the history list", async () => {
  const f = fixture(), target = document.createElement("div"); document.body.append(target);
  f.page.rows[0].signature = "invalid";
  const instance = mount(ChatAdminLedger, { target, props: {
    reads: { page: async () => f.page, detail: async () => f.detail },
    nameOfIdentity: () => "A real member name", channelName: () => "general",
  } });
  try {
    await settle();
    assert.match(target.textContent ?? "", /Unverified author/);
    assert.match(target.textContent ?? "", /Invalid signature/);
    assert.doesNotMatch(target.textContent ?? "", /A real member name/);
    button(target, "original post").click(); await settle();
    assert.match(target.textContent ?? "", /Claimed author/);
  } finally { await unmount(instance); target.remove(); }
});

test("a valid different signer with a colliding fingerprint cannot inherit a roster member's name", async () => {
  const f = fixture(), target = document.createElement("div"); document.body.append(target);
  const trusted = "22222222" + "aa".repeat(28), other = "22222222" + "bb".repeat(28);
  const roster = [{ identity: trusted, fingerprint: "22222222", you: false }];
  // Use the actual App resolver, rather than a test-only resolver that already fixes the bug.
  const app = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
  const start = app.indexOf("  function nameOfIdentity("), end = app.indexOf("  function colorOfIdentity(", start);
  assert.ok(start >= 0 && end > start);
  const nameOfIdentity = new Function("roster", "nameOf", "identityLabel", "rosterSharedIds",
    ts.transpile(app.slice(start, end), {target: ts.ScriptTarget.ES2022}) + ";return nameOfIdentity;")(
      roster, () => "Trusted owner", identityLabel, sharedFingerprints(roster));
  f.page.rows[0].authorId = other;
  f.detail.row.authorId = other;
  const instance = mount(ChatAdminLedger, { target, props: {
    reads: {page: async () => f.page, detail: async () => f.detail}, nameOfIdentity, channelName: () => "general",
  } });
  try {
    await settle();
    assert.doesNotMatch(target.textContent ?? "", /Trusted owner/);
    assert.match(target.textContent ?? "", /22222222/);
    button(target, "original post").click(); await settle();
    assert.match(target.textContent ?? "", new RegExp(other));
    assert.doesNotMatch(target.textContent ?? "", /Trusted owner/);
    // The positive control confirms exact current-roster identity still resolves normally.
    assert.equal(nameOfIdentity(trusted), "Trusted owner");
  } finally { await unmount(instance); target.remove(); }
});
