import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { MediaCaptureSession } from "./media-capture.ts";
import { hangupTargets, heartbeatRecovery, isCurrentVoiceRoom, legacyCallSignalAdmission, legacyCallSignalEgress, legacyCallSignalKind, mergePeerState } from "./voice-signaling.ts";
import { onlineCallTargets, rosterIdentities } from "./call-identity.ts";
import { mappableIcePort, mappingAddressPolicy, routerMappedCandidate, shouldSignalHostCandidate, mappedIceIsCurrent } from "./callroutes.ts";

const MEMBER = "a".repeat(64);
const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const script = source.slice(source.indexOf(">", source.indexOf("<script")) + 1, source.indexOf("</script>"));
const tree = ts.createSourceFile("App.ts", script, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
// The real async egress (sendSignal, broadcast/broadcastOn, announceHangup) runs too, so the
// tests below exercise roster reads held across teardown rather than a stubbed send.
const names = new Set(["createPeer", "negotiatePeer", "flushWaitingIce", "recoverPeer", "handleCallSignal", "isCurrentCallPeer", "isCurrentCallJoin", "sniffTransport", "signalMappedCandidate", "refreshDefaultRoute", "leaveVoice", "destroyVoiceOnUnmount", "releaseMappedCallPorts", "sendSignal", "broadcast", "broadcastOn", "announceHangup"]);
const functions = tree.statements.filter(node => ts.isFunctionDeclaration(node) && node.name && names.has(node.name.text));
assert.ok(functions.length >= 5, "exercise actual App RTC handlers and continuations");
const production = ts.transpileModule(functions.map(node => node.getFullText(tree)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

// A second extraction with the REAL presence-key functions: `${server}:${channel}` coercion is the
// boundary malformed room ids used to cross, so it must not be stubbed here.
const presenceNames = new Set(["handleCallSignal", "roomKey", "roomMembers", "recordPresence", "dropPresence"]);
const presenceProduction = ts.transpileModule(
  tree.statements.filter(node => ts.isFunctionDeclaration(node) && node.name && presenceNames.has(node.name.text)).map(node => node.getFullText(tree)).join("\n"),
  { compilerOptions: { target: ts.ScriptTarget.ES2022 } },
).outputText;
function presenceFixture(mode: "legacy" | "v2") {
  return new Function("legacyCallSignalAdmission", "legacyCallSignalKind", "isCurrentVoiceRoom", "mergePeerState", "heartbeatRecovery", `
    let callSignalMode = ${JSON.stringify(mode)}, inCall = true, callServer = 1, callChannel = "7", callSelfId = "b".repeat(64);
    let voiceRooms = {}, peerMeta = {}, myVideo = null;
    const VOICE_STALE_MS = 14000, legacyExcludedKinds = new Set(), diags = [], effects = [];
    const diagRecord = event => diags.push(event), maybeNotifyRoom = () => {}, b64dec = value => atob(value);
    const callPeers = {}, callDeafened = true, synthCtx = null;
    const createPeer = id => { effects.push("create:" + id); }, recoverPeer = () => effects.push("recover");
    const removePeer = id => effects.push("remove:" + id), jukeRecv = () => effects.push("juke"), jukeAnnounceTo = () => effects.push("juke");
    ${presenceProduction}
    return {
      raw(text) { return handleCallSignal("a".repeat(64), btoa(text), 1); },
      signal(payload) { return handleCallSignal("a".repeat(64), btoa(JSON.stringify(payload)), 1); },
      seed(id) { recordPresence(1, "7", id); },
      rooms() { return JSON.parse(JSON.stringify(voiceRooms)); },
      diags, effects,
    };
  `)(legacyCallSignalAdmission, legacyCallSignalKind, isCurrentVoiceRoom, mergePeerState, heartbeatRecovery);
}

test("malformed legacy room ids cannot write or delete protected-room presence through key coercion", async () => {
  // Review regression: {"callId":["7"]} failed the string-only V2 gate, then `${server}:${channel}`
  // turned it into the protected room "1:7".
  const malformedRooms = [["7"], 7, {}, null, true];
  for (const mode of ["v2", "legacy"] as const) {
    const app = presenceFixture(mode);
    app.seed("a".repeat(64)); // presence that already exists for the protected room
    const before = app.rooms();
    for (const callId of malformedRooms) {
      for (const type of ["hello", "voice-ping", "bye", "offer"]) await app.signal({ callId, type, mic: 0 });
    }
    for (const payload of [{ type: "hello" }, { callId: "7" }, { callId: "7", type: ["hello"] }, { callId: "", type: "hello" }]) {
      await app.signal(payload);
    }
    for (const text of ["null", "42", "[\"7\"]", "\"hello\""]) await app.raw(text);
    assert.deepEqual(app.rooms(), before, `${mode}: no presence written or deleted`);
    assert.deepEqual(app.effects, [], `${mode}: no peer, juke or teardown effect`);
  }
  // Control: a well-formed legacy signal still records presence through the same real functions.
  const legacy = presenceFixture("legacy");
  await legacy.signal({ callId: "8", type: "hello", mic: 0 });
  assert.ok(legacy.rooms()["1:8"]?.["a".repeat(64)], "well-formed legacy presence is unchanged");
});

function deferred<T = void>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}

/** Real App handlers, with RTC operations that can be held across peer/call replacement. */
function fixture() {
  return new Function("MediaCaptureSession", "heartbeatRecovery", "isCurrentVoiceRoom", "mappableIcePort", "mappingAddressPolicy", "routerMappedCandidate", "shouldSignalHostCandidate", "mappedIceIsCurrent", "legacyCallSignalAdmission", "legacyCallSignalKind", "mergePeerState", "legacyCallSignalEgress", "hangupTargets", "onlineCallTargets", "rosterIdentities", `
    let locked = false, inCall = true, callServer = 1, callChannel = "7", callSelfId = "b".repeat(64);
    let callSignalMode = "legacy";
    const legacyExcludedKinds = new Set();
    const callLifecycleSession = new MediaCaptureSession();
    let activeCallLease = callLifecycleSession.begin();
    const callPeers = {}, waitingIce = {};
    let callParticipants = [], callPeerStates = {}, remoteStreams = {}, peerVideoCodec = {}, peerTransport = {};
    const calls = { attached: [], removed: [], signals: [], stats: [], local: [], remote: [], ice: [], maps: [], stopped: 0, closed: 0, mappingEnds: 0, mappingDestroy: 0, diags: [], presence: [], dropped: [], juke: [] };
    const diagRecord = (event) => calls.diags.push(event);
    // Presence and jukebox owners the legacy handler writes before/after its room check.
    const roomMembers = () => [], maybeNotifyRoom = () => {};
    const recordPresence = (...args) => calls.presence.push(args);
    const jukeAnnounceTo = (...args) => calls.juke.push(args), jukeRecv = (...args) => calls.juke.push(args);
    const capVideo = () => {};
    const mappedCallPorts = new Map(), mappingGates = [], routeGates = [], rosterGates = [];
    let callMappedRoutes = 0;
    // The call server's roster: the peer under test is a member and online. A test may hold the
    // read (rosterGates) across teardown to replay the real asynchronous hangup/broadcast path.
    const ROSTER = [{ fingerprint: "a".repeat(8), identity: "a".repeat(64) }, { fingerprint: "b".repeat(8), identity: "b".repeat(64), you: true }];
    const invoke = async (command, args) => {
      if (command === "get_members" || command === "get_online_members") {
        const gate = rosterGates[0]; if (gate) await gate;
        return command === "get_members" ? ROSTER : ["a".repeat(8)];
      }
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
    // The real sendSignal reaches this native boundary; record exactly what would be sent.
    const invokeDebugged = async (command, args) => {
      if (command !== "send_call_signal") throw new Error("unexpected debugged command");
      calls.signals.push([args.server, args.target, JSON.parse(atob(args.payload))]);
      return { value: true };
    };
    const b64enc = value => btoa(value), classifyInvokeFailure = () => "fixture";
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
      jukeReset = () => {}, stopMeters = () => {}, onDeviceChange = () => {},
      dropPresence = (...args) => calls.dropped.push(args);
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
      signalRoom(id, server, room, value) { return handleCallSignal(id, btoa(JSON.stringify({ callId: room, ...value })), server); },
      mode(value) { if (value !== undefined) callSignalMode = value; return callSignalMode; },
      holdRoster(promise) { rosterGates.push(promise); },
      broadcast(msg) { broadcast(msg); },
      // What joinVoice establishes for another call, without its media/UI side effects.
      startCall(channel, mode) {
        callSignalMode = mode; inCall = true; callServer = 1; callChannel = channel;
        callSelfId = "b".repeat(64); activeCallLease = callLifecycleSession.begin();
      },
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
  `)(MediaCaptureSession, heartbeatRecovery, isCurrentVoiceRoom, mappableIcePort, mappingAddressPolicy, routerMappedCandidate, shouldSignalHostCandidate, mappedIceIsCurrent, legacyCallSignalAdmission, legacyCallSignalKind, mergePeerState, legacyCallSignalEgress, hangupTargets, onlineCallTargets, rosterIdentities);
}

const LEGACY_KINDS = ["hello", "voice-ping", "offer", "answer", "ice", "bye", "juke", "made-up"];
const legacyBody = (type: string) => ({ type, mic: 0, inst: 0, sdp: { type: "offer", sdp: "legacy" }, candidate: { candidate: "legacy ICE" } });

test("a V2 join's room admits no legacy writer: presence, peers, SDP, ICE, juke and bye are all excluded", async () => {
  const app = fixture();
  app.mode("v2");
  for (const type of LEGACY_KINDS) await app.signal(MEMBER, legacyBody(type));
  assert.equal(app.current(MEMBER), undefined, "no legacy path may create a peer in the V2 room");
  assert.deepEqual(app.calls.presence, [], "legacy hello/heartbeat cannot renew presence");
  assert.deepEqual(app.calls.dropped, [], "legacy bye cannot drop presence");
  assert.deepEqual(app.calls.removed, [], "legacy bye cannot remove anything");
  assert.deepEqual(app.calls.juke, []);
  assert.equal(app.calls.remote.length + app.calls.local.length + app.calls.ice.length, 0);
  assert.deepEqual(app.calls.diags.map((d: any) => d.fields.kind), [...LEGACY_KINDS.slice(0, 7), "other"],
    "each exclusion is diagnosed by protocol word only, never by remote text");
  // Stripped capability fields do not lower the protection: a bare heartbeat is excluded too.
  await app.signal(MEMBER, { type: "voice-ping" });
  assert.equal(app.current(MEMBER), undefined);
  // Five-second legacy pings into a V2 room cannot flood diagnostics: one record per kind per join.
  for (let i = 0; i < 20; i++) await app.signal(MEMBER, { type: "voice-ping", mic: 1 });
  assert.equal(app.calls.diags.length, LEGACY_KINDS.length);
});

test("a wholly legacy call keeps its legacy behaviour for the same signals", async () => {
  const app = fixture();
  await app.signal(MEMBER, { type: "voice-ping", mic: 1, inst: 0 });
  assert.ok(app.current(MEMBER), "a legacy heartbeat still repairs a missing peer");
  assert.equal(app.calls.presence.length, 1);
  await app.signal(MEMBER, { type: "bye" });
  assert.equal(app.current(MEMBER), undefined, "and a legacy bye still ends it");
  assert.equal(app.calls.dropped.length, 1);
  assert.deepEqual(app.calls.diags, []);
});

test("V2 mode leaves other rooms' legacy presence meaning intact", async () => {
  const app = fixture();
  app.mode("v2");
  await app.signalRoom(MEMBER, 1, "8", { type: "bye" }); // another channel on the call server
  await app.signalRoom(MEMBER, 2, "7", { type: "hello", mic: 0 }); // same channel id, other server
  assert.equal(app.calls.dropped.length, 1);
  assert.equal(app.calls.presence.length, 1);
  assert.equal(app.current(MEMBER), undefined, "other rooms never create peers in either mode");
  assert.deepEqual(app.calls.diags, []);
});

const sentTypes = (app: ReturnType<typeof fixture>) => app.calls.signals.map((s: any) => `${s[2].callId}:${s[2].type}`);

test("leaving a V2 call sends no legacy bye, even when the roster read outlives teardown", async () => {
  // Review regression: the old hangup decided egress only after its roster await, when leaveVoice
  // had already reset the mode, and a V2 join's farewell went out as a legacy bye.
  const app = fixture();
  app.mode("v2");
  const roster = deferred();
  app.holdRoster(roster.promise);
  app.destroy(); // unmount -> leaveVoice -> announceHangup, with the real sendSignal
  assert.equal(app.mode(), "legacy", "teardown reset the global mode synchronously");
  app.startCall("8", "legacy"); // and another call even began before the old roster returned
  roster.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(sentTypes(app), [], "no legacy farewell for the V2 room, by any route");
});

test("a legacy call's hangup still reaches edges and online members after its roster read", async () => {
  const app = fixture();
  app.create(MEMBER); // an established edge
  const roster = deferred();
  app.holdRoster(roster.promise);
  app.destroy();
  roster.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(sentTypes(app), ["7:bye"], "the compatible legacy farewell is unchanged");
  assert.equal(app.calls.signals[0][1], MEMBER);
});

test("a broadcast created in a V2 call never becomes a legacy send after teardown or a new call", async () => {
  const app = fixture();
  app.mode("v2");
  const roster = deferred();
  app.holdRoster(roster.promise);
  app.broadcast({ callId: "7", type: "voice-ping", mic: 0 });
  app.destroy();
  app.startCall("7", "legacy"); // even the SAME room id, now legacy
  roster.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.ok(!sentTypes(app).includes("7:voice-ping"), "the V2-era ping was decided at creation");
  // The new legacy call's own broadcast is ordinary legacy behaviour.
  app.broadcast({ callId: "7", type: "voice-ping", mic: 0 });
  await new Promise(resolve => setImmediate(resolve));
  assert.ok(sentTypes(app).includes("7:voice-ping"));
});

test("leaving resets the pinned mode, so a V2 join cannot leak into the next call", () => {
  const app = fixture();
  app.mode("v2");
  app.destroy();
  assert.equal(app.mode(), "legacy");
});

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
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(app.calls.attached.length, 0);
  // The real hangup (now exercised, not stubbed) sends this legacy call's farewell; the retired
  // peer itself may send nothing else: no ICE and no obsolete offer.
  const sent = app.calls.signals.map((s: any) => s[2].type);
  assert.deepEqual(sent.filter((type: string) => type !== "bye"), []);
  assert.deepEqual(sent, ["bye"]);
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
