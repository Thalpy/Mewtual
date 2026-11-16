import assert from "node:assert/strict";
import test from "node:test";
import {
  ChatReviewAuditReader, parseAdminReviewPage, parseHeldActionDetail, reviewEffectLabel, caseStateLabel,
  type ActionCaseRow, type AdminAuditRow, type AuditReads, type ReviewDecisionRow,
} from "./chat-review-audit.ts";

const id = (n: number) => n.toString(16).padStart(64, "0");
const held = (change: Partial<ActionCaseRow> = {}): ActionCaseRow => ({
  kind: "held-action", rowId: `held:${id(1)}`, caseId: id(1), actionCid: id(2), createCid: null,
  realm: id(3), groupId: "0102", channel: "7", removalCutCid: id(4), admissionId: id(5),
  basisCid: id(6), grantCid: id(7), authorPublicKey: id(8), authorId: id(9),
  authorFingerprint: id(9).slice(0, 8), selectedDecisionId: null,
  actionKind: "create", state: "held", presentedMs: 10, preview: { text: "retained <script>body</script>", truncated: false },
  evidenceManifestCid: id(10), proof: { signature: "valid", authority: "owner-attested", selection: "pending" }, ...change,
});
const decision = (): ReviewDecisionRow => ({
  ...held(), kind: "review-decision", rowId: `decision:${id(11)}`, decisionId: id(11),
  recordKind: "decision", choice: "reject", state: "rejected", issuerPublicKey: id(12), issuerId: id(13),
  issuerFingerprint: id(13).slice(0, 8), authorityBasis: id(14), predecessors: [],
  reason: "Keep the original evidence", effect: "selected",
});
const page = (revision = "r1", rows: AdminAuditRow[] = [held()], nextCursor: string | null = null) => ({
  status: "page", availability: "complete", revision, rows, knownRowCount: rows.length, nextCursor,
});
const detail = (revision = "r1", entry = held()) => ({
  status: "detail", availability: "complete", revision, case: entry,
  action: { kind: "create", text: "exact held body", replyTo: null },
  signedActionHex: "0102", signatureHex: "ab".repeat(64), factIds: [id(5)], missingFactIds: [], evidenceTruncated: false,
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function reader(reads: Partial<AuditReads> = {}) {
  return new ChatReviewAuditReader({ page: async () => page(), detail: async () => detail(), ...reads }, () => {});
}

test("privileged pages retain resolved cases and signed decisions without changing their text", () => {
  const reply = parseAdminReviewPage(page("r1", [decision(), held({ state: "approved" })]));
  assert.equal(reply.status, "page");
  if (reply.status !== "page") return;
  assert.deepEqual(reply.rows.map((r) => r.state), ["rejected", "approved"]);
  assert.equal((reply.rows[1] as ActionCaseRow).preview?.text, "retained <script>body</script>");
});

test("unknown labels, malformed bindings and invalid page inventory refuse", () => {
  const invalid: unknown[] = [
    { ...page(), status: "accepted" }, page("r1", [held({ state: "valid" as never })]),
    page("r1", [held({ actionKind: "message" as never })]),
    page("r1", [held({ authorId: "AA".repeat(32) })]),
    page("r1", [held({ authorFingerprint: "ffffffff" })]),
    page("r1", [held({ channel: "07" })]), page("r1", [held({ channel: (1n << 128n).toString() })]),
    page("r1", [held({ rowId: "held:another-case" })]),
    page("r1", [held(), held()]), { ...page(), knownRowCount: -1 }, { ...page(), knownRowCount: 2 ** 32 },
    page("r1", [], "nonadvancing"), page("r1", Array(33).fill(held())),
    page("r1", [held({ presentedMs: 8_640_000_000_000_001 })]),
  ];
  for (const value of invalid) assert.throws(() => parseAdminReviewPage(value));
});

test("byte and cursor limits apply before retaining a response", () => {
  assert.throws(() => parseAdminReviewPage(page("r1", [held({ preview: { text: "💜".repeat(129), truncated: true } })])));
  assert.throws(() => parseAdminReviewPage(page("r1", [held()], "x".repeat(513))));
  assert.throws(() => parseAdminReviewPage({ ...page(), padding: "x".repeat(1024 * 1024) }));
  assert.throws(() => parseAdminReviewPage(page("r1\n")));
});

test("page validation owns its JSON copy and preserves incomplete inventory", () => {
  const incoming = { ...page(), availability: "incomplete" };
  const parsed = parseAdminReviewPage(incoming);
  incoming.rows[0].authorPublicKey = id(50);
  assert.equal(parsed.status, "page");
  if (parsed.status === "page") { assert.equal(parsed.availability, "incomplete"); assert.equal(parsed.rows[0].authorPublicKey, id(8)); }
});

test("details require the exact signature length, action shape and target", () => {
  assert.equal(parseHeldActionDetail(detail()).status, "detail");
  assert.throws(() => parseHeldActionDetail({ ...detail(), signatureHex: "ab".repeat(32) }));
  assert.throws(() => parseHeldActionDetail({ ...detail(), action: { kind: "edit", createCid: id(20), text: "edit" } }));
  const deleted = { ...detail(), case: held({ actionKind: "delete", createCid: id(20) }), action: { kind: "delete", createCid: id(21) } };
  assert.throws(() => parseHeldActionDetail(deleted));
  assert.throws(() => parseHeldActionDetail({ ...detail(), factIds: Array(129).fill(id(1)) }));
});

test("an unknown response cannot fall back to public moderation or ordinary messages", async () => {
  let calls = 0;
  const r = reader({ page: async () => { calls++; return { status: "legacy", messages: ["private"] }; } });
  await r.load();
  assert.equal(calls, 1); assert.equal(r.state.page, null); assert.equal(r.state.detail, null);
  assert.equal(r.state.message, "Review history could not be loaded.");
});

test("disposal clears held bodies and permanently rejects an old page reply", async () => {
  const pending = deferred<unknown>();
  const r = reader({ page: () => pending.promise });
  const loading = r.load(); r.dispose(); pending.resolve(page()); await loading;
  assert.equal(r.state.page, null); assert.equal(r.state.loading, false);
  await r.load(); assert.equal(r.state.page, null);
});

test("disposal also fences an already authorized detail read", async () => {
  const pending = deferred<unknown>();
  const r = reader({ detail: () => pending.promise });
  await r.load(); const loading = r.open(id(1)); r.dispose(); pending.resolve(detail()); await loading;
  assert.equal(r.state.page, null); assert.equal(r.state.detail, null); assert.equal(r.state.detailLoading, false);
});

test("a superseded page or exception cannot replace a newer page", async () => {
  for (const rejects of [false, true]) {
    const old = deferred<unknown>(); let calls = 0;
    const r = reader({ page: () => ++calls === 1 ? old.promise : Promise.resolve(page("new")) });
    const pending = r.load(); await r.load();
    if (rejects) old.reject(new Error("old secret diagnostic")); else old.resolve(page("old"));
    await pending;
    assert.equal(r.state.page?.revision, "new"); assert.equal(r.state.message, ""); assert.equal(r.state.loading, false);
  }
});

test("native role, session or server loss clears both cached bodies and pending detail", async () => {
  for (const code of ["not-authorized", "session-unavailable", "server-unavailable", "server-superseded"]) {
    const pending = deferred<unknown>(); let calls = 0;
    const r = reader({ page: async () => ++calls === 1 ? page() : { status: "unavailable", code }, detail: () => pending.promise });
    await r.load(); const loading = r.open(id(1)); await r.load(); pending.resolve(detail()); await loading;
    assert.equal(r.state.page, null); assert.equal(r.state.detail, null); assert.equal(r.state.loading, false);
  }
});

test("an older same-scope refusal erases privileged content and fences newer outstanding positive reads", async () => {
  for (const code of ["not-authorized", "session-unavailable", "server-unavailable", "server-superseded"]) {
    for (const kind of ["page", "detail"] as const) {
      const old = deferred<unknown>(), newer = deferred<unknown>(); let calls = 0;
      const r = reader(kind === "page"
        ? { page: () => ++calls === 1 ? Promise.resolve(page()) : calls === 2 ? old.promise : calls === 3 ? newer.promise : Promise.resolve(page("reauthorized")) }
        : { detail: () => ++calls === 1 ? old.promise : newer.promise });
      await r.load();
      const a = kind === "page" ? r.load() : r.open(id(1));
      const b = kind === "page" ? r.load() : r.open(id(1));
      old.resolve({ status: "unavailable", code }); await a;
      assert.equal(r.state.page, null, `${kind}/${code} must erase before the newer reply`);
      newer.resolve(kind === "page" ? page("late") : detail()); await b;
      assert.equal(r.state.page, null); assert.equal(r.state.detail, null);
      await r.load(); assert.ok(r.state.page, "a new explicit native read can establish access again");
    }
  }
});

test("an approval proposal or unverified selected label never looks applied", () => {
  assert.equal(reviewEffectLabel({ ...decision(), effect: "unselected" }), "Not applied");
  assert.equal(reviewEffectLabel(decision()), "Unverified decision");
  assert.equal(reviewEffectLabel({ ...decision(), proof: { signature: "valid", authority: "owner-attested", selection: "final" } }), "Applied");
});

test("final case outcomes require verified evidence rather than an isolated native status label", () => {
  for (const state of ["approved", "rejected", "reopened"] as const) {
    const entry = held({ state });
    assert.equal(caseStateLabel(entry), "Waiting for evidence");
    assert.equal(caseStateLabel({ ...entry, proof: { signature: "valid", authority: "pending", selection: "final" } }), "Waiting for evidence");
    assert.equal(caseStateLabel({ ...entry, proof: { signature: "invalid", authority: "owner-attested", selection: "final" } }), "Unverified outcome");
    assert.equal(caseStateLabel({ ...entry, proof: { signature: "valid", authority: "owner-attested", selection: "conflict" } }), "Conflicting evidence");
    assert.equal(caseStateLabel({ ...entry, proof: { signature: "valid", authority: "owner-attested", selection: "final" } }),
      { approved: "Approved", rejected: "Rejected", reopened: "Reopened" }[state]);
  }
});

test("proof-only revision change invalidates details even at equal case IDs", async () => {
  const r = reader({ detail: async () => detail("proof-r2") });
  await r.load(); await r.open(id(1));
  assert.equal(r.state.detail, null); assert.equal(r.state.stale, true);
  assert.equal(r.state.message, "History changed. Refresh to continue.");
});

test("next pages cannot silently cross immutable view revisions", async () => {
  let calls = 0;
  const r = reader({ page: async () => ++calls === 1 ? page("r1", [held()], "next") : page("r2") });
  await r.load(); await r.load("next");
  assert.equal(r.state.page?.revision, "r1"); assert.equal(r.state.stale, true);
});

test("detail requests require a case in the current privileged page", async () => {
  const calls: unknown[] = [];
  const r = reader({ detail: async (...args) => { calls.push(args); return detail(); } });
  await r.open(id(1)); await r.load(); await r.open(id(99));
  assert.equal(calls.length, 0);
  await r.open(id(1)); assert.deepEqual(calls, [[id(1), "r1"]]); assert.equal(r.state.detail?.action.kind, "create");
});

test("a mismatched held-body scope never enters the detail cache", async () => {
  for (const change of [{ caseId: id(99), rowId: `held:${id(99)}` }, { actionCid: id(99) }, { realm: id(99) },
    { groupId: "abcd" }, { channel: "8" }, { admissionId: id(99) }, { removalCutCid: id(99) },
    { createCid: id(99) }, { basisCid: id(99) }, { grantCid: id(99) }, { authorPublicKey: id(99) },
    { authorId: id(99) }, { selectedDecisionId: id(99) }]) {
    const r = reader({ detail: async () => detail("r1", held(change)) });
    await r.load(); await r.open(id(1)); assert.equal(r.state.detail, null);
  }
});

test("temporary busy retains only an explicitly stale view and never raw diagnostic prose", async () => {
  let calls = 0;
  const r = reader({ page: async () => ++calls === 1 ? page() : { status: "unavailable", code: "busy" } });
  await r.load(); await r.load(); assert.ok(r.state.page); assert.equal(r.state.stale, true);
  assert.equal(r.state.message, "Review history could not be loaded.");
});
