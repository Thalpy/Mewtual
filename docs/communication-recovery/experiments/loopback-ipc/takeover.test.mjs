import assert from "node:assert/strict";
import test from "node:test";
import { createServer, request } from "node:http";
import { createConnection } from "node:net";
import { keylessTakeover } from "./takeover.mjs";
const bind = server => new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", () => resolve(`http://127.0.0.1:${server.address().port}`)); });
const close = server => new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
async function vacantOrigin() { const server = createServer(); const origin = await bind(server); await close(server); return origin; }
function post(origin, plaintext) {
  const header = Buffer.alloc(72); header.write("MWIP"); header[4] = 1; header[6] = 1; header.writeBigUInt64BE(1n, 8); header.fill(9, 16, 32); header.writeUInt32BE(8208, 32);
  const body = Buffer.alloc(8208); if (plaintext) for (let i = 0; i < 8192; i++) body[i] = (i * 17 + 3) & 255;
  return new Promise((resolve, reject) => {
    const outgoing = request(`${origin}/frame`, { method: "POST", headers: { origin: "http://tauri.localhost", "content-length": "8208", "x-probe-admission": header.toString("hex") } }, response => {
      const chunks = []; response.on("data", chunk => chunks.push(chunk)); response.on("end", () => resolve({ status: response.statusCode, headers: response.headers, body: Buffer.concat(chunks) }));
    });
    outgoing.on("error", reject); outgoing.end(body);
  });
}
test("keyless exact-port responder detects plaintext and emits only a syntactic unauthenticated reply", { timeout: 5000 }, async () => {
  for (const plaintext of [false, true]) {
    const origin = await vacantOrigin(), keyless = keylessTakeover(origin);
    try {
      await keyless.ready;
      // snapshot intentionally contains only counters, never captured bytes or a fixture key.
      const reply = await post(origin, plaintext);
      assert.equal(reply.status, 200); assert.equal(reply.body.length, 16);
      assert.equal(reply.headers["x-probe-admission"].length, 144);
      assert.ok(reply.headers["x-probe-admission"].endsWith("0".repeat(64)));
      assert.equal(Buffer.from(reply.headers["x-probe-admission"], "hex").readBigUInt64BE(8), (1n << 64n) - 1n, "forgery must pass the current reply floor");
      assert.deepEqual(keyless.snapshot(), { posts: 1, options: 0, ciphertextBytes: 8208, plaintextObserved: plaintext, listenerError: false });
      assert.equal((await post(origin, plaintext)).status, 403, "only one body capture is admitted");
    } finally { await keyless.close(); }
  }
});
test("a failed exact bind never replaces or closes the occupied listener", { timeout: 5000 }, async () => {
  const owner = createServer((_, response) => response.end("owner")), origin = await bind(owner);
  const keyless = keylessTakeover(origin);
  try {
    await assert.rejects(keyless.ready, { code: "EADDRINUSE" }); await keyless.close();
    assert.equal(await (await fetch(origin)).text(), "owner");
  } finally { await close(owner); }
});
test("an owned pending bind can be canceled before readiness", { timeout: 5000 }, async () => {
  const origin = await vacantOrigin(), keyless = keylessTakeover(origin);
  const outcome = keyless.ready.catch(() => null);
  await keyless.close(); await outcome;
  const replacement = keylessTakeover(origin);
  try { await replacement.ready; } finally { await replacement.close(); }
  for (const bad of ["http://localhost:1", "https://127.0.0.1:1", "http://127.0.0.1:1/elsewhere"]) assert.throws(() => keylessTakeover(bad));
});
test("dribbling headers or body cannot renew the original socket deadline", { timeout: 6000 }, async () => {
  await Promise.all(["header", "body"].map(async phase => {
    const origin = await vacantOrigin(), keyless = keylessTakeover(origin);
    let socket, timer;
    try {
      await keyless.ready;
      socket = createConnection({ host: "127.0.0.1", port: Number(new URL(origin).port) });
      socket.on("error", () => {}); // expiry may reset a peer that is still writing its partial body
      await new Promise(resolve => socket.once("connect", resolve));
      const start = performance.now();
      if (phase === "header") socket.write("POST /frame HTTP/1.1\r\nHost: ");
      else socket.write(`POST /frame HTTP/1.1\r\nHost: ${new URL(origin).host}\r\nOrigin: http://tauri.localhost\r\nContent-Length: 8208\r\nx-probe-admission: ${"0".repeat(144)}\r\n\r\nx`);
      let drips = 0;
      timer = setInterval(() => { drips++; socket.write("x"); }, 100);
      await new Promise(resolve => socket.once("close", resolve));
      assert.ok(drips >= 2, "traffic really continued while the original deadline elapsed");
      assert.ok(performance.now() - start < 3200, "traffic cannot extend the two-second timer indefinitely");
    } finally { clearInterval(timer); socket?.destroy(); await keyless.close(); }
  }));
});
