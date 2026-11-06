# Connected operational FirstCreate host

Implementation proposal based on primary `9d3e1c23`, extended for the composed Initializing1
repair owner. This document adds no operational runtime, module declaration, build or tests.
This is the next connected Root cohort after genesis recovery/settlement; the separately
reviewed Create/Append DATA packet remains undeclared until that cohort consumes it.

## Existing boundaries that determine the implementation

* `persistence/cold_source_read/genesis_seed/current.rs` and `current/cold.rs` already own the
  actual node, paid read/SDK/runtime resources, original task, mounted Current2 scope and
  installed writer. The composed `current/initializing.rs` adds the distinct actual repaired
  owner with its original ColdInitializing writer and scope2 continuation. Their private
  fields and Drop order are the handoff; no raw-node adopter
  or reconstructed Saved result is needed. `current/settlement.rs` already selects the exact
  seed-present/settled proof before validation.
* `critical_namespaces/archive_current_codec.rs::GenesisHeadData::validate_data` requires the
  original genesis header, `verify_genesis_binding` and empty acceptance tree. The settled
  cold reader repeats exact original genesis content, and `genesis_inventory.rs` rejects
  operational objects. These checks must stay strict. A later head needs its OWN selected
  receipt, inventory/cold selector and owned Current role in the same implementation cohort.
* `ReplayMath::{from_genesis,derive_successor}`, `AcceptanceRecord`, `AppendBody`,
  `cold_actions::PreparedActionPages`, `cold_records::{encode_record_batch,encode_header_row}`,
  `cold_nodes::encode_node_page`, `cold_manifest::encode_manifest` and
  `PreparedPrivateChatPage` supply bounded public math/page DATA. They supply no selected
  source, complete lookup, saved result or host writer.
* `cold_index::{encode_leaf_data,encode_branch_data,open_query_data}` is only structural
  DATA. The balanced COW updater and original selected-source query issuer do not exist.
  `NodeFrame::branch_row` is crate-private: a connected Core edit/child-descriptor interface
  is needed for the real App updater, rather than copying a second node parser into App.
* `owner_profile::AdmissionBody::matches_action_delegation` checks signatures/labels; it
  does not certify the Admission. `ProjectionGroupView` supplies a negative lifetime only,
  and `ProjectionAdmissionToken` has no production positive issuer. The existing v1
  `OwnerProfileExpectation` verifies v1 facts, not V2 headers/Admission bodies.

## Exact new Root owners and files

Keep the actual implementation in the Current family, with physical persistence in new
Store-private children. The proposed names describe concrete owners, not public constructors.
The host closes its original projection/publication lifetime BEFORE the retained original
wrapper/payload drops; new selected state and pending/debt retain their independently paid tails.

| File | Actual consumed types and responsibility |
| --- | --- |
| `persistence/cold_source_read/genesis_seed/current/operational.rs`, with narrow cold/repaired-family adapters | `PreparedV2TextHost<T,R>` retains the SAME actual warm/cold/repaired owner through every error; `V2TextHost<T,R>` owns it in a CLOSED Warm(original)/Cold(original)/Repaired(original) core plus selected operational state. Move the WHOLE wrapper: the existing wrappers implement Drop, so direct field extraction is not the safe handoff. Narrow private family methods call the concrete producer on their own node; no generic raw-node loan or unsafe extraction. Existing cancellation/payment order remains. |
| `persistence/v2_text/head.rs` | `PreparedV2TextHead`, `SelectedV2TextHead` and `ColdV2TextHead`; actual complete-frame identity, held objects, file/parent barriers, exact predecessor CAS and cold selected closure. Parsed head DATA cannot construct these receipts. |
| `persistence/v2_text/transaction.rs` | `PendingV2Create`, `SavedV2RetainedAction`, `SavedV2UnsignedAppendIntent`, `SavedV2AcceptedCreate`; complete immutable candidates plus actual fees enter Store custody before effects. Original owner holds node/sender; inert debt holds neither node, key, Store nor session. |
| `persistence/v2_text/accepted_index.rs` | The real selected-source query and deterministic COW update using the existing two scopes. Only this owner can turn its own full selected path into complete hit/miss. |
| `persistence/v2_text/request_index.rs` | NEW codec/domain/context paged map, keyed by actual source/Profile/full local signer plus token. Stores exact request/action/cipher binding and request phase/receipt; current Directory/Actions scopes must not be relabelled. Every committed locator is reachable from the selected head. |
| `persistence/v2_text/cold.rs` | Paid inspection/recovery of exact head plus pending seed/WAL and their dependencies BEFORE genesis fallback. New paid S0/SDK read; no historical warm token reconstruction. |
| `chat_admission/v2_text.rs`, canonical Current children | `V2TextCurrentAdmission` borrows the SAME loaded row2/nonce/funded owner and installed writer generation under actual admission. It checks the operational selected closure instead of calling a genesis-only SavedHead validator. No per-message scope rewrite, revision increment, owner or writer renewal. |
| Replication V2 authority/projection child, called only by the above host | A V2 selected-prefix expectation/verifier bound to the host's private original `ProjectionGroupView`; verifies exact founder delegation, selected accepted action and ordinary Create dependencies before issuing a channel effect token after SavedB. |

The Repaired case must consume its actual `UnexposedRepairedV2GenesisCurrent`, scope2
continuation and installed ColdInitializing generation. It cannot fabricate the older warm
or unowned-cold receipt, and must not force a close/remount merely to obtain a supported
owner type. Its operational admission is a third concrete private branch preserving the
same source/task/owner/nonce and kernel observation; a caller-selected kind/boolean or
generic adopting trait is not the issuer.

Before first operational effects, this same cohort must also add the concrete Repaired
settlement/read branch: the present repaired wrapper intentionally retains `activation.seed`
and currently has no cleanup adapter. Extend the closed original-owner/physical-evidence
family using the actual repaired scope2/head/task/writer, its fresh covering barrier and
original-seed cleanup. Cache/dispatch the same admitted unlink/parent-sync and settled-read
phases before the mandatory-seed validators, preserving the original node/nonce/generation.
Only that actual settled Repaired proof may enter promotion. Do not turn an older Warm/Cold
conversion, close/remount, or an optional missing-seed check into this issuer. Seed-present
operational promotion is not an alternative supported by this first design.

### Two identities prevent an index/head hash cycle

An immutable private **archive head** has `ArchiveHeadId` and covers the retained manifest,
public signed header/image, original pin/source and scope lineage. It does NOT contain
accepted-index/request/publication roots. A separate **operational selector** has its own
complete-frame ID, private generation and predecessor; it selects that archive head plus
the complete accepted-directory, request-map and publication-debt roots.

Prepare/seal archive head H_B first. The directory value can then bind
`prefix_head=H_B`, the exact Append manifest, OWN header/count/root and uniqueActionCount.
The outer selected head references H_B and that directory root. A manifest's `source_head`
references the PREVIOUS archive head, never itself or the outer selector. This preserves
the existing manifest grammar and avoids either a self-hash or stale-prefix completion.
The WAL stays outside this hash graph: it may retain the complete selector candidate and
parent, but that selector must not name a WAL ID which itself hashes the selector.

Promotion derives the initial archive anchor H_G from the actual original genesis receipt
and its complete original pages. It never casts `V2GenesisHeadId` bytes to `ArchiveHeadId`.
H_G and the initial selector may be prepared in the first transaction's exact seed bundle;
their authority still comes from the actual settled genesis owner. Explicit empty roots are
permitted only at this genuine empty founder genesis. Later missing roots are Incomplete.

The physical handler must classify these strict operational tags and fixed pending slots
before creating files. Genesis-only cold paths must refuse operational presence, never
retry it as bootstrap. Permanent group pin and existing ID2/ID3 declarations stay unchanged;
there is no inferred namespace upgrade or generic cleaner permission. Lifetime pages live
under a fixed content-addressed directory, not a scan of the growing history folder.
Proposed physical slots within the SAME original local target: existing `head.bin` selects
the new outer kind, existing `head.stage` stages its exact replacement, and new fixed
`text.seed`/`text.wal`/`text.wal.stage` protect the complete operation. New immutable objects
use `text.pages/<complete-private-frame-id>` under its original held directory. Source files
and group pin remain untouched. The actual codec/fee/path-capacity freeze must enumerate
these complete frames and every simultaneously held path before this role is implemented.

Before the FIRST such file/directory effect, cache the original parent, all complete
candidates/debt/fees and a closed `PreparedV2TextHost` promotion phase. For example, private
`Original -> PromotionPrepared -> OperationalSelected` phases must retain their actual
receipts and candidate closure. Dispatch validation by that admitted phase BEFORE the old
genesis/settled inventory verifier: it intentionally refuses the new objects. Pending
validation proves the same original parent or exact selected candidate and only its own
authorized artifacts; it never interprets an old-validator error as operational selection
or vacancy. Cold capture independently classifies complete authenticated pending sources
before choosing a role. Unknown/partial first seed stays held/incomplete and cannot mint
a new token or reconstructed warm phase. This phase rule also applies before later WAL effects.

## One actual Create, in transaction order

1. **Capture and fund.** The host proves its actual selected source under
   admission -> pins -> canonical -> Root, captures the exact private generation/public
   parent/index roots and stable request binding, then releases those locks and the
   node-backed SourceRef before mutating the node's retained RNG. Acquire actual producer
   fees outside writers: Create1MiB, Append2MiB, PLUS separately enumerated index paths,
   page/cipher/WAL/seed/request/publication/path/control overlaps. Existing SDK decode credit
   pays none of this. The resident ReplayMath/operational owner, view and projection window
   have their OWN retained invoice; neither producer's temporary fee nor the old wrapper's
   allowance silently funds them. Persistent byte/file allowances cannot reset on remount. Bound active
   pending work, not lifetime messages; retain fees until all payload/control tails drain.
2. **Retain A.** Complete request lookup comes from the selected request root. A matching
   token must return its original action/result; changed channel/text refuses. A missing or
   corrupt committed object is Incomplete, never permission to sign again. For a genuine
   new request call `prepare_create_data`, cache its original pending before work, detach
   once, and register the complete packet AND fee before any A I/O. Save exact action chunks
   and `RetainAction` manifest, original ciphertext/nonce, request binding and pending debt.
   A selects a new private selector/request root while leaving the public header unchanged.
   Caller Drop cannot discard Store's admitted immutable candidate. A's ciphertext remains
   UNACCEPTED: neither a warm nor cold drainer may publish it before SavedB.
3. **Query acceptance and plan.** Open the OWN admission's full selected accepted tree;
   verify original action/signature/record sources for a hit. Canonical hit returns the
   original receipt; conflict/incomplete blocks. A structural DATA miss or missing directory
   is not absence. Then call the frozen `prepare_create_append_data` (zero signs) and borrow
   its exact signing bytes, complete image, record/body and intent commitment.
4. **Persist one child BEFORE signing.** A private physical writer returns
   `SavedV2UnsignedAppendIntent` only after complete exact intent/seed/parent barriers.
   Bind the actual PUBLIC parent, exact unsigned signing bytes/image, original A and selected
   complete-index source plus an independent stable transaction identity. The intent cannot
   name a final cipher/header-representation ID containing a signature that does not exist
   yet; final signature and sealed candidates are retained once produced. The child key is
   Profile/issuer/tenure plus PUBLIC parent, across every A-only private generation. Install
   it under actual outer-selector CAS so later A-only updates cannot erase or bypass it.
   A pending/entered/ambiguous child cannot be replaced. Serialize
   this owner-scoped operation logically; no global mutex need survive page work. Native
   Stop/lock/session admission orders at this irreversible boundary and retains transaction
   custody through B: it cannot report an admitted action as unaccepted. Only the actual
   host with this receipt can reach its node's `sign_create_append_data`.
5. **Commit B.** Cache actual header signature/replay result before late failures. Prepare
   records, complete nodes and header/body/image, their three-page Append manifest and H_B;
   build inner/outer COW roots preserving all other admissions, plus request receipt and
   publication eligibility/debt. The exact seed/WAL covers ALL selected children; the
   manifest's three pages do not cover index/WAL/ciphertext debt. Sync every immutable child
   and parent before final operational-selector CAS against the SAME selected predecessor.
   An already-selected exact candidate is resynced; an unrelated selected head is not
   adopted. Any rebase must preserve the frozen signed child and prove unchanged required
   public parent/index ancestry, never rerun signing. Source/owner/writer/file proofs repeat
   after waits, with cheap negatives last. Only the actual writer returns SavedB.
6. **Expose the result.** After temporary fees/retired controls drain outside writers,
   repeat the full operational proof. Install the checked public cursor and issue a V2
   projection effect through the SAME private group view. Publication retries use the exact
   saved ciphertext, with actual current mount/session/Stop/eligibility fences. No resigning,
   resealing, old `ChannelSync::post`/`SealedOp` dispatch or raw ciphertext publisher replaces
   this path. A new V2 envelope decoder/topic consumer must pair before network readiness.
   A-only storage generations do not replace that original governing-support view. Its
   issuer still checks exact V2 root/prefix, fork, dependency and acceptance sources.

The accepted updater uses leaf32/branch192, deterministic16+17/96+97 splits and <=10-page
READ paths per scope. That is not a bound on total COW split outputs: old paths, sibling
inputs, all new nodes and root exceptions must be separately enumerated in the live invoice.
Source writers return the original acceptance and never append duplicate
leaves. Reader recovery preserves every original leaf/root and separates leafCount from
uniqueActionCount; exact valid aliases use earliest canonical header sequence/ordinal.
Unknown/conflicting claims remain present. The request map and publication queue likewise
need paid page paths, not a64/4096-entry lifetime map. Initial implementation retains all
immutable history and does no GC. `ChatLedger` is a bounded projection window only.

## Restart and required crash cuts

Cold selection reads head/stage/pending seed/WAL before deciding any source role. It selects
the exact parent or exact candidate, pays a new finite read/repair attempt, retains original
spent allowances, and opens dependencies by their saved IDs. Complete seeds permit exact
prefix repair; unknown/partial sources stay held. S0/closed SDK remain the same epoch0 source,
but the public cursor is rebuilt from actual genesis through a finite selected manifest/header
prefix. That O(history) replay must run in finite paid resumable turns on the SAME frozen
selected source/head; no unbounded single call, repeated restart-at-zero or lifetime history
cliff is allowed. No arbitrary current-image constructor or full-history action map is needed.
New cold runtime randomness comes from the sanctioned fresh runtime RNG stream: the unchanged
epoch exporter key is not permission to restore/reset a deterministic genesis nonce stream.

| Cut | Required continuation |
| --- | --- |
| Candidate before A's first write, or caller Drop during A | Store retains exact packet/fee; complete seed repairs the same bytes. Partial unknown seed is unavailable, not a new token/action. |
| SavedA before unsigned intent | Request map finds original A; acceptance lookup/planning resumes without new Clock/RNG/sign/export. |
| Saved intent before/during/after signing | Same unsigned child only. Complete signature is cached warm; cold retry must use the exact persisted child/signing state and actual signing admission, not fabricate historical warm proof or choose another intent. |
| B children or staged selector before CAS | Finish/resync the exact complete candidate against original parent; no accepted output yet. |
| CAS/parent barrier or reply lost | Reopen exact selected B, verify/resync all dependencies and return the original action/receipt. Missing selected index/locator/debt remains Incomplete. |
| SavedB before projection/publish, or publish reply lost | New actual Current proof; reissue only same supported effect and exact saved ciphertext debt. No acceptance duplication. |

Before this cohort is declared, genuine tests must cover two successive Creates -> close ->
new paid source/SDK/operational cold reopen -> exact-token replay; all cuts above;
actual repaired output -> same-owner settlement -> first A/B without restart, including
ambiguous unlink/parent retry and unchanged node/source/scope nonce/writer; same token
changed text/channel; two competing intents on one public parent across A-only generations;
same-CID bad signatures; missing/stale/conflicting index and request pages; cancellation and
replacement at final proof; Stop ordering around saved intent and cold debt eligibility; exact ciphertext after owner
Drop; SDK rows unchanged; unrelated group progress; and4097+ accepted requests without a
lifetime cliff. These are coupled host tests, not fake Saved receipts around DATA producers.

The smallest next implementation is this **selected-head/transaction/index/cold caller
cohort**, consuming the frozen producer packet. Repeated first-founder epoch0 Create is its
profile; Add/Remove/late removed history, other writers, edits/deletes, operational epoch SDK
admission and full native20/50-member chat remain subsequent explicit verticals.
