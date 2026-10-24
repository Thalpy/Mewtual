/** Bounded public native work observations. This is a scan, not VoiceV2Native.events and
 * not an action admission. Empty/Pending never prove absence, success or actual drainage.
 * Byte-bearing work must still use the original authenticated channel and native receipts.
 * No production broker or RTC activation is wired by this decoder. */
import type { VoiceIpcActionIdentity, VoiceIpcPurpose } from "./voice-ipc-actions.ts";

export type VoiceWorkPurpose =
  | Readonly<VoiceIpcPurpose & { kind: "derive" }>
  | Readonly<{ kind: "confirmation"; parent: string; parentPurpose: VoiceIpcPurpose & { kind: "derive" } } & (
    { request: "start"; predecessor: string | null } |
    { request: "respond"; trigger: string } |
    { request: "incoming"; role: number }
  )>;
/** "attached" retains an action identity even after native compaction; obtain current
 * effect/cleanup state only through the original authenticated history and receipts. */
export type VoiceWorkDescriptor = Readonly<{
  row: string; purpose: VoiceWorkPurpose;
} & (
  { state: "pending" | "closing"; actorHandoff: null; identity: null } |
  { state: "closedPending"; actorHandoff: boolean; identity: null } |
  { state: "attached"; actorHandoff: null; identity: VoiceIpcActionIdentity }
)>;
export type VoiceWorkSnapshot = Readonly<{
  kind: "snapshot"; owner: string; executor: string; nextSlot: number; work: VoiceWorkDescriptor | null;
}>;

const bad = (): never => { throw new Error("invalid voice work metadata"); };
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return bad();
  return value as Record<string, unknown>;
}
function exact(value: unknown, fields: readonly string[]): Record<string, unknown> {
  const object = record(value), names = Object.keys(object);
  if (names.length !== fields.length || names.some(name => !fields.includes(name))) return bad();
  return object;
}
function hex(value: unknown, digits: number): string {
  if (typeof value !== "string" || value.length !== digits || !/^[0-9a-f]+$/.test(value)) return bad();
  return value;
}
function decimal(value: unknown, zero: boolean): string {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]{0,19})$/.test(value) ||
      (!zero && value === "0") || BigInt(value) > 0xffffffffffffffffn) return bad();
  return value;
}
function row(value: unknown): string {
  const text = hex(value, 32);
  if (BigInt("0x" + text.slice(0, 16)) >= 32n || BigInt("0x" + text.slice(16)) === 0n) return bad();
  return text;
}
function slot(value: unknown): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0 || value > 32) return bad();
  return value;
}
function derive(value: unknown): VoiceIpcPurpose & { kind: "derive" } {
  const object = exact(value, ["kind", "peer", "join", "keyRef", "epoch"]);
  if (object.kind !== "derive") return bad();
  return Object.freeze({ kind: "derive", peer: hex(object.peer, 64), join: hex(object.join, 64),
    keyRef: hex(object.keyRef, 64), epoch: decimal(object.epoch, true) });
}
function purpose(value: unknown): VoiceWorkPurpose {
  const object = record(value);
  if (object.kind === "derive") return derive(value);
  if (object.kind !== "confirmation") return bad();
  const common = { kind: "confirmation" as const, parent: row(object.parent), parentPurpose: derive(object.parentPurpose) };
  switch (object.request) {
    case "start":
      exact(object, ["kind", "parent", "parentPurpose", "request", "predecessor"]);
      return Object.freeze({ ...common, request: "start", predecessor: object.predecessor === null ? null : hex(object.predecessor, 64) });
    case "respond":
      exact(object, ["kind", "parent", "parentPurpose", "request", "trigger"]);
      return Object.freeze({ ...common, request: "respond", trigger: row(object.trigger) });
    case "incoming":
      exact(object, ["kind", "parent", "parentPurpose", "request", "role"]);
      if (typeof object.role !== "number" || !Number.isInteger(object.role) || object.role < 1 || object.role > 9) return bad();
      return Object.freeze({ ...common, request: "incoming", role: object.role });
    default: return bad();
  }
}
function descriptor(value: unknown): VoiceWorkDescriptor {
  const object = exact(value, ["row", "purpose", "state", "actorHandoff", "identity"]);
  const common = { row: row(object.row), purpose: purpose(object.purpose) };
  if (common.purpose.kind === "confirmation" && (common.purpose.parent === common.row ||
      (common.purpose.request === "respond" && (common.purpose.trigger === common.row || common.purpose.trigger === common.purpose.parent)))) return bad();
  if (object.state === "attached") {
    if (object.actorHandoff !== null) return bad();
    const action = exact(object.identity, ["action", "version", "inputTag"]);
    const identity = Object.freeze({ action: decimal(action.action, false), version: decimal(action.version, false), inputTag: hex(action.inputTag, 64) });
    return Object.freeze({ ...common, state: "attached", actorHandoff: null, identity });
  }
  if (object.identity !== null) return bad();
  if (object.state === "closedPending") {
    if (typeof object.actorHandoff !== "boolean") return bad();
    return Object.freeze({ ...common, state: "closedPending", actorHandoff: object.actorHandoff, identity: null });
  }
  if ((object.state !== "pending" && object.state !== "closing") || object.actorHandoff !== null) return bad();
  return Object.freeze({ ...common, state: object.state, actorHandoff: null, identity: null });
}

/** Match the caller's ORIGINAL canonical channel selection and one slot scan. A snapshot
 * can become stale immediately; never turn it into paired/RTC authority, authenticated action
 * identity, a new work request or an assertion that native production has stopped. */
export function decodeVoiceWorkSnapshot(value: unknown, owner: string, executor: string, fromSlot: number): VoiceWorkSnapshot {
  if (!/^voice-[0-9a-f]{16}$/.test(owner) || hex(executor, 64) !== executor) return bad();
  slot(fromSlot);
  const object = exact(value, ["kind", "owner", "executor", "nextSlot", "work"]);
  if (object.kind !== "snapshot" || object.owner !== owner || object.executor !== executor) return bad();
  const nextSlot = slot(object.nextSlot), work = object.work === null ? null : descriptor(object.work);
  if (work === null) {
    if (nextSlot !== 32) return bad();
  } else {
    const selectedSlot = Number(BigInt("0x" + work.row.slice(0, 16)));
    if (selectedSlot < fromSlot || nextSlot !== selectedSlot + 1) return bad();
  }
  return Object.freeze({ kind: "snapshot", owner, executor, nextSlot, work });
}
