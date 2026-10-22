import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { MediaCaptureSession } from "./media-capture.ts";
import { heartbeatRecovery, isCurrentVoiceRoom } from "./voice-signaling.ts";
import { mappableIcePort, mappingAddressPolicy, routerMappedCandidate, shouldSignalHostCandidate, mappedIceIsCurrent } from "./callroutes.ts";

const MEMBER = "a".repeat(64);
const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const script = source.slice(source.indexOf(">", source.indexOf("<script")) + 1, source.indexOf("</script>"));
const tree = ts.createSourceFile("App.ts", script, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
const names = new Set(["createPeer", "negotiatePeer", "flushWaitingIce", "recoverPeer", "handleCallSignal", "isCurrentCallPeer", "isCurrentCallJoin", "sniffTransport", "signalMappedCandidate", "refreshDefaultRoute", "leaveVoice", "destroyVoiceOnUnmount", "releaseMappedCallPorts"]);
const functions = tree.statements.filter(node => ts.isFunctionDeclaration(node) && node.name && names.has(node.name.text));
assert.ok(functions.length >= 5, "exercise actual App RTC handlers and continuations");
const production = ts.transpileModule(functions.map(node => node.getFullText(tree)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

function deferred<T = void>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}

/** Real App handlers, with RTC operations that can be held across peer/call replacement. */
function fixture() {
  return new Function("MediaCaptureSession", "heartbeatRecovery", "isCurrentVoiceRoom", "mappableIcePort", "mappingAddressPolicy", "routerMappedCandidate", "shouldSignalHostCandidate", "mappedIceIsCurrent", `
    let locked = false, inCall = true, callServer = 1, callChannel = "7", callSelfId = "b".repeat(64);
    const callLifecycleSession = new MediaCaptureSession();
    let activeCallLease = callLifecycleSession.begin();
    const callPeers = {}, waitingIce = {};
    let callParticipants = [], callPeerStates = {}, remoteStreams = {}, peerVideoCodec = {}, peerTransport = {};
    const calls = { attached: [], removed: [], signals: [], stats: [], local: [], remote: [], ice: [], maps: [], stopped: 0, closed: 0, mappingEnds: 0, mappingDestroy: 0 };
    const mappedCallPorts = new Map(), mappingGates = [], routeGates = [];
    let callMappedRoutes = 0;
    const invoke = async (command, args) => {
      if (command === "default_route_address") return routeGates.shift() ?? null;
      if (command !== "map_call_port") throw new Error("unexpected native command");
      calls.maps.push(args);
      const gate = mappingGates.shift(); if (gate) await gate;
      return { ip: "203.0.113.9", port: args.port, mechanism: "fixture", confirmed: false };
    };
    const callMappingSession = {
      map: (lease, port, address) => invoke("map_call_port", { port, address }),
      endCall() { calls.mappingEnds++; }, destroy() { calls.mappingDestroy++; },
    };
    class RTCPeerConnection {
      signalingState = "stable"; connectionState = "new";
      localDescription = null; remoteDescription = null;
      localGate = null; remoteGate = null; statsGate = null;
      createDataChannel() { throw new Error("voice-only RTC fixture"); }
      getTransceivers() { return []; }
      async setLocalDescription() {
        calls.local.push(this);
        if (this.localGate) await this.localGate;
        this.localDescription = { type: this.remoteDescription?.type === "offer" ? "answer" : "offer", sdp: "fixture" };
      }
      async setRemoteDescription(value) {
        calls.remote.push(this);
        if (this.remoteGate) await this.remoteGate;
        this.remoteDescription = value;
      }
      async addIceCandidate(value) { calls.ice.push([this, value]); }
      async getStats() {
        calls.stats.push(this);
        if (this.statsGate) await this.statsGate;
        return new Map([
          ["out", { id: "out", type: "outbound-rtp", kind: "video", bytesSent: 1, codecId: "codec" }],
          ["codec", { id: "codec", mimeType: "video/VP8" }],
          ["transport", { id: "transport", type: "transport", selectedCandidatePairId: "pair" }],
          ["pair", { id: "pair", localCandidateId: "local", remoteCandidateId: "remote" }],
          ["local", { id: "local", candidateType: "relay" }],
          ["remote", { id: "remote", candidateType: "host" }],
        ]);
      }
      restartIce() {}
      close() { calls.closed++; this.connectionState = "closed"; this.signalingState = "closed"; }
    }
    const RTCSessionDescription = function(value) { return value; };
    const RTCIceCandidate = function(value) { return value; };
    const iceServers = () => [], isPoliteTowards = () => true;
    let localStream = null, camStream = null, myVideo = null;
    const drawClaims = null;
    let defaultRouteIpv4 = null;
    let suppressedIceCandidates = 0;
    const sendSignal = async (...args) => { calls.signals.push(args); };
    const attachRemote = (...args) => calls.attached.push(args);
    const remoteAudioRouter = { classify: () => "mic", noteVideo: () => false, clear() {} };
    const regroupRemoteAudio = () => {}, dropRemoteVideo = () => {}, jamRecMembership = () => {};
    const displayKey = value => value, b64dec = value => atob(value);
    const bufferIce = (old, value) => [...old, value];
    const console = { warn() {}, info() {} };
    function removePeer(id) { calls.removed.push(id); callPeers[id]?.pc.close(); delete callPeers[id]; delete waitingIce[id]; }
    // Non-RTC owners used by the actual leaveVoice body. Keep these explicit: the test runs
    // production teardown, including track stops and peer closure, rather than a copied cleanup.
    const stopStudioAudio = () => {}, instReleaseAll = () => {}, clearScreenAudioSources = () => {},
      stopDrawClaims = () => {}, stopJamMetTimer = () => {}, jamStopPlayback = () => {},
      jukeReset = () => {}, stopMeters = () => {}, onDeviceChange = () => {}, dropPresence = () => {},
      announceHangup = () => {};
    const videoCaptureSession = new MediaCaptureSession(), screenAudioCaptureSession = new MediaCaptureSession(), micCaptureSession = new MediaCaptureSession();
    const navigator = { mediaDevices: { removeEventListener() {} } };
    const peerChainKinds = new Map(), remoteHeldAt = new Map(), padFlashTimers = new Map(), jamPendingLocalEvents = new Map();
    const jamPublicationPacer = { reset() {} }, jamPublicationGeneration = { advance() {} }, jamLocalRender = { reset() {} }, jamTakeSuppression = { reset() {} };
    const synthCtx = null, jamCallCuePlayer = { cancel() {} };
    class JamLatestTaskQueue {}
    let screenAudioError = "", localVideoStream = null, focusSolo = "", callAudioContext = null,
      callHeld = [], remoteHeld = {}, peerMeta = {}, jamEngine = null, jamSelfChan = null, jamMySn = "", jamMyQ = 0,
      jamPublishedPatch = null, jamPublishedCanonical = null, jamEnsurePublication = null, jamPublicationQueue = null,
      padFlash = {}, jamAbuseMuted = {}, jamAnnTimer, jamMetPushTimer, jamMet = null, jamMetUi = null, jamMetRev = 0,
      jamPlaybackPreparationQueue = null, jamRec = null, jamRecTimer, jamRecUi = "off", jamRecGaps = 0, jamRecClock = 0,
      jamRecTimeline = {}, jamTakes = [], jamTakesOpen = false, keymapCapture = null, instOpen = false, stageOpen = false,
      focusOpen = false, focusDismissed = false, callDeafened = false, voiceMutedPeers = {}, jukeTakeProgressLease = null,
      pingTimer, callMuted = false, micOn = false, callChannelName = "", callServerName = "", callSharedIds = new Set(),
      callProfiles = {}, secInfoOpen = false;
    const jamPeerChans = {}, jamPeerDecs = {}, jamPeerBudgets = {}, jamPeerInbound = {}, jamPeerSn = {},
      jamSourceRenderGeneration = {}, jamSourceGateMuted = {}, jamClockSyncs = {}, jamProbes = {};
    ${production}
    return {
      create: createPeer, negotiate: negotiatePeer, calls,
      destroy: destroyVoiceOnUnmount,
      capture() {
        const stream = { getTracks: () => [{ stop() { calls.stopped++; } }] };
        localStream = stream; camStream = stream;
      },
      signal(id, value) { return handleCallSignal(id, btoa(JSON.stringify({ callId: "7", ...value })), 1); },
      replace(id) { delete callPeers[id]; delete waitingIce[id]; return createPeer(id); },
      invalidate() { callLifecycleSession.invalidate(); },
      lockBeforeCleanup() { locked = true; },
      rejoinBeforeCleanup() { activeCallLease = callLifecycleSession.begin(); },
      map(peer, candidate) {
        // Real gathered candidates have a local media description. Supply that RTC-owned input
        // explicitly so mapping tests exercise the production SDP generation check too.
        peer.pc.localDescription ??= { type: "offer", sdp: "v=0\\r\\nm=audio 9 UDP/TLS/RTP/SAVPF 111\\r\\na=mid:" + (candidate.sdpMid ?? "0") + "\\r\\na=ice-ufrag:" + (candidate.usernameFragment ?? "fixture") + "\\r\\n" };
        return signalMappedCandidate(peer, candidate);
      },
      holdMap(promise) { mappingGates.push(promise); },
      route: refreshDefaultRoute,
      holdRoute(promise) { routeGates.push(promise); },
      routeAddress() { return defaultRouteIpv4; },
      resetMappings() { mappedCallPorts.clear(); callMappedRoutes = 0; },
      mappings() { return { slots: mappedCallPorts.size, routes: callMappedRoutes }; },
      current(id) { return callPeers[id]; },
      waiting(id) { return waitingIce[id]; },
      states() { return callPeerStates; },
      transport() { return { peerVideoCodec, peerTransport }; },
    };
  `)(MediaCaptureSession, heartbeatRecovery, isCurrentVoiceRoom, mappableIcePort, mappingAddressPolicy, routerMappedCandidate, shouldSignalHostCandidate, mappedIceIsCurrent);
}

test("an old default-route lookup cannot change a replacement call's candidate filter", async () => {
  for (const retire of ["lockBeforeCleanup", "rejoinBeforeCleanup"]) {
    const app = fixture();
    app.holdRoute(Promise.resolve("192.168.1.1")); await app.route();
    assert.equal(app.routeAddress(), "192.168.1.1");
    const gate = deferred<string>(); app.holdRoute(gate.promise);
    const old = app.route(); app[retire](); gate.resolve("192.168.99.1"); await old;
    assert.equal(app.routeAddress(), null, "retired lookup cannot replace the safe unknown policy");
  }
});

test("a new call signals its real LAN candidate while the new default route is unknown", async () => {
  const app = fixture(); app.holdRoute(Promise.resolve("192.168.1.1")); await app.route();
  app.rejoinBeforeCleanup();
  const gate = deferred<string>(); app.holdRoute(gate.promise); const lookup = app.route();
  const peer = app.create(MEMBER);
  const candidate = { type: "host", protocol: "udp", address: "192.168.2.3", port: 1234, toJSON: () => ({ candidate: "new LAN ICE" }) };
  assert.equal(shouldSignalHostCandidate(candidate, "192.168.1.1"), false, "old route would suppress this valid candidate");
  peer.pc.onicecandidate({ candidate });
  assert.equal(app.calls.signals.length, 1);
  gate.resolve("192.168.2.3"); await lookup;
  assert.equal(app.routeAddress(), "192.168.2.3");
});

test("App destruction closes its old call and rejects queued RTC work before remount", async () => {
  const app = fixture(), old = app.create(MEMBER), gate = deferred();
  app.capture(); old.pc.localGate = gate.promise;
  const pendingOffer = app.negotiate(old);
  app.destroy();
  assert.equal(app.calls.closed, 1);
  assert.equal(app.calls.stopped, 2, "both microphone and camera tracks are stopped");
  assert.equal(app.calls.mappingEnds, 1);
  assert.equal(app.calls.mappingDestroy, 1);
  old.pc.ontrack({ streams: [{ id: "retired mic" }], track: { kind: "audio" } });
  old.pc.onicecandidate({ candidate: { toJSON: () => ({ candidate: "retired ICE" }) } });
  gate.resolve(); await pendingOffer;
  assert.equal(app.calls.attached.length, 0);
  assert.equal(app.calls.signals.length, 0);
});

test("queued RTC track, state and ICE callbacks cannot affect a replacement peer", async () => {
  const app = fixture(), old = app.create(MEMBER);
  const track = { streams: [{ id: "mic" }], track: { kind: "audio" } };
  old.pc.ontrack(track);
  assert.equal(app.calls.attached.length, 1, "a live peer can attach its own audio");
  const replacement = app.replace(MEMBER);
  old.pc.ontrack(track);
  old.pc.connectionState = "closed";
  old.pc.onconnectionstatechange();
  old.pc.onicecandidate({ candidate: { toJSON: () => ({ candidate: "old ICE" }) } });
  assert.equal(app.calls.attached.length, 1, "retired audio must not attach by identity alone");
  assert.equal(app.current(MEMBER), replacement, "an old closed event cannot remove the new peer");
  assert.deepEqual(app.states(), {});
  assert.equal(app.calls.signals.length, 0);
  replacement.pc.ontrack(track);
  assert.equal(app.calls.attached.length, 2);
  replacement.pc.connectionState = "closed";
  replacement.pc.signalingState = "closed";
  replacement.pc.onconnectionstatechange();
  assert.equal(app.current(MEMBER), undefined, "a current closed peer still owns cleanup");
  assert.deepEqual(app.calls.removed, [MEMBER]);
});

test("a local offer suspended across replacement cannot send obsolete SDP", async () => {
  const app = fixture(), old = app.create(MEMBER), gate = deferred();
  old.pc.localGate = gate.promise;
  const work = app.negotiate(old);
  assert.equal(app.calls.local.length, 1);
  app.replace(MEMBER); gate.resolve(); await work;
  assert.equal(app.calls.signals.length, 0);
});

test("a remote offer suspended across replacement cannot consume new ICE or send an answer", async () => {
  const app = fixture(), old = app.create(MEMBER), gate = deferred();
  old.pc.remoteGate = gate.promise;
  const work = app.signal(MEMBER, { type: "offer", sdp: { type: "offer", sdp: "old" } });
  assert.equal(app.calls.remote.length, 1);
  const replacement = app.replace(MEMBER);
  await app.signal(MEMBER, { type: "ice", candidate: { candidate: "new ICE" } });
  gate.resolve(); await work;
  assert.equal(app.current(MEMBER), replacement);
  assert.deepEqual(app.waiting(MEMBER), [{ candidate: "new ICE" }]);
  assert.equal(app.calls.local.length, 0);
  assert.equal(app.calls.ice.length, 0);
  assert.equal(app.calls.signals.length, 0);
});

test("lock, call invalidation and same-room rejoin revoke callbacks before peer-map cleanup", () => {
  for (const mode of ["lock", "invalidate", "rejoin"]) {
    const app = fixture(), peer = app.create(MEMBER);
    if (mode === "lock") app.lockBeforeCleanup();
    else if (mode === "rejoin") app.rejoinBeforeCleanup(); else app.invalidate();
    peer.pc.ontrack({ streams: [{ id: "old mic" }], track: { kind: "audio" } });
    peer.pc.onicecandidate({ candidate: { toJSON: () => ({ candidate: "old ICE" }) } });
    assert.equal(app.calls.attached.length, 0);
    assert.equal(app.calls.signals.length, 0);
  }
});

const candidate = { type: "host", protocol: "udp", address: "192.168.1.20", port: 49152 };

test("a peer replaced during mapping acquisition waits for the shared result and sends its own ICE", async () => {
  const app = fixture(), old = app.create(MEMBER), gate = deferred();
  app.holdMap(gate.promise);
  const retired = app.map(old, { ...candidate, foundation: "old", usernameFragment: "old-ufrag", sdpMid: "0", sdpMLineIndex: 0 });
  const current = app.replace(MEMBER);
  const waiting = app.map(current, { ...candidate, foundation: "new", usernameFragment: "new-ufrag", sdpMid: "1", sdpMLineIndex: 1 });
  assert.equal(app.calls.maps.length, 1);
  assert.equal(app.calls.signals.length, 0);
  gate.resolve(); await Promise.all([retired, waiting]);
  assert.deepEqual(app.mappings(), { slots: 1, routes: 1 });
  assert.equal(app.calls.signals.length, 1);
  const mapped = app.calls.signals[0][2].candidate;
  assert.match(mapped.candidate, /^candidate:newR /);
  assert.equal(mapped.usernameFragment, "new-ufrag");
  assert.equal(mapped.sdpMid, "1");
  assert.equal(mapped.sdpMLineIndex, 1);
});

test("a same-peer ICE restart shares the pending mapping and suppresses only the old generation", async () => {
  const app = fixture(), peer = app.create(MEMBER), gate = deferred();
  const sdp = (fragment: string) => ({ type: "offer", sdp: `v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\na=ice-ufrag:${fragment}\r\n` });
  peer.pc.localDescription = sdp("old");
  app.holdMap(gate.promise);
  const old = app.map(peer, { ...candidate, sdpMid: "0", sdpMLineIndex: 0, usernameFragment: "old" });
  peer.pc.localDescription = sdp("new");
  const current = app.map(peer, { ...candidate, sdpMid: "0", sdpMLineIndex: 0, usernameFragment: "new" });
  gate.resolve(); await Promise.all([old, current]);
  assert.equal(app.calls.maps.length, 1);
  assert.equal(app.calls.signals.length, 1);
  assert.equal(app.calls.signals[0][2].candidate.usernameFragment, "new");
  assert.deepEqual(app.mappings(), { slots: 1, routes: 1 });
});

test("a completed shared router mapping belongs to the call, while ICE sends belong to the exact peer", async () => {
  const app = fixture(), old = app.create(MEMBER), gate = deferred();
  app.holdMap(gate.promise); const work = app.map(old, candidate);
  const replacement = app.replace(MEMBER); gate.resolve(); await work;
  assert.deepEqual(app.mappings(), { slots: 1, routes: 1 }, "the live call retains its granted mapping for cleanup/reuse");
  assert.equal(app.calls.signals.length, 0, "the retired peer cannot send ICE after mapping");
  await app.map(replacement, candidate);
  assert.equal(app.calls.maps.length, 1);
  assert.equal(app.calls.signals.length, 1);
});

test("router completion after lock or same-room rejoin cannot populate current routes", async () => {
  for (const mode of ["lock", "rejoin"]) {
    const app = fixture(), old = app.create(MEMBER), gate = deferred();
    app.holdMap(gate.promise); const work = app.map(old, candidate);
    if (mode === "lock") app.lockBeforeCleanup(); else app.rejoinBeforeCleanup();
    app.resetMappings(); gate.resolve(); await work;
    assert.deepEqual(app.mappings(), { slots: 0, routes: 0 });
    assert.equal(app.calls.signals.length, 0);
  }
});

test("an old router completion cannot overwrite a replacement slot for the same port", async () => {
  const app = fixture(), peer = app.create(MEMBER), old = deferred(), current = deferred();
  app.holdMap(old.promise); const oldWork = app.map(peer, candidate);
  app.resetMappings(); app.holdMap(current.promise); const currentWork = app.map(peer, candidate);
  old.resolve(); await oldWork;
  assert.deepEqual(app.mappings(), { slots: 1, routes: 0 });
  assert.equal(app.calls.signals.length, 0);
  current.resolve(); await currentWork;
  assert.deepEqual(app.mappings(), { slots: 1, routes: 1 });
  assert.equal(app.calls.signals.length, 1);
});

test("delayed RTC statistics cannot relabel a replacement peer's codec or path", async () => {
  const app = fixture(), old = app.create(MEMBER), gate = deferred();
  old.pc.statsGate = gate.promise;
  old.pc.connectionState = "connected";
  old.pc.onconnectionstatechange();
  assert.equal(app.calls.stats.length, 1);
  app.replace(MEMBER); gate.resolve(); await Promise.resolve(); await Promise.resolve();
  assert.deepEqual(app.transport(), { peerVideoCodec: {}, peerTransport: {} });
});
