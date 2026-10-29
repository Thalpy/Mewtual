/**
 * Privileged review reads for the moderation log. These labels describe native evidence;
 * none of them grants permission or changes chat history. Keep this API separate from Msg
 * and the public moderation/vote getter: a held body must never leak through their fallback.
 * See CHAT-ADMIN-READ-CONTRACT.md for the native authorization and covering-save boundary.
 */
export type CaseState = "held" | "approved" | "rejected" | "reopened" | "awaiting-proof" | "conflict";
export type ActionKind = "create" | "edit" | "delete" | "join";
export type AuditProof = {
  signature: "valid" | "invalid" | "pending";
  authority: "owner-attested" | "invalid" | "pending";
  selection: "final" | "unselected" | "pending" | "conflict";
};
export type CaseRef = {
  caseId: string; actionCid: string; createCid: string | null;
  realm: string; groupId: string; channel: string;
  removalCutCid: string; admissionId: string; basisCid: string; grantCid: string;
  authorPublicKey: string; authorId: string; authorFingerprint: string;
  selectedDecisionId: string | null;
};
export type ActionCaseRow = CaseRef & {
  kind: "held-action"; rowId: string; actionKind: ActionKind; state: CaseState;
  presentedMs: number | null; preview: { text: string; truncated: boolean } | null;
  evidenceManifestCid: string; proof: AuditProof;
};
export type ReviewDecisionRow = CaseRef & {
  kind: "review-decision"; rowId: string; decisionId: string;
  recordKind: "recommendation" | "decision"; choice: "approve" | "reject" | "reopen";
  state: CaseState; issuerPublicKey: string; issuerId: string; issuerFingerprint: string;
  authorityBasis: string; predecessors: string[]; presentedMs: number | null;
  reason: string; evidenceManifestCid: string;
  effect: "advisory" | "unselected" | "selected" | "pending" | "conflict" | "invalid";
  proof: AuditProof;
};
export type AdminAuditRow = ActionCaseRow | ReviewDecisionRow;
export const CASE_STATE_LABELS: Record<CaseState, string> = {
  held: "Held for review", approved: "Approved", rejected: "Rejected", reopened: "Reopened",
  "awaiting-proof": "Waiting for evidence", conflict: "Conflicting evidence",
};
/** A retained native outcome label is not by itself evidence of a verified final decision. */
export function caseStateLabel(row: Pick<ActionCaseRow, "state" | "proof">): string {
  if (row.state === "conflict" || row.proof.selection === "conflict") return "Conflicting evidence";
  if (["approved", "rejected", "reopened"].includes(row.state)) {
    if (row.proof.signature === "invalid" || row.proof.authority === "invalid") return "Unverified outcome";
    if (row.proof.signature !== "valid" || row.proof.authority !== "owner-attested"
      || row.proof.selection !== "final") return "Waiting for evidence";
  }
  return CASE_STATE_LABELS[row.state];
}
/** A proposed approval must never look like an applied decision. Display only; no authority. */
export function reviewEffectLabel(row: ReviewDecisionRow): string {
  if (row.effect === "selected") return row.proof.signature === "valid"
    && row.proof.authority === "owner-attested" && row.proof.selection === "final" ? "Applied" : "Unverified decision";
  return { advisory: "Recommendation", unselected: "Not applied", pending: "Waiting for evidence",
    conflict: "Conflicting decisions", invalid: "Invalid · ignored" }[row.effect];
}
export type ReadUnavailableCode =
  | "not-authorized" | "session-unavailable" | "server-unavailable" | "server-superseded"
  | "profile-unavailable" | "evidence-unavailable" | "busy" | "invalid-request";
export type ReadUnavailable = { status: "unavailable"; code: ReadUnavailableCode };
export type StaleRead = { status: "stale"; revision: string };
export type AdminReviewPageReply =
  | { status: "page"; availability: "complete" | "incomplete"; revision: string;
      rows: AdminAuditRow[]; knownRowCount: number; nextCursor: string | null }
  | StaleRead | ReadUnavailable;
export type HeldActionBody =
  | { kind: "create"; text: string; replyTo: string | null }
  | { kind: "edit"; createCid: string; text: string }
  | { kind: "delete"; createCid: string }
  | { kind: "join"; createCid: string };
export type HeldActionDetailReply =
  | { status: "detail"; availability: "complete" | "incomplete"; revision: string;
      case: ActionCaseRow; action: HeldActionBody; signedActionHex: string;
      signatureHex: string; factIds: string[]; missingFactIds: string[]; evidenceTruncated: boolean }
  | StaleRead | ReadUnavailable;

const STATES = ["held", "approved", "rejected", "reopened", "awaiting-proof", "conflict"];
const KINDS = ["create", "edit", "delete", "join"];
const UNAVAILABLE = ["not-authorized", "session-unavailable", "server-unavailable", "server-superseded",
  "profile-unavailable", "evidence-unavailable", "busy", "invalid-request"];
const SCOPE_LOSS = ["not-authorized", "session-unavailable", "server-unavailable", "server-superseded"];
const encoder = new TextEncoder();
const MAX_RESPONSE_BYTES = 1024 * 1024;
function requireValue(ok: boolean): asserts ok { if (!ok) throw new Error("Invalid review history response"); }
function object(value: unknown): Record<string, unknown> {
  requireValue(value !== null && typeof value === "object" && !Array.isArray(value));
  return value as Record<string, unknown>;
}
function text(value: unknown, max: number): asserts value is string {
  requireValue(typeof value === "string" && encoder.encode(value).length <= max);
}
function hex(value: unknown, length = 64): asserts value is string {
  requireValue(typeof value === "string" && value.length === length && /^[0-9a-f]+$/.test(value));
}
function nullableHex(value: unknown) { if (value !== null) hex(value); }
function oneOf(value: unknown, choices: readonly string[]) { requireValue(typeof value === "string" && choices.includes(value)); }
function ascii(value: unknown, max: number) { text(value, max); requireValue(value.length > 0 && /^[\x21-\x7e]+$/.test(value)); }
function refs(value: unknown, max: number) {
  requireValue(Array.isArray(value) && value.length <= max);
  for (const item of value) hex(item);
}
function proof(value: unknown) {
  const v = object(value);
  oneOf(v.signature, ["valid", "invalid", "pending"]);
  oneOf(v.authority, ["owner-attested", "invalid", "pending"]);
  oneOf(v.selection, ["final", "unselected", "pending", "conflict"]);
}
function row(value: unknown) {
  const v = object(value);
  for (const field of ["caseId", "actionCid", "realm", "removalCutCid", "admissionId", "basisCid",
    "grantCid", "authorPublicKey", "authorId", "evidenceManifestCid"]) hex(v[field]);
  nullableHex(v.createCid); nullableHex(v.selectedDecisionId);
  hex(v.authorFingerprint, 8);
  requireValue((v.authorId as string).startsWith(v.authorFingerprint as string));
  text(v.groupId, 512);
  requireValue(v.groupId.length >= 2 && v.groupId.length % 2 === 0 && /^[0-9a-f]+$/.test(v.groupId));
  text(v.channel, 39);
  requireValue(/^(0|[1-9][0-9]*)$/.test(v.channel) && BigInt(v.channel) <= (1n << 128n) - 1n);
  oneOf(v.state, STATES); proof(v.proof);
  requireValue(v.presentedMs === null || (typeof v.presentedMs === "number"
    && Number.isSafeInteger(v.presentedMs) && Math.abs(v.presentedMs) <= 8_640_000_000_000_000));
  if (v.kind === "held-action") {
    requireValue(v.rowId === `held:${v.caseId}`); oneOf(v.actionKind, KINDS);
    if (v.preview !== null) {
      const p = object(v.preview); text(p.text, 512); requireValue(typeof p.truncated === "boolean");
    }
  } else {
    requireValue(v.kind === "review-decision");
    for (const field of ["decisionId", "issuerPublicKey", "issuerId", "authorityBasis"]) hex(v[field]);
    hex(v.issuerFingerprint, 8);
    requireValue((v.issuerId as string).startsWith(v.issuerFingerprint as string));
    requireValue(v.rowId === `decision:${v.decisionId}`);
    oneOf(v.recordKind, ["recommendation", "decision"]); oneOf(v.choice, ["approve", "reject", "reopen"]);
    oneOf(v.effect, ["advisory", "unselected", "selected", "pending", "conflict", "invalid"]);
    refs(v.predecessors, 32); text(v.reason, 2048);
  }
}
/** Own a bounded JSON copy; a delayed callback cannot mutate an already inspected page. */
function envelope(value: unknown) {
  const encoded = JSON.stringify(value);
  requireValue(typeof encoded === "string" && encoder.encode(encoded).length <= MAX_RESPONSE_BYTES);
  return object(JSON.parse(encoded));
}
function common(v: Record<string, unknown>): boolean {
  if (v.status === "unavailable") { oneOf(v.code, UNAVAILABLE); return true; }
  if (v.status === "stale") { ascii(v.revision, 256); return true; }
  oneOf(v.availability, ["complete", "incomplete"]); ascii(v.revision, 256);
  return false;
}
export function parseAdminReviewPage(value: unknown): AdminReviewPageReply {
  const v = envelope(value);
  if (common(v)) return v as AdminReviewPageReply;
  requireValue(v.status === "page" && Array.isArray(v.rows) && v.rows.length <= 32);
  requireValue(typeof v.knownRowCount === "number" && Number.isInteger(v.knownRowCount)
    && v.knownRowCount >= v.rows.length && v.knownRowCount <= 0xffff_ffff);
  let previous = "";
  for (const item of v.rows) { row(item); const id = object(item).rowId as string; requireValue(id > previous); previous = id; }
  if (v.nextCursor !== null) { ascii(v.nextCursor, 512); requireValue(v.rows.length > 0); }
  return v as AdminReviewPageReply;
}
export function parseHeldActionDetail(value: unknown): HeldActionDetailReply {
  const v = envelope(value);
  if (common(v)) return v as HeldActionDetailReply;
  requireValue(v.status === "detail"); row(v.case);
  const c = object(v.case); requireValue(c.kind === "held-action");
  const a = object(v.action); oneOf(a.kind, KINDS); requireValue(a.kind === c.actionKind);
  if (a.kind === "create" || a.kind === "edit") text(a.text, 128 * 1024);
  if (a.kind === "create") nullableHex(a.replyTo);
  else { hex(a.createCid); requireValue(a.createCid === c.createCid); }
  text(v.signedActionHex, 256 * 1024);
  requireValue(v.signedActionHex.length > 0 && v.signedActionHex.length % 2 === 0 && /^[0-9a-f]+$/.test(v.signedActionHex));
  hex(v.signatureHex, 128); refs(v.factIds, 128); refs(v.missingFactIds, 128);
  requireValue(typeof v.evidenceTruncated === "boolean");
  return v as HeldActionDetailReply;
}

export type AuditSnapshot = {
  page: Extract<AdminReviewPageReply, { status: "page" }> | null;
  detail: Extract<HeldActionDetailReply, { status: "detail" }> | null;
  loading: boolean; detailLoading: boolean; stale: boolean; message: string;
};
export type AuditReads = {
  page: (cursor: string | null) => Promise<unknown>;
  detail: (caseId: string, expectedRevision: string) => Promise<unknown>;
};
/** One mounted, authorized server view only. Disposal permanently fences every late reply. */
export class ChatReviewAuditReader {
  #reads: AuditReads;
  #changed: (state: AuditSnapshot) => void;
  #alive = true;
  #pageRequest = 0;
  #detailRequest = 0;
  #state: AuditSnapshot = { page: null, detail: null, loading: false, detailLoading: false, stale: false, message: "" };
  constructor(reads: AuditReads, changed: (state: AuditSnapshot) => void) { this.#reads = reads; this.#changed = changed; }
  get state() { return this.#state; }
  #set(update: Partial<AuditSnapshot>) { this.#state = { ...this.#state, ...update }; this.#changed(this.#state); }
  dispose() {
    this.#alive = false; this.#pageRequest++; this.#detailRequest++;
    this.#set({ page: null, detail: null, loading: false, detailLoading: false, stale: false, message: "" });
  }
  closeDetail() { this.#detailRequest++; this.#set({ detail: null, detailLoading: false }); }
  #refused(reply: StaleRead | ReadUnavailable) {
    // Authority/identity loss is not a transient stale cache. Erase bodies immediately,
    // and invalidate another read that may have started while this refusal was in flight.
    if (reply.status === "unavailable" && SCOPE_LOSS.includes(reply.code)) {
      this.#pageRequest++; this.#detailRequest++;
      this.#set({ page: null, detail: null, loading: false, detailLoading: false, stale: false, message: "Review history is unavailable." });
    } else {
      this.#set({ detail: null, stale: !!this.#state.page, message: reply.status === "stale"
        ? "History changed. Refresh to continue." : "Review history could not be loaded." });
    }
  }
  async load(cursor: string | null = null) {
    if (!this.#alive) return;
    const request = ++this.#pageRequest;
    this.#detailRequest++;
    const revision = cursor === null ? null : this.#state.page?.revision;
    this.#set({ loading: true, detail: null, detailLoading: false, message: "" });
    try {
      const reply = parseAdminReviewPage(await this.#reads.page(cursor));
      if (!this.#alive) return;
      // A newer outstanding read cannot hide observed loss of this ORIGINAL scope.
      // Erase even an older refusal and fence every already-dispatched positive reply.
      if (reply.status === "unavailable" && SCOPE_LOSS.includes(reply.code)) { this.#refused(reply); return; }
      if (request !== this.#pageRequest) return;
      if (reply.status !== "page") { this.#refused(reply); return; }
      // A native bug must not stitch a next page from a different immutable view into this one.
      if (cursor !== null && (!revision || reply.revision !== revision)) {
        this.#refused({ status: "stale", revision: reply.revision }); return;
      }
      this.#set({ page: reply, stale: false, message: "" });
    } catch {
      if (this.#alive && request === this.#pageRequest) this.#set({ stale: !!this.#state.page, message: "Review history could not be loaded." });
    } finally {
      if (this.#alive && request === this.#pageRequest) this.#set({ loading: false });
    }
  }
  async open(caseId: string) {
    const page = this.#state.page;
    if (!this.#alive || !page || this.#state.stale || this.#state.loading) return;
    // Only cases in this exact privileged page may become detail requests. A chat message
    // ID, stale decision or ordinary member-provided link is never a held-body lookup.
    if (!page.rows.some((row) => row.caseId === caseId)) return;
    const request = ++this.#detailRequest;
    this.#set({ detail: null, detailLoading: true, message: "" });
    try {
      const reply = parseHeldActionDetail(await this.#reads.detail(caseId, page.revision));
      if (!this.#alive) return;
      if (reply.status === "unavailable" && SCOPE_LOSS.includes(reply.code)) { this.#refused(reply); return; }
      if (request !== this.#detailRequest) return;
      if (reply.status !== "detail") { this.#refused(reply); return; }
      if (reply.revision !== page.revision || reply.case.caseId !== caseId) {
        this.#refused({ status: "stale", revision: reply.revision }); return;
      }
      const expected = page.rows.find((row) => row.caseId === caseId)!;
      if (reply.case.actionCid !== expected.actionCid || reply.case.realm !== expected.realm
        || reply.case.groupId !== expected.groupId || reply.case.channel !== expected.channel
        || reply.case.admissionId !== expected.admissionId || reply.case.removalCutCid !== expected.removalCutCid
        || reply.case.createCid !== expected.createCid || reply.case.basisCid !== expected.basisCid
        || reply.case.grantCid !== expected.grantCid || reply.case.authorPublicKey !== expected.authorPublicKey
        || reply.case.authorId !== expected.authorId || reply.case.selectedDecisionId !== expected.selectedDecisionId) {
        throw new Error("Mismatched held action");
      }
      this.#set({ detail: reply });
    } catch {
      if (this.#alive && request === this.#detailRequest) this.#set({ message: "The held message could not be loaded." });
    } finally {
      if (this.#alive && request === this.#detailRequest) this.#set({ detailLoading: false });
    }
  }
}
