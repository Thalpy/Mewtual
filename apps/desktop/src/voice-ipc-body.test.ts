import { test } from "node:test";
import assert from "node:assert/strict";
import { readVoiceIpcBody, VOICE_IPC_PILOT_CIPHER_MAX, VoiceIpcBodyError } from "./voice-ipc-body.ts";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>(yes => { resolve = yes; });
  return { promise, resolve };
}
function packet(bytes: number, options: { length?: number; chunk?: number; cancel?: () => Promise<void> } = {}) {
  let sent = 0;
  const offered: { offset: number; length: number; backing: number }[] = [];
  const stream = new ReadableStream<Uint8Array>({
    type: "bytes",
    pull(controller: ReadableByteStreamController) {
      const request = controller.byobRequest!;
      const view = request.view!;
      offered.push({ offset: view.byteOffset, length: view.byteLength, backing: view.buffer.byteLength });
      const take = Math.min(view.byteLength, bytes - sent, options.chunk ?? bytes);
      for (let index = 0; index < take; index++) new Uint8Array(view.buffer, view.byteOffset, view.byteLength)[index] = (sent + index) % 251;
      sent += take;
      request.respond(take);
      if (sent === bytes) controller.close();
    },
    cancel: options.cancel,
  } as UnderlyingByteSource);
  const response = new Response(stream, { headers: { "content-length": String(options.length ?? bytes) } });
  return { response, offered };
}
const signal = () => new AbortController().signal;
const refused = (code: string) => (error: unknown) => error instanceof VoiceIpcBodyError && error.code === code;

test("actual BYOB follows transferred backing ownership through small reads and a paid EOF probe", async () => {
  const f = packet(VOICE_IPC_PILOT_CIPHER_MAX, { chunk: 13 });
  const body = await readVoiceIpcBody(f.response, VOICE_IPC_PILOT_CIPHER_MAX, signal());
  assert.deepEqual(body, Uint8Array.from({ length: VOICE_IPC_PILOT_CIPHER_MAX }, (_, i) => i % 251));
  assert.equal(f.response.body!.locked, false);
  for (const view of f.offered) {
    assert.equal(view.backing, VOICE_IPC_PILOT_CIPHER_MAX);
    assert.equal(view.length, VOICE_IPC_PILOT_CIPHER_MAX - view.offset);
  }
});

test("maximum body delivered in a single actual read still requires EOF", async () => {
  const f = packet(VOICE_IPC_PILOT_CIPHER_MAX);
  assert.equal((await readVoiceIpcBody(f.response, VOICE_IPC_PILOT_CIPHER_MAX, signal())).length, VOICE_IPC_PILOT_CIPHER_MAX);
  assert.equal(f.offered.length, 1);
});

test("short body is refused and the actual reader is released", async () => {
  const f = packet(19, { length: 20 });
  await assert.rejects(readVoiceIpcBody(f.response, 20, signal()), refused("truncated"));
  assert.equal(f.response.body!.locked, false);
});

test("excess body is detected with at most one additional owned byte", async () => {
  const f = packet(23, { length: 20 });
  await assert.rejects(readVoiceIpcBody(f.response, 20, signal()), refused("excess"));
  assert.deepEqual(f.offered, [{ offset: 0, length: 20, backing: 20 }, { offset: 0, length: 1, backing: 1 }]);
  assert.equal(f.response.body!.locked, false);
});

test("unsupported actual stream is refused without falling back to its default reader", async () => {
  const started = deferred(), drained = deferred();
  let underlyingCanceled = false, finished = false;
  const response = new Response(new ReadableStream({
    cancel: async () => { started.resolve(); await drained.promise; underlyingCanceled = true; },
  }), { headers: { "content-length": "20" } });
  const work = readVoiceIpcBody(response, 20, signal()).finally(() => { finished = true; });
  const rejection = assert.rejects(work, refused("unsupported"));
  await started.promise;
  assert.equal(finished, false, "outer custody must survive real stream cancellation");
  drained.resolve();
  await rejection;
  assert.equal(underlyingCanceled, true);
  assert.equal(response.body!.locked, false);
});

test("abort wakes a stalled BYOB read but cannot finish before cancellation drains", async () => {
  const entered = deferred(), cancelEntered = deferred(), cancelDrained = deferred();
  const abort = new AbortController();
  const stream = new ReadableStream({
    type: "bytes",
    pull() { entered.resolve(); },
    async cancel() { cancelEntered.resolve(); await cancelDrained.promise; },
  });
  const response = new Response(stream, { headers: { "content-length": "20" } });
  let finished = false;
  const work = readVoiceIpcBody(response, 20, abort.signal).finally(() => { finished = true; });
  const rejection = assert.rejects(work, refused("closed"));
  await entered.promise;
  abort.abort();
  await cancelEntered.promise;
  assert.equal(finished, false);
  assert.equal(response.body!.locked, true);
  cancelDrained.resolve();
  await rejection;
  assert.equal(response.body!.locked, false);
});

test("already aborted intake cancels the response and waits before releasing its caller", async () => {
  const canceled = deferred(), released = deferred();
  const abort = new AbortController(); abort.abort();
  const f = packet(20, { cancel: async () => { canceled.resolve(); await released.promise; } });
  let finished = false;
  const work = readVoiceIpcBody(f.response, 20, abort.signal).finally(() => { finished = true; });
  const rejection = assert.rejects(work, refused("closed"));
  await canceled.promise;
  assert.equal(finished, false);
  released.resolve();
  await rejection;
});

test("source failure after buffer transfer returns a closed error without masking cleanup", async () => {
  const stream = new ReadableStream({
    type: "bytes",
    pull(controller) { controller.error(new Error("private network details")); },
  });
  const response = new Response(stream, { headers: { "content-length": "20" } });
  await assert.rejects(readVoiceIpcBody(response, 20, signal()), refused("reader"));
  assert.equal(response.body!.locked, false);
});

test("a stalled EOF probe also retains actual cancellation custody", async () => {
  const eofEntered = deferred(), cancelEntered = deferred(), cancelDrained = deferred();
  const abort = new AbortController();
  let written = false, finished = false;
  const stream = new ReadableStream({
    type: "bytes",
    pull(controller) {
      const request = controller.byobRequest!;
      if (!written) {
        written = true;
        new Uint8Array(request.view!.buffer).fill(1);
        request.respond(20);
      } else {
        assert.equal(request.view!.byteLength, 1);
        eofEntered.resolve();
      }
    },
    async cancel() { cancelEntered.resolve(); await cancelDrained.promise; },
  });
  const response = new Response(stream, { headers: { "content-length": "20" } });
  const work = readVoiceIpcBody(response, 20, abort.signal).finally(() => { finished = true; });
  const rejection = assert.rejects(work, refused("closed"));
  await eofEntered.promise;
  abort.abort();
  await cancelEntered.promise;
  assert.equal(finished, false);
  cancelDrained.resolve();
  await rejection;
  assert.equal(response.body!.locked, false);
});

test("invalid lengths, encoding and status refuse before any application BYOB allocation", async () => {
  for (const bytes of [-1, 0, 15, VOICE_IPC_PILOT_CIPHER_MAX + 1, Number.NaN, 20.5]) {
    const f = packet(20);
    await assert.rejects(readVoiceIpcBody(f.response, bytes, signal()), refused("metadata"));
    assert.deepEqual(f.offered, []);
  }
  for (const headers of [{ "content-length": "020" }, { "content-length": "20", "content-encoding": "gzip" }]) {
    const f = packet(20);
    const response = new Response(f.response.body, { headers });
    await assert.rejects(readVoiceIpcBody(response, 20, signal()), refused("metadata"));
    assert.deepEqual(f.offered, []);
  }
  const f = packet(20);
  await assert.rejects(readVoiceIpcBody(new Response(f.response.body, { status: 503, headers: { "content-length": "20" } }), 20, signal()), refused("metadata"));
});
