/** Test-only Node endpoint for native/WebCrypto interoperability. It imports the actual
 * broker, uses its non-extractable selected key and actual fetch/BYOB channel, and never
 * sends crypto Input, Result, private keys or plaintext over this control pipe. The Rust
 * parent accepts a closed set of public-metadata callbacks. No app command invokes this. */
import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import {
  VoiceCryptoBroker, createVoiceCryptoOwner,
  type VoiceCryptoOwner, type VoiceJoinKeyHandle, type VoiceIpcChannel,
  type VoiceCryptoChain, type VoiceIpcNative, type VoiceSelectedKeyNative,
  type VoiceSelectedKeyAdmission, type VoiceSelectedKeyPublication, type VoiceIpcSetupReply,
} from "../src/voice-crypto.ts";
import type { VoiceIpcPurpose, VoiceIpcActionIdentity, VoiceIpcConfirmationPurpose } from "../src/voice-ipc-actions.ts";
import { decodeVoiceWorkSnapshot } from "../src/voice-ipc-work.ts";

const MAX_FRAME = 4096;
const scratch = Buffer.alloc(MAX_FRAME + 4);
let used = 0, stepRunning = false;
let callback: { resolve(value: unknown): void; reject(reason: Error): void } | null = null;
let broker: VoiceCryptoBroker, owner: VoiceCryptoOwner, key: VoiceJoinKeyHandle;
let channel: VoiceIpcChannel, chain: VoiceCryptoChain;
let channelExecutor: string | null = null;
let parent: Readonly<{ purpose: VoiceIpcPurpose; row: string; identity: VoiceIpcActionIdentity }>;
let availability: ReturnType<typeof setInterval> | null = null;
// Fixture-only bounded stage trace. No keys, Input, Result or ciphertext is copied/logged.
// A closed native refusal code is metadata; every other crypto result contributes length.
const stages: string[] = [];
let capacityRefusals = 0;
// Test control only: Rust stops a long-lived automatic cycle after independently observing
// actual native receipts/ACKs/Returns. No row, role, input or key is passed into this step.
let stopPump: (() => void) | null = null;
function stage(value: string): void { if (stages.length === 8) stages.shift(); stages.push(value); }
const provider = new Proxy(webcrypto.subtle as unknown as SubtleCrypto, { get(target, name) {
  const value = Reflect.get(target, name);
  if (typeof value !== "function") return value;
  return async (...args: unknown[]) => {
    const result = await Reflect.apply(value, target, args);
    let detail = String(name);
    if (result instanceof ArrayBuffer) {
      detail += ":" + result.byteLength;
      const view = new Uint8Array(result);
      if (name === "decrypt" && view.length === 3 && view[0] === 1 && view[1] === 3) {
        detail += "/refused-" + view[2];
        if (view[2] === 7) capacityRefusals++;
      }
    }
    stage(detail); return result;
  };
} });
function stopAvailability(): void {
  if (availability !== null) clearInterval(availability);
  availability = null;
}

function send(value: unknown): void {
  const body = Buffer.from(JSON.stringify(value));
  assert.ok(body.length <= MAX_FRAME, "control frame exceeds fixed metadata budget");
  const head = Buffer.alloc(4); head.writeUInt32BE(body.length);
  process.stdout.write(head); process.stdout.write(body);
}
type Method = "admitKey" | "publishKey" | "settleKey" | "setup" | "settleSetup" | "collectChannel" | "readWork" | "observeTerminal";
async function rpc<T>(method: Method, args: Record<string, string | number>): Promise<T> {
  assert.equal(callback, null, "only one native metadata call may be outstanding");
  const answer = new Promise<unknown>((resolve, reject) => { callback = { resolve, reject }; });
  send({ tag: "call", method, args });
  return await answer as T;
}
const selectedNative: VoiceSelectedKeyNative = Object.freeze({
  admit: (owner: string, executor: string) => rpc<VoiceSelectedKeyAdmission>("admitKey", { owner, executor }),
  publish: (owner: string, executor: string, permit: string, point: Uint8Array) => {
    assert.equal(point.byteLength, 65);
    return rpc<VoiceSelectedKeyPublication>("publishKey", { owner, executor, permit, point: Buffer.from(point).toString("hex") });
  },
  settle: (owner: string, executor: string) => rpc<void>("settleKey", { owner, executor }),
});
const native: VoiceIpcNative = Object.freeze({
  setup: (owner: string, executor: string) => {
    if (channelExecutor === null) channelExecutor = executor;
    else assert.equal(executor, channelExecutor, "one original native channel selection");
    return rpc<VoiceIpcSetupReply>("setup", { owner, executor });
  },
  settleSetup: (owner: string, executor: string) => rpc<void>("settleSetup", { owner, executor }),
  collectChannel: (owner: string, executor: string) => rpc<void>("collectChannel", { owner, executor }),
  readWork: (owner: string, executor: string, fromSlot: number) => rpc<unknown>("readWork", { owner, executor, fromSlot }),
});
const realFetch = globalThis.fetch;
// Node does not supply a WebView Origin. This is the sole fetch shim: actual HTTP,
// response stream, MAC/AEAD and BYOB reader remain untouched. This proves no WebView policy.
const transport: typeof fetch = (url, init) => {
  const headers = new Headers(init?.headers); headers.set("origin", "http://tauri.localhost");
  return realFetch(url, { ...init, headers });
};

async function idle(): Promise<void> {
  const deadline = performance.now() + 5000;
  while (broker.counts().active !== 0) {
    assert.ok(performance.now() < deadline, "actual broker did not drain");
    await new Promise<void>(resolve => setImmediate(resolve));
  }
}
type Step =
  | { tag: "step"; name: "prepare"; owner: string; group: string }
  | { tag: "step"; name: "derive"; purpose: VoiceIpcPurpose }
  | { tag: "step"; name: "child"; request: VoiceIpcConfirmationPurpose["request"] }
  | { tag: "step"; name: "pump" | "close" | "wedge" };
async function step(value: Step): Promise<unknown> {
  switch (value.name) {
    case "pump": {
      assert.equal(stopPump, null);
      let stopped = false;
      stopPump = () => { stopped = true; };
      const observer = broker.observeNativeWork(owner, channel, key);
      try {
        // Production correctly stops old-work scans at Finish custody. Test completion
        // must not depend on another readWork callback. One independent bounded metadata
        // observer waits for actual native receipts/Returns AND Claim; it adopts nothing.
        while (!stopped) {
          if (callback === null) await rpc("observeTerminal", { owner: owner.token, executor: channelExecutor! });
          if (!stopped) await new Promise<void>(yes => setTimeout(yes, 50));
        }
        await observer.close(); await idle(); return { kind: "observed" };
      }
      finally { stopPump = null; }
    }
    case "prepare": {
      assert.equal(owner, undefined, "one actual native owner per endpoint");
      owner = createVoiceCryptoOwner(value.owner, value.group, () => true);
      const forbidden = async () => { throw new Error("generic byte-bearing IPC is forbidden in this fixture"); };
      broker = new VoiceCryptoBroker({ admit: forbidden, settle: forbidden, commitInbound: forbidden }, provider);
      const selection = broker.selectJoinKey(owner, selectedNative);
      // One active-step availability wake, never a new broker/key/channel or renewed
      // native permit. An uncertain selected attempt otherwise parks its retained driver.
      availability = setInterval(() => broker.retrySelectedJoinKeys(), 100);
      try {
        let selected = await selection.attempt();
        while (selected.kind === "uncertain" || selected.kind === "busy") {
          await new Promise<void>(resolve => setTimeout(resolve, 100));
          selected = await selection.attempt();
        }
        assert.equal(selected.kind, "accepted");
        if (selected.kind !== "accepted") throw new Error("selected key refused");
        key = selected.key; await idle();
        channel = broker.nativeChannel(owner, key, native, transport);
        let status = await channel.status();
        while (status.kind === "uncertain") {
          await new Promise<void>(resolve => setTimeout(resolve, 100));
          status = await channel.status();
        }
        assert.equal(status.kind, "ready");
        assert.ok("nativeSetupDrained" in status && status.nativeSetupDrained);
        await idle(); return { kind: "ready" };
      } finally { stopAvailability(); }
    }
    case "derive": {
      assert.equal(parent, undefined, "one original pair per test endpoint");
      assert.notEqual(channelExecutor, null);
      const work = decodeVoiceWorkSnapshot(await rpc("readWork", { owner: owner.token, executor: channelExecutor!, fromSlot: 0 }),
        owner.token, channelExecutor!, 0).work;
      assert.ok(work && work.state === "attached" && work.purpose.kind === "derive", "actual native parent must be attached");
      if (!work || work.state !== "attached" || work.purpose.kind !== "derive") throw new Error("original native work unavailable");
      const { kind: _kind, ...purpose } = work.purpose;
      assert.deepEqual(purpose, value.purpose, "public selection cannot substitute another native purpose");
      parent = Object.freeze({ purpose, row: work.row, identity: work.identity });
      const intake = broker.nativeDerivationFromChannel(owner, channel, key, parent.purpose, parent.row, parent.identity);
      const initialRefusals = capacityRefusals;
      let before = capacityRefusals;
      let ready = await intake.attempt();
      // A fresh authenticated native Capacity observation authorizes retry of THIS intake.
      // A stale trace or unexplained uncertainty must fail rather than hide a new defect.
      while (ready.kind === "uncertain" && capacityRefusals > before) {
        await new Promise<void>(resolve => setTimeout(resolve, 25));
        before = capacityRefusals;
        ready = await intake.attempt();
      }
      assert.equal(ready.kind, "ready");
      if (ready.kind !== "ready") throw new Error("native derivation input unavailable");
      assert.ok(capacityRefusals > initialRefusals, "real native Input Capacity refusal must be observed before recovery");
      chain = ready.chain;
      const outcome = await ready.derivation.attempt();
      assert.equal(outcome.kind, "accepted");
      await broker.returnNativeDerivation(owner, channel, chain);
      await idle();
      return { kind: outcome.kind };
    }
    case "child": {
      assert.ok(chain && parent, "original derived chain required");
      const purpose = Object.freeze({ parent: parent.purpose, parentRow: parent.row, request: value.request });
      // Native maintenance, not this control step or a Begin(true), must have selected
      // and attached this exact original child. Pull its bounded public descriptor using
      // the same production metadata handler/strict decoder as the parent.
      let fromSlot = 0;
      let selected: ReturnType<typeof decodeVoiceWorkSnapshot>["work"] = null;
      while (fromSlot < 32) {
        const snapshot = decodeVoiceWorkSnapshot(await rpc("readWork", {
          owner: owner.token, executor: channelExecutor!, fromSlot,
        }), owner.token, channelExecutor!, fromSlot);
        fromSlot = snapshot.nextSlot;
        const work = snapshot.work;
        if (!work || work.state !== "attached" || work.purpose.kind !== "confirmation" || work.purpose.parent !== parent.row) continue;
        const request = work.purpose.request === "start" ? { kind: "start", predecessor: work.purpose.predecessor }
          : work.purpose.request === "respond" ? { kind: "respond", triggerRow: work.purpose.trigger }
            : { kind: "incoming", role: work.purpose.role };
        if (JSON.stringify(request) !== JSON.stringify(value.request)) continue;
        assert.deepEqual(work.purpose.parentPurpose, { kind: "derive", ...parent.purpose });
        selected = work; break;
      }
      assert.ok(selected?.state === "attached", "native maintenance must attach the original child before browser execution");
      if (!selected || selected.state !== "attached") throw new Error("native child unavailable");
      const handle = broker.nativeConfirmationFromChannel(owner, channel, chain, purpose, selected.row, selected.identity);
      let outcome = await handle.attempt();
      // Real bridge ingress need not be ready at the first bounded observation. Retry only
      // the SAME descriptor/Phase and original chain while native reports local pressure or
      // unresolved work; neither a new purpose nor a new deadline is constructed here.
      while (chain.isCurrent() && (outcome.kind === "uncertain" ||
        (outcome.kind === "refused" && outcome.code === "capacity") ||
        (outcome.kind === "status" && outcome.pending === "open"))) {
        await new Promise<void>(resolve => setTimeout(resolve, 25));
        outcome = await handle.attempt();
      }
      assert.equal(outcome.kind, "accepted");
      assert.ok(handle.row(), "authenticated original child row required");
      await idle();
      return { kind: outcome.kind, row: handle.row(), confirmation: outcome.kind === "accepted" ? outcome.receipt.confirmation : null };
    }
    case "close": {
      owner.retire(); broker.retrySelectedJoinKeys();
      await channel.close(); await idle();
      assert.equal(broker.counts().bytes, 0, "actual original browser owners must collect");
      return { kind: "closed", bytes: broker.counts().bytes };
    }
    case "wedge": {
      send({ tag: "blocked" });
      return await new Promise<never>(() => {});
    }
    default: throw new Error("unknown control step");
  }
}

function accept(value: unknown): void {
  assert.ok(value && typeof value === "object" && !Array.isArray(value));
  const frame = value as Record<string, unknown>;
  if (frame.tag === "reply") {
    assert.ok(callback, "unsolicited native control response");
    const pending = callback; callback = null;
    if (frame.stopPump === true) {
      assert.ok(stopPump, "only a running automatic step accepts its native observation stop");
      stopPump();
    }
    if (frame.ok === true) pending.resolve(frame.value);
    else pending.reject(new Error("native metadata callback refused"));
  } else {
    assert.equal(frame.tag, "step"); assert.equal(stepRunning, false, "control steps are serialized");
    stepRunning = true;
    void step(frame as Step).then(value => {
      stepRunning = false; send({ tag: "done", value });
    }, error => {
      stepRunning = false;
      // Fixed-size diagnostics only: no Input/Result/body bytes are ever serialized here.
      send({ tag: "failed", reason: (String(error instanceof Error ? error.message : "endpoint failed").slice(0, 100) + " stages=" + stages.join(",")).slice(0, 256) });
    });
  }
}
process.stdin.on("data", (bytes: Buffer) => {
  try {
    // One control step/callback response at a time. Refuse excess before constructing an
    // accumulator; arbitrary parent input cannot grow a queued JSON/message buffer.
    assert.ok(used + bytes.length <= scratch.length, "oversized control pipe input");
    bytes.copy(scratch, used); used += bytes.length;
    if (used < 4) return;
    const length = scratch.readUInt32BE();
    assert.ok(length > 0 && length <= MAX_FRAME);
    if (used < length + 4) return;
    assert.equal(used, length + 4, "only one frame may be outstanding");
    const frame = JSON.parse(scratch.subarray(4, used).toString("utf8"));
    scratch.fill(0, 0, used); used = 0; accept(frame);
  } catch {
    process.exitCode = 1; process.stdin.destroy();
  }
});
process.stdin.on("end", () => {
  stopAvailability();
  if (used !== 0 || callback || stepRunning) process.exitCode = 1;
  process.exit(process.exitCode ?? 0);
});
