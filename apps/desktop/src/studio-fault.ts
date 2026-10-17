import { StudioNativeError, type RecoveryTargetArgs, type StudioIpc } from "./studio-native.ts";

export type FaultScope = "source" | "registry";
export type FaultCandidate = Readonly<{ receipt: string; closedEpoch: string; closeRecord: string; seedChange: string; inheritedEpoch: string | null; locallyInstalled: boolean }>;
export type FaultView = Readonly<{
  scope: { kind: "source" } | { kind: "registryBucket"; bucket: number };
  phase: "open" | "closing" | "settled" | "fault";
  candidates: readonly FaultCandidate[];
  mayDecide: boolean; blockedBy: string | null; waiting: number; preservedOperations: number;
  repairing: boolean; lastOutcome: string; lastDetail: string;
}>;
const fail = (): never => { throw new StudioNativeError("invalid fault response"); };
const hash = (v: unknown): string => typeof v === "string" && /^[a-f0-9]{64}$/.test(v) ? v : fail();
const decimal = (v: unknown): string => typeof v === "string" && /^(0|[1-9][0-9]*)$/.test(v) && BigInt(v) <= (1n << 64n) - 1n ? v : fail();
const count = (v: unknown): number => Number.isSafeInteger(v) && (v as number) >= 0 ? v as number : fail();
const flag = (v: unknown): boolean => typeof v === "boolean" ? v : fail();
const record = (v: unknown): Record<string, unknown> => v !== null && typeof v === "object" && !Array.isArray(v) ? v as Record<string, unknown> : fail();

/** Parse only native observation evidence; imported receipt/package labels grant no authority. */
export function parseFaultView(value: unknown, target: RecoveryTargetArgs, expected: FaultScope): FaultView {
  const v = record(value), s = record(v.scope), source = record(v.source);
  if (v.v !== 1 || v.kind !== "fault" || v.channel !== target.channel || v.object !== target.object ||
      s.kind !== (expected === "source" ? "source" : "registryBucket")) fail();
  const phase = source.phase;
  if (phase !== "open" && phase !== "closing" && phase !== "settled" && phase !== "fault") fail();
  const candidates = v.candidates === null ? [] : Array.isArray(v.candidates) && v.candidates.length === 2 ? v.candidates.map(x => {
    const c = record(x);
    return { receipt: hash(c.receipt), closedEpoch: decimal(c.closedEpoch), closeRecord: hash(c.closeRecord), seedChange: hash(c.seedChange), inheritedEpoch: c.inheritedEpoch === null ? null : decimal(c.inheritedEpoch), locallyInstalled: flag(c.locallyInstalled) };
  }) : fail();
  if (candidates.length === 2 && candidates[0].receipt === candidates[1].receipt) fail();
  const mayDecide = flag(v.mayDecide);
  if (mayDecide && candidates.length !== 2) fail();
  const blocked = v.blockedBy;
  if (blocked !== null && !["notOwner", "tenureUnverified", "tenureUnobserved", "heldRepair", "noFault", "scheduled"].includes(blocked as string)) fail();
  const repair = v.repair === null ? null : record(v.repair), attempt = v.lastAttempt === null ? null : record(v.lastAttempt);
  return { scope: expected === "source" ? { kind: "source" } : { kind: "registryBucket", bucket: count(s.bucket) }, phase: phase as FaultView["phase"], candidates,
    mayDecide, blockedBy: blocked as string | null, waiting: count(v.waiting), preservedOperations: count(v.preservedOperations), repairing: repair !== null && flag(repair.held),
    lastOutcome: attempt && typeof attempt.outcome === "string" ? attempt.outcome : "",
    // Native diagnostic text is untrusted display data. Bound it and let Svelte escape it;
    // no failure/hold message may be interpreted as permission to retry or select a winner.
    lastDetail: attempt && typeof attempt.error === "string" ? attempt.error.slice(0, 2048)
      : attempt && typeof attempt.hold === "string" ? attempt.hold.slice(0, 256) : "" };
}

export async function readStudioFault(ipc: StudioIpc, target: RecoveryTargetArgs, scope: FaultScope): Promise<FaultView> {
  return parseFaultView(await ipc.invoke("studio_fault_read", { ...target, scope }), target, scope);
}
export async function repairStudioFault(ipc: StudioIpc, target: RecoveryTargetArgs, scope: FaultScope, view: FaultView, selected: string): Promise<"scheduled" | "busy"> {
  if (!view.mayDecide || view.candidates.length !== 2 || !view.candidates.some(c => c.receipt === selected)) throw new StudioNativeError("choose a current decidable receipt");
  const decision = { receiptA: view.candidates[0].receipt, receiptB: view.candidates[1].receipt, selected };
  const v = record(await ipc.invoke("studio_fault_repair", { ...target, scope, decision }));
  const s = record(v.scope);
  if (v.v !== 1 || v.kind !== "faultRepairStarted" || v.channel !== target.channel || v.refreshRequired !== true ||
      s.kind !== view.scope.kind || (s.kind === "registryBucket" && s.bucket !== (view.scope as { bucket: number }).bucket) ||
      (v.start !== "scheduled" && v.start !== "busy")) fail();
  return v.start as "scheduled" | "busy";
}
