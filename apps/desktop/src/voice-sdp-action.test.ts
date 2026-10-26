import test from "node:test";
import assert from "node:assert/strict";
import { webcrypto, createHash } from "node:crypto";
import { VoiceSdpActionOwner, VoiceSdpInputLoan, voiceSdpUtf8, VOICE_SDP_BROWSER_BOOK_BYTES, type VoiceSdpActionNative } from "./voice-sdp-action.ts";
import type { VoiceSdpInputExpectation } from "./voice-ipc-sdp-input.ts";
import type { VoiceSdpHistory } from "./voice-ipc-sdp-history.ts";
import type { VoiceIpcReceipt } from "./voice-ipc-replies.ts";

const real = webcrypto.subtle;
const hash = (n: number) => n.toString(16).padStart(64, "0");
const row = "00000000000000010000000000000001", parentRow = "00000000000000000000000000000001";
const identity = Object.freeze({ action: "9007199254741101", version: "1", inputTag: hash(8) });
const gate = () => { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; };
async function until(check: () => boolean) {
  for (let n = 0; n < 2000; n++) { if (check()) return; await new Promise<void>(yes => setImmediate(yes)); }
  assert.fail("bounded observer did not see the expected stage");
}
function descriptor(expected: VoiceSdpInputExpectation, body: Uint8Array, aad: Uint8Array): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(277 + aad.length + body.length), view = new DataView(bytes.buffer);
  const put = (at: number, hex: string) => { for (let i = 0; i < hex.length / 2; i++) bytes[at + i] = Number.parseInt(hex.slice(2 * i, 2 * i + 2), 16); };
  bytes.set([8, 3]); put(2, row); put(18, parentRow); view.setBigUint64(34, BigInt(identity.action)); view.setBigUint64(42, 1n);
  put(50, identity.inputTag); view.setBigUint64(82, 1n); put(90, expected.ready); bytes[122] = 1; put(123, expected.key);
  view.setBigUint64(155, 1n); put(163, expected.sdkProof); put(195, expected.storage); put(227, expected.pcProof);
  view.setBigUint64(259, 5n); view.setUint32(267, 4 * (aad.length + body.length + (expected.purpose === "seal" ? 16 : 0)) + 3 * 8192 + 2048);
  bytes[271] = Number(expected.purpose === "open"); bytes[272] = 0; // LOW seals Offer; HIGH opens Offer.
  view.setUint16(273, aad.length); view.setUint16(275, body.length); bytes.set(aad, 277); bytes.set(body, 277 + aad.length); return bytes;
}
/** Real bounded WebCrypto operation, mocked native authority and metadata only. The
 * separate retained-channel tests exercise authenticated IPC framing with this driver. */
async function fixture(purpose: "seal" | "open" = "seal", length = 8192, brokenTag = false) {
  const expected: VoiceSdpInputExpectation = { row, parentRow, identity, purpose, generation: "1", ready: hash(1), key: hash(2),
    sdkGeneration: "1", sdkProof: hash(3), storage: hash(4), pcProof: hash(5), localIsLow: purpose === "seal" };
  const plaintext = new Uint8Array(length).fill(97), aad = new Uint8Array(579).fill(4), iv = new Uint8Array(12);
  new DataView(iv.buffer).setBigUint64(4, 5n);
  const key = await real.importKey("raw", new Uint8Array(32).fill(7), "AES-GCM", false, ["encrypt", "decrypt"]);
  const ciphertext = new Uint8Array(await real.encrypt({ name: "AES-GCM", iv, additionalData: aad }, key, plaintext));
  if (brokenTag) ciphertext[0] ^= 1;
  const body = purpose === "seal" ? plaintext : ciphertext, original = descriptor(expected, body, aad);
  const counts = { begin: 0, resolve: 0, input: 0, admit: 0, compute: 0, result: 0, ack: 0, settle: 0, returned: 0 };
  let live = true, began = false, settled = false, acknowledged = false, drained = false;
  let receipt: VoiceIpcReceipt | null = null, previousOutput: Uint8Array | null = null;
  let lose: keyof typeof counts | null = null, hold: keyof typeof counts | null = null, held = gate();
  let inputBacking: Uint8Array | null = null, capacity = false, throwSdk = false, admissionHistory = false;
  let positionAvailable = true, positions = 0, peakPositions = 0;
  const oldReceipt = (): VoiceIpcReceipt => ({ action: identity.action, inputTag: identity.inputTag, outputTag: hash(90), retired: false, digest: hash(91),
    outcome: { kind: "sealed", message: hash(92), hash: hash(93) } });
  const history = (): VoiceSdpHistory => ({ row, parentRow, purpose, pending: "not-pending", identity, settled, acknowledged,
    localDrained: drained, receipt: receipt ? { ...receipt, retired: settled && acknowledged } : null,
    delivery: receipt?.outcome.kind === "opened" ? "prepared" : "none" });
  const reply = () => ({ kind: "history" as const, history: history() });
  const step = async (name: keyof typeof counts) => {
    counts[name]++; if (hold === name) await held.promise;
    if (lose === name) { lose = null; throw new Error("actual reply lost"); }
  };
  const native: VoiceSdpActionNative = {
    live: () => live,
    async reserveEngine() {
      if (!positionAvailable || positions) return null;
      positions++; peakPositions = Math.max(peakPositions, positions);
      let held = true;
      return () => { assert.equal(held, true, "one actual position return"); held = false; positions--; };
    },
    async begin(bytes) { began = true; assert.equal(bytes === null, purpose === "open"); if (bytes) assert.deepEqual(bytes, plaintext); await step("begin"); return reply(); },
    async resolve(close) { await step("resolve"); return began ? reply() : { kind: "missing", parentRow, purpose, closeRecorded: close }; },
    async input(_row, _identity, loan) { await step("input"); inputBacking = original.slice(); loan.accept(inputBacking, expected); return null; },
    async admit(_row, _identity, engineBytes) { await step("admit"); if (capacity) return { kind: "refused", code: "capacity" };
      if (admissionHistory) { receipt = oldReceipt(); return reply(); }
      return { kind: "admitted", row, identity, permit: identity.action, engineBytes }; },
    async compute(input) {
      await step("compute"); if (throwSdk) throw new Error("provider unavailable");
      try { return new Uint8Array(await real[purpose === "seal" ? "encrypt" : "decrypt"]({ name: "AES-GCM", iv, additionalData: input.aad }, key, input.body)); }
      catch (error) { if (purpose === "open" && error instanceof DOMException && error.name === "OperationError") return null; throw error; }
    },
    async digest(output) { return createHash("sha256").update(output).digest("hex"); },
    async result(_row, _identity, output, tag) {
      if (output !== null) {
        assert.equal(tag, createHash("sha256").update(output).digest("hex"));
        assert.equal(inputBacking?.every(value => value === 0), true, "Input is cleared before Result");
        if (previousOutput) assert.deepEqual(output, previousOutput); else previousOutput = output.slice();
      }
      receipt = { action: identity.action, inputTag: identity.inputTag, outputTag: tag ?? hash(90), retired: false, digest: hash(91),
        outcome: output === null ? { kind: "rejected", code: "malformed" } : purpose === "seal" ? { kind: "sealed", message: hash(92), hash: hash(93) } : { kind: "opened" } };
      await step("result"); return reply();
    },
    async ack(_row, _identity, digest) { assert.equal(digest, receipt!.digest); acknowledged = true; await step("ack"); return reply(); },
    async settle() { settled = true; await step("settle"); return reply(); },
    async returned() { assert.ok(settled && (!receipt || acknowledged)); drained = true; await step("returned"); return reply(); },
  };
  const bytes = purpose === "seal" ? voiceSdpUtf8("a".repeat(length)) : null, beginBacking = bytes?.buffer;
  const owner = new VoiceSdpActionOwner(native, bytes);
  return { owner, counts, beginBacking, input: () => inputBacking, output: () => previousOutput, history,
    end() { live = false; owner.close(); }, lose(name: keyof typeof counts) { lose = name; },
    hold(name: keyof typeof counts) { hold = name; return held; }, capacity(value: boolean) { capacity = value; },
    failSdk() { throwSdk = true; },
    positionAvailable(value: boolean) { positionAvailable = value; },
    positions: () => ({ active: positions, peak: peakPositions }),
    historical() { receipt = oldReceipt(); },
    admissionHistory() { admissionHistory = true; },
    oldExecutorSettled() { settled = true; },
  };
}

test("bounded UTF8 refuses multibyte overflow without an oversized temporary or truncation", () => {
  assert.equal(voiceSdpUtf8("a".repeat(8192)).byteLength, 8192);
  assert.equal(voiceSdpUtf8("é".repeat(4096)).byteLength, 8192);
  for (const text of ["", "a".repeat(8193), "é".repeat(4097), "€".repeat(8192), "😀".repeat(4096)]) assert.throws(() => voiceSdpUtf8(text));
  assert.deepEqual([...voiceSdpUtf8("\ud800")], [239, 191, 189], "TextEncoder's standard replacement behavior is preserved");
});

test("maximum original Seal and Open execute actual crypto once and public observers retain no bytes", async () => {
  for (const purpose of ["seal", "open"] as const) {
    const f = await fixture(purpose); const first = f.owner.attempt(); assert.equal(f.owner.attempt(), first);
    assert.equal(await first, undefined); assert.equal(await f.owner.attempt(), undefined);
    assert.equal(f.counts.compute, 1); assert.equal(f.counts.result, 1); assert.equal(f.counts.input, 1);
    assert.equal(f.input()!.every(value => value === 0), true);
    if (f.beginBacking) assert.equal(new Uint8Array(f.beginBacking).every(value => value === 0), true);
    f.end(); await f.owner.cleanup(); assert.equal(f.owner.returned, true);
    const retained = f.owner as unknown as Record<string, unknown>;
    for (const name of ["beginBytes", "loan", "output", "flight"]) assert.equal(retained[name], null, name);
  }
});

for (const name of ["begin", "admit", "result", "ack", "settle"] as const) test(`lost ${name} reply preserves original ownership without repeating SDK`, async () => {
  const f = await fixture(); f.lose(name); await assert.rejects(f.owner.attempt());
  await f.owner.attempt(); assert.equal(f.counts.compute, 1); assert.equal(f.counts.input, 1);
  assert.equal(f.counts.begin, 1); if (name === "begin") assert.equal(f.counts.resolve, 1);
  f.end(); await f.owner.cleanup();
});

test("ordinary engine pressure is retryable and never settles or recomputes before admission", async () => {
  const f = await fixture(); f.capacity(true); await f.owner.attempt();
  assert.equal(f.counts.compute, 0); assert.equal(f.counts.settle, 0); assert.equal(f.counts.input, 1);
  f.capacity(false); await f.owner.attempt(); assert.equal(f.counts.compute, 1); assert.equal(f.counts.input, 1);
  f.end(); await f.owner.cleanup();
});

for (const stage of ["input", "admit", "compute", "result"] as const) test(`End during held ${stage} joins it before native settlement/Return`, async () => {
  const f = await fixture(); const held = f.hold(stage), flight = f.owner.attempt();
  await until(() => f.counts[stage] === 1); f.end();
  let done = false; const cleanup = f.owner.cleanup().then(() => { done = true; }, () => {});
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(done, false); assert.equal(f.counts.settle, 0);
  held.release(); await flight; await cleanup;
  if (!f.owner.returned) await f.owner.cleanup();
  assert.equal(f.owner.returned, true); assert.equal(f.counts.compute, stage === "compute" || stage === "result" ? 1 : 0);
});

test("unknown Begin cleanup resolves the attempted purpose before settlement and Return", async () => {
  const f = await fixture(); f.lose("begin"); await assert.rejects(f.owner.attempt()); f.end(); await f.owner.cleanup();
  assert.equal(f.counts.resolve, 1); assert.equal(f.counts.compute, 0); assert.equal(f.counts.settle, 1); assert.equal(f.owner.returned, true);
});

test("genuine tag-only Open success and actual AEAD failure have distinct native result domains", async () => {
  const empty = await fixture("open", 0); await empty.owner.attempt();
  assert.equal(empty.output()!.byteLength, 0); assert.equal(empty.history().receipt!.outcome.kind, "opened");
  empty.end(); await empty.owner.cleanup();
  const bad = await fixture("open", 0, true); await bad.owner.attempt();
  assert.equal(bad.output(), null); assert.equal(bad.history().receipt!.outcome.kind, "rejected");
  bad.end(); await bad.owner.cleanup();
});

test("provider failure cannot masquerade as AEAD rejection or invoke the engine again", async () => {
  const f = await fixture("open"); f.failSdk(); await f.owner.attempt();
  assert.equal(f.counts.result, 0); assert.equal(f.counts.compute, 1); assert.equal(f.owner.returned, true);
  await f.owner.attempt(); assert.equal(f.counts.compute, 1);
});

test("historical terminal cannot attest an earlier executor's SDK settlement on End", async () => {
  const f = await fixture(); f.historical(); await f.owner.attempt();
  assert.equal(f.counts.compute, 0); assert.equal(f.counts.admit, 0); assert.equal(f.counts.ack, 1);
  f.end(); await assert.rejects(f.owner.cleanup()); assert.equal(f.counts.settle, 0); assert.equal(f.owner.returned, false);
  f.oldExecutorSettled(); await f.owner.cleanup(); assert.equal(f.counts.settle, 0); assert.equal(f.owner.returned, true);
});

test("engine History does not turn the attempt latch into SDK-join provenance", async () => {
  const f = await fixture(); f.admissionHistory(); await f.owner.attempt();
  assert.equal(f.counts.input, 1); assert.equal(f.counts.admit, 1); assert.equal(f.counts.compute, 0);
  assert.equal(f.input()!.every(value => value === 0), true);
  f.end(); await assert.rejects(f.owner.cleanup()); assert.equal(f.counts.settle, 0);
  f.oldExecutorSettled(); await f.owner.cleanup(); assert.equal(f.owner.returned, true); assert.equal(f.counts.settle, 0);
});

test("Input loan has one exact backing and one irreversible disposer", async () => {
  const f = await fixture(); await f.owner.attempt();
  const loan = new VoiceSdpInputLoan(); loan.clear(); assert.throws(() => loan.input());
  assert.throws(() => loan.accept(new Uint8Array(300), {} as VoiceSdpInputExpectation)); f.end(); await f.owner.cleanup();
});

test("serial browser stage inventory fits the original64KiB without retaining Begin during Result", () => {
  // Application-owned backing plus explicit SDK/fetch copies, exact fixed frame/header
  // scratch, metadata strings and closures. This is not a JavaScript VM/RSS guarantee.
  const fixed = VOICE_SDP_BROWSER_BOOK_BYTES;
  // Request admission and response prefix text coexist. The helper allowance includes
  // bounded hex/HMAC/AAD wrappers and arrays, not an unbounded HTTP-header materialization.
  const wireScratch = 144 + 2 * 288 + 4 * 40 + 2 * 32 + 2 * 12 + 1 + 1024;
  const originalBindingStrings = 2 * (32 + 20 + 20 + 64 + 64 + 64 + 1 + 64 + 1 + 64 + 64);
  const historyStrings = 2 * (32 + 32 + 4 + 11 + 20 + 20 + 64 + 20 + 64 + 64 + 6 + 64 + 64 + 64 + 8);
  const existingSdkPc = 4096; // Entire separately tested original SDK/PC record graph.
  const callbacksAndFrames = 16 * 128 + 8 * 256; // adapter closures, current/expected/decode,
  // one coalesced public flight, nested void continuations, request/descriptor/history wrappers.
  const rowClaimsAndLoans = 2 * 512 + 1024;
  // Concrete RTC owner: scalar/peer/promise/slot/observer fields, current/Clock closures,
  // one composite void flight and bounded gathering listener/timer wrappers. The concrete
  // SDK constructor retains no arbitrary caller-factory/retire closure graph.
  const rtcInventory = 256 + 4 * 128 + 3 * 128 + 128 + 256;
  // Serial composition adds only a fixed stage field and two void helper frames. The
  // first Action/history/adapter graph ends before its successor is constructed.
  const composition = 16 + 2 * 256;
  const sdkPositionLoan = 128 + 3 * 128 + 2 * 128; // bounded FIFO entry, return cell, promise/continuations
  const fixedInventory = existingSdkPc + originalBindingStrings + 2 * historyStrings + callbacksAndFrames + rowClaimsAndLoans + rtcInventory + composition + sdkPositionLoan;
  assert.ok(fixedInventory <= fixed, `complete fixed browser owner: ${fixedInventory}/${fixed}`);
  const begin = 8192 + 2 * 8419 + 2 * 8435 + 2 * 236 + 220 + wireScratch + fixed;
  const input = 2 * 66 + 2 * 82 + 2 * 9232 + 9216 + wireScratch + fixed;
  const crypto = 9064 + (8208 + 579) + 8208 + 8208 + fixed;
  const result = 8208 + 2 * 8308 + 2 * 8324 + 2 * 272 + 256 + wireScratch + fixed;
  for (const [stage, bytes] of Object.entries({ begin, input, crypto, result })) assert.ok(bytes <= 65536, `${stage}: ${bytes}/65536`);
  // Original UTF16 is consumed before the async Begin frame; it cannot coexist with the
  // later Result frame. Input's descriptor scope ends before Result encoding starts.
  assert.ok(16384 + 8192 + fixed <= 65536);
  // Remote application is serial AFTER fresh26 finally, before true26/Answer crypto.
  // Include the one owned UTF16 string AND a known RTC conversion snapshot; neither
  // survives into the60649B crypto Result stage. Opaque PC/GC heap is outside this proof.
  assert.ok(8192 + 2 * 16384 + fixed + 1024 <= 65536);
});


test("live action finish retries the same Return without repeating successful crypto", async () => {
  const f = await fixture(); await f.owner.attempt(); assert.equal(f.owner.complete, true);
  f.lose("returned"); await assert.rejects(f.owner.finish()); assert.equal(f.owner.returned, false);
  await f.owner.finish(); assert.equal(f.owner.returned, true); assert.equal(f.counts.returned, 2);
  assert.equal(f.counts.compute, 1); assert.equal(f.counts.result, 1); await f.owner.finish(); assert.equal(f.counts.returned, 2);
});

test("live action finish refuses unfinished, historical-only and unapplied Open work", async () => {
  const waiting = await fixture(); assert.throws(() => waiting.owner.finish());
  const opened = await fixture("open"); await opened.owner.attempt();
  assert.equal(opened.owner.complete, false); assert.throws(() => opened.owner.finish());
  const history = await fixture(); history.historical(); await history.owner.attempt();
  assert.equal(history.owner.complete, false); assert.throws(() => history.owner.finish());
  const rejected = await fixture("open", 10, true); await rejected.owner.attempt();
  assert.equal(rejected.owner.complete, false); assert.throws(() => rejected.owner.finish());
  history.oldExecutorSettled(); // Historical recovery cannot settle another executor's SDK.
  for (const f of [waiting, opened, history, rejected]) { f.end(); await f.owner.cleanup(); }
});


test("browser position pressure precedes Input and native engine admission", async () => {
  const f = await fixture(); f.positionAvailable(false); await f.owner.attempt();
  assert.equal(f.counts.begin, 1); assert.equal(f.counts.input, 0); assert.equal(f.counts.admit, 0);
  assert.equal(f.counts.compute, 0); assert.equal(f.counts.settle, 0);
  f.positionAvailable(true); await f.owner.attempt();
  assert.equal(f.counts.input, 1); assert.equal(f.counts.admit, 1); assert.equal(f.counts.compute, 1);
  assert.deepEqual(f.positions(), { active: 0, peak: 1 });
  f.end(); await f.owner.cleanup();
});

for (const stage of ["input", "admit", "compute"] as const) test(`actual browser position remains owned through held ${stage} and End`, async () => {
  const f = await fixture(), held = f.hold(stage), flight = f.owner.attempt();
  await until(() => f.counts[stage] === 1); assert.equal(f.positions().active, 1);
  f.end(); const cleanup = f.owner.cleanup().catch(() => {}); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(f.positions().active, 1); held.release(); await flight; await cleanup;
  if (!f.owner.returned) await f.owner.cleanup();
  assert.deepEqual(f.positions(), { active: 0, peak: 1 });
});
