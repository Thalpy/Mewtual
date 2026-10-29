# Privileged retained chat audit reads

Native/TypeScript contract for the existing moderation timeline. These are read-only main-window
commands; native implementation and real retained-case integration are still pending. Parent owns
frontend consumption/fixture/TypeScript command ledger; backend owner owns Rust DTOs, Actor/native
reads, native command policy and capabilities. Production activation remains gated.

Commands use literal names and camelCase request/response fields:

- `get_admin_action_reviews({ server, cursor: null | string, limit })`, integer limit 1..32.
- `get_held_action_detail({ server, caseId, expectedRevision })`.

All fixed digests/full keys/full device IDs are 64 lowercase hex characters. `groupId` is full
lowercase hex of 1..256 original group bytes; `channel` is canonical unsigned decimal `u128`.
`authorId`/`issuerId` are full native `DeviceId::from_public_key_bytes(key)` text, not an origin
label. Fingerprints are their first eight hex characters for display/name lookup only; exact
evidence binding and distinct graph identities retain the full keys/IDs.
`signatureHex` is exactly 128 lowercase hex characters; `signedActionHex` encodes the bounded full
frame. `knownRowCount` is a nonnegative safe integer at most `u32::MAX`; do not allocate that many
frontend objects from the count, and consume only the bounded pages actually returned.

```ts
type CaseState =
  | "held" | "approved" | "rejected" | "reopened" | "awaiting-proof" | "conflict";
type ActionKind = "create" | "edit" | "delete" | "join";
type ReviewChoice = "approve" | "reject" | "reopen";
type ReviewKind = "recommendation" | "decision";
type AuditProof = {
  signature: "valid" | "invalid" | "pending";
  authority: "owner-attested" | "invalid" | "pending";
  selection: "final" | "unselected" | "pending" | "conflict";
};
type CaseRef = {
  caseId: string; actionCid: string; createCid: string | null;
  realm: string; groupId: string; channel: string;
  removalCutCid: string; admissionId: string; basisCid: string; grantCid: string;
  authorPublicKey: string; authorId: string; authorFingerprint: string;
  selectedDecisionId: string | null;
};
type ActionCaseRow = CaseRef & {
  kind: "held-action"; rowId: string; actionKind: ActionKind; state: CaseState;
  presentedMs: number | null;
  preview: { text: string; truncated: boolean } | null;
  evidenceManifestCid: string; proof: AuditProof;
};
type ReviewDecisionRow = CaseRef & {
  kind: "review-decision"; rowId: string; decisionId: string;
  recordKind: ReviewKind; choice: ReviewChoice; state: CaseState;
  issuerPublicKey: string; issuerId: string; issuerFingerprint: string;
  authorityBasis: string; predecessors: string[];
  presentedMs: number | null; reason: string; evidenceManifestCid: string;
  effect: "advisory" | "unselected" | "selected" | "pending" | "conflict" | "invalid";
  proof: AuditProof;
};
type AdminAuditRow = ActionCaseRow | ReviewDecisionRow;
type ReadUnavailableCode =
  | "not-authorized" | "session-unavailable" | "server-unavailable"
  | "server-superseded" | "profile-unavailable" | "evidence-unavailable"
  | "busy" | "invalid-request";
type ReadUnavailable = { status: "unavailable"; code: ReadUnavailableCode };
type StaleRead = { status: "stale"; revision: string };
type AdminReviewPageReply =
  | { status: "page"; availability: "complete" | "incomplete"; revision: string;
      rows: AdminAuditRow[]; knownRowCount: number; nextCursor: string | null }
  | StaleRead | ReadUnavailable;
type HeldActionBody =
  | { kind: "create"; text: string; replyTo: string | null }
  | { kind: "edit"; createCid: string; text: string }
  | { kind: "delete"; createCid: string }
  | { kind: "join"; createCid: string };
type HeldActionDetailReply =
  | { status: "detail"; availability: "complete" | "incomplete"; revision: string;
      case: ActionCaseRow; action: HeldActionBody; signedActionHex: string;
      signatureHex: string; factIds: string[]; missingFactIds: string[];
      evidenceTruncated: boolean }
  | StaleRead | ReadUnavailable;
```

`rowId` is `held:<caseId>` or `decision:<decisionId>`. A held-action case remains in this audit
after approval/rejection/reopening; it is not a pending queue. Historical decisions also remain.
`selectedDecisionId` and proof/effect labels are derived native read results, never authority inputs.
An old selected rejection can stay in the signed decision chain after a selected reopening; the
case's current state and exact selected decision reference explain that difference. Admin
recommendations are advisory and cannot release the action.

`presentedMs` comes only from the signed action or signed decision's presentation timestamp. Native
returns null outside the safe JavaScript Date range; it never substitutes local first-receipt time.
The canonical backend page order is ascending ASCII `rowId`, independent of timestamps and arrival.
The UI may present rows by `(presentedMs-is-null, presentedMs, rowId)` for display only. CID tie breaks
here choose a rendering order, never authority, admission or a decision winner.

Revision is an opaque string of at most 256 ASCII characters, cursor at most 512. Bind a cursor to
the exact numeric server and installed Actor/store incarnation, authorization session/policy, pin,
immutable archive/view revision and row position, with bounded Clock expiry. Changes in proof
availability/authority/conflict or selected admission advance revision even at equal IDs/counts.
A well-formed old view returns `stale` with current revision; invalid spelling/authentication returns
`invalid-request`. No live page mixes revisions. `availability` describes known archive inventory;
the frontend has fetched that inventory completely only after consuming all pages to null cursor.
An incomplete or unavailable archive is never represented as an empty complete log.

Previews are at most 512 UTF-8 bytes, truncated at a scalar boundary with `truncated: true`.
Reasons are at most 2 KiB; predecessor references at most 32. Detail retains the original typed
action (at most 128 KiB before hex encoding), exact device signature and proof references; each
fact-reference list is at most 128, with explicit truncation/incompleteness. Entire response is at
most 1 MiB encoded JSON. Missing original action bytes return `evidence-unavailable`, not fabricated
text or an empty body. A page may contain less than the requested limit to meet the byte bound.

Native checks the current caller's full-key owner/admin permission and visible/unlocked generation
before obtaining privileged bodies, then rechecks original session/server custody across await.
The Actor repeats role checks at the authoritative read point. This gates the reader, not historical
issuers: today's roster/admin list cannot erase retained decisions or change action admission.
The Actor's materialization is the already-admitted read boundary for role changes after that point;
it is not a new permission to read again. Native still rejects replaced session/server custody before
reply, and frontend role/view generations discard late replies after access was cleared. Current
not-authorized/session/server-superseded results take precedence over stale/cursor/revision metadata.
Only covering-save committed retained case rows are returned; a private preparation or uncertain
candidate is not an audit success. Pending evidence/storage work yields incomplete/unavailable
inventory without exposing hidden candidate records.
Do not add these bodies to `Msg`, `get_messages` or all-member `get_moderation`. Admin-only UI is not
cryptographic moderator-only secrecy from members holding the shared replicated plaintext/keys.

A timeline link calls this exact held-detail endpoint and displays retained evidence within the
moderation pane. It never jumps to a hidden message or fabricates one. Stale revision resets/reloads
the view; scope/session/authorization loss clears displayed privileged state. Temporary busy/storage
unavailability can retain an explicitly stale prior view, without a completeness or success claim.
Unknown status/state/action variants fail closed rather than falling back to ordinary message data.

Both commands must appear in native handler/policy/generated permissions/main capability inventory,
literal invokes/TypeScript security ledger and visual fixture tests before activation. Tests must use
the actual retained case and verify both opposite arrival orders, resolved-case retention, proof-only
revision invalidation, role/session/incarnation loss, bounded pages, exact detail scope and missing
proof/body truth. Fixture coverage complements that backend path; it cannot replace it.
