// Fixture-only codec, deliberately sequential. Not application authority or production IPC.
// Buffer clearing is best effort; no bound/zeroization claim covers browser/crypto internals.
export const MAX_PLAIN = 8192;
const HEADER = 40, TAG = 32, U64_MAX = (1n << 64n) - 1n;
const text = new TextEncoder();
const fail = () => { throw new Error("encrypted fixture frame refused"); };
const equal = (a, b) => a.length === b.length && a.every((byte, i) => byte === b[i]);
const join = (a, b) => { const bytes = new Uint8Array(a.length + b.length); bytes.set(a); bytes.set(b, a.length); return bytes; };
function header(direction, grant, sequence, id, length) {
  if (![0, 1].includes(direction) || ![1, 2].includes(grant) || typeof sequence !== "bigint" || sequence < 1n || sequence > U64_MAX
      || !(id instanceof Uint8Array) || id.length !== 16 || !Number.isInteger(length) || length < 16 || length > MAX_PLAIN + 16) fail();
  const bytes = new Uint8Array(HEADER), view = new DataView(bytes.buffer);
  bytes.set(text.encode("MWIP")); bytes[4] = 1; bytes[5] = direction; bytes[6] = grant;
  view.setBigUint64(8, sequence); bytes.set(id, 16); view.setUint32(32, length);
  return bytes;
}
function parse(admission) {
  if (typeof admission !== "string" || !/^[0-9a-f]{144}$/.test(admission)) fail();
  const bytes = Uint8Array.from(admission.match(/../g), byte => Number.parseInt(byte, 16));
  const raw = bytes.subarray(0, HEADER), view = new DataView(raw.buffer);
  const direction = raw[5], grant = raw[6], sequence = view.getBigUint64(8), id = raw.slice(16, 32), length = view.getUint32(32);
  if (!equal(raw, header(direction, grant, sequence, id, length))) fail();
  return { raw, tag: bytes.subarray(HEADER), direction, grant, sequence, id, length };
}
function nonce(raw) {
  const nonce = new Uint8Array(12);
  nonce.set(text.encode(raw[5] === 0 ? "REQ1" : "RSP1")); nonce.set(raw.subarray(8, 16), 4);
  return nonce;
}
const hex = bytes => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");

export async function createCodec(seed, grant) {
  if (!(seed instanceof Uint8Array) || seed.length !== 32 || ![1, 2].includes(grant)) fail();
  let material;
  try { material = await crypto.subtle.importKey("raw", seed, "HKDF", false, ["deriveKey"]); }
  finally { seed.fill(0); }
  const derive = (label, algorithm, usages) => crypto.subtle.deriveKey(
    { name: "HKDF", hash: "SHA-256", salt: text.encode("Mewtual IPC fixture v1"), info: text.encode(label) }, material, algorithm, false, usages);
  const [requestMac, requestBody, responseMac, responseBody] = await Promise.all([
    derive("v1 c2n header HMAC-SHA256", { name: "HMAC", hash: "SHA-256", length: 256 }, ["sign"]),
    derive("v1 c2n body AES-256-GCM", { name: "AES-GCM", length: 256 }, ["encrypt"]),
    derive("v1 n2c header HMAC-SHA256", { name: "HMAC", hash: "SHA-256", length: 256 }, ["verify"]),
    derive("v1 n2c body AES-256-GCM", { name: "AES-GCM", length: 256 }, ["decrypt"]),
  ]);
  material = null;
  let sequence = 0n, responseFloor = 0n, closed = false, headerRefusals = 0;
  const activeReaders = new Set();
  const current = signal => { if (closed || signal?.aborted) fail(); };
  return {
    snapshot() { return { headerRefusals }; }, // closed fixture diagnostics, never keys/payloads
    // This promise drains current reader cancellation only. It is not engine/OS-copy retirement.
    close() { closed = true; return Promise.allSettled([...activeReaders].map(cancel => cancel())); },
    async request(plain, id, signal) {
      current(signal);
      if (!(plain instanceof Uint8Array) || plain.length > MAX_PLAIN || sequence >= 64n) fail();
      // Spend before the first async operation. Failure never reuses a physical encryption nonce.
      const snapshot = new Uint8Array(plain);
      try {
        const raw = header(0, grant, ++sequence, id, snapshot.length + 16);
        const tag = new Uint8Array(await crypto.subtle.sign("HMAC", requestMac, raw));
        current(signal);
        const cipher = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv: nonce(raw), additionalData: raw }, requestBody, snapshot));
        try { current(signal); return Object.freeze({ raw, admission: hex(join(raw, tag)), cipher }); }
        catch (error) { cipher.fill(0); throw error; }
      } finally { snapshot.fill(0); } // actual crypto promise has settled, even on close/abort
    },
    async open(response, request, signal) {
      let reader, cipher, plain, closing;
      const closeReader = () => {
        if (closing) return closing;
        if (!reader) return Promise.resolve();
        const owned = reader;
        closing = (async () => { try { await owned.cancel(); } finally { owned.releaseLock(); reader = null; } })();
        return closing;
      };
      const cancel = () => { const operation = closeReader(); void operation.catch(() => {}); return operation; };
      try {
        current(signal);
        const parsed = parse(response.headers.get("x-probe-admission"));
        const original = parse(request.admission);
        if (original.direction !== 0 || original.grant !== grant || !equal(original.raw, request.raw)
            || parsed.direction !== 1 || parsed.grant !== grant || !equal(parsed.id, original.id)
            || parsed.sequence <= responseFloor || response.headers.has("content-encoding")
            || response.headers.get("content-length") !== String(parsed.length) || response.status !== 200) fail();
        const aad = join(parsed.raw, original.raw);
        if (!await crypto.subtle.verify("HMAC", responseMac, parsed.tag, aad)) { headerRefusals++; fail(); }
        current(signal);
        if (activeReaders.size >= 2) fail();
        // Authenticated length is now known. Browser/network-owned chunks remain outside this
        // explicit copied-buffer bound; never claim this to be an engine-wide allocation firewall.
        reader = response.body?.getReader(); if (!reader) fail();
        activeReaders.add(cancel); signal?.addEventListener("abort", cancel, { once: true });
        cipher = new Uint8Array(parsed.length);
        let used = 0;
        for (;;) {
          current(signal);
          const { done, value } = await reader.read();
          current(signal);
          if (done) break;
          if (value.byteLength > cipher.length - used) fail();
          cipher.set(value, used); used += value.byteLength;
        }
        if (used !== cipher.length) fail();
        plain = new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: nonce(parsed.raw), additionalData: aad }, responseBody, cipher));
        // A late earlier completion cannot overtake a newer accepted reply or an ended owner.
        await closeReader();
        current(signal);
        if (parsed.sequence <= responseFloor) fail();
        responseFloor = parsed.sequence;
        const result = plain; plain = null; return result;
      } finally {
        cipher?.fill(0); plain?.fill(0);
        // Header/HMAC refusal also cancels the actual response stream. Cancellation completion
        // still does not establish last-owner retirement of browser/network internal copies.
        if (reader) { try { await closeReader(); } catch { /* refuse or preserve original failure */ } }
        else if (!closing && !response.body?.locked) { try { await response.body?.cancel(); } catch { /* already closed */ } }
        activeReaders.delete(cancel); signal?.removeEventListener("abort", cancel);
      }
    },
  };
}
