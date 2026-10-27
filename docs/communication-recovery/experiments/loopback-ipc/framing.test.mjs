import assert from "node:assert/strict";
import test from "node:test";
import { createCodec, MAX_PLAIN } from "./web/framing.mjs";
const encoder = new TextEncoder(), id = new Uint8Array(16).fill(9);
const seed = () => new Uint8Array(32).fill(7);
const concat = (a, b) => { const bytes = new Uint8Array(a.length + b.length); bytes.set(a); bytes.set(b, a.length); return bytes; };
const hex = bytes => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
async function response(request, { sequence = 1n, direction = 1, grant = 1, bytes = new Uint8Array([3]), wrongSeed = false, corrupt = false, excess = false, truncate = false, encoding = false, delayed, cancel } = {}) {
  const raw = new Uint8Array(40), view = new DataView(raw.buffer);
  raw.set(encoder.encode("MWIP")); raw[4] = 1; raw[5] = direction; raw[6] = grant;
  view.setBigUint64(8, sequence); raw.set(request.raw.subarray(16, 32), 16); view.setUint32(32, bytes.length + 16);
  const material = await crypto.subtle.importKey("raw", wrongSeed ? new Uint8Array(32).fill(8) : seed(), "HKDF", false, ["deriveKey"]);
  const derive = (label, algorithm, usage) => crypto.subtle.deriveKey({ name: "HKDF", hash: "SHA-256", salt: encoder.encode("Mewtual IPC fixture v1"), info: encoder.encode(label) }, material, algorithm, false, [usage]);
  const mac = await derive("v1 n2c header HMAC-SHA256", { name: "HMAC", hash: "SHA-256", length: 256 }, "sign");
  const key = await derive("v1 n2c body AES-256-GCM", { name: "AES-GCM", length: 256 }, "encrypt");
  const aad = concat(raw, request.raw), nonce = new Uint8Array(12);
  nonce.set(encoder.encode("RSP1")); nonce.set(raw.subarray(8, 16), 4);
  const tag = new Uint8Array(await crypto.subtle.sign("HMAC", mac, aad));
  let body = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv: nonce, additionalData: aad }, key, bytes));
  if (corrupt) body[0] ^= 1;
  if (excess) body = concat(body, new Uint8Array(1));
  if (truncate) body = body.slice(0, -1);
  const headers = { "content-length": String(bytes.length + 16), "x-probe-admission": hex(concat(raw, tag)) };
  if (encoding) headers["content-encoding"] = "gzip";
  let canceled = false;
  return new Response(new ReadableStream({ async start(controller) { if (delayed) await delayed; if (canceled) return; controller.enqueue(body); if (!cancel) controller.close(); }, cancel(reason) { canceled = true; cancel?.(reason); } }), { status: 200, headers });
}
test("maximum request and reply interoperate with real WebCrypto and clear the bootstrap seed", async () => {
  const input = seed(), codec = await createCodec(input, 1);
  assert.ok(input.every(byte => byte === 0));
  const plain = new Uint8Array(MAX_PLAIN).fill(3), request = await codec.request(plain, id);
  assert.equal(request.cipher.length, MAX_PLAIN + 16);
  assert.deepEqual(await codec.open(await response(request, { bytes: plain }), request), plain);
  await assert.rejects(codec.request(new Uint8Array(MAX_PLAIN + 1), id));
});
test("bad header, wrong grant/direction/key and content encoding refuse before reading and cancel the stream", async () => {
  const codec = await createCodec(seed(), 1), request = await codec.request(new Uint8Array([3]), id);
  for (const options of [{ grant: 2 }, { direction: 0 }, { wrongSeed: true }, { encoding: true }]) {
    let canceled = false;
    const reply = await response(request, { ...options, cancel() { canceled = true; } });
    await assert.rejects(codec.open(reply, request)); assert.equal(canceled, true);
  }
  const reply = await response(request);
  reply.headers.set("x-probe-admission", "0".repeat(144));
  await assert.rejects(codec.open(reply, request));
});
test("body corruption, truncation and excess refuse without advancing the accepted reply floor", async () => {
  const codec = await createCodec(seed(), 1), request = await codec.request(new Uint8Array([3]), id);
  for (const options of [{ corrupt: true }, { truncate: true }, { excess: true }]) {
    await assert.rejects(codec.open(await response(request, options), request));
  }
  assert.deepEqual(await codec.open(await response(request), request), new Uint8Array([3]));
  await assert.rejects(codec.open(await response(request), request), "exact reply replay refuses");
});
test("an unseen old reply cannot be adopted by a new request that reuses its ID", async () => {
  const codec = await createCodec(seed(), 1), old = await codec.request(new Uint8Array([3]), id);
  const newer = await codec.request(new Uint8Array([3]), id), unseen = await response(old);
  await assert.rejects(codec.open(unseen, newer));
  assert.deepEqual(await codec.open(await response(newer), newer), new Uint8Array([3]));
});
test("timeout and close fence late body completion without exposing plaintext", async () => {
  for (const end of ["abort", "close"]) {
    const codec = await createCodec(seed(), 1), request = await codec.request(new Uint8Array([3]), id);
    const stop = new AbortController(); let release;
    const delayed = new Promise(resolve => { release = resolve; });
    const opened = codec.open(await response(request, { delayed }), request, stop.signal);
    if (end === "abort") stop.abort(); else codec.close();
    release(); await assert.rejects(opened);
  }
});
test("physical source nonce budget cannot wrap or reset after failures", async () => {
  const codec = await createCodec(seed(), 1);
  for (let i = 1; i <= 64; i++) {
    const request = await codec.request(new Uint8Array(), id);
    assert.equal(new DataView(request.raw.buffer).getBigUint64(8), BigInt(i));
    request.cipher.fill(0);
  }
  await assert.rejects(codec.request(new Uint8Array(), id));
});

async function stalled(request, sequence, cancel) {
  const template = await response(request, { sequence }); await template.body.cancel();
  let entered;
  const reading = new Promise(resolve => { entered = resolve; });
  const body = new ReadableStream({ pull() { entered(); return new Promise(() => {}); }, cancel }, { highWaterMark: 0 });
  return { reply: new Response(body, { headers: template.headers }), reading };
}
test("abort and close cancel a pending read without help from its stalled producer", async () => {
  for (const end of ["abort", "close"]) {
    const codec = await createCodec(seed(), 1), request = await codec.request(new Uint8Array([3]), id);
    let canceled = false;
    const pending = await stalled(request, 1n, () => { canceled = true; });
    const stop = new AbortController();
    const rejected = assert.rejects(codec.open(pending.reply, request, stop.signal));
    await pending.reading;
    if (end === "abort") stop.abort(); else await codec.close();
    await rejected; assert.equal(canceled, true);
  }
});
test("reader capacity remains occupied until actual cancellation completes", async () => {
  const codec = await createCodec(seed(), 1), request = await codec.request(new Uint8Array([3]), id);
  let release, began;
  const cancelled = new Promise(resolve => { began = resolve; });
  const cancellation = new Promise(resolve => { release = resolve; });
  const first = await stalled(request, 1n, () => { began(); return cancellation; });
  const second = await stalled(request, 2n, () => {});
  const stop = new AbortController(), stopSecond = new AbortController();
  const firstReject = assert.rejects(codec.open(first.reply, request, stop.signal));
  const secondReject = assert.rejects(codec.open(second.reply, request, stopSecond.signal));
  await Promise.all([first.reading, second.reading]);
  stop.abort(); await cancelled;
  await assert.rejects(codec.open(await response(request, { sequence: 3n }), request), "cancellation intent cannot return a reader position");
  release(); await firstReject;
  assert.deepEqual(await codec.open(await response(request, { sequence: 3n }), request), new Uint8Array([3]));
  stopSecond.abort(); await secondReject; await codec.close();
});
test("request snapshots caller plaintext before deferred HMAC and keeps it through encryption", async () => {
  const codec = await createCodec(seed(), 1), originalSign = crypto.subtle.sign;
  let release;
  const wait = new Promise(resolve => { release = resolve; });
  crypto.subtle.sign = async (...args) => { await wait; return originalSign.apply(crypto.subtle, args); };
  const caller = new Uint8Array([3, 20, 37]);
  let request;
  try {
    const pending = codec.request(caller, id); caller.fill(255); release(); request = await pending;
  } finally { crypto.subtle.sign = originalSign; }
  const material = await crypto.subtle.importKey("raw", seed(), "HKDF", false, ["deriveKey"]);
  const key = await crypto.subtle.deriveKey({ name: "HKDF", hash: "SHA-256", salt: encoder.encode("Mewtual IPC fixture v1"), info: encoder.encode("v1 c2n body AES-256-GCM") }, material, { name: "AES-GCM", length: 256 }, false, ["decrypt"]);
  const nonce = new Uint8Array(12); nonce.set(encoder.encode("REQ1")); nonce.set(request.raw.subarray(8, 16), 4);
  const plain = await crypto.subtle.decrypt({ name: "AES-GCM", iv: nonce, additionalData: request.raw }, key, request.cipher);
  assert.deepEqual(new Uint8Array(plain), new Uint8Array([3, 20, 37]));
});
test("a fresh forged u64-max reply reaches HMAC refusal and cannot poison the response floor", async () => {
  const codec = await createCodec(seed(), 1), request = await codec.request(new Uint8Array([3]), id);
  await codec.open(await response(request, { sequence: 2n }), request);
  const before = codec.snapshot().headerRefusals;
  await assert.rejects(codec.open(await response(request, { sequence: (1n << 64n) - 1n, wrongSeed: true }), request));
  assert.equal(codec.snapshot().headerRefusals, before + 1, "refusal must occur at authentication, after fresh-sequence validation");
  assert.deepEqual(await codec.open(await response(request, { sequence: 3n }), request), new Uint8Array([3]));
});
test("completed reader cleanup cannot add an asynchronous retirement gap after the final plaintext check", async () => {
  const codec = await createCodec(seed(), 1), request = await codec.request(new Uint8Array([3]), id);
  const reply = await response(request), originalCancel = reply.body.cancel.bind(reply.body);
  let retiredDuringExtraCleanup = false;
  reply.body.cancel = async () => { retiredDuringExtraCleanup = true; await codec.close(); return originalCancel(); };
  const result = await codec.open(reply, request).then(plain => ({ plain }), () => ({ refused: true }));
  assert.ok(!retiredDuringExtraCleanup || result.refused, "a close after final checks cannot still deliver plaintext");
  assert.equal(retiredDuringExtraCleanup, false, "the actual reader already completed cancellation before the final check");
  assert.deepEqual(result.plain, new Uint8Array([3]));
});
