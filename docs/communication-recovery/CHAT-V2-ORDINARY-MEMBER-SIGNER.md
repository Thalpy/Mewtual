# Ordinary-member signer: founder-as-member vertical

Implementation checkpoint, 11 November, branch `chat-v2-ordinary-member-signing`
(from `b24fd196`), merged into the operational integration branch at `bb5f0290`.
The primary's ten focused commands pass, including all 33 new cases and neighboring
regressions; full root/native validation is running. See HANDOVER for exact counts
and the remaining strict-lint gate. Chat V2 stays disabled. This implements the
"Signing boundary" of [CHAT-V2-ORDINARY-MEMBER-PATH](CHAT-V2-ORDINARY-MEMBER-PATH.md) for
one source, the Profile founder acting as an ordinary member, and the Sync seams a
joined-member source will use. Durable save, publication, projection install, native
dispatch and nonfounder App orchestration are not part of it.

## Chain

| Layer | Item | Role |
|---|---|---|
| MLS | `MlsDevice::retained_ed25519_key_matches`, `sign_retained_ed25519_bounded` | Allocation-free full-key equality check; bounded sign with the retained `SignatureKeyPair`. No provider, SDK group, export or ratchet. |
| Core | `V2OrdinarySigningGuard::{signer_data, spec_data}` | The admitted Draft's full signer and spec, so a Sync signer can refuse a wrong key or a Draft not built from this request's draw BEFORE `begin_key`. DATA only. |
| Sync | `V2MemberDraftInputs`, `V2MemberSignRefusal` | One request's once-only Clock/nonce cell, bound at first draw to the exact node (founder seed observation or `RegistrySyncInstance`). |
| Sync | `UnboundV2GenesisNode::{ordinary_member_signer_data, draw_ordinary_draft_inputs, sign_ordinary_admitted}` | Founder vertical. Reuses the founder Create producer's closed source check unchanged; shares nothing with the Append lane. |
| Sync | `ChannelSync::{ordinary_member_signer_data, draw_ordinary_draft_inputs, sign_ordinary_admitted}` | Joined-member seams on the live node. Consumed by the separate joined-member source (not yet delivered). |
| App | `SelectedV2TextHead::genesis_member_anchor_data` | Genesis HOST anchor from the authenticated original pin, requiring the selected O to be the empty generation-1 promotion and still selected. No retained-A coupling. |
| App | `V2TextProjectionDomain::{prepare_member, general_verifier_from_genesis, begin_channel_view}` | Member-role domain; the general verifier can be minted only on it, at most once; the founder verifier never on it. |
| App | `PreparedV2TextHost::retry_sign_member_request` and `take/restore_member_signed_original`, `finish_member_request_saved`, `abandon_member_request` | The member lane (`operational/host/member.rs`). |
| App | `MemberSignedOriginal::from_completed_plan` | Agreed with the member-outbox owner; pure DATA from the exact Plan/observation/Draft/cached raw and the Plan's historical Admission. |

## Why the founder genesis node is a sound source here

[CHAT-V2-ORDINARY-SIGNING-PLAN](CHAT-V2-ORDINARY-SIGNING-PLAN.md) says the Genesis-only
Source cannot be relabelled to supply the production boundary. This vertical does not
relabel it as a general source. It is sound only under all of these, each enforced in code:

- The node passes the unchanged founder Create `source()` check: epoch 0, exactly one
  member, designated committer, active group, live original scope reservation, device key
  equal to the context signer and to the Profile founder and founder Admission subject.
- The HOST anchor is produced from the independently retained original pin and source
  expectation (`original_context`), never from the Profile/Header being verified.
- The selected operational O is the empty generation-1 promotion: Genesis parent, no
  pending transaction, empty token and publication roots, archive G equal to the public
  source genesis. Any founder certification advances O, so while it holds the governing
  prefix is exactly genesis and the founder Admission is the only current one.
- Every key cut repeats `validate_under_current_guard` on the SAME captured O frames
  (byte equality) before Core admission, so an advanced O refuses with no key call.
- The member lane and the founder Create lane are mutually exclusive on one host: the
  founder token lookup and the founder next-request entry refuse while the lane exists,
  and the lane refuses while any founder lookup/request/resident state exists. Two token
  namespaces never author for one request.
- The anchor producer refuses while a founder `text.wal` or `text.wal.stage.v1` exists
  (only a proven NotFound is absence), so a crash-left founder transaction can never be
  recovered later beside member authoring for the same request.

A later governing prefix (any Add, Append or Remove) needs the separate replay producer
owned by the joined-member/governance work. This vertical refuses rather than guessing.

## Order of one request

1. Pure DATA first: a Plan that already holds raw64 completes outside every writer before
   any positive gate. A late lock or negative therefore never strands a signed original.
2. Shape checks on borrowed input, then the request fee, then the owned copy.
3. Lane, once: lane fee, member domain (own fee), paid parent capture; under the Current
   group guard, capture the selected O, produce and bind the anchor; outside writers,
   build the general verifier. A new lane is installed in the host only once its verifier
   exists. If it fails earlier (a malformed request, or the anchor refusing beside a
   crash-left founder WAL) it is dropped: nothing was drawn or signed, and the founder lane
   stays available, so the host is never wedged with neither lane able to proceed.
4. One Clock read and one nonce through the original node's own seams into the request's
   draw cell, which binds the request to that node. Retries never draw again.
5. One immutable Draft with canonical parents; ONE Plan for it with caller-supplied
   dependency loans sorted into Core's order, each resolved to a current Admission.
6. Key cut inside `with_v2_text_current` plus the group guard: domain source, selected-O
   proof (IO) and the node's full source check, all BEFORE `admit_signing`. After it, only
   the node's cut runs: the draw cell must name this exact node, the Draft must carry
   exactly that cell's drawn time and nonce, the genesis scope reservation must still be
   live (a cancellation flag read), and the retained key must equal the Draft signer. No
   IO, allocation, codec or SDK lookup; then the one bounded key call. The node caches
   raw64 in the Plan before returning.
7. Completion outside writers into `MemberSignedOriginal`.

## Request parents and tokens

The lane never derives parents from tips or ledger state. Parents are exactly the request's
list: at most 64 entries AS SUPPLIED (bounded before dedup and before any fee), then
canonicalised once (sort, dedup) by the function that also builds `spec.parents`; a Create
must name exactly its reply target. Every call, including retries, re-checks those shape
bounds before any comparison or dependency resolution. The request composer must persist
the exact token, channel, body and parent list it sent: a retry after restart with
different parents is a conflict by design, not a hit (member outbox lookup).

One pending request per lane. A finished token (saved, abandoned or forgotten) cannot author
again in this host lifetime; the bounded ring keeps the last 64. Cross-restart and
older-token deduplication is the member outbox's exact token lookup, which must run before
authoring.

- `take_member_signed_original` moves the original out and leaves the request Lent with its
  fee.
- `restore_member_signed_original` accepts only the same token, the exact lent ActionId,
  the Plan's Admission and the Plan's own cached signature bytes.
- `finish_member_request_saved(token, receipt ActionId)` releases the request after
  SavedLocal or AlreadySaved and returns whether the saved original is the lent one. When it
  is not (AlreadySaved naming an earlier signing of the same request), the caller must drop
  its lent copy and never publish it; the outbox's stored entry is the only published one.
- `abandon_member_request` retires a request whose original the outbox never wrote. A
  Pending request (refused, terminal or uncertain key work) may always be abandoned. A raw
  cached by an interrupted cut is completed first, so abandonment never hides it. A
  Complete (restored) original may be abandoned ONLY after the caller (the primary's
  outbox integration) has one of these, because the host cannot see outbox outcomes:

  | Outbox outcome | Caller action |
  |---|---|
  | `prepare_append` TokenConflict, Foreign, IndexExhausted | abandon (nothing written) |
  | Exact lookup with the original request DATA returns Absent | restore and retry, or abandon |
  | `run()` Uncertain | recover, then look up: Hit means finish with the receipt ActionId, Absent as above |
  | AlreadySaved | finish with the receipt ActionId, never abandon |
  | DuplicateAction | the ActionId is already retained under another token, possibly submitted or Held; reconcile its existing receipt/debt and report the invariant breach, never abandon as a fresh unsaved action |
  | Busy, Stale (including admit refusal), NeedsRecovery, Incomplete, Payment, `prepare_append` Failed (read error, nothing stored) | keep the original, retry later |
  | Pressure | keep it and retry; if the product drops it, the UI must say "not saved" |
- `forget_lent_member_request` retires a Lent request whose value was lost, and only after
  the outbox's exact lookup proved Absent. It exists so a lost value cannot wedge the lane.

## Lifecycle and close

`cancel_and_drain_admitted_promotion` and host Drop also cancel the member domain; the
former reports "still closing" until the member group's real gate drains. An admitted key
operation that races a close still caches its raw and completes; the close completion
waits for Core's reader. New signing refuses after closure. A completed original remains
retrievable as DATA after lock and grants no save or publication right.

## Payments (logical invoices, not RSS claims)

- Lane: replay math 1 MiB, general verifier metadata, 8 channel ledgers/views, 64 finished
  tokens, lane layout. The member domain and parent capture pay their own existing fees.
- Request: one Draft plus completion overlap, Plan metadata, the outside-writer
  authorization transient, the request copy and 65 dependency loans. Dependency actions
  are borrowed and paid by their owner.

These do not establish that the configured pools fit activation.

## Not covered (honest gaps)

- Nonfounder sends: the App owner of a joined member's `ChannelSync`, its trusted V2 pin
  and replayed governing prefix are the joined-member source's deliverable. The Sync seams
  exist; no App orchestration consumes them yet.
- Removal/rejoin A1/A2 and Held removed actions in App: Core pins them; App needs the
  joined-member source.
- Durable save admission, the outbox context-spec getter and parent provisioning: member
  outbox owner and primary.
- Native/UI command generation check: the primary's dispatcher, as for every operational
  host entry point. The Store session lock is enforced through `with_v2_text_current`.
- Cross-crash pre-raw signing uncertainty: not solved; exact stored raw reuse is the
  outbox's job after save.
- The per-channel ledgers are empty view anchors; the Plan's channel binding does not follow
  the real chat projection.
- A founder WAL that appears AFTER a lane already holds its anchor is excluded only by lane
  exclusivity on this host (the founder lane cannot run here). The founder next-request
  exclusion has no direct regression yet: reaching it needs a completed founder resident.
- Eight channels per host lifetime; beyond that new channels refuse.
- The preconditions for abandoning a Complete original and for forgetting a Lent one (an
  outbox refusal that stored nothing, or an exact Absent lookup) are asserted by the caller,
  not proven to the host. A caller that breaks them can only provoke an outbox
  AlreadySaved/TokenConflict later, not a second publication; a typed Absent receipt
  argument is a possible later tightening.
