import { test } from "node:test";
import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { readVoiceIpcResponse, readVoiceIpcSdpInputResponse, VOICE_IPC_SDP_INPUT_CIPHER_MAX, VoiceIpcBodyError } from "./voice-ipc-body.ts";
import { decodeVoiceIpcAdmission, encodeVoiceIpcAdmission, voiceIpcResponseAad, type VoiceIpcHeader } from "./voice-ipc-frame.ts";

function deferred() { let resolve!: () => void; const promise = new Promise<void>(yes => { resolve = yes; }); return { promise, resolve }; }
const signal = () => new AbortController().signal;
const refused = (code: string) => (error: unknown) => error instanceof VoiceIpcBodyError && error.code === code;
const request: VoiceIpcHeader = { direction: 0, operation: 17, kind: 0, slot: 1, sequence: 2n, logical: "00".repeat(15) + "01", length: 82 };
async function packet(length = VOICE_IPC_SDP_INPUT_CIPHER_MAX, options: {
  kind?: 0 | 2; extra?: number; chunk?: number; corruptMac?: boolean; cancel?: () => Promise<void>;
} = {}) {
  const header: VoiceIpcHeader = { ...request, direction: 1, kind: options.kind ?? 2, length, sequence: 3n };
  const key = await webcrypto.subtle.importKey("raw", new Uint8Array(32).fill(9), { name: "HMAC", hash: "SHA-256" }, false, ["sign", "verify"]);
  const mac = new Uint8Array(await webcrypto.subtle.sign("HMAC", key, voiceIpcResponseAad(header, request)));
  if (options.corruptMac) mac[0] ^= 1;
  const prefix = encodeVoiceIpcAdmission(header, mac), total = 144 + length + (options.extra ?? 0);
  let sent = 0, authenticated = false, floor = 2n;
  const offered: { backing: number; offset: number; length: number; authenticated: boolean }[] = [];
  const stream = new ReadableStream({ type: "bytes",
    pull(controller: ReadableByteStreamController) {
      const byob = controller.byobRequest!, view = byob.view!;
      offered.push({ backing: view.buffer.byteLength, offset: view.byteOffset, length: view.byteLength, authenticated });
      const take = Math.min(view.byteLength, total - sent, options.chunk ?? total);
      const bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
      for (let i = 0; i < take; i++) bytes[i] = sent + i < 144 ? prefix.charCodeAt(sent + i) : (sent + i - 144) % 251;
      sent += take;
      // Node's BYOB EOF requires closing before the zero-byte EOF response.
      if (take === 0) { controller.close(); byob.respond(0); }
      else { byob.respond(take); if (sent === total) controller.close(); }
    }, cancel: options.cancel,
  });
  const response = new Response(stream);
  const authenticate = async (admission: string): Promise<VoiceIpcHeader> => {
    const decoded = decodeVoiceIpcAdmission(admission);
    assert.equal(await webcrypto.subtle.verify("HMAC", key, decoded.tag, voiceIpcResponseAad(decoded.header, request)), true);
    assert.ok(decoded.header.sequence > floor); floor = decoded.header.sequence; authenticated = true;
    return decoded.header;
  };
  return { response, offered, authenticate, header };
}

test("large SDP Input requires a real exact-request MAC before its BYOB allocation", async () => {
  const f = await packet(undefined, { chunk: 31 });
  const bytes = await readVoiceIpcSdpInputResponse(f.response, signal(), f.authenticate);
  assert.equal(bytes.byteLength, 9232); assert.equal(f.response.body!.locked, false);
  assert.ok(f.offered.some(view => view.backing === 144 && !view.authenticated));
  assert.ok(f.offered.some(view => view.backing === 9232 && view.authenticated));
  assert.ok(f.offered.every(view => view.backing === 144 || view.authenticated));
  assert.deepEqual(bytes, Uint8Array.from({ length: 9232 }, (_, i) => i % 251));
});

test("generic encrypted reader keeps its original ceiling even after a valid large Input MAC", async () => {
  const f = await packet();
  await assert.rejects(readVoiceIpcResponse(f.response, signal(), async text => (await f.authenticate(text)).length), refused("metadata"));
  assert.ok(f.offered.every(view => view.backing === 144));
});

test("invalid MAC cannot allocate an SDP body; bounded refusal hides provider error details", async () => {
  const f = await packet(undefined, { corruptMac: true });
  await assert.rejects(readVoiceIpcSdpInputResponse(f.response, signal(), f.authenticate), refused("reader"));
  assert.ok(f.offered.every(view => view.backing === 144)); assert.equal(f.response.body!.locked, false);
});

test("only original opcode17 response classes and their independent exact ceilings admit a body", async () => {
  for (const patch of [{ direction: 0 }, { operation: 1 }, { operation: 18 }, { kind: 1 }, { kind: 0, length: 273 },
    { length: 9233 }, { length: 15 }, { length: Number.NaN }, { length: 16.5 }]) {
    const f = await packet();
    await assert.rejects(readVoiceIpcSdpInputResponse(f.response, signal(), async text => ({ ...await f.authenticate(text), ...patch }) as VoiceIpcHeader), refused("metadata"));
    assert.ok(f.offered.every(view => view.backing === 144));
  }
  for (const length of [16, 20, 236, 272]) {
    const f = await packet(length, { kind: 0 });
    assert.equal((await readVoiceIpcSdpInputResponse(f.response, signal(), f.authenticate)).byteLength, length);
  }
});

test("authenticated header callback fields cannot change after class validation", async () => {
  const f = await packet(); const counts = new Map<PropertyKey, number>();
  const bytes = await readVoiceIpcSdpInputResponse(f.response, signal(), async text => new Proxy(await f.authenticate(text), {
    get(target, field) { if (field === "then") return undefined; /* Promise assimilation, not a header field. */ const count = (counts.get(field) ?? 0) + 1; counts.set(field, count); assert.equal(count, 1); return Reflect.get(target, field); },
  }));
  assert.equal(bytes.length, 9232); assert.deepEqual([...counts.keys()], ["direction", "operation", "kind", "length"]);
});

test("extra Input bytes use only the paid one-byte EOF probe and truncation rejects", async () => {
  const excess = await packet(undefined, { extra: 1 });
  await assert.rejects(readVoiceIpcSdpInputResponse(excess.response, signal(), excess.authenticate), refused("excess"));
  assert.equal(excess.offered.at(-1)!.backing, 1);
  const short = await packet(undefined, { extra: -1 });
  await assert.rejects(readVoiceIpcSdpInputResponse(short.response, signal(), short.authenticate), refused("truncated"));
  assert.equal(short.response.body!.locked, false);
});

test("abort during MAC work retains custody through both authentication and real stream cancellation", async () => {
  const entered = deferred(), authenticateDrained = deferred(), cancelEntered = deferred(), cancelDrained = deferred();
  const f = await packet(undefined, { cancel: async () => { cancelEntered.resolve(); await cancelDrained.promise; } });
  const controller = new AbortController(); let finished = false;
  const work = readVoiceIpcSdpInputResponse(f.response, controller.signal, async text => {
    const header = await f.authenticate(text); entered.resolve(); await authenticateDrained.promise; return header;
  }).finally(() => { finished = true; });
  const rejected = assert.rejects(work, refused("closed"));
  await entered.promise; controller.abort(); await cancelEntered.promise;
  assert.equal(finished, false); cancelDrained.resolve(); await Promise.resolve(); assert.equal(finished, false);
  authenticateDrained.resolve(); await rejected;
  assert.equal(f.response.body!.locked, false); assert.ok(f.offered.every(view => view.backing === 144));
});

test("untrusted HTTP length strings are never materialized by the large reader", async () => {
  const f = await packet(); f.response.headers.set("content-length", "0".repeat(65536));
  f.response.headers.get = () => { throw new Error("untrusted HTTP field copied"); };
  assert.equal((await readVoiceIpcSdpInputResponse(f.response, signal(), f.authenticate)).length, 9232);
});
