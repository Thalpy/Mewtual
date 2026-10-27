/** Test-only length-framed metadata reader. OS pipe chunks are NOT protocol frames:
 * one chunk may finish a reply and contain the next step. Decode sequentially through
 * ONE fixed frame buffer; the caller retains its one-step/one-callback ownership checks.
 * No input queue, deferred parsing, SDK/plaintext body or arbitrary diagnostic is kept. */
export class NativeControlFrames {
  readonly #buffer: Uint8Array;
  readonly #limit: number;
  #used = 0;
  #length = 0;
  #failed = false;
  #entered = false;
  constructor(limit = 4096) {
    if (!Number.isInteger(limit) || limit <= 0 || limit > 4096) throw new Error("invalid control frame bound");
    this.#limit = limit; this.#buffer = new Uint8Array(limit + 4);
  }
  get pendingBytes(): number { return this.#used; }
  get declaredBytes(): number { return this.#length; }
  feed(input: Uint8Array, accept: (frame: unknown) => void): void {
    if (this.#failed) throw new Error("control reader unavailable");
    if (this.#entered) {
      this.#failed = true; this.#buffer.fill(0);
      throw new Error("control reader unavailable");
    }
    this.#entered = true;
    try {
      let offset = 0;
      while (offset < input.length) {
        if (this.#used < 4) {
          const count = Math.min(4 - this.#used, input.length - offset);
          this.#buffer.set(input.subarray(offset, offset + count), this.#used);
          this.#used += count; offset += count;
          if (this.#used < 4) return;
          this.#length = new DataView(this.#buffer.buffer).getUint32(0);
          if (this.#length === 0 || this.#length > this.#limit) throw new Error("invalid control frame length");
        }
        const count = Math.min(4 + this.#length - this.#used, input.length - offset);
        this.#buffer.set(input.subarray(offset, offset + count), this.#used);
        this.#used += count; offset += count;
        if (this.#used !== 4 + this.#length) return;
        const text = new TextDecoder("utf-8", {fatal: true}).decode(this.#buffer.subarray(4, this.#used));
        const value: unknown = JSON.parse(text);
        this.#buffer.fill(0, 0, this.#used); this.#used = 0; this.#length = 0;
        // No buffered payload survives this callback; an error closes this same reader.
        // Native protocol ownership, NOT chunk boundaries, decides which frame is valid.
        accept(value);
        // Catching a rejected reentrant feed cannot restore this original stream.
        if (this.#failed) throw new Error("control reader unavailable");
      }
    } catch (error) { this.#failed = true; this.#buffer.fill(0); throw error; }
    finally { this.#entered = false; }
  }
  finish(): void {
    if (this.#failed || this.#used !== 0) throw new Error("incomplete control stream");
  }
}
