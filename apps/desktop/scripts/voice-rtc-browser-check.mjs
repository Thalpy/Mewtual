// Same-desktop real-browser SDK acceptance for the shipped private RTC owner.
// Admission/currentness are controlled fixtures. This does NOT run the native
// authenticated V2 channel, capture audio, exercise WebView2, or prove a physical
// network/relay route. No microphone permission or fake-media override is used.
// Run: node scripts/voice-rtc-browser-check.mjs (EDGE_PATH/CHROME_PATH optional).
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync } from "node:fs";
import { createServer } from "node:http";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { ownedDebuggerEndpoint } from "./flow-processes.mjs";
import { ownedRtcBrowserCleanup, installRtcBrowserSignals } from "./voice-rtc-browser-process.mjs";

const desktop = fileURLToPath(new URL("..", import.meta.url));
const target = fileURLToPath(new URL("../../../target/", import.meta.url));
const sleep = ms => new Promise(yes => setTimeout(yes, ms));
const browserPath = [process.env.EDGE_PATH, process.env.CHROME_PATH,
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
].filter(Boolean).find(existsSync);
if (!browserPath) throw new Error("Set EDGE_PATH or CHROME_PATH to a Chromium browser");

const bundle = await build({ stdin: { contents: `
  export { VoiceSdpRtcOwner } from "./src/voice-sdp-rtc.ts";
  export { voiceAudioSdpShape } from "./src/voice-audio-profile.ts";
`, resolveDir: desktop, loader: "ts" }, bundle: true, format: "iife",
  globalName: "VoiceRtcCheck", write: false, target: "es2022" });
const server = createServer((_request, response) => {
  response.setHeader("Content-Type", "text/html; charset=utf-8");
  response.end("<!doctype html><script>" + bundle.outputFiles[0].text + "</script>");
});
await new Promise(yes => server.listen(0, "127.0.0.1", yes));
const url = `http://127.0.0.1:${server.address().port}/`;
mkdirSync(target, { recursive: true });
const profile = mkdtempSync(join(target, "voice-rtc-edge-"));
let browser, socket;
const cleanup = ownedRtcBrowserCleanup({ getChild: () => browser, getSocket: () => socket, server, profile, target });
const removeSignals = installRtcBrowserSignals(cleanup);
let serial = 0;
const pending = new Map();
function send(method, params = {}) {
  return new Promise((yes, no) => {
    const id = ++serial;
    const timer = setTimeout(() => { pending.delete(id); no(new Error(`CDP ${method} timed out`)); }, 40_000);
    pending.set(id, { finish(message) {
      clearTimeout(timer);
      if (message.error) no(new Error(`CDP ${method} refused`)); else yes(message);
    }, reject() { clearTimeout(timer); no(new Error("Owned RTC browser disconnected")); } });
    socket.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const reply = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (reply.result.exceptionDetails) throw new Error("RTC browser check failed: " + reply.result.exceptionDetails.text);
  return reply.result.result.value;
}

try {
  browser = spawn(browserPath, ["--headless=new", "--no-first-run", "--no-default-browser-check",
    "--disable-extensions", "--disable-background-timer-throttling", "--disable-renderer-backgrounding",
    "--disable-backgrounding-occluded-windows", "--remote-debugging-port=0", "--user-data-dir=" + profile, url,
  ], { windowsHide: true, stdio: "ignore" });
  let failure = null;
  browser.on("error", error => { failure = error; });
  browser.on("exit", () => { failure = new Error("Owned RTC browser exited"); });
  const endpoint = await ownedDebuggerEndpoint(profile, () => failure);
  let page = null;
  const deadline = Date.now() + 10_000;
  while (!page && Date.now() < deadline) {
    if (failure) throw failure;
    const pages = await (await fetch(endpoint + "/json/list", { signal: AbortSignal.timeout(2000) })).json();
    page = pages.find(item => item.type === "page" && item.url === url);
    if (!page) await sleep(50);
  }
  if (!page) throw new Error("Owned RTC page did not appear");
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((yes, no) => {
    const timer = setTimeout(() => no(new Error("RTC debugger socket did not open")), 10_000);
    socket.onopen = () => { clearTimeout(timer); yes(); };
    socket.onerror = () => { clearTimeout(timer); no(new Error("RTC debugger socket refused")); };
  });
  socket.onmessage = event => {
    const message = JSON.parse(event.data), original = pending.get(message.id);
    if (original) { pending.delete(message.id); original.finish(message); }
  };
  socket.onclose = () => { for (const original of pending.values()) original.reject(); pending.clear(); };
  await send("Runtime.enable"); await send("Page.enable");
  let ready = false;
  const bundleDeadline = Date.now() + 10_000;
  while (!ready && Date.now() < bundleDeadline) {
    try { ready = await evaluate("typeof VoiceRtcCheck === 'object'"); } catch { /* navigation replaced the blank context */ }
    if (!ready) await sleep(50);
  }
  if (!ready) throw new Error("Owned RTC bundle did not become ready");

  const result = await evaluate(`(async () => {
    const results = [], Original = RTCPeerConnection;
    for (const direction of ["sendrecv", "sendonly", "recvonly", "inactive"]) {
      const peers = []; let live = true;
      // Test-only constructor observation; SDK identities remain private in production.
      globalThis.RTCPeerConnection = class extends Original {
        constructor(configuration) { super(configuration); peers.push(this); }
        addTransceiver(kind, options) {
          const original = super.addTransceiver(kind, options);
          original.direction = direction; return original;
        }
      };
      const clock = { now: () => performance.now(), schedule: (work, ms) => setTimeout(work, ms), cancel: handle => clearTimeout(handle) };
      const low = new VoiceRtcCheck.VoiceSdpRtcOwner(true, () => live, clock);
      const high = new VoiceRtcCheck.VoiceSdpRtcOwner(false, () => live, clock);
      try {
        await low.prepareLocal(async () => performance.now() + 30_000, "offer");
        const offer = low.takeLocal(), shape = VoiceRtcCheck.voiceAudioSdpShape(new TextDecoder().decode(offer));
        await high.applyRemote(offer, "offer");
        await high.prepareLocal(async () => performance.now() + 30_000, "answer");
        const answer = high.takeLocal();
        VoiceRtcCheck.voiceAudioSdpShape(new TextDecoder().decode(answer), shape);
        await low.applyRemote(answer, "answer");
        results.push({ direction, peers: peers.length, media: peers.map(peer => peer.getTransceivers().map(original => ({
          mid: original.mid, kind: original.receiver.track.kind, current: original.currentDirection,
        }))) });
        offer.fill(0); answer.fill(0);
      } finally {
        live = false; await low.drain(); await high.drain();
        globalThis.RTCPeerConnection = Original;
      }
    }
    return results;
  })()`);
  const expected = { sendrecv: ["sendrecv", "sendrecv"], sendonly: ["sendonly", "recvonly"],
    recvonly: ["recvonly", "sendonly"], inactive: ["inactive", "inactive"] };
  assert.equal(result.length, 4);
  for (const row of result) {
    assert.equal(row.peers, 2);
    assert.deepEqual(row.media.map(media => media.length), [1, 1]);
    assert.deepEqual(row.media.map(media => media[0].mid), ["0", "0"]);
    assert.deepEqual(row.media.map(media => media[0].kind), ["audio", "audio"]);
    assert.deepEqual(row.media.map(media => media[0].current), expected[row.direction]);
  }
  console.log(JSON.stringify({ scope: "same-desktop browser SDK; controlled admission; no captured audio", results: result }, null, 2));
} finally {
  try { await cleanup(); } finally { removeSignals(); }
}
