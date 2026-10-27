/** Test-only metadata observations for the real native/browser coordinator fixture.
 * No snapshot, Row, Action, receipt, SDK object or payload is retained. The sole object
 * identity is the production negotiation's inert `generation = {}` token. Never pass
 * the negotiation itself here. These observations grant and drive no native operation. */
import { decodeVoiceIpcActionReply } from "../src/voice-ipc-replies.ts";

const STAGES = new Set(["waiting", "confirmation", "handoff", "pc", "rtc", "media",
  "retire", "compact", "failed", "advance", "stopped"]);
// Invoke the intrinsic on actual storage, bypassing an own byteLength getter and
// rejecting typed-array Proxies. No species/constructor getter participates in copying.
const ByteArray = Uint8Array;
const byteLength = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(ByteArray.prototype), "byteLength")?.get;
type Plan = { linked: number; due: number; failed: number | null; rejected: string };

/** One instance PER control step, so an early failure on a later observation cannot
 * report an old step's refusals. All arrays have fixed ceilings before publication. */
export class NativeCoordinatorDiagnostics {
  readonly #began: number;
  readonly #refusals: string[] = [];
  readonly #stages: string[] = [];
  readonly #plans: Plan[] = [];
  #previousStage: string | null = null;
  #generation: object | null = null;

  constructor(began: number) { this.#began = typeof began === "number" ? began : NaN; }
  #elapsed(now: number): number | null {
    // Bound diagnostic numbers, including hostile/nonfinite input. This is not a
    // protocol clock, expiry transition, or permission to release any real owner.
    if (!Number.isFinite(now) || !Number.isFinite(this.#began)) return null;
    const elapsed = now - this.#began;
    return Number.isFinite(elapsed) && elapsed >= 0 && elapsed <= 120_000 ? Math.floor(elapsed) : null;
  }
  /** Call only after actual AEAD return. The closed decoder filters malformed replies;
   * copy only numeric refusal tags, never the caller's bytes or decoder object. */
  refusal(bytes: Uint8Array, now: number): void {
    try {
      if (this.#refusals.length >= 4 || !byteLength || Reflect.apply(byteLength, bytes, []) !== 3) return;
      const elapsed = this.#elapsed(now); if (elapsed === null) return;
      const snapshot = new ByteArray(3);
      snapshot[0] = bytes[0]; snapshot[1] = bytes[1]; snapshot[2] = bytes[2];
      const refused = decodeVoiceIpcActionReply(snapshot, null);
      if (refused.kind === "refused" && refused.code !== "capacity" && this.#refusals.length < 4)
        this.#refusals.push(snapshot[1] + "." + snapshot[2] + "@" + elapsed);
    } catch { /* Invalid bytes are not an authenticated closed refusal observation. */ }
  }
  /** Primitive metadata only. Expires uses the SAME performance clock as the actual
   * fixture broker. A new generation beyond the first two cannot append to an old plan. */
  observe(stage: string, generation: object, expires: number | null, rejected: unknown, now: number, failed = false): void {
    const elapsed = this.#elapsed(now); if (elapsed === null) return;
    const selectedStage = STAGES.has(stage) ? stage : "?";
    if (expires !== null && Number.isFinite(expires) && generation !== this.#generation && this.#plans.length < 2) {
      const due = expires - this.#began;
      if (Number.isFinite(due) && Math.abs(due) <= 120_000) {
        this.#plans.push({ linked: elapsed, due: Math.floor(due), failed: null, rejected: "none" });
        this.#generation = generation;
      }
    }
    // Scheduling stage alone is insufficient: successful retirement can close Source
    // while the same media stays healthy. Only the actual negotiation's negative latch
    // records a local failure intent; even that is not a native terminal/drain receipt.
    if (failed && expires !== null && generation === this.#generation) {
      const plan = this.#plans.at(-1);
      if (plan && plan.failed === null) {
        plan.failed = elapsed; // Irreversible: no repeated failed-stage suffix growth.
        // Actual ActionOutcome uses this closed RejectionCode union. Unknown values
        // contribute a literal marker, never arbitrary SDK/SDP error text.
        plan.rejected = rejected === "retired" || rejected === "invalid" || rejected === "capacity" ||
          rejected === "inventory-overflow" || rejected === "none" ? rejected : "unknown";
      }
    }
    if (selectedStage !== this.#previousStage) {
      if (this.#stages.length === 6) this.#stages.shift();
      this.#stages.push(selectedStage.slice(0, 4) + "@" + elapsed);
      this.#previousStage = selectedStage;
    }
  }
  /** Only this known metadata may use the320-character failure prefix. General
   * exception messages keep the fixture's existing160-character redaction bound. */
  summary(completed: number, peers: number): string {
    const count = (n: number) => Number.isInteger(n) && n >= 0 && n <= 16 ? String(n) : "?";
    const plans = this.#plans.map(plan => "link@" + plan.linked + "/due@" + plan.due +
      (plan.failed === null ? "" : "/fail@" + plan.failed + "/" + plan.rejected)).join(";");
    return (`coordinator stalled: done=${count(completed)} sdk=${count(peers)} first=${this.#refusals.join(",")} ` +
      `plans=${plans} stages=${this.#stages.join(",")}`).slice(0, 320);
  }
}
