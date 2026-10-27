import assert from "node:assert/strict";
import test from "node:test";
import { NativeCoordinatorDiagnostics } from "./voice-coordinator-diagnostics.ts";

test("coordinator diagnostics preserve the first four closed non-Capacity refusals", () => {
  const diagnostic = new NativeCoordinatorDiagnostics(100);
  for (const bytes of [[1, 3, 7], [1, 3, 99], [9, 3, 4], [1, 0, 0]])
    diagnostic.refusal(Uint8Array.from(bytes), 110);
  for (const [offset, bytes] of [[1, 3, 4], [1, 3, 5], [1, 1, 0], [1, 2, 0], [1, 3, 8]].entries())
    diagnostic.refusal(Uint8Array.from(bytes), 120 + offset);
  const summary = diagnostic.summary(1, 1);
  assert.match(summary, /first=3\.4@20,3\.5@21,1\.0@22,2\.0@23/);
  assert.doesNotMatch(summary, /3\.7|3\.99|3\.8/);
});

test("coordinator diagnostics copy tags, never retain caller reply bytes or arbitrary bodies", () => {
  const diagnostic = new NativeCoordinatorDiagnostics(0), bytes = Uint8Array.from([1, 3, 4]);
  diagnostic.refusal(bytes, 1); bytes.fill(255);
  diagnostic.refusal(new TextEncoder().encode("private SDP and microphone credential"), 2);
  const summary = diagnostic.summary(0, 0);
  assert.match(summary, /3\.4@1/);
  assert.doesNotMatch(summary, /255|SDP|credential/);
});

test("coordinator diagnostic malformed providers cannot replace the original failure", () => {
  const diagnostic = new NativeCoordinatorDiagnostics(0);
  const broken = new Proxy(new Uint8Array(3), { get() { throw new Error("provider failure"); } });
  assert.doesNotThrow(() => diagnostic.refusal(broken, 1));
  assert.doesNotThrow(() => diagnostic.refusal(null, 1));
  assert.match(diagnostic.summary(0, 0), /first= plans=/);
});

test("own byteLength getters cannot reenter recording or overfill its four-entry bound", () => {
  const diagnostic = new NativeCoordinatorDiagnostics(0), bytes = Uint8Array.from([1, 3, 4]);
  let getters = 0;
  Object.defineProperty(bytes, "byteLength", { get() {
    getters++;
    for (let i = 0; i < 4; i++) diagnostic.refusal(Uint8Array.from([1, 3, 5]), 1);
    return 3;
  } });
  diagnostic.refusal(bytes, 2);
  assert.equal(getters, 0);
  for (let i = 0; i < 20; i++) diagnostic.refusal(Uint8Array.from([1, 3, 8]), 3 + i);
  const first = diagnostic.summary(0, 0).split("first=")[1].split(" plans=")[0];
  assert.equal(first.split(",").length, 4);
  assert.match(first, /^3\.4@2,/);
});

test("switching typed-array Proxies cannot substitute unvalidated diagnostic text", () => {
  const diagnostic = new NativeCoordinatorDiagnostics(0);
  let getters = 0;
  const bytes = new Proxy(Uint8Array.from([1, 3, 4]), { get(target, key) {
    getters++;
    if (key === "byteLength") return 3;
    if (getters > 4) return "private SDP and microphone credential";
    return Reflect.get(target, key);
  } });
  diagnostic.refusal(bytes, 1);
  assert.equal(getters, 0, "intrinsic storage validation rejects Proxy without callbacks");
  const summary = diagnostic.summary(0, 0);
  assert.match(summary, /first= plans=/);
  assert.doesNotMatch(summary, /SDP|credential/);
});

test("coordinator stage history keeps only six closed metadata stages", () => {
  const diagnostic = new NativeCoordinatorDiagnostics(0), generation = {};
  for (const [now, stage] of ["waiting", "confirmation", "handoff", "pc", "rtc", "media", "retire", "compact", "failed", "advance", "stopped"].entries())
    diagnostic.observe(stage, generation, null, null, now);
  const summary = diagnostic.summary(0, 0);
  assert.match(summary, /stages=medi@5,reti@6,comp@7,fail@8,adva@9,stop@10/);
  assert.doesNotMatch(summary, /wait@|conf@|hand@/);
  diagnostic.observe("private SDP and microphone credential", generation, null, null, 11);
  assert.match(diagnostic.summary(0, 0), /\?@11/);
  assert.doesNotMatch(diagnostic.summary(0, 0), /SDP|credential/);
});

test("coordinator plan observations cannot append a third generation's failure to an old plan", () => {
  const diagnostic = new NativeCoordinatorDiagnostics(100), first = {}, second = {}, third = {};
  diagnostic.observe("waiting", first, 115, null, 100);
  diagnostic.observe("failed", first, 115, "retired", 120, true);
  diagnostic.observe("waiting", second, 130, null, 121);
  diagnostic.observe("failed", second, 130, "capacity", 140, true);
  diagnostic.observe("waiting", third, 150, null, 141);
  diagnostic.observe("failed", third, 150, "invalid", 160, true);
  const summary = diagnostic.summary(1, 1);
  assert.match(summary, /plans=link@0\/due@15\/fail@20\/retired;link@21\/due@30\/fail@40\/capacity/);
  assert.doesNotMatch(summary, /link@41|\/fail@60|\/invalid/);
});

test("a closed-source scheduling stage during successful retirement is not a failed negotiation", () => {
  const diagnostic = new NativeCoordinatorDiagnostics(0), generation = {};
  diagnostic.observe("waiting", generation, 15_000, null, 0);
  diagnostic.observe("failed", generation, 15_000, "retired", 20_000, false);
  assert.doesNotMatch(diagnostic.summary(1, 1), /\/fail@/);
  // A separately observed actual negative latch records local intent, not drainage.
  diagnostic.observe("failed", generation, 15_000, "retired", 21_000, true);
  assert.match(diagnostic.summary(1, 1), /\/fail@21000\/retired/);
});

test("repeated failure observations cannot grow a plan summary or leak SDK error text", () => {
  const diagnostic = new NativeCoordinatorDiagnostics(0), generation = {};
  diagnostic.observe("waiting", generation, 15_000, null, 0);
  for (let i = 1; i <= 1000; i++) {
    diagnostic.observe("failed", generation, 15_000, "private SDP and microphone credential", i * 2, true);
    diagnostic.observe("waiting", generation, 15_000, null, i * 2 + 1);
  }
  const summary = diagnostic.summary(0, 0);
  assert.equal((summary.match(/\/fail@/g) ?? []).length, 1);
  assert.match(summary, /\/fail@2\/unknown/);
  assert.ok(summary.length <= 320);
  assert.doesNotMatch(summary, /SDP|credential/);
});

test("a new coordinator observation cannot reuse a prior invocation's refusal or failure", () => {
  const old = new NativeCoordinatorDiagnostics(0);
  old.refusal(Uint8Array.from([1, 3, 4]), 10);
  old.observe("failed", {}, 15, "retired", 20, true);
  assert.match(old.summary(0, 0), /3\.4@10.*\/fail@20/);
  const next = new NativeCoordinatorDiagnostics(100);
  const summary = next.summary(0, 0);
  assert.match(summary, /first= plans= stages=$/);
  assert.doesNotMatch(summary, /3\.4|fail@|retired/);
});

test("invalid clock/count metadata neither coerces provider objects nor grows output", () => {
  const hostile = { valueOf() { throw new Error("must not coerce"); } };
  const diagnostic = new NativeCoordinatorDiagnostics(hostile);
  diagnostic.refusal(Uint8Array.from([1, 3, 4]), hostile);
  diagnostic.observe("waiting", {}, Infinity, hostile, 1);
  assert.match(diagnostic.summary(hostile, Infinity), /done=\? sdk=\? first= plans= stages=$/);
  const bounded = new NativeCoordinatorDiagnostics(0);
  for (const [i, code] of [4, 5, 8, 9].entries()) bounded.refusal(Uint8Array.from([1, 3, code]), 100_000 + i);
  for (let i = 0; i < 2; i++) {
    const generation = {};
    bounded.observe("confirmation", generation, -120_000, null, 100_000 + i);
    bounded.observe("failed", generation, -120_000, "inventory-overflow", 110_000 + i, true);
  }
  assert.ok(bounded.summary(16, 16).length <= 320);
  const before = bounded.summary(1, 1);
  for (const now of [-1, Infinity, NaN, 120_001]) bounded.observe("stopped", {}, 1, null, now);
  assert.equal(bounded.summary(1, 1), before);
});
