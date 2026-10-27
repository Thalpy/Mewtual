// Orderly lock/close continuity only. This is not a write-ahead log: a hard crash between native
// commit and the next snapshot can still lose a receipt. Restored work never replays automatically.
import { canonicalJson, PIX_MAX_BYTES } from "./studio-contract.ts";
import { decodePix } from "./pix.ts";
import { validateJamPatch } from "./jam-patch.ts";
import { base64ToBytes, bytesToBase64, parseRecoveryApplied } from "./studio-native.ts";
import type { SaveRecord, StudioScope } from "./studio-session.ts";

export const STUDIO_DRAFT_BYTES = 12 * 1024 * 1024;
export type StudioDraft = { version: 1; scope: StudioScope; group: string; records: SaveRecord[] };
function fail(): never { throw new Error("Saved Studio edits are invalid. The sealed copy has been kept."); }
const obj = (v: unknown): Record<string, unknown> => v !== null && typeof v === "object" && !Array.isArray(v) ? v as Record<string, unknown> : fail();
const exact = (v: Record<string, unknown>, keys: string[]) => { if (Object.keys(v).some(k => !keys.includes(k))) fail(); };
const hex = (v: unknown, n: number): v is string => typeof v === "string" && new RegExp(`^[0-9a-f]{${n}}$`).test(v);
const int = (v: unknown, min: number, max = Number.MAX_SAFE_INTEGER): v is number => Number.isSafeInteger(v) && (v as number) >= min && (v as number) <= max;
const text = (v: unknown, max: number): v is string => typeof v === "string" && v.length <= max;

/** Closed domain grammar, independent of native authorization (which remains mandatory on retry). */
export function validateDraftOperation(value: unknown, family: "index" | "flipnote" | "score"): void {
  const v = obj(value), op = v.op;
  const keys: Record<string, string[]> = {
    set_header: ["op", "field", "value"], set_patch: ["op", "patch", "descriptor"], remove_patch: ["op", "patch"],
    insert_frame: ["op", "frame", "after", "cid", "bytes"], replace_frame: ["op", "frame", "cid", "bytes"], remove_frame: ["op", "frame"],
    set_sfx: ["op", "sfx", "frame", "patch", "note", "duration_ms"], remove_sfx: ["op", "sfx"],
    set_export: ["op", "export", "cid", "bytes", "expiry"], remove_export: ["op", "export"],
    set_cell: ["op", "step", "row", "value"], clear_cell: ["op", "step", "row"],
    set_title: ["op", "object", "title"], set_expiry: ["op", "object", "expiry"], tombstone_object: ["op", "object"],
    put_object: ["op", "object", "kind", "title", "created_by", "ts", "expiry"],
  };
  if (typeof op !== "string" || !Object.hasOwn(keys, op)) fail(); exact(v, keys[op]);
  if (keys[op].some(k => k !== "expiry" && !Object.hasOwn(v, k))) fail();
  const allowed = family === "index" ? ["set_title", "set_expiry", "tombstone_object", "put_object"] : family === "score"
    ? ["set_header", "set_patch", "remove_patch", "set_cell", "clear_cell"]
    : ["set_header", "set_patch", "remove_patch", "insert_frame", "replace_frame", "remove_frame", "set_sfx", "remove_sfx", "set_export", "remove_export"];
  if (!allowed.includes(op)) fail();
  for (const field of ["frame", "sfx", "export", "object"]) if (field in v && !hex(v[field], 32)) fail();
  for (const field of ["patch", "cid", "created_by"]) if (field in v && !hex(v[field], 64)) fail();
  if ("after" in v && v.after !== null && !hex(v.after, 32)) fail();
  if ("bytes" in v && !int(v.bytes, 1, op === "set_export" ? 9 * 1024 * 1024 : PIX_MAX_BYTES)) fail();
  if ("expiry" in v && v.expiry !== null && !int(v.expiry, 0)) fail();
    if ("title" in v && !text(v.title, 65536)) fail();
  if (op === "set_patch" && !validateJamPatch(v.descriptor).ok) fail();
  if (op === "set_sfx" && (!int(v.note, 0, 127) || !int(v.duration_ms, 1, 600_000))) fail();
  if (op === "put_object" && ((v.kind !== "score" && v.kind !== "flipnote") || !int(v.ts, 0))) fail();
  if (op === "set_cell" || op === "clear_cell") {
    if (!int(v.step, 0, 255) || !int(v.row, 0, 24)) fail();
    if (op === "set_cell") {
      const cell = obj(v.value); exact(cell, cell.k === "n" ? ["k", "p", "len"] : ["k", "pad"]);
      if (cell.k === "n" ? !hex(cell.p, 64) || (cell.len !== undefined && !int(cell.len, 1, 256)) : cell.k !== "d" || !int(cell.pad, 0, 9)) fail();
    }
  }
  if (op === "set_header") {
    if (v.field === "title") { if (!text(v.value, 65536)) fail(); }
    else if (family === "flipnote" && v.field === "score") { if (v.value !== null && !hex(v.value, 32)) fail(); }
    else {
      const bounds = family === "flipnote" ? { fps: [1, 24] } : { bpm: [40, 240], bpb: [1, 8], steps: [1, 256] };
      const bound = bounds[v.field as keyof typeof bounds] as number[] | undefined;
      if (!bound || !int(v.value, bound[0], bound[1])) fail();
    }
  }
}

export function encodeStudioDraft(draft: StudioDraft): string {
  const json = JSON.stringify(draft, (key, value) => key === "pix" ? bytesToBase64(value as Uint8Array) : key === "settle" ? undefined : value);
  if (new TextEncoder().encode(json).length > STUDIO_DRAFT_BYTES) fail();
  return json;
}

/** Reject the whole record rather than silently dropping one exact retry or one frame. */
export function decodeStudioDraft(json: string): StudioDraft {
  if (new TextEncoder().encode(json).length > STUDIO_DRAFT_BYTES) fail();
  const root = obj(JSON.parse(json)); exact(root, ["version", "scope", "group", "records"]);
  const scope = obj(root.scope); exact(scope, ["server", "channel"]);
  if (root.version !== 1 || !int(scope.server, 0) || typeof scope.channel !== "string" || !/^(0|[1-9][0-9]{0,38})$/.test(scope.channel) || BigInt(scope.channel) > (1n << 128n) - 1n ||
      typeof root.group !== "string" || !/^(?:[0-9a-f]{2}){1,256}$/.test(root.group) || !Array.isArray(root.records) || root.records.length > 512) fail();
  const ids = new Set<number>(); let pixels = 0;
  const records = root.records.map(raw => {
    const v = obj(raw);
    const base = ["kind", "id", "label", "status", "attempts", "error", "mayHaveCommitted", "epochChanged", "binding", "object"];
    const extra: Record<string, string[]> = { create: ["request"], frame: ["frame", "op", "after", "pix", "published", "route", "apply", "overlay", "overlayRetry", "force"],
      apply: ["request", "op"], applyIndex: ["request", "op"], overlayApply: ["objectKind", "route", "request", "overlayRetry", "op"],
      recoveryApply: ["objectKind", "edit", "applied"] };
    if (typeof v.kind !== "string" || !Object.hasOwn(extra, v.kind)) fail(); exact(v, [...base, ...extra[v.kind]]);
    if (!int(v.id, 1) || ids.has(v.id) || !text(v.label, 8192) || !text(v.error, 16384) || !int(v.attempts, 0, 1_000_000) ||
        !["queued", "inflight", "blocked", "uncertain"].includes(String(v.status)) || typeof v.mayHaveCommitted !== "boolean" || typeof v.epochChanged !== "boolean" ||
        (v.object !== null && !hex(v.object, 32)) || (v.object === null && v.kind !== "overlayApply" && v.kind !== "recoveryApply")) fail();
    if ((v.status === "inflight" || v.status === "uncertain" || v.mayHaveCommitted) && v.attempts === 0 || v.status === "uncertain" && !v.mayHaveCommitted) fail();
    ids.add(v.id);
    if (v.binding !== undefined) {
      const b = obj(v.binding); exact(b, ["epochId", "awaiting", "phase"]);
      if (!hex(b.epochId, 32) || typeof b.awaiting !== "boolean" || (b.awaiting ? b.phase !== null : !["open", "closing", "settled", "fault"].includes(String(b.phase)))) fail();
    }
    const request = (value: unknown, overlay: boolean, family: "index" | "flipnote" | "score") => {
      const r = obj(value); exact(r, overlay ? ["basis", "branch", "nonce", "body"] : family === "index" ? ["epochId", "nonce", "body"] : ["object", "objectKind", "epochId", "nonce", "body"]);
      if (!hex(r.nonce, 32) || !text(r.body, 65536) || new TextEncoder().encode(r.body).length > 65536 ||
          (overlay ? !hex(r.basis, 64) || !hex(r.branch, 64) : !hex(r.epochId, 32)) ||
          (!overlay && family !== "index" && (r.object !== v.object || (r.objectKind ?? "flipnote") !== family))) fail();
      const op = JSON.parse(r.body); validateDraftOperation(op, family);
      if (canonicalJson(op) !== r.body || (v.kind !== "frame" && canonicalJson(v.op) !== r.body)) fail();
    };
    if (v.kind === "create") {
      const r = obj(v.request); exact(r, ["object", "nonce", "title", "createdAtMs", "objectKind"]);
      if (r.object !== v.object || !hex(r.nonce, 32) || !text(r.title, 65536) || !int(r.createdAtMs, 0) || !["flipnote", "score"].includes(String(r.objectKind ?? "flipnote"))) fail();
    } else if (v.kind === "frame") {
      if (!hex(v.frame, 32) || (v.op !== "insert" && v.op !== "replace") || (v.after !== null && !hex(v.after, 32)) ||
          !["ordinary", "unconfirmed", "closing"].includes(String(v.route)) || !text(v.pix, 4 * Math.ceil(PIX_MAX_BYTES / 3)) || (v.force !== undefined && typeof v.force !== "boolean")) fail();
      v.pix = base64ToBytes(v.pix); decodePix(v.pix as Uint8Array); pixels += (v.pix as Uint8Array).length;
      if (v.published !== null) { const p = obj(v.published); exact(p, ["cid", "bytes"]); if (!hex(p.cid, 64) || p.bytes !== (v.pix as Uint8Array).length) fail(); }
      if (v.apply !== null) request(v.apply, false, "flipnote");
      if (v.overlay !== null) request(v.overlay, true, "flipnote");
      if (v.apply !== null && v.route !== "ordinary" || v.overlay !== null && v.route === "ordinary") fail();
      const frozen = v.apply ?? v.overlay;
      if (frozen) {
        const op = JSON.parse(String(obj(frozen).body)), p = obj(v.published);
        if (op.op !== (v.op === "insert" ? "insert_frame" : "replace_frame") || op.frame !== v.frame || op.cid !== p.cid || op.bytes !== p.bytes || (v.op === "insert" && op.after !== v.after)) fail();
      }
    } else if (v.kind === "apply" || v.kind === "applyIndex") {
      const family = v.kind === "applyIndex" ? "index" : obj(v.request).objectKind ?? "flipnote";
      if (family !== "index" && family !== "flipnote" && family !== "score") fail();
      request(v.request, false, family);
      if (family === "index" && obj(v.op).object !== v.object) fail();
    } else if (v.kind === "overlayApply") {
      if (v.route !== "closing" && v.route !== "unconfirmed" || v.objectKind !== undefined && v.objectKind !== "score" && v.objectKind !== "flipnote") fail();
      const family = v.object === null ? "index" : v.objectKind ?? "flipnote";
      validateDraftOperation(v.op, family as "index" | "score" | "flipnote");
      if (v.request !== null) request(v.request, true, family as "index" | "score" | "flipnote");
    } else {
      // Recovery's preview payload is retained verbatim; native revalidates the snapshot/choice
      // against authenticated historical evidence. Never derive a fresh preview on restoration.
      const e = obj(v.edit); exact(e, ["snapshot", "choice", "mode", "epochId", "expectedProjection", "nonce", "body"]);
      if (!hex(e.snapshot, 64) || !hex(e.epochId, 32) || !hex(e.expectedProjection, 64) || !hex(e.nonce, 32) || !text(e.body, 65536) || !["copy", "restore"].includes(String(e.mode))) fail();
      const choice = obj(e.choice);
      const keys: Record<string, string[]> = { soundRecord: ["kind", "value"], scoreCell: ["kind", "step", "row", "value"], scorePatch: ["kind", "patch", "value"],
        scoreHeader: ["kind", "field", "value"], frame: ["kind", "id", "value"], frameDeletion: ["kind", "id"], title: ["kind", "value"], fps: ["kind", "value"],
        object: ["kind", "id"], objectTitle: ["kind", "id", "value"], objectExpiry: ["kind", "id", "value"], objectDeletion: ["kind", "id"] };
      if (typeof choice.kind !== "string" || !Object.hasOwn(keys, choice.kind)) fail(); exact(choice, keys[choice.kind]);
      if (keys[choice.kind].some(k => !Object.hasOwn(choice, k)) || "value" in choice && !hex(choice.value, 64) || "id" in choice && !hex(choice.id, 32) || "patch" in choice && !hex(choice.patch, 64) ||
          choice.kind === "scoreCell" && (!int(choice.step, 0, 255) || !int(choice.row, 0, 24)) || choice.kind === "scoreHeader" && !["title", "bpm", "bpb", "steps"].includes(String(choice.field))) fail();
      const family = v.object === null ? "index" : v.objectKind ?? "flipnote";
      if (family !== "index" && family !== "flipnote" && family !== "score") fail();
      const permitted = family === "index" ? ["object", "objectTitle", "objectExpiry", "objectDeletion"] : family === "score" ? ["scoreCell", "scorePatch", "scoreHeader"] : ["frame", "frameDeletion", "title", "fps", "soundRecord"];
      if (!permitted.includes(choice.kind)) fail();
      const body = JSON.parse(e.body as string); validateDraftOperation(body, family);
      if (canonicalJson(body) !== e.body) fail();
      if (v.applied !== null) parseRecoveryApplied(v.applied);
    }
    if ((v.kind === "frame" || v.kind === "overlayApply") && v.overlayRetry !== null && !["sameRequest", "newTicket", "none", "afterExplicitAccess"].includes(String(v.overlayRetry))) fail();
    return v as unknown as SaveRecord;
  });
  if (pixels > 8 * 1024 * 1024) fail();
  return { version: 1, scope: scope as StudioScope, group: root.group as string, records };
}
