# Full retained chat ledger: privileged read contract

Proposed extension for independent review, 29 October 2026. The user wants the FULL retained
chat ledger in the existing admin panel, including original posts, every edit/deletion, concurrent
branches and decisions. Ordinary concurrent actions merge automatically and non-destructively;
removed-member unproven late actions remain the unique review exception. An owner-finality breach
is a separate conditional-profile failure, never a request for users to repair ordinary chat forks.
No runtime or production activation is implied by this document.

The existing [held-review page/detail contract](CHAT-ADMIN-READ-CONTRACT.md) is unchanged. Its
`CaseRef` requires an actual removal cut/case and MUST NOT be fabricated for an ordinary action.
This full-ledger view has separate ordinary action references. A nullable real review-case link
opens the existing exact held-detail path when applicable.

## Commands and common read boundary

Main-window literal commands, camelCase arguments and replies:

- `get_admin_chat_ledger({ server, cursor: null | string, limit })`, limit integer 1..32.
- `get_admin_chat_ledger_detail({ server, rowId, expectedRevision, frameOffset })`, offset a safe
  nonnegative integer, initially zero. The same exact row/revision must be used for every chunk.

Both require current full-key Owner/Admin reader permission and the visible/unlocked original
session/Actor/store custody. They do not reuse all-member `get_moderation` or `get_messages`.
Authorization/session/replacement failure takes precedence over cursor/stale/revision metadata.
Role changes after authoritative Actor materialization have the already-admitted-read boundary
specified by the held contract; original native session/incarnation and frontend role/view guards
still reject stale replies. Neither public event payloads nor normal chat DTOs gain privileged bodies.

Common fixed IDs, full device IDs, signatures, groups, channels, nullable presentation timestamps,
visible-ASCII revision/cursor tokens, 1 MiB encoded reply limit, request validation, explicit stale
reset, cursor expiry/incarnation/policy binding and unavailable codes follow the held contract.
Canonical page order is ascending ASCII `rowId`, NEVER a timestamp/arrival authority rule.
No read returns partial rows with an implicit completeness claim.

## Exact row DTOs

The fields below are closed. Fixed-key/digest strings use the existing lowercase hex grammar.
All counts are safe nonnegative integers at most `u32::MAX`; native refuses larger inventories
instead of rounding. `preview` is null or `{ text: string; truncated: boolean }`, at most512 UTF-8
bytes, scalar-boundary truncated. Preview strings are presentation, not reconstructed author proof.

```ts
type ActionRef = {
  actionCid: string; createCid: string | null;
  realm: string; groupId: string; channel: string;
  authorPublicKey: string; authorId: string; authorFingerprint: string;
  admissionId: string | null; basisCid: string; grantCid: string;
};
type TypedLedgerRow = ActionRef & {
  kind: "typed-action"; rowId: string; actionKind: ActionKind;
  semantic: "valid" | "pending" | "invalid";
  admission: "provisional" | "accepted" | "held" | "approved" | "rejected"
    | "awaiting-proof" | "conflict";
  effect: "effective" | "withheld" | "unavailable";
  signature: "valid" | "invalid" | "pending";
  presentedMs: number | null; preview: Preview | null;
  reviewCaseId: string | null;
};
type LegacyLedgerRow = {
  kind: "legacy-change"; rowId: string; opHash: string;
  namespaceGroupId: string; channel: string;
  authorPublicKey: string | null; authorId: string | null; authorFingerprint: string | null;
  claimedKeyByteCount: number; keyShape: "ed25519" | "invalid-length";
  signedDocType: number; signedDocId: string; scope: "matching" | "mismatched";
  signature: "valid" | "invalid" | "pending" | "not-checked";
  groupBinding: "not-signed"; authority: "legacy-unproven";
  changeHash: string | null;
  graph: "applied" | "pending-dependencies" | "unavailable";
  presentedMs: number | null; preview: Preview | null;
};
type UnattributedLedgerRow = {
  kind: "unattributed-change"; rowId: string; changeHash: string;
  namespaceGroupId: string; channel: string;
  signature: "missing"; authority: "unproven";
  presentedMs: number | null; preview: Preview | null;
};
type OwnerFactLedgerRow = {
  kind: "owner-fact"; rowId: string; factCid: string;
  factKind: "profile" | "admission" | "realm-grant" | "basis"
    | "acceptance" | "removal" | "join-bootstrap";
  groupId: string; channel: string | null; profileCid: string;
  issuerPublicKey: string; issuerId: string; issuerFingerprint: string;
  signature: "valid" | "invalid" | "pending";
  verification: "owner-attested" | "unbound" | "pending" | "invalid" | "conflict";
  presentedMs: null;
};
type AdminLedgerRow = TypedLedgerRow | LegacyLedgerRow | UnattributedLedgerRow
  | OwnerFactLedgerRow | ReviewDecisionRow;
type LedgerCoverage = {
  source: "retained-local-inventory";
  typed: "complete" | "not-active" | "incomplete";
  legacy: "retained-only" | "incomplete";
  ownerEvidence: "complete" | "not-active" | "incomplete";
  unattributedChanges: "present" | "absent" | "unknown";
};
type AdminChatLedgerPageReply =
  | { status: "page"; availability: "complete" | "incomplete"; revision: string;
      coverage: LedgerCoverage; rows: AdminLedgerRow[];
      knownRowCount: number; nextCursor: string | null }
  | StaleRead | ReadUnavailable;
```

`ActionKind`, `ReviewDecisionRow`, `StaleRead` and `ReadUnavailable` retain their held-contract
definitions. `Preview` means the bounded preview shape above. Row IDs are `action:<ActionCID>`,
`legacy:<channel>:<SignedOpHash>`, `unattributed:<channel>:<ChangeHash>`, `fact:<FactCID>` or the
existing `decision:<DecisionID>`. Do not choose one concurrent branch or collapse records merely
because their message ID, nonce, text, actor/sequence or timestamp matches. A typed semantic ID
normalizes genuinely valid signature variants as already specified; a legacy SignedOpHash names
its original full envelope and makes no typed semantic-equivalence claim.

A typed action with unresolved authority has no invented admission identity. It remains retained
in the raw action inventory; `admissionId` is its exact referenced verified admission when
available and null otherwise. No nullable value is accepted as a verified positive binding. A Create's `createCid` is its OWN
ActionCID; Edit/Delete/Join use their exact signed target. Existing CaseRef fields stay unchanged.

`namespaceGroupId` describes where the legacy/unattributed record is retained now. Legacy v1
SignedOp signs doc type/id, key and delta, NOT MLS group or authored epoch. A valid signature
proves only exact envelope authorship. It cannot become typed membership, removal-cut admission
or historical moderator authority. Current roster lookup never upgrades this label. A missing
signed original for an actual retained Automerge change yields an unattributed row, not invented
authorship or a complete original signed record. Actor bytes and mutable `author` fields cannot
supply the missing signature. If inventory is incomplete, report `unknown`, not `absent`.

`channel` is the containing local channel namespace; `signedDocType` is the exact encoded u16 tag
and `signedDocId` the canonical u128 decimal actually inside the envelope. A valid signature for a
DIFFERENT document/type is still scope mismatched here. `claimedKeyByteCount` preserves malformed
key shape without dumping an unbounded field into a page: identities are null for a non32-byte key,
and raw key bytes remain in exact original frame chunks. A32-byte key is only a claimed identity
until its signature verifies. Restore currently decodes retained SignedOps without re-verifying or
checking their containing scope; do not assume a saved log entry passed those checks. Covering a
graph change as attributed requires an exact scope-matching, verified single-change envelope with
matching original bytes/hash and the existing actor binding. An invalid signature, malformed key,
multi-change payload or valid foreign-doc envelope cannot suppress the unattributed graph row.

`complete` means this revision's KNOWN retained local inventory was fully inspected, not that a
replica possesses every action ever authored in the group. `legacy: retained-only` always discloses
that absence of old originals cannot establish historical absence. Every open retained chat-channel
document, including hidden/deleted messages and concurrent root message lists, participates. No
filter through current `messages()`, current member roster, visible IDs or current winning values.
Retained proof/decision records remain after case resolution. Owner fact verification is conditional
owner attestation, never a claim that MLS privately stored transitions were publicly replayed.

## Exact detail and original bytes

Details never reconstruct an overwritten original from today's winning message. All original
frame data comes from the retained exact record; missing original yields `evidence-unavailable`.
The frame chunk is at most64 KiB before hex encoding, with the exact complete byte count declared.
No attacker-controlled length/count causes allocation for the entire original. Offset past the
exact end, noncanonical row ID, requested server/channel/row-namespace mismatch or invalid revision
spelling is invalid-request; a retained LegacyRow whose SIGNED scope is mismatched remains readable
with that honest label. A well-formed obsolete revision is stale after authorization. The last chunk returns nextOffset null.
Chunks bind to the same immutable record+revision, so a client cannot silently concatenate versions.

Implement byte-identical v1 frame length/slice/stream hashing over existing SignedOp fields;
DO NOT call allocating SignedOp::encode/hash or build its whole signing preimage just to identify
or chunk a retained large envelope. A hash cursor advances over at most512 KiB per owner turn and
at most128 records; a single huge envelope resumes its exact hash state across bounded turns.
Verification allocates its canonical preimage only after validating a32-byte key and measured
preimage at most256 KiB; a larger historical original gets signature not-checked (not infinite busy
or valid-by-existence), explicit incomplete verification, and still has its exact chunks. No silent
skip or whole-field clone for a huge malformed key/delta. Decode remains the independent64 KiB raw
budget above. A finite inventory build may take multiple turns but a bounded per-record policy must
always terminate honestly. Actual work/allocation counters pin these limits in regression tests.

```ts
type OriginalFrameChunk = {
  encoding: "hex"; bytes: string; offset: number; totalBytes: number;
  nextOffset: number | null;
};
type LegacyMutation = {
  operationId: string; objectId: string;
  key: { kind: "map"; value: string }
    | { kind: "sequence"; elementId: string | null };
  operation: "make-map" | "make-list" | "make-text" | "make-table"
    | "set" | "delete" | "increment" | "mark" | "unmark" | "unknown";
  insert: boolean;
  value: { kind: "string" | "int" | "uint" | "float" | "bool" | "null"
    | "bytes" | "counter" | "timestamp" | "unknown"; text: string } | null;
  predecessors: string[];
};
type LegacyDecodedChange = {
  status: "decoded" | "unsupported" | "too-large" | "invalid";
  actor: string | null; sequence: string | null; startOp: string | null;
  parents: string[]; mutations: LegacyMutation[];
  mutationsTruncated: boolean;
};
type AdminChatLedgerDetailReply =
  | { status: "detail"; availability: "complete" | "incomplete"; revision: string;
      row: AdminLedgerRow; original: OriginalFrameChunk;
      typedBody: HeldActionBody | null;
      legacyChange: LegacyDecodedChange | null;
      signatureHex: string | null;
      factIds: string[]; missingFactIds: string[]; evidenceTruncated: boolean }
  | StaleRead | ReadUnavailable;
```

Original frame means SignedAction, SignedOp, raw Automerge Change for an unattributed row, or exact
signed owner/review fact respectively. An unattributed row has signatureHex null. TypedBody is
present only for typed actions; no legacy batch is coerced into one Create/Edit/Delete. Legacy
changes can hold MANY low-level mutations in one signed envelope; their sequence/object/operation
IDs are exact Automerge identifiers, not timestamps or fabricated message IDs. Detail displays
all bounded original mutations and their property names/values, including overwritten text and
deleted list entries. Concurrent edits/branches remain separate records with exact causal parents;
an advisory display summary may describe recognized chat fields but never replaces original evidence.

Add a narrow raw observer helper in replication which checks the raw kind1 framing,64 KiB
byte cap, operation count/RLE/predecessor/string budgets BEFORE Change::from_bytes or any Automerge
parse. The existing public `decode_change_projection` takes an ALREADY parsed Change, so calling
it after parsing an untrusted original is too late for this reader boundary. Reuse the private
bounded_change preflight deliberately; do not broaden released legacy ingestion. Unsupported
historical actor lengths, compressed grammar or excess expansion get explicit unsupported/too-large
while exact original chunks remain available, with no parser invocation on preflight refusal.
Legacy projection refusal never alters ingestion or erases history. Mutations cap512,
parents/predecessors cap4096, expanded scalar/key/ID text total64 KiB; an exceeded expansion bound
means no partial decoded-success claim. Typed bodies keep their128 KiB original action bound. All numeric64-bit counters in decoded change
metadata/value text are canonical strings, never rounded JS numbers. Parents or exact evidence lists
over128 references are explicitly truncated/incomplete at the DTO, without silently dropping raw
frame bytes. Page/detail limits are independently enforced after JSON encoding.

## Runtime integration and meaningful tests

A useful FIRST read vertical may expose the current legacy retained originals/unattributed inventory
through these commands while typed owner production remains inactive. It must state typed and
ownerEvidence not-active, preserve original bodies/edits/deletes/branches and show legacy-unproven.
This provides a real admin ledger now; it is not completion of typed removal/approval authority.
Then add the SAME actual founder/join/action/removal/held case to this ledger and its exact case link.
Do not replace it with a disconnected fixture or turn fixture-pinned issuer labels into production.

Build a bounded incremental read index over ORIGINAL retained channel SignedOps plus actual change
inventory. Capture one immutable original Actor/store/session revision and publish the page only
when its inventory is coherent; time-slice scans/hashing/signature/projection work with explicit
operation/byte allowances, returning busy while incomplete. Never rescan the whole retained log on
each page, retain unbounded cursor sessions, or trust a mutable row count as the revision. Pin/proof
only changes dirty the view even with equal IDs/counts. Existing server-only moderation-updated
refreshes an open authorized log; no held body enters the broadcast event. Current permission loss,
server replacement/Leave and lock clear the privileged index/cache; historical authors are not
filtered by today's roster. Getters must not mutate documents, signing queues or network state.

Native handler/policy/generated permissions/main capability, literal TypeScript ledger/invokes and
visual fixture inventory must cover BOTH new commands before activation, alongside existing held
read commands. Parent owns frontend; backend owner owns Rust/native read and policy paths.

Regressions: actual signed Create -> repeated edits -> Delete still exposes each original; opposite
concurrent edit/list-root bubbles have identical exact retained union without user repair; signed
legacy v1 group-reseal never claims group tenure; raw snapshot with missing/invalid SignedOp, malformed key or valid foreign-doc/type SignedOp shows
unattributed/unproven and no fabricated author; an invalid signature with the same ChangeHash does
not cover that graph change; modern actor bytes alone cannot upgrade it; exact
chunk reconstruction at0/64KiB/end, huge retained delta/key fields have bounded measured allocation/work,
and compressed/tiny-RLE cases invoke no parser before refusal; all list-root
branches and deleted nodes are indexed; restore preserves records and known incompleteness; mirror
modlog deletion cannot retract typed original/proofs; auth/session/role/instance loss precedes cursor
metadata and blocks late UI repopulation; proof-only revisions stale every old page/chunk; resolved
cases/decisions remain; the SAME actual held case opens exact evidence and no normal message lookup.
