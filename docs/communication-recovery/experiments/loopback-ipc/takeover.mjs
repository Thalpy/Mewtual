// Independent keyless responder for the exact old listener port. It has no bootstrap DTO,
// native handle or crypto key. This is an actual local port replacement, not a production API.
import { createServer } from "node:http";

export function keylessTakeover(origin) {
  const url = new URL(origin);
  if (url.protocol !== "http:" || url.hostname !== "127.0.0.1" || !url.port || url.username || url.password || url.pathname !== "/" || url.search || url.hash) throw new Error("unexpected takeover endpoint");
  const expectedOrigin = "http://tauri.localhost", maximum = 8192 + 16;
  const stats = { posts: 0, options: 0, ciphertextBytes: 0, plaintextObserved: false, listenerError: false };
  const server = createServer({ maxHeaderSize: 4096, headersTimeout: 2000, requestTimeout: 2000, keepAliveTimeout: 1 }, (request, response) => {
    const headers = { "connection": "close", "cache-control": "no-store", "access-control-allow-origin": expectedOrigin };
    if (request.url !== "/frame" || request.headers.host !== url.host || request.headers.origin !== expectedOrigin) {
      response.writeHead(403, { ...headers, "content-length": "0" }); response.end(); return;
    }
    if (request.method === "OPTIONS") {
      stats.options++;
      response.writeHead(204, { ...headers, "content-length": "0", "access-control-allow-methods": "POST", "access-control-allow-headers": "x-probe-admission", "access-control-allow-private-network": "true" }); response.end(); return;
    }
    const admission = request.headers["x-probe-admission"];
    if (request.method !== "POST" || stats.posts !== 0 || request.headers["content-length"] !== String(maximum) || typeof admission !== "string" || !/^[0-9a-f]{144}$/.test(admission) || request.headers["transfer-encoding"]) {
      response.writeHead(403, { ...headers, "content-length": "0" }); response.end(); return;
    }
    stats.posts++;
    const body = Buffer.alloc(maximum); let used = 0;
    request.on("data", chunk => {
      if (chunk.length > body.length - used) { request.destroy(); return; }
      body.set(chunk, used); used += chunk.length;
    });
    request.on("close", () => body.fill(0));
    request.on("error", () => body.fill(0));
    request.on("end", () => {
      try {
        if (used !== maximum) { request.destroy(); return; }
        stats.ciphertextBytes = used;
        const pattern = Buffer.alloc(8192); for (let i = 0; i < pattern.length; i++) pattern[i] = (i * 17 + 3) & 255;
        stats.plaintextObserved = body.includes(pattern); pattern.fill(0);
        // A syntactically plausible reflected header with a zero MAC. The real browser must
        // receive this HTTP200 reply and reject its authentication, without trusting CORS alone.
        const fake = Buffer.alloc(72); Buffer.from(admission.slice(0, 80), "hex").copy(fake);
        fake[5] = 1; fake.writeBigUInt64BE((1n << 64n) - 1n, 8); fake.writeUInt32BE(16, 32);
        response.writeHead(200, { ...headers, "content-type": "application/octet-stream", "content-length": "16",
          "access-control-expose-headers": "x-probe-admission", "x-probe-admission": fake.toString("hex") });
        response.end(Buffer.alloc(16));
      } finally { body.fill(0); }
    });
  });
  server.maxConnections = 2;
  server.dropMaxConnection = true;
  server.on("connection", socket => {
    // An inactivity timeout would be renewed forever by dribbling input. This is the original
    // acceptance deadline and stays fixed through partial headers, body and response handling.
    const deadline = setTimeout(() => socket.destroy(), 2000);
    socket.once("close", () => clearTimeout(deadline));
  });
  const stop = new AbortController();
  let resolveClosed, rejectReady, readyPublished = false;
  const closed = new Promise(resolve => { resolveClosed = resolve; });
  server.once("close", () => { resolveClosed(); if (!readyPublished) rejectReady(new Error("keyless listener closed before readiness")); });
  const ready = new Promise((resolve, reject) => {
    rejectReady = reject;
    server.on("error", error => { stats.listenerError = true; reject(error); if (!server.listening) resolveClosed(); });
    server.listen({ host: "127.0.0.1", port: Number(url.port), exclusive: true, signal: stop.signal }, () => { readyPublished = true; resolve(); });
  });
  let closing;
  return {
    ready,
    snapshot() { return { ...stats }; },
    close() {
      if (!closing) { stop.abort(); server.closeAllConnections(); closing = closed; }
      return closing;
    },
  };
}
