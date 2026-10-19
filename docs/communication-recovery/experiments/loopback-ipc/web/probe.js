// Disposable packaged-WebView experiment. Never opens an account or carries application data.
// Only metadata escapes this closure. The fixture bootstrap key must not enter runner logs.
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
  bootstrap.secret.fill(0); // no cryptographic payload operation is implemented in this slice
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
  document.getElementById("phase").textContent = "Packaged-origin checks complete. No application voice or media was used.";
  return { surface, packaged, secure: isSecureContext, lost, exhausted, healthy, cspBlocked, corsBlocked };
})();
window.__probe.then(result => { window.__probeResult = result; document.documentElement.dataset.probeReady = "yes"; },
  error => { window.__probeError = String(error).slice(0, 200); document.documentElement.dataset.probeReady = "failed"; });
