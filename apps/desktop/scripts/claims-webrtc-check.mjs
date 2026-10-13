// Gate 5 frame claims over REAL WebRTC. Each member is its own headless Chromium process (the
// engine WebView2 ships) holding real RTCPeerConnections with a fake microphone, the negotiated
// `inst` (id 7) and `draw` (id 8) data channels, and the shipped claim session and channel wiring
// (src/draw-claims.ts and src/draw-wiring.ts, bundled as they are). Node relays the SDP and drives
// each page over plain CDP, the same stance as flow-check.mjs: no automation framework.
//
// What this proves that the unit tests cannot: Claim, Ask, Pass, refresh, expiry and contested
// claims across genuine data channels and event ordering; a genuine ICE disconnect and recovery
// (the other member's whole browser is frozen with SIGSTOP, so its traffic and consent checks stop
// for real) through `onconnectionstatechange` into the session; leave and rejoin; and the
// mixed-version case, a build with no channel 8, which must keep its voice and jam traffic while
// we send on stream 8. What it does not prove: the Studio UI and the native backend (covered by
// the compiled Studio tests and the Rust suites), or any engine other than the one it launches;
// WKWebView and WebKitGTK need the same run on macOS and Linux desktops.
//
// Usage (from apps/desktop): node scripts/claims-webrtc-check.mjs
//   CHROME_PATH=/path/to/chrome-or-msedge to choose the browser (default: Playwright's Chromium
//   under /opt/pw-browsers, then Edge on Windows). Freezing a member needs a POSIX host. Exit
//   status is non-zero if any check fails; CLAIMS_WEBRTC_REPORT=path writes the JSON record.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import process from "node:process";
import { build } from "esbuild";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const OBJ = "0123456789abcdef0123456789abcdef";
const F = (n) => String(n).repeat(2).padStart(2, "0").repeat(16);
const ID = { A: "aa".repeat(32), B: "bb".repeat(32), C: "cc".repeat(32) };
const CLAIM_BURST = 10; // CLAIM_FRAME_BUCKET_BURST

function findBrowser() {
  const candidates = [process.env.CHROME_PATH, process.env.EDGE_PATH];
  const pw = "/opt/pw-browsers";
  if (existsSync(pw)) {
    for (const d of readdirSync(pw).filter((n) => n.startsWith("chromium-")).sort().reverse()) {
      candidates.push(join(pw, d, "chrome-linux", "chrome"));
    }
  }
  candidates.push(
    "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
  );
  const found = candidates.filter(Boolean).find((p) => existsSync(p));
  if (!found) throw new Error("no Chromium-family browser found; set CHROME_PATH");
  return found;
}

// --- the page --------------------------------------------------------------------------------

async function bundle() {
  const out = await build({
    stdin: {
      contents: `
        export { DrawClaimsSession, snapshotClaims, frameClaimState, DRAW_CHANNEL_ID } from "./src/draw-claims.ts";
        export { openDrawEdge, drawConnectionState } from "./src/draw-wiring.ts";
      `,
      resolveDir: process.cwd(),
      loader: "ts",
    },
    bundle: true,
    format: "iife",
    globalName: "Claims",
    write: false,
    target: "es2022",
  });
  return out.outputFiles[0].text;
}

// The page's own glue: peer connections, the fake microphone and `inst`, exactly as an edge has
// them, plus read-outs for the check. Everything claim-related goes through the shipped modules.
const PAGE = String.raw`
const H = (window.harness = {
  claims: null, self: "", skew: 0, peers: {}, notices: [], recv: [], states: {}, inst: {}, timer: 0,
  start(self) {
    this.self = self;
    this.claims = new Claims.DrawClaimsSession({
      self, now: () => performance.now() + H.skew,
      randomHex: (n) => Array.from(crypto.getRandomValues(new Uint8Array(n)), (b) => b.toString(16).padStart(2, "0")).join(""),
    });
    this.claims.onnotice = (n) => H.notices.push(n);
    const receive = this.claims.receive.bind(this.claims);
    this.claims.receive = (ch, raw) => { const r = receive(ch, raw); H.recv.push({ at: performance.now(), peer: ch.source.slice(0, 2), ok: r.ok, why: r.ok ? r.action : r.reason }); return r; };
    clearInterval(this.timer);
    this.timer = setInterval(() => H.claims.tick(), 1000); // the app's 1 s claim timer
  },
  leaveCall() { for (const p of Object.values(this.peers)) p.pc.close(); this.peers = {}; this.claims.dispose(); },
  async mic() { if (!this.stream) this.stream = await navigator.mediaDevices.getUserMedia({ audio: true }); return this.stream; },
  async makePeer(peer, draw) {
    const pc = new RTCPeerConnection({ iceServers: [] });
    for (const t of (await this.mic()).getAudioTracks()) pc.addTrack(t, this.stream);
    const inst = pc.createDataChannel("inst", { negotiated: true, id: 7, ordered: true });
    this.inst[peer] = { got: 0 };
    inst.onmessage = () => { this.inst[peer].got += 1; };
    const p = { pc, inst, edge: null, draw };
    this.states[peer] = [];
    if (draw) p.edge = Claims.openDrawEdge(pc, this.claims, peer, { live: () => H.peers[peer] === p, changed: () => {} });
    pc.ondatachannel = (e) => this.states[peer].push("ondatachannel:" + e.channel.label);
    pc.onconnectionstatechange = () => {
      this.states[peer].push(pc.connectionState);
      if (draw && H.peers[peer] === p) Claims.drawConnectionState(this.claims, peer, pc.connectionState, p.edge, () => {});
    };
    this.peers[peer] = p;
    return p;
  },
  async gathered(pc) {
    if (pc.iceGatheringState === "complete") return;
    await new Promise((r) => { pc.onicegatheringstatechange = () => pc.iceGatheringState === "complete" && r(); });
  },
  async offer(peer, draw) {
    const { pc } = await this.makePeer(peer, draw);
    await pc.setLocalDescription(await pc.createOffer());
    await this.gathered(pc);
    return pc.localDescription.sdp;
  },
  async answer(peer, draw, sdp) {
    const { pc } = await this.makePeer(peer, draw);
    await pc.setRemoteDescription({ type: "offer", sdp });
    await pc.setLocalDescription(await pc.createAnswer());
    await this.gathered(pc);
    return pc.localDescription.sdp;
  },
  async accept(peer, sdp) { await this.peers[peer].pc.setRemoteDescription({ type: "answer", sdp }); },
  ping(peer, n) { const ch = this.peers[peer].inst; for (let i = 0; i < n; i++) ch.send("ping"); },
  async dcStats(peer) {
    const out = [];
    (await this.peers[peer].pc.getStats()).forEach((s) => {
      if (s.type === "data-channel") out.push({ id: s.dataChannelIdentifier, label: s.label, state: s.state, sent: s.messagesSent, received: s.messagesReceived });
    });
    return out;
  },
  async audioIn(peer) {
    let packets = 0;
    (await this.peers[peer].pc.getStats()).forEach((s) => { if (s.type === "inbound-rtp" && s.kind === "audio") packets += s.packetsReceived; });
    return packets;
  },
  view() {
    const snap = Claims.snapshotClaims(this.claims);
    return {
      entries: snap ? snap.entries.map((e) => [e.peer.slice(0, 2), e.fr.slice(0, 2), e.kind, e.secondsLeft]) : null,
      local: snap ? { claim: snap.local.claim?.fr.slice(0, 2) ?? null, ask: snap.local.ask?.fr.slice(0, 2) ?? null } : null,
      asksOnMine: snap ? snap.asksOnMine.map((e) => e.peer.slice(0, 2)) : null,
      notices: this.notices.map((n) => [n.kind, n.from.slice(0, 2), n.fr.slice(0, 2)]),
      conn: Object.fromEntries(Object.entries(this.peers).map(([k, p]) => [k.slice(0, 2), p.pc.connectionState])),
      drawDc: Object.fromEntries(Object.entries(this.peers).map(([k, p]) => [k.slice(0, 2), p.edge ? p.edge.dc.readyState : "none"])),
    };
  },
  contested(obj, fr) { return Claims.frameClaimState(Claims.snapshotClaims(this.claims), obj, fr).contested; },
});
`;

function servePage(script) {
  const html = `<!doctype html><meta charset="utf-8"><title>claims</title><script>${script}</script><script>${PAGE}</script>`;
  return new Promise((resolve) => {
    const server = createServer((_req, res) => {
      res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      res.end(html);
    });
    server.listen(0, "127.0.0.1", () => resolve(server));
  });
}

// --- a member: one browser process and its page, over CDP ------------------------------------

class Member {
  constructor(name, browserPath, port) {
    this.name = name;
    this.port = port;
    this.dir = mkdtempSync(join(tmpdir(), `claims-${name}-`));
    this.proc = spawn(browserPath, [
      "--headless=new", "--no-sandbox", "--no-first-run", "--no-default-browser-check", "--disable-gpu",
      `--remote-debugging-port=${port}`, `--user-data-dir=${this.dir}`,
      "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream",
      // Host candidates instead of mDNS names: the members are separate processes on one host.
      "--disable-features=WebRtcHideLocalIpsWithMdns",
      "--disable-background-timer-throttling", "--disable-renderer-backgrounding", "--disable-backgrounding-occluded-windows",
      "--disable-background-networking", "--disable-component-update", "--disable-sync", "--no-pings",
      "about:blank",
    ], { stdio: "ignore", detached: process.platform !== "win32" });
    this.id = 0;
    this.waiting = new Map();
  }
  async connect(url) {
    let ws;
    for (let i = 0; i < 100 && !ws; i++) {
      try {
        const list = await (await fetch(`http://127.0.0.1:${this.port}/json/list`)).json();
        ws = list.find((t) => t.type === "page")?.webSocketDebuggerUrl;
      } catch { /* not up yet */ }
      if (!ws) await sleep(100);
    }
    if (!ws) throw new Error(`${this.name}: no CDP page target`);
    this.ws = new WebSocket(ws);
    await new Promise((r, j) => { this.ws.onopen = r; this.ws.onerror = j; });
    this.ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      const w = this.waiting.get(msg.id);
      if (w) { this.waiting.delete(msg.id); msg.error ? w.reject(new Error(msg.error.message)) : w.resolve(msg.result); }
    };
    await this.cdp("Page.enable");
    await this.cdp("Page.navigate", { url });
    for (let i = 0; i < 100; i++) {
      if (await this.eval("!!window.harness").catch(() => false)) return;
      await sleep(100);
    }
    throw new Error(`${this.name}: page never loaded`);
  }
  cdp(method, params = {}) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve, reject) => {
      this.waiting.set(id, { resolve, reject });
      setTimeout(() => { if (this.waiting.delete(id)) reject(new Error(`${this.name}: ${method} timed out`)); }, 30_000);
    });
  }
  async eval(expression) {
    const r = await this.cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(`${this.name}: ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`);
    return r.result.value;
  }
  freeze() { process.kill(-this.proc.pid, "SIGSTOP"); }
  thaw() { process.kill(-this.proc.pid, "SIGCONT"); }
  stop() {
    try { this.ws?.close(); } catch { /* closed */ }
    try { process.kill(process.platform === "win32" ? this.proc.pid : -this.proc.pid, "SIGKILL"); } catch { /* gone */ }
    try { rmSync(this.dir, { recursive: true, force: true }); } catch { /* still held */ }
  }
}

async function link(x, xId, y, yId, { xDraw = true, yDraw = true } = {}) {
  const offer = await x.eval(`harness.offer(${JSON.stringify(yId)}, ${xDraw})`);
  const answer = await y.eval(`harness.answer(${JSON.stringify(xId)}, ${yDraw}, ${JSON.stringify(offer)})`);
  await x.eval(`harness.accept(${JSON.stringify(yId)}, ${JSON.stringify(answer)})`);
}

// --- checks ------------------------------------------------------------------------------------

const results = [];
function record(name, ok, detail) {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail === undefined ? "" : `  ${JSON.stringify(detail)}`}`);
}
async function until(name, m, predicate, timeoutMs = 15_000) {
  const t0 = Date.now();
  let v;
  while (Date.now() - t0 < timeoutMs) {
    v = await m.eval("harness.view()");
    if (predicate(v)) { record(name, true, { ms: Date.now() - t0 }); return v; }
    await sleep(100);
  }
  record(name, false, v);
  return v;
}
const has = (v, peer, fr, kind) => !!v.entries?.some((e) => e[0] === peer && e[1] === fr && e[2] === kind);

async function main() {
  const browser = findBrowser();
  const server = await servePage(await bundle());
  const url = `http://localhost:${server.address().port}/`;
  const m = { A: new Member("A", browser, 9441), B: new Member("B", browser, 9442), C: new Member("C", browser, 9443) };
  console.log(`browser: ${browser}`);
  try {
    for (const [k, mem] of Object.entries(m)) {
      await mem.connect(url);
      await mem.eval(`harness.start(${JSON.stringify(ID[k])})`);
    }
    console.log(`user agent: ${await m.A.eval("navigator.userAgent")}`);
    const { A, B, C } = m;
    const a = "aa", b = "bb", c = "cc";

    // 1. Two current builds connect: voice, inst and draw.
    await link(A, ID.A, B, ID.B);
    await until("A and B connect with the draw channel open on both sides", A, (v) => v.conn[b] === "connected" && v.drawDc[b] === "open");
    await until("B sees the edge open too", B, (v) => v.conn[a] === "connected" && v.drawDc[a] === "open");

    // 2. Claim, Ask, Pass.
    await A.eval(`harness.claims.claim(${JSON.stringify(OBJ)}, ${JSON.stringify(F(1))})`);
    await until("B sees A's claim with a fresh 90 s countdown", B, (v) => v.entries.some((e) => e[0] === a && e[1] === "11" && e[2] === "claim" && e[3] >= 89));
    await B.eval(`harness.claims.ask(${JSON.stringify(OBJ)}, ${JSON.stringify(F(1))})`);
    await until("A gets B's Ask on the frame it holds", A, (v) => v.asksOnMine.includes(b));
    await A.eval(`harness.claims.pass(${JSON.stringify(OBJ)}, ${JSON.stringify(F(1))})`);
    await until("Pass hands the frame to B, with a notice", B, (v) => v.local.claim === "11" && v.local.ask === null && v.notices.some((n) => n[1] === a));
    await until("A now sees B holding it", A, (v) => has(v, b, "11", "claim") && v.local.claim === null);

    // 3. A contested frame stays visible on both sides; nothing resolves it.
    await A.eval(`harness.claims.claim(${JSON.stringify(OBJ)}, ${JSON.stringify(F(1))})`);
    await until("A's explicit claim reaches B", B, (v) => has(v, a, "11", "claim"));
    const contested = [await A.eval(`harness.contested(${JSON.stringify(OBJ)}, ${JSON.stringify(F(1))})`), await B.eval(`harness.contested(${JSON.stringify(OBJ)}, ${JSON.stringify(F(1))})`)];
    record("the frame shows as contested on both sides", contested[0] && contested[1], contested);

    // 4. Refresh and expiry on each receiver's own clock.
    await B.eval("harness.skew += 20000; harness.claims.tick()");
    await until("B's countdown for A's claim runs on B's clock", B, (v) => v.entries.some((e) => e[0] === a && e[3] <= 71));
    await A.eval("harness.skew += 30000");
    await until("A's 30 s refresh restarts B's countdown", B, (v) => v.entries.some((e) => e[0] === a && e[3] >= 89), 5_000);
    await A.eval(`harness.claims.release()`);
    await until("A's release clears its claim at B", B, (v) => !has(v, a, "11", "claim"));
    await A.eval(`harness.claims.claim(${JSON.stringify(OBJ)}, ${JSON.stringify(F(2))})`);
    await until("A claims frame 22", B, (v) => has(v, a, "22", "claim"));
    await A.eval("clearInterval(harness.timer)"); // A stops refreshing (a crashed or wedged sender)
    await B.eval("harness.skew += 90500; harness.claims.tick()");
    await until("B expires A's unrefreshed claim at 90 s", B, (v) => !has(v, a, "22", "claim"), 3_000);
    await A.eval("harness.timer = setInterval(() => harness.claims.tick(), 1000); harness.claims.release()");
    await B.eval(`harness.claims.release()`);

    // 5. A genuine ICE disconnect and recovery: A's whole browser is frozen. B must hide A at once,
    //    hold whatever A sends while B still sees the link down, and on recovery apply it in order,
    //    so the Pass A sends straight after waking lands.
    await A.eval(`harness.claims.claim(${JSON.stringify(OBJ)}, ${JSON.stringify(F(3))})`);
    await until("A claims frame 33", B, (v) => has(v, a, "33", "claim"));
    await B.eval(`harness.claims.ask(${JSON.stringify(OBJ)}, ${JSON.stringify(F(3))})`);
    await until("A has B's Ask on frame 33", A, (v) => v.asksOnMine.includes(b));
    const recvMark = (await B.eval("harness.recv.length"));
    A.freeze();
    const down = await until("B sees A's connection leave connected while A is frozen", B, (v) => v.conn[a] === "disconnected" || v.conn[a] === "failed", 20_000);
    record("B hides the unreachable A's claim at once", !has(down, a, "33", "claim"), down.entries);
    A.thaw();
    await A.eval(`harness.claims.pass(${JSON.stringify(OBJ)}, ${JSON.stringify(F(3))})`);
    const after = await until("after recovery the Pass lands: B holds frame 33", B, (v) => v.conn[a] === "connected" && v.local.claim === "33", 30_000);
    record("B's notice names A", after.notices.some((n) => n[1] === a && n[2] === "33"), after.notices);
    const recv = (await B.eval("harness.recv")).slice(recvMark);
    // Either order is legitimate: "held" when the Pass arrived before B saw the link recover
    // (applied on recovery), "release" when it arrived after. Both must end with B holding it.
    record("B's deliveries from A around the gap (held = arrived while B still saw A unreachable)", true, recv.map((r) => r.why));
    const statesB = await B.eval(`harness.states[${JSON.stringify(ID.A)}]`);
    record("B's connection states for A", true, statesB);

    // 6. Leave and rejoin: B leaves the call; A drops B's claims; B rejoins and claims again.
    await until("A sees B's claim on 33 before B leaves", A, (v) => has(v, b, "33", "claim"));
    await B.eval("harness.leaveCall()");
    await until("B leaving drops its claims at A", A, (v) => !v.entries.some((e) => e[0] === b), 20_000);
    await B.eval(`harness.start(${JSON.stringify(ID.B)})`);
    await A.eval(`delete harness.peers[${JSON.stringify(ID.B)}]`);
    await link(A, ID.A, B, ID.B);
    await until("A and B reconnect", A, (v) => v.conn[b] === "connected" && v.drawDc[b] === "open");
    await B.eval(`harness.claims.claim(${JSON.stringify(OBJ)}, ${JSON.stringify(F(4))})`);
    await until("B's claim after rejoining reaches A", A, (v) => has(v, b, "44", "claim"));
    await A.eval(`harness.claims.claim(${JSON.stringify(OBJ)}, ${JSON.stringify(F(5))})`);
    await until("A's claim reaches the rejoined B", B, (v) => has(v, a, "55", "claim"));

    // 7. Mixed versions: C is an older build with no channel 8. A sends claims on stream 8 to it;
    //    C's voice and jam traffic, and the connection, must carry on unaffected.
    await link(A, ID.A, C, ID.C, { yDraw: false });
    await until("A and the old build C connect", A, (v) => v.conn[c] === "connected");
    await sleep(1_500);
    await A.eval(`harness.ping(${JSON.stringify(ID.C)}, 20)`);
    await C.eval(`harness.ping(${JSON.stringify(ID.A)}, 20)`);
    await sleep(500);
    const instBefore = [await C.eval(`harness.inst[${JSON.stringify(ID.A)}].got`), await A.eval(`harness.inst[${JSON.stringify(ID.C)}].got`)];
    const audioBefore = [await C.eval(`harness.audioIn(${JSON.stringify(ID.A)})`), await A.eval(`harness.audioIn(${JSON.stringify(ID.C)})`)];
    const aDrawToC = (await A.eval("harness.view()")).drawDc[c];
    record("A's draw channel toward the old build opens on A's side (negotiated)", aDrawToC === "open", aDrawToC);
    for (let i = 0; i < 60; i++) {
      await A.eval(`harness.claims.claim(${JSON.stringify(OBJ)}, ${JSON.stringify(((i % 9) + 1).toString(16).repeat(32))}); harness.claims.ask(${JSON.stringify(OBJ)}, ${JSON.stringify(F(9))})`);
    }
    await sleep(3_000);
    await A.eval(`harness.ping(${JSON.stringify(ID.C)}, 20)`);
    await C.eval(`harness.ping(${JSON.stringify(ID.A)}, 20)`);
    await sleep(5_000);
    const instAfter = [await C.eval(`harness.inst[${JSON.stringify(ID.A)}].got`), await A.eval(`harness.inst[${JSON.stringify(ID.C)}].got`)];
    const audioAfter = [await C.eval(`harness.audioIn(${JSON.stringify(ID.A)})`), await A.eval(`harness.audioIn(${JSON.stringify(ID.C)})`)];
    const vA = await A.eval("harness.view()");
    const vC = await C.eval("harness.view()");
    const statesC = await C.eval(`harness.states[${JSON.stringify(ID.A)}]`);
    const drawStatsA = (await A.eval(`harness.dcStats(${JSON.stringify(ID.C)})`)).find((d) => d.id === 8);
    const statsC = await C.eval(`harness.dcStats(${JSON.stringify(ID.A)})`);
    // 120 claim/ask changes; the per-edge pacing (burst 10, then 2/s) coalesces them, so the engine
    // sends a few dozen frames, every one of which lands on a stream the old build never opened.
    record("A really sent claim frames on stream 8 toward the old build (engine stats)", (drawStatsA?.sent ?? 0) >= CLAIM_BURST, drawStatsA);
    record("the old build has no channel 8 at all", !statsC.some((d) => d.id === 8), statsC);
    record("jam (inst, id 7) still flows both ways after stream-8 traffic", instAfter[0] === instBefore[0] + 20 && instAfter[1] === instBefore[1] + 20, { instBefore, instAfter });
    record("voice still flows both ways", audioAfter[0] > audioBefore[0] && audioAfter[1] > audioBefore[1], { audioBefore, audioAfter });
    record("the A-C connection stayed connected", vA.conn[c] === "connected" && vC.conn[a] === "connected", { A: vA.conn[c], C: vC.conn[a] });
    record("the old build saw no new channel and no state churn", !statesC.some((s) => s.startsWith("ondatachannel")) && statesC.filter((s) => s !== "connecting" && s !== "connected").length === 0, statesC);
    record("A's draw channel toward the old build is still open (no stream reset)", vA.drawDc[c] === "open", vA.drawDc[c]);
    record("A's other edge (to B) was unaffected", vA.conn[b] === "connected" && vA.drawDc[b] === "open", { conn: vA.conn[b], draw: vA.drawDc[b] });
  } finally {
    for (const mem of Object.values(m)) mem.stop();
    server.close();
  }
  const failed = results.filter((r) => !r.ok);
  if (process.env.CLAIMS_WEBRTC_REPORT) {
    writeFileSync(process.env.CLAIMS_WEBRTC_REPORT, JSON.stringify({ browser, at: new Date().toISOString(), results }, null, 2));
  }
  console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
  process.exit(failed.length ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
