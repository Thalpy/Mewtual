import assert from "node:assert/strict";
import test from "node:test";
import type { ActionCaseRow, AuditReads, ReviewDecisionRow } from "./chat-review-audit.ts";
const { default: Audit } = await import("./ChatReviewAudit.svelte");
const { mount, unmount, flushSync, tick } = await import("svelte");

const id = (n: number) => n.toString(16).padStart(64, "0");
const held: ActionCaseRow = {
  kind: "held-action", rowId: `held:${id(1)}`, caseId: id(1), actionCid: id(2), createCid: null,
  realm: id(3), groupId: "0102", channel: "7", removalCutCid: id(4), admissionId: id(5),
  basisCid: id(6), grantCid: id(7), authorPublicKey: id(8), authorId: id(9), authorFingerprint: id(9).slice(0, 8),
  selectedDecisionId: null, actionKind: "create", state: "approved", presentedMs: 10,
  preview: { text: "<img src=x onerror=alert(1)>", truncated: false }, evidenceManifestCid: id(10),
  proof: { signature: "valid", authority: "owner-attested", selection: "final" },
};
const page = { status: "page", availability: "complete", revision: "r1", rows: [held], knownRowCount: 1, nextCursor: null };
const detail = { status: "detail", availability: "complete", revision: "r1", case: held,
  action: { kind: "create", text: "<script>retained body</script>", replyTo: null }, signedActionHex: "0102",
  signatureHex: "ab".repeat(64), factIds: [], missingFactIds: [], evidenceTruncated: false };
async function settle() { await tick(); await Promise.resolve(); flushSync(); }
function surface(reads: AuditReads, nameOfIdentity: (identity: string) => string = () => "Member") {
  const target = document.createElement("div"); document.body.appendChild(target);
  const mounted = mount(Audit, { target, props: { reads, nameOfIdentity } }); flushSync();
  return { target, async click(label: string) {
    const button = [...target.querySelectorAll("button")].find((b) => b.textContent?.trim() === label);
    assert.ok(button, label); button.click(); await settle();
  }, async done() { await unmount(mounted); target.remove(); } };
}

test("compiled admin log retains an approved case and renders held text as text", async () => {
  const calls: unknown[] = [];
  const ui = surface({ page: async () => page, detail: async (...args) => { calls.push(args); return detail; } });
  try {
    await settle(); assert.match(ui.target.textContent!, /Approved/);
    assert.match(ui.target.textContent!, /<img src=x onerror=alert\(1\)>/);
    assert.equal(ui.target.querySelector("img"), null);
    await ui.click("View details"); assert.deepEqual(calls, [[id(1), "r1"]]);
    assert.match(ui.target.textContent!, /<script>retained body<\/script>/);
    assert.equal(ui.target.querySelector("script"), null);
    await ui.click("Close details"); assert.equal(ui.target.querySelector('[aria-label="Held message details"]'), null);
  } finally { await ui.done(); }
});

test("compiled admin log distinguishes unavailable and incomplete from a complete empty log", async () => {
  for (const reply of [{ status: "unavailable", code: "evidence-unavailable" },
    { ...page, availability: "incomplete", rows: [], knownRowCount: 0 }]) {
    const ui = surface({ page: async () => reply, detail: async () => { throw new Error("No detail request expected"); } });
    try {
      await settle(); assert.doesNotMatch(ui.target.textContent!, /No held messages or review decisions/);
      assert.match(ui.target.textContent!, reply.status === "unavailable" ? /could not be loaded/ : /Some review history is still missing/);
    } finally { await ui.done(); }
  }
});

test("compiled log qualifies final case outcomes in both the row and detail header", async () => {
  for (const state of ["approved", "rejected", "reopened"] as const) {
    for (const [proof, expected] of [
      [{ signature: "valid", authority: "pending", selection: "pending" }, "Waiting for evidence"],
      [{ signature: "invalid", authority: "owner-attested", selection: "final" }, "Unverified outcome"],
      [{ signature: "valid", authority: "owner-attested", selection: "conflict" }, "Conflicting evidence"],
    ] as const) {
      const entry = { ...held, state, proof };
      const ui = surface({ page: async () => ({ ...page, rows: [entry] }), detail: async () => ({ ...detail, case: entry }) });
      try {
        await settle(); assert.equal(ui.target.querySelector(".review-state")?.textContent, expected);
        await ui.click("View details"); assert.match(ui.target.querySelector("h4")!.textContent!, new RegExp(expected));
      } finally { await ui.done(); }
    }
  }
});

test("compiled admin log erases previews after native access loss and never opens ordinary chat", async () => {
  let reads = 0;
  const ui = surface({ page: async () => ++reads === 1 ? page : { status: "unavailable", code: "not-authorized" },
    detail: async () => detail });
  try {
    await settle(); assert.ok(ui.target.querySelector("[data-review-row]"));
    await ui.click("Refresh"); assert.equal(ui.target.querySelector("[data-review-row]"), null);
    assert.doesNotMatch(ui.target.textContent!, /onerror/); assert.equal(ui.target.querySelector("a"), null);
  } finally { await ui.done(); }
});

test("unmounting the compiled log fences delayed held detail", async () => {
  let resolve!: (value: unknown) => void;
  const pending = new Promise((yes) => { resolve = yes; });
  const ui = surface({ page: async () => page, detail: () => pending });
  await settle(); await ui.click("View details"); await ui.done(); resolve(detail); await settle();
  assert.equal(ui.target.textContent, "");
});

function decision(n: number): ReviewDecisionRow {
  return { ...held, kind: "review-decision", rowId: `decision:${id(n)}`, decisionId: id(n),
    recordKind: "decision", choice: "approve", issuerPublicKey: id(n + 30), issuerId: id(n + 40),
    issuerFingerprint: id(n + 40).slice(0, 8), authorityBasis: id(50), predecessors: [],
    reason: "Reviewed", effect: "selected" };
}

test("held authors with colliding fingerprints resolve only by their full verified identity", async () => {
  const first = held;
  const second = { ...held, caseId: id(20), rowId: `held:${id(20)}`, authorId: id(21) };
  assert.equal(first.authorFingerprint, second.authorFingerprint);
  const seen: string[] = [];
  const ui = surface({ page: async () => ({ ...page, rows: [first, second], knownRowCount: 2 }), detail: async () => detail },
    identity => { seen.push(identity); return identity === first.authorId ? "First author" : "Second author"; });
  try {
    await settle(); assert.deepEqual(new Set(seen), new Set([first.authorId, second.authorId]));
    assert.match(ui.target.textContent!, /First author/); assert.match(ui.target.textContent!, /Second author/);
  } finally { await ui.done(); }
});

test("decision issuers with colliding fingerprints resolve separately from the target author", async () => {
  const first = decision(30), second = decision(31);
  assert.equal(first.issuerFingerprint, second.issuerFingerprint);
  const seen: string[] = [];
  const ui = surface({ page: async () => ({ ...page, rows: [first, second], knownRowCount: 2 }), detail: async () => detail },
    identity => { seen.push(identity); return identity === first.issuerId ? "First reviewer" : "Second reviewer"; });
  try {
    await settle(); assert.deepEqual(new Set(seen), new Set([first.issuerId, second.issuerId]));
    assert.match(ui.target.textContent!, /First reviewer/); assert.match(ui.target.textContent!, /Second reviewer/);
    assert.ok(!seen.includes(first.authorId), "a decision proof must not verify the target author's attribution");
  } finally { await ui.done(); }
});

test("invalid and pending author or issuer signatures never resolve a trusted profile name", async () => {
  for (const signature of ["invalid", "pending"] as const) {
    for (const row of [held, decision(32)]) {
      const entry = { ...row, proof: { ...row.proof, signature } };
      const ui = surface({ page: async () => ({ ...page, rows: [entry] }), detail: async () => detail },
        () => { assert.fail("unverified identity reached trusted-name resolver"); });
      try {
        await settle(); assert.match(ui.target.textContent!, row.kind === "held-action" ? /Unverified author/ : /Unverified reviewer/);
      } finally { await ui.done(); }
    }
  }
});
