/** Exact body intake for the disabled encrypted voice handoff pilot. Call only after the
 * response header and length have been authenticated, with exclusive ownership of Response.
 * The enclosing physical-flight loan must already cover this buffer and one EOF probe and
 * remain held through this promise (including read and actual cancellation settlement).
 * This helper supplies neither channel authentication nor a complete flight/copy budget. */
import type { VoiceIpcHeader } from "./voice-ipc-frame.ts";

export const VOICE_IPC_PILOT_CIPHER_MAX = 2048 + 16;
/** Native opcode17 has its own authenticated class, not a generic Data fallback. */
export const VOICE_IPC_SDP_INPUT_CIPHER_MAX = 8192 + 1024 + 16;
type BodyFailure = "unsupported" | "metadata" | "truncated" | "excess" | "closed" | "reader";
export class VoiceIpcBodyError extends Error {
  readonly code: BodyFailure;
  constructor(code: BodyFailure) {
    super("voice handoff body refused");
    this.name = "VoiceIpcBodyError";
    this.code = code;
  }
}

/** Require BYOB on this actual response stream. A default reader may hand the application an
 * arbitrarily large SDK chunk before its length can be checked; it is not a bounded fallback.
 * BYOB transfers/detaches the supplied backing buffer on every read, so follow the returned
 * owned view rather than retaining or copying from a detached earlier Uint8Array. */
export async function readVoiceIpcBody(
  response: Response, authenticatedBytes: number, signal: AbortSignal,
): Promise<Uint8Array<ArrayBuffer>> {
  return readBody(response, authenticatedBytes, signal, VOICE_IPC_PILOT_CIPHER_MAX);
}

/** Authenticate an EXACT144-byte ASCII prefix from the bounded stream, then admit its body.
 * No HTTP header value is materialized: a keyless process taking over a stale loopback port
 * cannot force an application-sized string allocation before the MAC/length check. The
 * caller must verify original direction/request/session and debit RX inside authenticate.
 * Returned bytes remain owned by the enclosing prepaid flight until explicitly disposed. */
export async function readVoiceIpcResponse(
  response: Response, signal: AbortSignal, authenticate: (admission: string) => Promise<number>,
): Promise<Uint8Array<ArrayBuffer>> {
  return readBody(response, null, signal, VOICE_IPC_PILOT_CIPHER_MAX, authenticate);
}

/** Separate original SDP Input intake. The caller must authenticate the exact response
 * MAC, original request/session/position and replay counter BEFORE returning this header.
 * Only opcode17's class2 Input or fixed Control history/refusal can select a length. No
 * untrusted HTTP length, caller-selected ceiling or generic Data widening is accepted.
 *
 * The enclosing exclusive paid flight covers the complete ciphertext, prefix, EOF probe,
 * WebCrypto copies and eventual decrypted backing; this reader does not fund those owners.
 * Retain that loan through actual read/cancel settlement and then through the last use of
 * the returned bytes. Production broker/SDP dispatch remains disabled. */
export async function readVoiceIpcSdpInputResponse(
  response: Response, signal: AbortSignal,
  authenticate: (admission: string) => Promise<Readonly<Pick<VoiceIpcHeader, "direction" | "operation" | "kind" | "length">>>,
): Promise<Uint8Array<ArrayBuffer>> {
  return readBody(response, null, signal, VOICE_IPC_SDP_INPUT_CIPHER_MAX, async admission => {
    const header = await authenticate(admission);
    // Snapshot each field once: metadata supplied across an asynchronous callback must
    // not change its class after the check and before the positive allocation.
    if (!header) throw new VoiceIpcBodyError("metadata");
    const direction = header.direction, operation = header.operation, kind = header.kind, length = header.length;
    const limit = kind === 0 ? 272 : kind === 2 ? VOICE_IPC_SDP_INPUT_CIPHER_MAX : 0;
    if (direction !== 1 || operation !== 17 || !Number.isSafeInteger(length) || length < 16 || length > limit) {
      throw new VoiceIpcBodyError("metadata");
    }
    return length;
  });
}

async function readBody(
  response: Response, authenticatedBytes: number | null, signal: AbortSignal, maximum: number,
  authenticate?: (admission: string) => Promise<number>,
): Promise<Uint8Array<ArrayBuffer>> {
  let reader: ReadableStreamBYOBReader | null = null;
  let body: Uint8Array<ArrayBuffer> | null = null;
  let eof: Uint8Array<ArrayBuffer> | null = null;
  let prefix: Uint8Array<ArrayBuffer> | null = null;
  let cleanup: Promise<void> | null = null;
  const closed = () => { if (signal.aborted) throw new VoiceIpcBodyError("closed"); };
  const cancel = (): Promise<void> => {
    if (cleanup) return cleanup;
    const owned = reader;
    cleanup = (async () => {
      // A refused/unsupported response still owns real fetch work. Await its cancellation
      // instead of merely rejecting while the outer loan is returned to a later action.
      try {
        if (owned) await owned.cancel();
        else if (response.body && !response.body.locked) await response.body.cancel();
      } finally {
        owned?.releaseLock();
        reader = null;
      }
    })();
    return cleanup;
  };
  const abort = () => { void cancel().catch(() => {}); };
  try {
    closed();
    const validLength = (value: number | null): value is number => typeof value === "number" &&
      Number.isSafeInteger(value) && value >= 16 && value <= maximum;
    if ((authenticatedBytes !== null && !validLength(authenticatedBytes)) || response.status !== 200 ||
        response.headers.has("content-encoding")) {
      throw new VoiceIpcBodyError("metadata");
    }
    if (!response.body || response.body.locked) throw new VoiceIpcBodyError("reader");
    try { reader = response.body.getReader({ mode: "byob" }); }
    catch { throw new VoiceIpcBodyError("unsupported"); }
    signal.addEventListener("abort", abort, { once: true });
    closed(); // no async gap, but keep the original signal gate next to allocation
    if (authenticate) {
      prefix = new Uint8Array(144);
      let used = 0;
      while (used < 144) {
        if (!prefix) throw new VoiceIpcBodyError("reader");
        const result: ReadableStreamReadResult<Uint8Array<ArrayBuffer>> = await reader.read(prefix.subarray(used));
        prefix = result.value ? new Uint8Array(result.value.buffer) : null;
        closed();
        if (result.done || !result.value || !result.value.byteLength) throw new VoiceIpcBodyError("truncated");
        if (result.value.buffer.byteLength !== 144 || result.value.byteOffset !== used || result.value.byteLength > 144 - used) throw new VoiceIpcBodyError("reader");
        used += result.value.byteLength;
      }
      // Construct at most144 characters from our OWN fixed buffer, never Headers.get().
      let admission = "";
      if (!prefix) throw new VoiceIpcBodyError("reader");
      for (const byte of prefix) {
        if (!((byte >= 48 && byte <= 57) || (byte >= 97 && byte <= 102))) throw new VoiceIpcBodyError("metadata");
        admission += String.fromCharCode(byte);
      }
      authenticatedBytes = await authenticate(admission);
      closed(); prefix.fill(0); prefix = null;
    }
    if (!validLength(authenticatedBytes)) throw new VoiceIpcBodyError("metadata");
    body = new Uint8Array(authenticatedBytes);
    let used = 0;
    while (used < authenticatedBytes) {
      if (!body) throw new VoiceIpcBodyError("reader");
      const { done, value }: ReadableStreamReadResult<Uint8Array<ArrayBuffer>> =
        await reader.read(body.subarray(used));
      // Cancellation can settle read with no returned buffer. Do not attempt to clear a
      // detached earlier view: ownership has moved through the actual stream operation.
      body = value ? new Uint8Array(value.buffer) : null;
      closed();
      if (done || !value || !value.byteLength) throw new VoiceIpcBodyError("truncated");
      if (value.buffer.byteLength !== authenticatedBytes || value.byteOffset !== used ||
          value.byteLength > authenticatedBytes - used) throw new VoiceIpcBodyError("reader");
      used += value.byteLength;
    }
    // Reaching the authenticated length is insufficient: reject a longer body. This is a
    // separately prepaid one-byte BYOB buffer, not an unbounded default-reader EOF check.
    eof = new Uint8Array(1);
    const end = await reader.read(eof);
    eof = end.value ? new Uint8Array(end.value.buffer) : null;
    closed();
    if (!end.done || (end.value?.byteLength ?? 0) !== 0) throw new VoiceIpcBodyError("excess");
    await cancel();
    closed();
    const result = body;
    if (!result || result.byteLength !== authenticatedBytes) throw new VoiceIpcBodyError("reader");
    body = null;
    return result;
  } catch (error) {
    if (error instanceof VoiceIpcBodyError) throw error;
    // Browser/network exceptions can include URLs or arbitrary platform details. Keep the
    // adapter contract closed and never replace it with detached-buffer cleanup exceptions.
    throw new VoiceIpcBodyError(signal.aborted ? "closed" : "reader");
  } finally {
    // Keep the promise pending while any underlying read/cancel work is still observable.
    // Clearing is best effort; neither this helper nor the loan proves physical SDK/GC erase.
    try { await cancel(); }
    catch { /* retain the original refusal; success already awaited this same cancellation */ }
    finally {
      if (body?.byteLength) body.fill(0);
      if (eof?.byteLength) eof.fill(0);
      if (prefix?.byteLength) prefix.fill(0);
      signal.removeEventListener("abort", abort);
    }
  }
}
