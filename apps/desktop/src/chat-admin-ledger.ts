/** Privileged retained-original reads. Never substitute the public moderation feed or current
 * messages for an original. A reader belongs to one immutable native server/UI/role mount.
 * The first production consumer is deliberately legacy-only: typed/owner activation requires
 * its actual codec/authority integration, and unknown variants fail closed here. */
export type LegacyLedgerRow = {
  kind: "legacy-change"; rowId: string; opHash: string; namespaceGroupId: string; channel: string;
  authorPublicKey: string | null; authorId: string | null; authorFingerprint: string | null;
  claimedKeyByteCount: number; keyShape: "ed25519" | "invalid-length";
  signedDocType: number; signedDocId: string; scope: "matching" | "mismatched";
  signature: "valid" | "invalid" | "pending" | "not-checked";
  groupBinding: "not-signed"; authority: "legacy-unproven"; changeHash: string | null;
  graph: "applied" | "pending-dependencies" | "unavailable";
  presentedMs: number | null; preview: { text: string; truncated: boolean } | null;
};
export type LedgerCoverage = {
  source: "retained-local-inventory"; typed: "not-active"; legacy: "retained-only" | "incomplete";
  ownerEvidence: "not-active"; unattributedChanges: "present" | "absent" | "unknown";
};
export type LedgerUnavailableCode = "not-authorized" | "session-unavailable" | "server-unavailable"
  | "server-superseded" | "profile-unavailable" | "evidence-unavailable" | "busy" | "invalid-request";
export type LedgerRefusal = { status: "stale"; revision: string }
  | { status: "unavailable"; code: LedgerUnavailableCode };
export type LedgerPage = {
  status: "page"; availability: "complete" | "incomplete"; revision: string;
  coverage: LedgerCoverage; rows: LegacyLedgerRow[]; knownRowCount: number; nextCursor: string | null;
};
export type LegacyMutation = {
  operationId: string; objectId: string;
  key: { kind: "map"; value: string } | { kind: "sequence"; elementId: string | null };
  operation: "make-map" | "make-list" | "make-text" | "make-table" | "set" | "delete"
    | "increment" | "mark" | "unmark" | "unknown";
  insert: boolean;
  value: { kind: "string" | "int" | "uint" | "float" | "bool" | "null" | "bytes"
    | "counter" | "timestamp" | "unknown"; text: string } | null;
  predecessors: string[];
};
export type LegacyDecodedChange = {
  status: "decoded" | "unsupported" | "too-large" | "invalid";
  actor: string | null; sequence: string | null; startOp: string | null;
  parents: string[]; mutations: LegacyMutation[]; mutationsTruncated: boolean;
};
export type LedgerDetail = {
  status: "detail"; availability: "complete" | "incomplete"; revision: string;
  row: LegacyLedgerRow;
  original: { encoding: "hex"; bytes: string; offset: number; totalBytes: number; nextOffset: number | null };
  typedBody: null; legacyChange: LegacyDecodedChange | null; signatureHex: string;
  factIds: string[]; missingFactIds: string[]; evidenceTruncated: boolean;
};

const encoder = new TextEncoder();
const MAX_REPLY_BYTES = 1024 * 1024;
const UNAVAILABLE = ["not-authorized", "session-unavailable", "server-unavailable", "server-superseded",
  "profile-unavailable", "evidence-unavailable", "busy", "invalid-request"];
const SCOPE_LOSS = ["not-authorized", "session-unavailable", "server-unavailable", "server-superseded"];
function requireValue(ok: boolean): asserts ok { if (!ok) throw new Error("Invalid chat history response"); }
function object(value: unknown): Record<string, unknown> {
  requireValue(value !== null && typeof value === "object" && !Array.isArray(value));
  return value as Record<string, unknown>;
}
function closed(value: unknown, keys: string): Record<string, unknown> {
  const v = object(value), expected = keys.split(" ");
  const found = Object.keys(v);
  requireValue(found.length === expected.length && found.every((key) => expected.includes(key)));
  return v;
}
function text(value: unknown, max: number): asserts value is string {
  requireValue(typeof value === "string" && value.length <= max && encoder.encode(value).length <= max);
}
function ascii(value: unknown, max: number) { text(value, max); requireValue(value.length > 0 && /^[\x21-\x7e]+$/.test(value)); }
function oneOf(value: unknown, choices: readonly string[]) { requireValue(typeof value === "string" && choices.includes(value)); }
function count(value: unknown, max = 0xffffffff): asserts value is number {
  requireValue(typeof value === "number" && Number.isSafeInteger(value) && value >= 0 && value <= max);
}
function hex(value: unknown, length = 64): asserts value is string {
  requireValue(typeof value === "string" && value.length === length && /^[0-9a-f]+$/.test(value));
}
function decimal(value: unknown, bits: number, positive = false): asserts value is string {
  text(value, 39);
  requireValue(/^(0|[1-9][0-9]*)$/.test(value) && BigInt(value) < (1n << BigInt(bits)) && (!positive || value !== "0"));
}
function signedDecimal(value: unknown) {
  text(value, 20); requireValue(/^(0|-?[1-9][0-9]*)$/.test(value));
  const n = BigInt(value); requireValue(n >= -(1n << 63n) && n < (1n << 63n));
}
function opId(value: unknown) {
  text(value, 85);
  const match = /^(0|[1-9][0-9]*)@([0-9a-f]{32}|[0-9a-f]{64})$/.exec(value);
  requireValue(!!match); decimal(match[1], 64, true);
}
function refs(value: unknown, max: number, inspect: (value: unknown) => void): asserts value is unknown[] {
  requireValue(Array.isArray(value) && value.length <= max); for (const item of value) inspect(item);
}
/** IPC already has a native 1MiB encoder cap; own a second bounded JSON copy so delayed mock/
 * bridge callbacks cannot mutate inspected records. This is not an untrusted graph traversal API. */
function envelope(value: unknown) {
  const encoded = JSON.stringify(value);
  requireValue(typeof encoded === "string" && encoded.length <= MAX_REPLY_BYTES
    && encoder.encode(encoded).length <= MAX_REPLY_BYTES);
  return object(JSON.parse(encoded));
}
function common(v: Record<string, unknown>): boolean {
  if (v.status === "unavailable") { closed(v, "status code"); oneOf(v.code, UNAVAILABLE); return true; }
  if (v.status === "stale") { closed(v, "status revision"); ascii(v.revision, 256); return true; }
  oneOf(v.availability, ["complete", "incomplete"]); ascii(v.revision, 256); return false;
}
function row(value: unknown) {
  const v = closed(value, "kind rowId opHash namespaceGroupId channel authorPublicKey authorId authorFingerprint claimedKeyByteCount keyShape signedDocType signedDocId scope signature groupBinding authority changeHash graph presentedMs preview");
  requireValue(v.kind === "legacy-change"); hex(v.opHash); decimal(v.channel, 128); decimal(v.signedDocId, 128);
  requireValue(v.rowId === `legacy:${v.channel}:${v.opHash}`);
  text(v.namespaceGroupId, 512);
  requireValue(v.namespaceGroupId.length >= 2 && v.namespaceGroupId.length % 2 === 0 && /^[0-9a-f]+$/.test(v.namespaceGroupId));
  count(v.claimedKeyByteCount); count(v.signedDocType, 0xffff);
  oneOf(v.keyShape, ["ed25519", "invalid-length"]);
  if (v.claimedKeyByteCount === 32) {
    requireValue(v.keyShape === "ed25519"); hex(v.authorPublicKey); hex(v.authorId); hex(v.authorFingerprint, 8);
    requireValue(v.authorId.startsWith(v.authorFingerprint));
  } else {
    requireValue(v.keyShape === "invalid-length" && v.authorPublicKey === null && v.authorId === null && v.authorFingerprint === null);
  }
  oneOf(v.scope, ["matching", "mismatched"]);
  requireValue((v.scope === "matching") === (v.signedDocType === 1 && v.signedDocId === v.channel));
  oneOf(v.signature, ["valid", "invalid", "pending", "not-checked"]);
  requireValue(v.signature !== "valid" || v.claimedKeyByteCount === 32);
  requireValue(v.groupBinding === "not-signed" && v.authority === "legacy-unproven");
  if (v.changeHash !== null) hex(v.changeHash);
  oneOf(v.graph, ["applied", "pending-dependencies", "unavailable"]);
  requireValue(v.presentedMs === null || (typeof v.presentedMs === "number" && Number.isSafeInteger(v.presentedMs)
    && Math.abs(v.presentedMs) <= 8_640_000_000_000_000));
  if (v.preview !== null) { const p = closed(v.preview, "text truncated"); text(p.text, 512); requireValue(typeof p.truncated === "boolean"); }
}
export function parseLedgerPage(value: unknown): LedgerPage | LedgerRefusal {
  const v = envelope(value); if (common(v)) return v as LedgerRefusal;
  closed(v, "status availability revision coverage rows knownRowCount nextCursor");
  requireValue(v.status === "page");
  const coverage = closed(v.coverage, "source typed legacy ownerEvidence unattributedChanges");
  requireValue(coverage.source === "retained-local-inventory" && coverage.typed === "not-active" && coverage.ownerEvidence === "not-active");
  oneOf(coverage.legacy, ["retained-only", "incomplete"]); oneOf(coverage.unattributedChanges, ["present", "absent", "unknown"]);
  requireValue(v.availability !== "complete" || (coverage.legacy === "retained-only" && coverage.unattributedChanges !== "unknown"));
  requireValue(Array.isArray(v.rows) && v.rows.length <= 32); for (const item of v.rows) row(item);
  const rows = v.rows as LegacyLedgerRow[];
  requireValue(rows.every((item, index) => !index || rows[index - 1].rowId < item.rowId));
  requireValue(rows.every((item) => item.namespaceGroupId === rows[0]?.namespaceGroupId));
  count(v.knownRowCount); requireValue(v.knownRowCount >= rows.length);
  if (v.nextCursor !== null) { ascii(v.nextCursor, 512); requireValue(rows.length > 0 && v.knownRowCount > rows.length); }
  return v as LedgerPage;
}
function mutation(value: unknown): number {
  const v = closed(value, "operationId objectId key operation insert value predecessors");
  opId(v.operationId); if (v.objectId !== "_root") opId(v.objectId);
  const key = object(v.key);
  if (key.kind === "map") { closed(key, "kind value"); text(key.value, 64 * 1024); }
  else { closed(key, "kind elementId"); requireValue(key.kind === "sequence"); if (key.elementId !== null) opId(key.elementId); }
  oneOf(v.operation, ["make-map", "make-list", "make-text", "make-table", "set", "delete", "increment", "mark", "unmark", "unknown"]);
  requireValue(typeof v.insert === "boolean");
  if (v.value !== null) {
    const scalar = closed(v.value, "kind text"); oneOf(scalar.kind, ["string", "int", "uint", "float", "bool", "null", "bytes", "counter", "timestamp", "unknown"]);
    text(scalar.text, 64 * 1024);
    if (scalar.kind === "uint") decimal(scalar.text, 64);
    if (["int", "counter", "timestamp"].includes(scalar.kind as string)) signedDecimal(scalar.text);
    if (scalar.kind === "bool") oneOf(scalar.text, ["true", "false"]);
    if (scalar.kind === "null") requireValue(scalar.text === "null");
    if (scalar.kind === "bytes") requireValue(scalar.text.length % 2 === 0 && /^[0-9a-f]*$/.test(scalar.text));
  }
  refs(v.predecessors, 4096, opId);
  const fields = [v.operationId, v.objectId, v.operation,
    ...(key.kind === "map" ? [key.value] : key.elementId === null ? [] : [key.elementId]),
    ...(v.value === null ? [] : [object(v.value).kind, object(v.value).text]), ...v.predecessors];
  return fields.reduce<number>((sum, field) => sum + encoder.encode(field as string).length, 0);
}
function decoded(value: unknown) {
  const v = closed(value, "status actor sequence startOp parents mutations mutationsTruncated");
  oneOf(v.status, ["decoded", "unsupported", "too-large", "invalid"]);
  requireValue(typeof v.mutationsTruncated === "boolean"); refs(v.parents, 128, (id) => hex(id));
  requireValue(Array.isArray(v.mutations) && v.mutations.length <= 512);
  let expandedBytes = 0;
  for (const item of v.mutations) {
    expandedBytes += mutation(item); requireValue(expandedBytes <= 64 * 1024);
  }
  if (v.status === "decoded") {
    text(v.actor, 64); requireValue(/^(?:[0-9a-f]{32}|[0-9a-f]{64})$/.test(v.actor));
    decimal(v.sequence, 64, true); decimal(v.startOp, 64, true);
  } else {
    requireValue(v.actor === null && v.sequence === null && v.startOp === null && v.parents.length === 0 && v.mutations.length === 0 && !v.mutationsTruncated);
  }
  // Match the native DTO's separate expansion invoice, including canonical identifiers and
  // enum/value text. A collection of individually small fields must not bypass the total cap.
  for (const field of [v.actor, v.sequence, v.startOp, ...v.parents]) {
    if (field !== null) expandedBytes += encoder.encode(field as string).length;
  }
  requireValue(expandedBytes <= 64 * 1024);
}
export function parseLedgerDetail(value: unknown): LedgerDetail | LedgerRefusal {
  const v = envelope(value); if (common(v)) return v as LedgerRefusal;
  closed(v, "status availability revision row original typedBody legacyChange signatureHex factIds missingFactIds evidenceTruncated");
  requireValue(v.status === "detail" && v.typedBody === null); row(v.row); hex(v.signatureHex, 128);
  refs(v.factIds, 128, (id) => hex(id)); refs(v.missingFactIds, 128, (id) => hex(id)); requireValue(typeof v.evidenceTruncated === "boolean");
  const chunk = closed(v.original, "encoding bytes offset totalBytes nextOffset");
  requireValue(chunk.encoding === "hex"); text(chunk.bytes, 128 * 1024);
  requireValue(chunk.bytes.length % 2 === 0 && /^[0-9a-f]*$/.test(chunk.bytes));
  count(chunk.offset, Number.MAX_SAFE_INTEGER); count(chunk.totalBytes, Number.MAX_SAFE_INTEGER);
  const next = chunk.offset + chunk.bytes.length / 2;
  requireValue(Number.isSafeInteger(next) && next <= chunk.totalBytes);
  requireValue(chunk.nextOffset === (next === chunk.totalBytes ? null : next));
  requireValue(chunk.bytes.length > 0 || chunk.offset === chunk.totalBytes);
  if (v.legacyChange !== null) decoded(v.legacyChange);
  return v as LedgerDetail;
}

function sameRow(a: LegacyLedgerRow, b: LegacyLedgerRow): boolean {
  const { preview: ap, ...af } = a, { preview: bp, ...bf } = b;
  // JSON field order is not protocol authority. Compare exact semantic fields, including all
  // scope/proof labels, rather than rejecting identical replies whose object keys reordered.
  return (Object.keys(af) as (keyof typeof af)[]).every((key) => af[key] === bf[key])
    && (ap === null ? bp === null : bp !== null && ap.text === bp.text && ap.truncated === bp.truncated);
}
function sameCoverage(a: LedgerCoverage, b: LedgerCoverage) {
  return a.source === b.source && a.typed === b.typed && a.legacy === b.legacy
    && a.ownerEvidence === b.ownerEvidence && a.unattributedChanges === b.unattributedChanges;
}

export type LedgerReads = {
  page: (cursor: string | null) => Promise<unknown>;
  detail: (rowId: string, revision: string, frameOffset: number) => Promise<unknown>;
};
export type LedgerSnapshot = {
  page: LedgerPage | null; detail: LedgerDetail | null;
  loading: boolean; detailLoading: boolean; stale: boolean; message: string;
};
type LoadOutcome = "loaded" | "busy" | "refused" | "ignored";
/** Bounded current page and ONE current original chunk only. Never accumulate every page,
 * trust knownRowCount as an array size, or silently concatenate evidence across revisions. */
export class ChatLedgerReader {
  #reads: LedgerReads; #changed: (snapshot: LedgerSnapshot) => void;
  #alive = true; #lost = false; #pageRequest = 0; #detailRequest = 0;
  #state: LedgerSnapshot = { page: null, detail: null, loading: false, detailLoading: false, stale: false, message: "" };
  constructor(reads: LedgerReads, changed: (snapshot: LedgerSnapshot) => void) { this.#reads = reads; this.#changed = changed; }
  get state() { return this.#state; }
  #set(update: Partial<LedgerSnapshot>) { this.#state = { ...this.#state, ...update }; this.#changed(this.#state); }
  dispose() {
    this.#alive = false; this.#pageRequest++; this.#detailRequest++;
    this.#set({ page: null, detail: null, loading: false, detailLoading: false, stale: false, message: "" });
  }
  closeDetail() { this.#detailRequest++; this.#set({ detail: null, detailLoading: false }); }
  #refused(reply: LedgerRefusal): LoadOutcome {
    if (reply.status === "unavailable" && SCOPE_LOSS.includes(reply.code)) {
      this.#lost = true; this.#pageRequest++; this.#detailRequest++;
      this.#set({ page: null, detail: null, loading: false, detailLoading: false, stale: false,
        message: "History is unavailable. Reopen the admin panel to try again." });
    } else if (reply.status === "unavailable" && reply.code === "busy") {
      this.#set({ message: "Loading history…" }); return "busy";
    } else {
      this.#set({ detail: null, stale: !!this.#state.page, message: reply.status === "stale"
        ? "History changed. Refresh to continue." : "Couldn't load history. Try again." });
    }
    return "refused";
  }
  async load(cursor: string | null = null): Promise<LoadOutcome> {
    if (!this.#alive || this.#lost) return "ignored";
    const request = ++this.#pageRequest; this.#detailRequest++;
    const previous = cursor === null ? null : this.#state.page;
    this.#set({ loading: true, detail: null, detailLoading: false,
      stale: cursor === null ? !!this.#state.page : this.#state.stale, message: "" });
    try {
      const raw = await this.#reads.page(cursor);
      if (!this.#alive || this.#lost) return "ignored";
      const reply = parseLedgerPage(raw);
      // An OLDER negative authority result still erases a newer positive projection. Request
      // ordering may reject obsolete positives; it must never hide observed scope retirement.
      if (reply.status === "unavailable" && SCOPE_LOSS.includes(reply.code)) return this.#refused(reply);
      if (request !== this.#pageRequest) return "ignored";
      if (reply.status !== "page") return this.#refused(reply);
      if (cursor !== null && (!previous || reply.revision !== previous.revision
        || reply.knownRowCount !== previous.knownRowCount || !sameCoverage(reply.coverage, previous.coverage)
        || (!!reply.rows.length && !!previous.rows.length && reply.rows[0].rowId <= previous.rows.at(-1)!.rowId))) {
        return this.#refused({ status: "stale", revision: reply.revision });
      }
      this.#set({ page: reply, stale: false, message: "" }); return "loaded";
    } catch {
      if (this.#alive && !this.#lost && request === this.#pageRequest) this.#set({ detail: null, stale: !!this.#state.page, message: "Couldn't load history. Try again." });
      return "refused";
    } finally { if (this.#alive && !this.#lost && request === this.#pageRequest) this.#set({ loading: false }); }
  }
  async open(rowId: string, offset = 0): Promise<LoadOutcome> {
    const page = this.#state.page, prior = this.#state.detail;
    if (!this.#alive || this.#lost || !page || this.#state.stale || this.#state.loading) return "ignored";
    const expected = page.rows.find((row) => row.rowId === rowId); if (!expected) return "ignored";
    if (offset !== 0 && (!prior || prior.row.rowId !== rowId || prior.revision !== page.revision || prior.original.nextOffset !== offset)) return "ignored";
    const request = ++this.#detailRequest;
    this.#set({ detail: null, detailLoading: true, message: "" });
    try {
      const raw = await this.#reads.detail(rowId, page.revision, offset);
      if (!this.#alive || this.#lost) return "ignored";
      const reply = parseLedgerDetail(raw);
      if (reply.status === "unavailable" && SCOPE_LOSS.includes(reply.code)) return this.#refused(reply);
      if (request !== this.#detailRequest) return "ignored";
      if (reply.status !== "detail") {
        const outcome = this.#refused(reply);
        if (outcome === "busy" && prior) this.#set({ detail: prior });
        return outcome;
      }
      if (reply.revision !== page.revision || !sameRow(reply.row, expected)
        || reply.original.offset !== offset || (offset !== 0 && prior
          && (reply.original.totalBytes !== prior.original.totalBytes || reply.signatureHex !== prior.signatureHex))) {
        return this.#refused({ status: "stale", revision: reply.revision });
      }
      this.#set({ detail: reply }); return "loaded";
    } catch {
      if (this.#alive && !this.#lost && request === this.#detailRequest) this.#set({ message: "Couldn't load this original. Try again." });
      return "refused";
    } finally { if (this.#alive && !this.#lost && request === this.#detailRequest) this.#set({ detailLoading: false }); }
  }
}
