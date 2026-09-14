// Builders for native-shaped Studio responses and a scripted fake bridge, shared by the adapter
// and session tests. The shapes mirror studio.rs / studio/recovery.rs serialization exactly;
// nothing here is imported by the app.

import { encodePix } from "./pix.ts";
import { FLIPNOTE_H, FLIPNOTE_W } from "./studio-contract.ts";
import { DEFAULT_PALETTE } from "./studio-store.ts";
import type { StudioIpc } from "./studio-native.ts";

export const ME = "a1".repeat(32);
export const ROOK = "b2".repeat(32);
export const CHANNEL = "340282366920938463463374607431768211455"; // u128::MAX, deliberately not a JS number
export const SERVER = 7;

export const id32 = (n: number) => n.toString(16).padStart(32, "0");
export const id64 = (n: number) => n.toString(16).padStart(64, "0");

/// A flat 192x144 frame filled with palette index `fill`, as real PIX1 bytes.
export function pixBytes(fill = 0): Uint8Array {
  const pixels = new Uint8Array(FLIPNOTE_W * FLIPNOTE_H).fill(fill);
  return encodePix({ w: FLIPNOTE_W, h: FLIPNOTE_H, palette: DEFAULT_PALETTE.map((e) => ({ ...e })), pixels });
}

export const isource = (n: number, author = ME) => ({ opId: id64(n), author, nonce: id32(n) });
export const fsource = (n: number, author = ME, ts = 1_000 + n) => ({ ...isource(n, author), ts });
export const reg = <T>(value: T, source: unknown, conflicts: { value: T; source: unknown }[] = []) => ({ selected: { value, source }, conflicts });

export type FrameSpec = { id: string; cid: string; bytes: number; author?: string; op?: number; conflicts?: { cid: string; bytes: number; author: string; op: number }[]; after?: string | null };

export function flipnoteContent(opts: { title?: string | null; fps?: number | null; frames: FrameSpec[]; overCap?: Record<string, { count: boolean; bytes: boolean }>; tombstones?: Record<string, unknown[]> }) {
  const frames: Record<string, unknown> = {};
  let n = 100;
  for (const f of opts.frames) {
    const op = f.op ?? n++;
    frames[f.id] = {
      pixels: reg({ cid: f.cid, bytes: f.bytes }, fsource(op, f.author ?? ME), (f.conflicts ?? []).map((c) => ({ value: { cid: c.cid, bytes: c.bytes }, source: fsource(c.op, c.author) }))),
      insertions: [{ value: { checkpoint: false, after: f.after ?? null, anchor: null, before: null, blob: { cid: f.cid, bytes: f.bytes } }, source: fsource(op, f.author ?? ME) }],
    };
  }
  return {
    kind: "flipnote",
    title: opts.title === null ? null : reg(opts.title ?? "moon cat", fsource(1)),
    fps: opts.fps === null ? null : reg(opts.fps ?? 12, fsource(2)),
    timeline: opts.frames.map((f) => f.id),
    frames,
    declaredFrameBytes: opts.frames.reduce((s, f) => s + f.bytes, 0),
    overCap: opts.overCap ?? {},
    tombstones: opts.tombstones ?? {},
  };
}

export function indexEntry(opts: { kind?: "flipnote" | "score"; title: string; createdBy?: string; ts?: number; expiry?: unknown; op?: number; titleConflicts?: { value: string; source: unknown }[] }) {
  const op = opts.op ?? 5;
  const expiry = opts.expiry ?? { kind: "unrecorded" };
  return {
    creations: [{ source: isource(op, opts.createdBy ?? ME), value: { kind: opts.kind ?? "flipnote", title: opts.title, createdBy: opts.createdBy ?? ME, ts: opts.ts ?? 5_000, expiry } }],
    title: reg(opts.title, isource(op, opts.createdBy ?? ME), opts.titleConflicts ?? []),
    expiry: reg(expiry, isource(op, opts.createdBy ?? ME)),
  };
}

export function indexContent(opts: { objects?: Record<string, unknown>; overflow?: Record<string, unknown>; deletedObjects?: Record<string, unknown>; tombstones?: Record<string, unknown[]> } = {}) {
  return { kind: "index", objects: opts.objects ?? {}, overflow: opts.overflow ?? {}, deletedObjects: opts.deletedObjects ?? {}, tombstones: opts.tombstones ?? {} };
}

export function ordinaryView(content: unknown, opts: { epochId?: string; epoch?: string; phase?: string } = {}) {
  return { v: 1, epochId: opts.epochId ?? id32(0xe1), epoch: opts.epoch ?? "3", channel: CHANNEL, publication: "local", provisional: true, phase: opts.phase ?? "open", content };
}
export function previewView(content: unknown, opts: { epochId?: string; epoch?: string } = {}) {
  return { v: 1, epochId: opts.epochId ?? id32(0xe2), epoch: opts.epoch ?? "4", channel: CHANNEL, provisional: true, awaitingTenureReceipt: true, content };
}

export function recoveryListing(opts: { object?: string | null; versions?: unknown[]; evictionPending?: unknown; pendingIntents?: number; kind?: string; source?: unknown } = {}) {
  return {
    v: 1,
    kind: opts.kind ?? "recoveryList",
    channel: CHANNEL,
    object: opts.object ?? null,
    source: opts.source === undefined ? { epochId: id32(0xe1), epoch: "3", provisional: true, phase: "open" } : opts.source,
    versions: opts.versions ?? [],
    evictionPending: opts.evictionPending ?? null,
    pendingIntents: opts.pendingIntents ?? 0,
  };
}
export const version = (n: number, opts: { staged?: boolean; reason?: string; epoch?: string } = {}) => ({ snapshot: id64(0x500 + n), epoch: opts.epoch ?? String(n), staged: opts.staged ?? false, bytes: 9_000 + n, reason: opts.reason ?? "rewound" });

export function b64(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
}

export type Call = { cmd: string; args: Record<string, unknown> };
export type FakeIpc = StudioIpc & {
  calls: Call[];
  handlers: Record<string, (event: { payload: unknown }) => void>;
  /// Per-command responders; a responder may return a Promise to hold a call open.
  on: (cmd: string, fn: (args: Record<string, unknown>) => unknown) => void;
  emit: (name: string, payload: unknown) => void;
};

export function fakeIpc(): FakeIpc {
  const calls: Call[] = [];
  const responders: Record<string, (args: Record<string, unknown>) => unknown> = {};
  const handlers: Record<string, (event: { payload: unknown }) => void> = {};
  const ipc: FakeIpc = {
    calls,
    handlers,
    on: (cmd, fn) => { responders[cmd] = fn; },
    emit: (name, payload) => handlers[name]?.({ payload }),
    invoke: async <T>(cmd: string, args: Record<string, unknown> = {}) => {
      // A snapshot of what was sent: a caller that later mutates its argument object must not
      // rewrite the recorded history a test compares against.
      calls.push({ cmd, args: structuredClone(args) });
      const r = responders[cmd];
      if (!r) throw new Error(`unscripted command ${cmd}`);
      return (await r(args)) as T;
    },
    listen: async <T>(name: string, handler: (event: { payload: T }) => void) => {
      handlers[name] = handler as (event: { payload: unknown }) => void;
      return () => { delete handlers[name]; };
    },
  };
  return ipc;
}

/// A promise whose settlement the test controls.
export function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void; reject: (e: unknown) => void } {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

/// Let queued microtasks and the fake's promise chains settle.
export async function settle(rounds = 8): Promise<void> {
  for (let i = 0; i < rounds; i++) await new Promise((r) => setTimeout(r, 0));
}
