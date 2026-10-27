// Disposable packaged-WebView experiment. Never opens an account or carries application data.
// Only metadata escapes this closure. The fixture bootstrap key must not enter runner logs.
import { createCodec, MAX_PLAIN } from "./framing.mjs";
window.__probe = (async () => {
  const surface = window.__PROBE_SURFACE__ ?? "main";
  const invoke = (...args) => window.__TAURI_INTERNALS__.invoke(...args);
  if (surface !== "main") {
    try { await invoke("bootstrap"); return { surface, denied: false }; }
    catch { return { surface, denied: true, bridge: !!window.__TAURI_INTERNALS__ }; }
  }
  const packaged = location.origin === "http://tauri.localhost";
  if (!packaged || !window.isSecureContext || !crypto.subtle) throw new Error("expected the real secure packaged Windows origin");
  // Consume one real custom-protocol response, then inject reply loss. Tauri's unmodified
  // transport must use its actual postMessage fallback and spend the second native allowance.
  const realFetch = window.fetch.bind(window);
  const bootstrapUrl = window.__TAURI_INTERNALS__.convertFileSrc("bootstrap", "ipc");
  let lost = false;
  window.fetch = async (...args) => {
    const response = await realFetch(...args);
    if (!lost && String(args[0]) === bootstrapUrl && response.headers.get("Tauri-Response") === "ok") {
      lost = true;
      const withheld = new Uint8Array(await response.arrayBuffer()); // fixed bootstrap only
      withheld.fill(0);
      throw new Error("injected bootstrap reply loss");
    }
    return response;
  };
  let bootstrap;
  try { bootstrap = await invoke("bootstrap"); }
  catch (error) {
    if (String(error).includes("allowance exhausted")) return { surface, refreshedRefusesBootstrap: true, packaged };
    throw error;
  } finally { window.fetch = realFetch; }
  const seed = Uint8Array.from(bootstrap.secret); bootstrap.secret.fill(0);
  const codec = await createCodec(seed, bootstrap.grant);
  const origin = new URL(bootstrap.origin), blocked = new URL(bootstrap.blocked_origin);
  for (const url of [origin, blocked]) {
    if (url.protocol !== "http:" || url.hostname !== "127.0.0.1" || !url.port || url.username || url.password || url.pathname !== "/") throw new Error("invalid fixture listener");
  }
  let exhausted = false;
  try { await invoke("bootstrap"); } catch (error) { exhausted = String(error).includes("allowance exhausted"); }
  const get = url => realFetch(url, { method: "GET", headers: { "x-probe": "1" }, credentials: "omit", redirect: "error", cache: "no-store", referrerPolicy: "no-referrer", signal: AbortSignal.timeout(5000) });
  const reply = await get(new URL("/ok", origin));
  // Even this unencrypted fixed test response is read incrementally under a fixed bound.
  // This is not authenticated framing, which remains a separate experiment slice.
  if (reply.headers.get("content-length") !== "17") throw new Error("unexpected fixed response length");
  const reader = reply.body.getReader(), bytes = new Uint8Array(17);
  let used = 0;
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      if (used + value.length > bytes.length) throw new Error("fixed response exceeds its bound");
      bytes.set(value, used); used += value.length;
    }
  } finally { await reader.cancel(); reader.releaseLock(); }
  const healthy = used === 17 && new TextDecoder().decode(bytes) === "fixed probe bytes";
  const cspBlocked = await get(new URL("/ok", blocked)).then(() => false, () => true);
  const corsBlocked = await get(new URL("/wrong-cors", origin)).then(() => false, () => true);
  const post = request => realFetch(new URL("/frame", origin), { method: "POST", headers: { "x-probe-admission": request.admission }, body: request.cipher,
    credentials: "omit", redirect: "error", cache: "no-store", referrerPolicy: "no-referrer", signal: AbortSignal.timeout(5000) });
  const pattern = new Uint8Array(MAX_PLAIN); for (let i = 0; i < pattern.length; i++) pattern[i] = (i * 17 + 3) & 255;
  const id = new Uint8Array(16).fill(9);
  let maximum, badHeader, badBody, next, opened;
  let encryptedMaximum, headerTamperRefused, bodyTamperRefused, replayRefused, continueTakeover = false;
  try {
    maximum = await codec.request(pattern, id);
    opened = await codec.open(await post(maximum), maximum);
    encryptedMaximum = opened.length === pattern.length && opened.every((byte, i) => byte === pattern[i]); opened.fill(0); opened = null;
    badHeader = await codec.request(pattern, id);
    headerTamperRefused = await post({ ...badHeader, admission: badHeader.admission.slice(0, -1) + (badHeader.admission.endsWith("0") ? "1" : "0") }).then(() => false, () => true);
    badBody = await codec.request(pattern, id); badBody.cipher[0] ^= 1;
    bodyTamperRefused = await post(badBody).then(() => false, () => true);
    next = await codec.request(pattern, id);
    opened = await codec.open(await post(next), next); opened.fill(0); opened = null;
    // Deliberately replay the SAME ciphertext, never re-encrypt using its nonce. Native refuses
    // the spent sequence before another body allocation, even though this is a valid old frame.
    replayRefused = await post(next).then(() => false, () => true);
    continueTakeover = true;
  } finally {
    pattern.fill(0); opened?.fill(0); if (!continueTakeover) await codec.close();
    for (const request of [maximum, badHeader, badBody, next]) request?.cipher.fill(0);
  }
  // The runner never receives a seed/key/plaintext. After native has really dropped BOTH
  // listeners, a separate keyless Node process binds the exact CSP-permitted old port.
  window.__probeTakeover = async () => {
    window.__probeTakeover = null; // finite once-only continuation, not a payload API
    const plain = new Uint8Array(MAX_PLAIN); for (let i = 0; i < plain.length; i++) plain[i] = (i * 17 + 3) & 255;
    let request, result;
    try {
      request = await codec.request(plain, id);
      const response = await post(request);
      if (response.status !== 200) throw new Error("keyless fixture did not return its response");
      const before = codec.snapshot().headerRefusals;
      try { result = await codec.open(response, request); return { refused: false }; }
      catch { return { refused: true, headerAuthenticationRefused: codec.snapshot().headerRefusals === before + 1 }; }
    } finally { plain.fill(0); result?.fill(0); request?.cipher.fill(0); await codec.close(); }
  };
  document.getElementById("phase").textContent = "Encrypted fixture checks complete. No application voice or media was used.";
  return { surface, packaged, secure: isSecureContext, lost, exhausted, healthy, cspBlocked, corsBlocked,
    encryptedMaximum, headerTamperRefused, bodyTamperRefused, replayRefused };
})();
window.__probe.then(result => { window.__probeResult = result; document.documentElement.dataset.probeReady = "yes"; },
  error => { window.__probeError = String(error).slice(0, 200); document.documentElement.dataset.probeReady = "failed"; });
