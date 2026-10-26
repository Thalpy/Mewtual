/** Test-only Node endpoint for native/WebCrypto interoperability. It imports the actual
 * broker, uses its non-extractable selected key and actual fetch/BYOB channel, and never
 * sends crypto Input, Result, private keys or plaintext over this control pipe. The Rust
 * parent accepts a closed set of public-metadata callbacks. No app command invokes this. */
import assert from "node:assert/strict";
import { NativeControlFrames } from "./voice-native-control.ts";
import { NativeSdpReplyLoss, type NativeSdpObservation } from "./voice-native-reply-loss.ts";
import { webcrypto } from "node:crypto";
import {
  VoiceCryptoBroker, createVoiceCryptoOwner,
  type VoiceCryptoOwner, type VoiceJoinKeyHandle, type VoiceIpcChannel,
  type VoiceCryptoChain, type VoiceIpcNative, type VoiceSelectedKeyNative,
  type VoiceSelectedKeyAdmission, type VoiceSelectedKeyPublication, type VoiceIpcSetupReply,
} from "../src/voice-crypto.ts";
import type { VoiceIpcPurpose, VoiceIpcActionIdentity, VoiceIpcConfirmationPurpose } from "../src/voice-ipc-actions.ts";
import { decodeVoiceWorkSnapshot } from "../src/voice-ipc-work.ts";
import { decodeVoiceMedia } from "../src/voice-ipc-media.ts";

const MAX_FRAME = 4096;
const control = new NativeControlFrames(MAX_FRAME);
let stepRunning = false, currentStep = "none";
let callback: { resolve(value: unknown): void; reject(reason: Error): void } | null = null;
let broker: VoiceCryptoBroker, owner: VoiceCryptoOwner, key: VoiceJoinKeyHandle;
let channel: VoiceIpcChannel, chain: VoiceCryptoChain;
let channelExecutor: string | null = null;
let parent: Readonly<{ purpose: VoiceIpcPurpose; row: string; identity: VoiceIpcActionIdentity }>;
let availability: ReturnType<typeof setInterval> | null = null;
// Fixture-only bounded stage trace. No keys, Input, Result or ciphertext is copied/logged.
// A closed native refusal code is metadata; every other crypto result contributes length.
const stages: string[] = [];
let capacityRefusals = 0, providerCalls = 0;
let mediaRequestMode = -1;
type MediaLoss = { modes: Set<number>; lose: boolean; detach: boolean; history: boolean };
const mediaLosses = new Map<string, MediaLoss>();
function mediaLoss(): MediaLoss {
  const loss = mediaLosses.get(parent.row); assert.ok(loss); return loss;
}
const mediaEvents: string[] = [];
let mediaStarted = 0, mediaCurrentObservations = 0;
// At most two actual peers in this fixture. Public parent IDs only label observations;
// they are never supplied as an authority hint to the ordinary discovery pump.
const mediaRecords: { plan: VoiceCryptoChain; parent: typeof parent; calls: { closed: number }; currents: number }[] = [];
let mediaRequestParent: string | null = null, mediaRequestProof: string | null = null;
let originalSession: { keys: unknown; bootstrap: unknown; tx: bigint } | null = null;
// Separate bounded observations for the unscripted production coordinator. These counters
// never grant a native operation and keep no plan, crypto key, SDP or Session reference.
const coordinatedPeers: { local: number; remote: number; closed: number }[] = [];
let previousCoordinatedRtc: typeof RTCPeerConnection | undefined;
let coordinatedObserver: ReturnType<VoiceCryptoBroker["observeNativeLifecycle"]> | null = null;
function installCoordinatedRtc(): void {
  if (coordinatedObserver) return;
  previousCoordinatedRtc = globalThis.RTCPeerConnection;
  class Peer {
    private local: "offer" | "answer" | null = null;
    private remote: "offer" | "answer" | null = null;
    private readonly calls = { local: 0, remote: 0, closed: 0 };
    constructor() { assert.ok(coordinatedPeers.length < 2); coordinatedPeers.push(this.calls); }
    iceGatheringState = "complete";
    get localDescription() { assert.ok(this.local); return { type: this.local, sdp: "s".repeat(8192) }; }
    addTransceiver() { assert.equal(this.remote, null); }
    async setLocalDescription() {
      assert.equal(++this.calls.local, 1); this.local = this.remote === "offer" ? "answer" : "offer";
    }
    async setRemoteDescription(value: RTCSessionDescriptionInit) {
      assert.equal(++this.calls.remote, 1);
      assert.equal(value.type, this.local === "offer" ? "answer" : "offer");
      assert.equal(value.sdp, "s".repeat(8192)); this.remote = value.type as "offer" | "answer";
    }
    addEventListener() {} removeEventListener() {}
    close() { this.calls.closed++; }
  }
  globalThis.RTCPeerConnection = Peer as unknown as typeof RTCPeerConnection;
}
function currentObservations(): number {
  const record = mediaRecords.find(record => record.plan === chain);
  assert.ok(record, "original actual media record required");
  return record.currents;
}
function mediaEvent(value: string): void { if (mediaEvents.length === 6) mediaEvents.shift(); mediaEvents.push(value + "@" + Math.floor(performance.now() - mediaStarted)); }
// Test control only: Rust stops a long-lived automatic cycle after independently observing
// actual native receipts/ACKs/Returns. No row, role, input or key is passed into this step.
let stopPump: (() => void) | null = null;
function stage(value: string): void { if (stages.length === 8) stages.shift(); stages.push(value); }
const provider = new Proxy(webcrypto.subtle as unknown as SubtleCrypto, { get(target, name) {
  const value = Reflect.get(target, name);
  if (typeof value !== "function") return value;
  return async (...args: unknown[]) => {
    providerCalls++;
    if (name === "encrypt" && args[2] instanceof Uint8Array) {
      const body = args[2];
      if (body[0] === 9 && (body.byteLength === 226 || body.byteLength === 82)) {
        mediaRequestMode = body[1]; mediaEvent(">" + mediaRequestMode);
        mediaRequestProof = mediaRequestMode === 4 && body.byteLength === 82 ? Buffer.from(body.subarray(50, 82)).toString("hex") : null;
      }
    }
    const result = await Reflect.apply(value, target, args);
    let detail = String(name);
    if (result instanceof ArrayBuffer) {
      detail += ":" + result.byteLength;
      const view = new Uint8Array(result);
      const loss = mediaRequestParent === null ? undefined : mediaLosses.get(mediaRequestParent);
      if (name === "decrypt" && view.length === 39 && view[0] === 9 && view[1] === 0) {
        stage(`media:${mediaRequestMode}/${view[2]}`); mediaEvent("<" + view[2]);
        if(mediaRequestMode===4 && view[2]===9) {
          const receipt = decodeVoiceMedia(view);
          assert.ok(receipt && receipt.current && receipt.admitted && !receipt.closed && !receipt.released);
          assert.equal(receipt.proof, mediaRequestProof, "strictly decoded Current must match this actual request proof");
          mediaRequestProof = null;
          mediaCurrentObservations++;
          const record = mediaRecords.find(record => record.parent.row === mediaRequestParent);
          if (record) record.currents++;
        }
      }
      if (loss && loss.history && mediaRequestMode===8 && name==="decrypt" && view.length===39 &&
          view[0]===9 && view[1]===0 && view[2]===1 && !loss.modes.has(8)) {
        // Lose only a positively authenticated completed native history observation.
        // Capacity/absence never satisfy this case, and no proof/plaintext is retained.
        loss.modes.add(8); view.fill(0); throw new Error("test-only completed native history reply lost");
      }
      if (loss && loss.lose && name === "decrypt" && view.length === 39 && view[0] === 9 && view[1] === 0 &&
          !loss.modes.has(mediaRequestMode) &&
          (((mediaRequestMode === 0 || mediaRequestMode === 4 || (loss.detach && mediaRequestMode === 7)) && view[2] === 9) || (mediaRequestMode === 3 && view[2] === 7))) {
        // Test-only lost observation AFTER actual HTTP+AEAD authentication. Count only
        // exact positive native replies, never Capacity. Retain no plaintext/proof;
        // native independently checks canonical admission and eventual collection.
        loss.modes.add(mediaRequestMode); view.fill(0);
        throw new Error("test-only authenticated media observation lost");
      }
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
type Method = "admitKey" | "publishKey" | "settleKey" | "setup" | "settleSetup" | "collectChannel" | "readWork" | "observeTerminal" | "observeSdpRound";
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
let loseSdpReplies = false;
let mediaPeerCalls: { closed: number } | null = null;
let retirementWork:Promise<void>|null=null, retirementDone=false, retirementFailed=false, retirementBefore=0;
const replyLoss = new NativeSdpReplyLoss();
// Node does not supply a WebView Origin. This is the sole fetch shim: actual HTTP,
// response stream, MAC/AEAD and BYOB reader remain untouched. This proves no WebView policy.
const transport: typeof fetch = async (url, init) => {
  const headers = new Headers(init?.headers); headers.set("origin", "http://tauri.localhost");
  const response = await realFetch(url, { ...init, headers });
  const operation = Number.parseInt((headers.get("x-voice-admission") ?? "").slice(12, 14), 16);
  // The global actual codec is serial. Correlate a positive decrypted Current with the
  // original full parent in its authenticated header, not the mutable active chain.
  if (operation === 28) mediaRequestParent = (headers.get("x-voice-admission") ?? "").slice(32, 64);
  if (loseSdpReplies && [19, 20, 24, 26].includes(operation)) {
    // Native observations prove the ORIGINAL effect before this fixture drops its reply.
    // No success receipt/SDP/input is supplied over the metadata pipe. A refused response
    // alone cannot satisfy the loss case or cause the test to count an uncommitted effect.
    const row = (headers.get("x-voice-admission") ?? "").slice(32, 64);
    let state = await rpc<NativeSdpObservation>("observeSdpRound", { owner: owner.token, executor: channelExecutor! });
    // The native oracle's try-lock refusal is explicit absence of evidence. Retry this
    // bounded metadata read before forwarding the original response; never credit the
    // other action's result/Return or count one Result twice.
    for (let retry = 0; state.firstRow === null && retry < 10; retry++) {
      await new Promise<void>(yes => setTimeout(yes, 25));
      state = await rpc<NativeSdpObservation>("observeSdpRound", { owner: owner.token, executor: channelExecutor! });
    }
    const discard = replyLoss.discard(operation, row, state);
    if (discard) {
      // Fixed Control response, not a generic unbounded buffering shim. Join the actual
      // stream/EOF before simulating a lost browser observation of this completed effect.
      const reader = response.body!.getReader(); let bytes = 0;
      try {
        for (;;) { const next = await reader.read(); if (next.done) break; bytes += next.value.byteLength; assert.ok(bytes <= 512); }
      } finally { await reader.cancel(); reader.releaseLock(); }
      throw new Error("test-only native SDP reply lost after canonical effect");
    }
  }
  return response;
};

async function idle(): Promise<void> {
  const deadline = performance.now() + 5000;
  while (broker.counts().active !== 0) {
    if (performance.now() >= deadline) {
      // Fixed flags only: identify active ownership without exposing rows/keys/SDP.
      const retained = (broker as unknown as { ipcChannels: Map<string, { activity: number; driving: boolean; retired: boolean; negotiation: {sdpRoundStage: string} }> }).ipcChannels.get(owner.token);
      throw new Error(`idle:${currentStep} a${broker.counts().active} c${retained?.activity} d${Number(retained?.driving)} r${Number(retained?.retired)} s${retained?.negotiation.sdpRoundStage}`);
    }
    await new Promise<void>(resolve => setImmediate(resolve));
  }
}
type Step =
  | { tag: "step"; name: "coordinator" | "coordinator-end"; peers: 1 | 2 }
  | { tag: "step"; name: "prepare"; owner: string; group: string }
  | { tag: "step"; name: "derive"; purpose: VoiceIpcPurpose }
  | { tag: "step"; name: "sdp-custody"; purpose: VoiceIpcPurpose; row: string; identity: VoiceIpcActionIdentity }
  | { tag: "step"; name: "child"; request: VoiceIpcConfirmationPurpose["request"] }
  | { tag: "step"; name: "pump" | "close" | "wedge" | "pc-custody" | "rtc-round" | "rtc-round-loss" | "rtc-round-media" | "rtc-round-media-loss" | "media-revoked" | "media-retire" | "media-retire-loss" | "media-retire-start" | "media-retire-progress" | "media-retire-finish" | "media-current-after-history" | "media-history-pending" | "advance-pair" | "media-current-all" | "end-peers" };
async function step(value: Step): Promise<unknown> {
  switch (value.name) {
    case "coordinator": {
      assert.ok(value.peers === 1 || value.peers === 2);
      installCoordinatedRtc();
      const observer = broker.observeNativeLifecycle(owner, channel, key);
      if (coordinatedObserver) assert.equal(observer, coordinatedObserver);
      coordinatedObserver = observer;
      // Start only the normal production lifecycle. Native callbacks remain serviced by
      // Rust during this entire step. The test merely observes completed browser history;
      // Rust separately checks original native History::Collected, not this boolean.
      const deadline = performance.now() + 55_000;
      for (;;) {
        const session = (broker as unknown as { ipcChannels: Map<string, {
          retired: boolean; peers: Map<unknown, { compacted: boolean; compactionReturned: boolean; advanced: boolean }>;
        }> }).ipcChannels.get(owner.token);
        assert.ok(session && !session.retired, "production coordinator retired before completion");
        const completed = [...session.peers.values()].filter(peer => peer.compacted && peer.compactionReturned && peer.advanced).length;
        if (completed === value.peers) break;
        assert.ok(performance.now() < deadline, `coordinator history stalled: completed=${completed} sdk=${coordinatedPeers.length}`);
        await new Promise<void>(yes => setTimeout(yes, 25));
      }
      assert.equal(coordinatedPeers.length, value.peers);
      for (const peer of coordinatedPeers) assert.deepEqual(peer, { local: 1, remote: 1, closed: 0 });
      return { kind: "coordinated", peers: value.peers };
    }
    case "coordinator-end": {
      assert.equal(coordinatedPeers.length, value.peers);
      owner.retire();
      for (const peer of coordinatedPeers) assert.equal(peer.closed, 1, "End must synchronously close every coordinated peer");
      return { kind: "coordinator-ended", peers: value.peers };
    }
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
        await observer.close(); if (mediaRecords.length === 0) await idle(); return { kind: "observed" };
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
    case "sdp-custody": {
      // The automatic pump owns the original accepted intake. Recover that SAME retained
      // handle using metadata only; a past Derived receipt cannot reconstruct missing keys.
      const calls = providerCalls;
      const ready = await broker.nativeDerivationFromChannel(owner, channel, key, value.purpose, value.row, value.identity).attempt();
      assert.equal(ready.kind, "ready");
      if (ready.kind !== "ready") throw new Error("original browser chain unavailable");
      assert.equal(providerCalls, calls, "cached intake recovery must not reconstruct keys");
      assert.equal(ready.derivation.inspect(), "accepted");
      chain = ready.chain;
      parent = Object.freeze({ purpose: value.purpose, row: value.row, identity: value.identity });
      if (!mediaLosses.has(parent.row)) {
        assert.ok(mediaLosses.size < 2, "bounded per-parent reply-loss observations");
        mediaLosses.set(parent.row, { modes: new Set(), lose: false, detach: false, history: false });
      }
      const before = broker.counts().bytes;
      assert.ok(before < 96 * 1024, "actual old browser variable envelope returned");
      const deadline = performance.now() + 5000;
      let observed: number;
      do {
        assert.ok(performance.now() < deadline, "original SDK custody stayed busy");
        observed = capacityRefusals;
        await broker.prepareNativeSdpHandoff(owner, channel, ready.chain);
        if (mediaRecords.length === 0) await idle();
        // Retry ONLY an actual authenticated native Capacity on this same original
        // intent. The Rust parent independently requires installed SDK custody below.
        if (capacityRefusals > observed) await new Promise<void>(yes => setTimeout(yes, 25));
      } while (capacityRefusals > observed);
      assert.equal(broker.counts().bytes, before + 64 * 1024);
      return { kind: "custody-attempted", bytes: 64 * 1024 };
    }
    case "pc-custody": {
      const before = broker.counts().bytes;
      const deadline = performance.now() + 5000;
      let observed: number;
      do {
        assert.ok(performance.now() < deadline, "original PC intent stayed busy");
        observed = capacityRefusals;
        await broker.prepareNativePcIntent(owner, channel, chain);
        if (mediaRecords.length === 0) await idle();
        if (capacityRefusals > observed) await new Promise<void>(yes => setTimeout(yes, 25));
      } while (capacityRefusals > observed);
      // This Promise<void> is deliberately not the success oracle. Rust independently
      // requires an original Root intent attached by the actual HTTP14 dispatch below.
      assert.equal(broker.counts().bytes, before);
      return { kind: "pc-attempted" };
    }
    case "rtc-round":
    case "rtc-round-loss":
    case "rtc-round-media":
    case "rtc-round-media-loss": {
      loseSdpReplies = value.name === "rtc-round-loss";
      // Controlled SDK ONLY: this validates real native/HTTP/WebCrypto/TCP composition,
      // not a WebView, SDP parser, ICE connectivity, media tracks or audible WebRTC.
      const previous = globalThis.RTCPeerConnection;
      const calls = { created: 0, local: 0, remote: 0, closed: 0 };
      class Peer {
        private local: "offer" | "answer" | null = null;
        private remote: "offer" | "answer" | null = null;
        constructor() { calls.created++; assert.equal(calls.created, 1); }
        iceGatheringState = "complete";
        get localDescription() { assert.ok(this.local); return { type: this.local, sdp: "s".repeat(8192) }; }
        addTransceiver() { assert.equal(this.remote, null); }
        async setLocalDescription() { calls.local++; assert.equal(calls.local, 1); this.local = this.remote === "offer" ? "answer" : "offer"; }
        async setRemoteDescription(value: RTCSessionDescriptionInit) {
          calls.remote++; assert.equal(calls.remote, 1);
          assert.equal(value.type, this.local === "offer" ? "answer" : "offer");
          assert.equal(value.sdp, "s".repeat(8192)); this.remote = value.type as "offer" | "answer";
        }
        addEventListener() {} removeEventListener() {}
        close() { calls.closed++; mediaEvent("peer-close"); }
      }
      globalThis.RTCPeerConnection = Peer as unknown as typeof RTCPeerConnection;
      const before = broker.counts().bytes;
      try {
        // Independent native histories are the oracle. A void attempt or controlled SDK
        // success cannot prove both accepted results, Applied, settlement, ACK and Return.
        let complete = false;
        while (!complete) {
          await broker.completeNativeSdpRtcRound(owner, channel, chain); if (mediaRecords.length === 0) await idle();
          complete = (await rpc<{ complete: boolean }>("observeSdpRound", { owner: owner.token, executor: channelExecutor! })).complete;
          if (!complete) await new Promise<void>(yes => setTimeout(yes, 50));
        }
        assert.deepEqual(calls, { created: 1, local: 1, remote: 1, closed: 0 });
        assert.equal(broker.counts().bytes, before);
        if (value.name === "rtc-round-media" || value.name === "rtc-round-media-loss") {
          mediaLoss().lose = value.name === "rtc-round-media-loss"; mediaStarted = performance.now();
          // Read-only test observation of the actual retained driver, never a new app
          // authority/connected flag. Rust independently observes original admission.
          const retained = (broker as unknown as { ipcChannels: Map<string, { negotiation: {media: { adopted: boolean; closed: boolean } | null} }> }).ipcChannels.get(owner.token)!;
          const deadline = performance.now() + 5000;
          do {
            assert.ok(performance.now() < deadline, `media c${Number(retained.negotiation.media?.closed)} sdk${calls.closed} lost${[...mediaLoss().modes].join("/")} ${mediaEvents.join(",")}`);
            await broker.completeNativeMediaHandoff(owner, channel, chain); if (mediaRecords.length === 0) await idle();
            if (!retained.negotiation.media?.adopted) await new Promise<void>(yes => setTimeout(yes, 25));
          } while (!retained.negotiation.media?.adopted);
          assert.equal(retained.negotiation.media.closed, false);
          assert.equal(broker.counts().bytes, before + 8192);
          assert.equal(calls.created, 1); assert.equal(calls.closed, 0);
          mediaPeerCalls = calls;
          assert.ok(mediaRecords.length < 2, "bounded actual peer observations");
          assert.ok(!mediaRecords.some(record => record.plan === chain));
          mediaRecords.push({ plan: chain, parent, calls, currents: 0 });
          if (originalSession) {
            const session = (broker as unknown as { ipcChannels: Map<string, { keys: unknown; bootstrap: unknown; tx: bigint }> }).ipcChannels.get(owner.token)!;
            assert.equal(session.keys, originalSession.keys); assert.equal(session.bootstrap, originalSession.bootstrap);
            assert.ok(session.tx > originalSession.tx, "second peer must use increasing original Session counters");
            // The test observation must not keep the actual key/bootstrap backing alive
            // after application collection. No identity witness survives its final check.
            originalSession = null;
          }
          assert.deepEqual([...mediaLoss().modes], mediaLoss().lose ? [0, 4] : []);
        }
        const lost = replyLoss.count;
        assert.equal(lost, loseSdpReplies ? 4 : 0);
        return { kind: "sdp-round", peers: calls.created, local: calls.local, remote: calls.remote, lost };
      } finally { globalThis.RTCPeerConnection = previous; loseSdpReplies = false; }
    }
    case "media-retire-start": {
      assert.ok(mediaPeerCalls); assert.equal(retirementWork,null); retirementBefore=broker.counts().bytes;
      retirementWork=broker.retireNativeNegotiation(owner,channel,chain).then(() => { retirementDone=true; },() => {retirementFailed=true;});
      return {kind:"retirement-started"};
    }
    case "media-retire-progress": {
      assert.ok(mediaPeerCalls); assert.ok(retirementWork); assert.equal(retirementFailed,false);
      await broker.completeNativeMediaHandoff(owner,channel,chain);
      const before=currentObservations(); await broker.completeNativeMediaHandoff(owner,channel,chain);
      assert.ok(currentObservations()>before); assert.equal(mediaPeerCalls.closed,0);
      return {done:retirementDone,paid:broker.counts().bytes===retirementBefore,oldCurrent:chain.isCurrent(),closed:mediaPeerCalls.closed};
    }
    case "media-retire-finish": {
      assert.ok(retirementWork); await retirementWork; assert.equal(retirementFailed,false); assert.ok(mediaPeerCalls);
      assert.equal(mediaPeerCalls.closed,0); assert.equal(broker.counts().bytes,retirementBefore-65536);
      return {kind:"negotiation-retired",peers:1,closed:0,releasedBytes:65536};
    }
    case "media-retire":
    case "media-retire-loss": {
      assert.ok(mediaPeerCalls); mediaLoss().detach=value.name === "media-retire-loss";
      const before=broker.counts().bytes;
      await broker.retireNativeNegotiation(owner,channel,chain);
      const retained=(broker as unknown as {ipcChannels:Map<string,{negotiation:{negotiationRetired:boolean}}>}).ipcChannels.get(owner.token)!;
      assert.equal(retained.negotiation.negotiationRetired,true,"original retirement returned before completion; peerClosed="+mediaPeerCalls.closed);
      assert.equal(mediaPeerCalls.closed,0);
      assert.equal(chain.isCurrent(),false); assert.equal(broker.counts().bytes,before-65536);
      // First join any earlier flight, then require a NEW actual authenticated Current
      // after SDK/key/Plan retirement. A historical admitted receipt cannot satisfy this.
      await broker.completeNativeMediaHandoff(owner,channel,chain);
      const observed=currentObservations();
      await broker.completeNativeMediaHandoff(owner,channel,chain);
      assert.ok(currentObservations()>observed); assert.equal(mediaPeerCalls.closed,0);
      if(mediaLoss().detach) assert.ok(mediaLoss().modes.has(7));
      return {kind:"negotiation-retired",peers:1,closed:0,releasedBytes:65536};
    }
    case "media-history-pending": {
      mediaLoss().history=true;
      assert.equal(await broker.collectNativeNegotiationHistory(owner,channel,chain),false);
      return {kind:"history-pending"};
    }
    case "media-current-after-history": {
      assert.ok(mediaPeerCalls);
      const paid=broker.counts().bytes, deadline=performance.now()+5000;
      while (!await broker.collectNativeNegotiationHistory(owner,channel,chain)) {
        assert.ok(performance.now()<deadline,"same native history collection did not recover");
        await new Promise<void>(yes=>setTimeout(yes,25));
      }
      assert.ok(mediaLoss().modes.has(8),"actual completed authenticated mode8 reply was lost and recovered");
      assert.equal(broker.counts().bytes,paid,"native history does not refund browser fixed graph");
      // Join any timer/observer flight that began before native history disappeared.
      // The second call must start its own physical Current after that join returned.
      await broker.completeNativeMediaHandoff(owner,channel,chain);
      const before = currentObservations();
      await broker.completeNativeMediaHandoff(owner,channel,chain);
      assert.ok(currentObservations() > before, "fresh authenticated Current after native history removal");
      assert.equal(mediaPeerCalls.closed,0);
      // Native independently held/collected the original parent and child, and the first
      // authenticated mode8 completion was lost above. Now exercise the actual browser
      // transaction on that SAME Session rather than treating native absence as JS refund.
      const state = broker as unknown as { chains: WeakMap<object, {fixedBytes: number}>;
        ipcChannels: Map<string, {keys: unknown; bootstrap: unknown; tx: bigint}> };
      const old = state.chains.get(chain)!;
      assert.ok(old); const fixed = old.fixedBytes;
      const session = state.ipcChannels.get(owner.token)!;
      const keys = session.keys, bootstrap = session.bootstrap, tx = session.tx;
      assert.equal(await broker.compactNativeNegotiation(owner,channel,chain),true);
      assert.equal(broker.counts().bytes,paid-fixed+2048);
      assert.equal(state.chains.has(chain),false); assert.equal(chain.isCurrent(),false);
      assert.equal(session.keys,keys); assert.equal(session.bootstrap,bootstrap); assert.ok(session.tx>=tx);
      assert.throws(()=>broker.compactNativeNegotiation(owner,channel,{...chain}),/original browser history/);
      // Old metadata is gone; exact original media still resolves independently. Require
      // another physically started Current after compaction, not an old joined observer.
      await broker.completeNativeMediaHandoff(owner,channel,chain);
      const afterCompaction=currentObservations();
      await broker.completeNativeMediaHandoff(owner,channel,chain);
      assert.ok(currentObservations()>afterCompaction); assert.equal(mediaPeerCalls.closed,0);
      return { kind: "indexed-media-current", peers:1 };
    }
    case "advance-pair": {
      assert.ok(parent); assert.equal(mediaRecords.length, 1);
      // Actual encrypted Begin(false) through this same Session must return native
      // Unauthorized after collection. A local exception/uncertain result cannot pass.
      let historical = await broker.recoverChannelHistory(owner, channel, parent.purpose, parent.row);
      const deadline = performance.now() + 5000;
      while (historical.kind === "busy" || (historical.kind === "refused" && historical.code === "capacity")) {
        assert.ok(performance.now() < deadline);
        await new Promise<void>(yes => setTimeout(yes, 25));
        historical = await broker.recoverChannelHistory(owner, channel, parent.purpose, parent.row);
      }
      assert.deepEqual(historical, { kind: "refused", code: "unauthorized" });
      const session = (broker as unknown as { ipcChannels: Map<string, { keys: unknown; bootstrap: unknown; tx: bigint }> }).ipcChannels.get(owner.token)!;
      originalSession = { keys: session.keys, bootstrap: session.bootstrap, tx: session.tx };
      assert.equal(broker.advanceNativeNegotiation(owner, channel, chain), true);
      assert.equal(broker.advanceNativeNegotiation(owner, channel, chain), true, "old marker only observes its prior advance");
      return { kind: "advanced" };
    }
    case "media-current-all": {
      assert.ok(mediaRecords.length > 0 && mediaRecords.length <= 2);
      // Join a preexisting observer first, then require a physically new authenticated
      // Current for EACH original full parent after both native histories disappeared.
      for (const record of mediaRecords) {
        await broker.completeNativeMediaHandoff(owner, channel, record.plan);
        const before = record.currents;
        await broker.completeNativeMediaHandoff(owner, channel, record.plan);
        assert.ok(record.currents > before, "fresh authenticated Current on exact retained peer");
        assert.equal(record.calls.closed, 0);
      }
      return { kind: "all-media-current", peers: mediaRecords.length };
    }
    case "end-peers": {
      assert.ok(mediaRecords.length > 0 && mediaRecords.length <= 2);
      for (const record of mediaRecords) assert.equal(record.calls.closed, 0);
      // This is deliberately BEFORE native End. Check the synchronous browser fence,
      // without letting prior native revocation make the assertion trivially true.
      originalSession = null;
      owner.retire();
      for (const record of mediaRecords) assert.equal(record.calls.closed, 1);
      return { kind: "peers-ended", peers: mediaRecords.length };
    }
    case "media-revoked": {
      assert.ok(mediaPeerCalls);
      const deadline = performance.now() + 6000;
      // Native End already happened; leave browser owner live. Only the actual media
      // refresh/expiry driver may close the SDK peer in this step.
      while (mediaRecords.some(record => record.calls.closed === 0)) {
        assert.ok(performance.now() < deadline, "native revocation did not close actual peer");
        await new Promise<void>(yes => setTimeout(yes, 25));
      }
      for (const record of mediaRecords) assert.equal(record.calls.closed, 1);
      return { kind: "media-stopped" };
    }
    case "close": {
      originalSession = null;
      owner.retire(); broker.retrySelectedJoinKeys();
      for (const record of mediaRecords) assert.equal(record.calls.closed, 1, "End synchronously closes EVERY original adopted peer");
      await channel.close(); await idle();
      if (coordinatedObserver) {
        await coordinatedObserver.close();
        for (const peer of coordinatedPeers) assert.equal(peer.closed, 1);
        globalThis.RTCPeerConnection = previousCoordinatedRtc!;
      }
      assert.equal(broker.counts().bytes, 0, "actual original browser owners must collect");
      for (const record of mediaRecords) {
        const loss = mediaLosses.get(record.parent.row)!;
        const expectedLostModes = loss.lose ? (loss.detach ? [0,4,7] : [0,4]) : [];
        if (loss.history) expectedLostModes.push(8); // EACH original parent lost its own completed history reply.
        if (loss.lose) expectedLostModes.push(3);
        assert.deepEqual([...loss.modes], expectedLostModes);
      }
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
    stepRunning = true; currentStep = String(frame.name);
    void step(frame as Step).then(value => {
      stepRunning = false; send({ tag: "done", value });
    }, error => {
      stepRunning = false;
      // Fixed-size diagnostics only: no Input/Result/body bytes are ever serialized here.
      send({ tag: "failed", reason: ("step="+currentStep+" "+String(error instanceof Error ? error.message : "endpoint failed").slice(0,160)+
        " stages="+stages.join(",")+" media="+mediaEvents.join(",")).slice(0,512) });
    }).catch(() => {
      // send()/response-handler failures belong to this same control step. Never leave
      // its returned Promise unobserved or print arbitrary assertion/frame payloads.
      const step = currentStep.replace(/[^a-z-]/g,"?").slice(0,40);
      process.stderr.write(`control-egress-failed step=${step}\n`);
      process.exitCode = 1; stopAvailability(); process.stdin.destroy();
    });
  }
}
process.stdin.on("data", (bytes: Buffer) => {
  try {
    // OS chunks may contain a reply followed by the next step. The fixed reader clears
    // each frame BEFORE accept(), whose original one-step/one-callback checks still apply.
    control.feed(bytes, accept);
  } catch (error) {
    // Diagnostic metadata only. Never stringify the input frame or arbitrary assertion
    // payload (which could contain values); classify only our known fixed messages.
    const message = error instanceof Error ? error.message : "";
    const reason = message.startsWith("invalid control frame length") ? "frame-size"
      : message.startsWith("control reader unavailable") ? "reader-state"
      : message.startsWith("unsolicited native control response") ? "no-callback"
      : message.startsWith("control steps are serialized") ? "overlapped-step" : "invalid-metadata";
    const step = currentStep.replace(/[^a-z-]/g,"?").slice(0,40);
    process.stderr.write(`control-ingress:${reason} step=${step} used=${control.pendingBytes} declared=${control.declaredBytes} chunk=${bytes.length} callback=${Number(callback!==null)} running=${Number(stepRunning)}\n`);
    process.exitCode = 1; stopAvailability(); process.stdin.destroy();
  }
});
process.stdin.on("end", () => {
  stopAvailability();
  if (retirementWork && !retirementDone) {
    process.stderr.write("native media retirement incomplete at control EOF\n"); process.exitCode=1;
  }
  try { control.finish(); } catch { process.exitCode = 1; }
  if (callback || stepRunning) process.exitCode = 1;
  process.exit(process.exitCode ?? 0);
});
