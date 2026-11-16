/** Test-only actual production facade. The pipe carries bounded public invoke metadata;
 * selected keys/action bytes remain in the real private broker/HTTP channel. No app invokes
 * this script, and Node Origin injection proves neither CSP nor WebView audio acceptance. */
import assert from "node:assert/strict";
import { NativeControlFrames } from "./voice-native-control.ts";
import { windowVoiceAppCalls } from "../src/voice-app-native.ts";
import { windowVoiceCryptoBroker } from "../src/voice-crypto.ts";

const control = new NativeControlFrames(4096);
const pending = new Map<number, { resolve(value: unknown): void; reject(error: Error): void }>();
let serial = 0, lost = false, lastCommand: string | null = null;
let observedWork!: () => void;
const workReturned = new Promise<void>(resolve => { observedWork = resolve; });
const write = (value: unknown) => {
  const bytes = Buffer.from(JSON.stringify(value)); assert.ok(bytes.length <= 4096);
  const head = Buffer.alloc(4); head.writeUInt32BE(bytes.length); process.stdout.write(head); process.stdout.write(bytes);
};
const invoke = <T>(command: string, args: Record<string, unknown>): Promise<T> => {
  assert.ok(pending.size < 4 && serial < 128, "bounded native metadata callbacks");
  lastCommand = command;
  const request = ++serial;
  const reply = new Promise<unknown>((resolve, reject) => { pending.set(request, { resolve, reject }); });
  write({ tag: "invoke", request, command, args });
  return reply.then(value => {
    // Lose AFTER actual native installation and its full metadata reply. The production
    // facade must recover that SAME consumed candidate; never prepare a fresh owner.
    if (command === "activate_voice_owner" && (value as { group?: unknown })?.group && !lost) {
      lost = true; throw new Error("test: accepted activation reply lost");
    }
    if (command === "read_voice_channel_work") observedWork();
    return value as T;
  });
};
process.stdin.on("data", (bytes: Buffer) => control.feed(bytes, value => {
  const reply = value as { tag: string; request: number; ok: boolean; value: unknown };
  assert.equal(reply.tag, "reply"); const caller = pending.get(reply.request); assert.ok(caller);
  pending.delete(reply.request);
  if (reply.ok) caller.resolve(reply.value); else caller.reject(new Error("native metadata refused"));
}));
process.stdin.on("end", () => { control.finish(); assert.equal(pending.size, 0); });

// Only Node needs this test transport shim. Real WebView sets its own Origin; the native
// service still checks the original Origin plus authenticated private Session framing.
const fetch = globalThis.fetch;
globalThis.fetch = (input, init) => {
  const headers = new Headers(init?.headers); headers.set("Origin", "http://tauri.localhost");
  return fetch(input, { ...init, headers });
};
const host = {};
const unavailable = async (): Promise<never> => { throw new Error("generic crypto must never run"); };
const broker = windowVoiceCryptoBroker({ admit: unavailable, settle: unavailable, commitInbound: unavailable }, host);
const calls = windowVoiceAppCalls(invoke, host);
assert.equal(windowVoiceAppCalls(invoke, host), calls);
const call = calls.prepare(7, process.argv[2]);
assert.equal(calls.prepare(7, process.argv[2]), call);
void (async () => {
  try {
    assert.equal(await call.start(), "ready"); assert.equal(lost, true);
    assert.equal(call.snapshot().phase, "ready");
    await workReturned; // Observe ordinary lifecycle work; never inject a scheduling hint.
    write({ tag: "ready" });
    const first = call.close(); assert.equal(call.close(), first); await first;
    assert.deepEqual(broker.counts(), { active: 0, queued: 0, bytes: 8192 });
    assert.equal(call.snapshot().phase, "closed"); assert.equal(pending.size, 0);
    write({ tag: "done" });
  } catch {
    // Deliberately omit identities, credentials, addresses and native/private byte output.
    write({ tag: "failed", phase: call.snapshot().phase, lastCommand }); process.exitCode = 1;
  }
})();
