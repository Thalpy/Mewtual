# Automatic multi-peer admission: failing reproduction and next boundary

27 October 2026. This is an open implementation requirement, not a completed protocol.
App V2 remains disabled. The isolated composition branch deliberately contains a failing
positive acceptance test. Do not merge it as a green checkpoint or enable V2 from it.

## What the new test actually establishes

`three_production_browser_coordinators_adopt_every_peer_without_pair_selection` starts
three genuine native/browser coordinators against independent Root actors and real TCP.
Each must compact two native histories naming the two other actual DeviceIds, retain two
SDK peers, and synchronously close both on End. It supplies no peer selector, role,
ExpectedRow, factory, receipt or phase permission to production scheduling.

Direct endpoint1--2 TCP is forbidden. Helper0 joins no call. Endpoint3 is also an eligible
member helper, so the test does not claim exclusive forwarding through0. The RTC provider
is controlled: this is native/HTTP/WebCrypto/TCP composition, not RTC parsing, ICE,
captured/audible audio, WebView or member-media-relay acceptance.

The original two-endpoint coordinator acceptance remains unchanged. The new test retains
its55s browser observation and60s native control-step bounds. Original protocol15s windows,
source attempts, resource ceilings, history and cursor behavior are unchanged.

An earlier run failed in61.21s. Two browser observers reported one completed peer/one SDK
peer each; the third reported zero/zero. The last six lifecycle changes on each successful
one-peer browser included confirmation around32.1s, waiting around32.4s, failed around47.6s,
then advance and waiting. The zero-peer browser also reported failed/advance/waiting. Every
retained native Derive parent observed at failure had cached settled/ACK/drained flags and
an issued, acknowledged Plan final. These cached fields do not identify which plan was a
success and must not be presented as present-time Root authority.

All three original browser observations are joined even if one fails; failure never restarts
a coordinator. All constructed endpoint collectors are attempted before rethrowing the
original error. They join native End, Node/IO and their actual HTTP service; they do not claim
successful browser final ACK or graceful full Scenario/mesh shutdown after a failed scenario.
The existing strict Scenario admission gate retains its failure behavior.

The earlier log is local `target/composition-all-stages-focused.log`. A later enhanced
run (`target/composition-three-peer-final.log`) fails61.80s with ALL three endpoints at
zero completed/SDK peers and no collected native media. Its native parents, in slot order,
show these selection cycles:

| Endpoint | First selected peer | Second selected peer |
| --- | --- | --- |
|1|3|2|
|2|1|3|
|3|2|1|

No two first negotiations match. Native admission has one uncertified parent at a time;
protected ingress requires the exact pre-existing Root pair and chain before admitting
an encrypted confirmation (`voice_endpoint/ingress/encrypted.rs` ->
`presence/confirmation/protected.rs::admit_protected_confirmation`). This combination
supports a mismatched-pair wait cycle for THIS run. Both browser windows on each endpoint
expire within a few tens of milliseconds of their original15s bounds; there is no recorded
non-Capacity native IPC refusal. Clearing the resulting known selectors would bypass replay/
history protection rather than coordinate a matching pair.

The original two-peer positive passes40.19s on the same inputs. A subsequent run with the
reviewed defensive diagnostics fails60.89s with two endpoints at one peer and the third at
zero. It confirms the exact native collected-peer labels; the zero-peer endpoint's two
original windows expire while the others progress. Different scheduling orders therefore
expose more than one incomplete path. Do not assign the61.80s cycle as the cause of every
previous failure, or infer exact network receipt from these metadata observations.

A sampled `failed` scheduling stage can also occur while a SUCCESSFUL Source is closing
and indexed media remains healthy. The final helper records a plan's failure time only
from the actual negotiation's latched negative flag; it is local failure intent, not a
native terminal/drain certificate. The first refusal tags likewise need their original
operation/context before causal attribution. Eleven helper regressions cover these
truthfulness/bounds/privacy/reentry cases. The final negative-latch run
(`target/composition-three-peer-negative-latch.log`) remains red in60.67s: native media
between peers1--2 is collected and stays usable, while peer3 reaches neither RTC nor a
collected media entry. Successful first plans carry no false failure marker. No protocol fix or attempt/deadline change is
implemented here.

## Confirmed enforcement paths

- Native `discover` skips exact `Ledger::derivation_known` selectors. Failed original rows
  stay known; Plan-final/ACK is not permission to forget their history.
- `automatic_parent_available` permits another selector after certified retirement, while
  keeping the original failed selector terminal. It grants no retry of that old pair.
- Root `admit_protected_pair` reuses the exact peer join/key binding. Its original deadline,
  allocator, replay state and allowance cannot be recreated after expiry or retirement.
- The browser has one active negotiation and advances only after actual old work returns.
  Native availability alone cannot prove that browser callbacks or its paid graph drained.
- Successful native media Current is local pair authority, not proof that the remote endpoint
  retained/applied the last Answer. A helper queue acknowledgement is not that proof either.

These protections are intentional. Do not fix the reproduction by clearing known selectors,
resetting cursors/deadlines, relabelling old rows, increasing budgets, or refunding an attempt
after an ambiguous transport handoff. Local admission failure and network uncertainty must
remain distinct.

## Required design before new pair admission

The existing signed presence/watch proof establishes a fresh member endpoint, not its free
negotiation capacity. No current wire object implements the bilateral admission below.
Adding a new signed message without binding the resulting round into Root and browser key
derivation would leave the current terminal-handshake problem intact.

1. Only the lower full DeviceId initiates admission to a higher DeviceId. A HIGH endpoint
   accepts only while its original native and browser negotiation slots are genuinely free.
   Waiting edges then strictly increase DeviceId, preventing reservation cycles. Use bounded
   fair scheduling/cooldowns across peers; sorted preference alone must not starve another
   eligible peer under repeated joins or a silent target.
   Obtain local role/identity from the same paid native Actor snapshot/admission result;
   SelectedHandoffOwner currently carries no local DeviceId. UI identity and fixture labels
   cannot supply that missing enforcement authority. Root must revalidate orientation.
2. Before advertising availability, reserve one inert browser preclaim from the existing
   intake envelope. Do not start a ChainIntake or its15s deadline while waiting for the other
   endpoint. Queued peers carry bounded metadata, not a full paid preclaim per peer. Native
   needs equivalent original capacity and actual old-work return evidence. Transfer those
   same preclaims at admission; no new pool or extra execution headroom is implied.
   Define an authenticated exact-Session preclaim generation and close fence. Native must
   retain that exact prepaid generation before emitting Request/Grant; cancellation fences
   it before browser refund. A delayed grant/publication cannot enter a cancelled preclaim
   merely because the shared Session remains live. The renderer still cannot select peers.
3. Bind both signed reservations to exact group/policy/room, membership epoch, endpoint
   DeviceIds, local/remote joins and key references, protocol/profile, independent reservation
   nonces and bounded expiry. A native UI/Group/policy/owner fence still orders first effect
   against lock, End, replacement and epoch change after all waits.
4. Lost replies prevent finite handshakes from making both starts atomic. Define an explicit
   round identity and conservative recovery for a lost committed-start message. The round
   must enter the Root pair/transcript, native Purpose/history, browser input validation and
   derived keys. A new nonce on a control packet alone cannot renew the old handshake.
   Select the upgraded request shape only after measuring its complete inventory. An opaque
   RoundRef32 looked up in the original native record may avoid expanding every legacy
   selector. Adding 32 bytes to the longest 154-byte child prefix makes its ACK ciphertext
   282 bytes, including the 80-byte suffix and 16-byte AEAD tag, exceeding Control's 272 bytes.
   Even an eight-byte ordinal needs the complete 12-KiB handler-graph check. Neither
   representation is implemented or approved.
5. Retain the old round's replay/terminal floor and actual outstanding owners. A fresh round
   needs its own bounded native allowance and admission; no terminal boolean manufactures
   drainage. Define collection and restart semantics before discarding a failed round.
   Fresh intent cannot silently exhaust a lifetime table or replay an old effect.
   Define bounded retained state and refillable admission rate limits separately from the
   three Hello attempts inside an existing plan. A lifetime fresh-round allowance merely
   postpones exhaustion during a long call. Root retains at most 16 confirmation-pair entries
   and 16 packet-pair entries per LocalJoin, keyed by handshake. Its 32-slot ActionQueue is an
   occupancy limit, not a lifetime action counter: returned slot credits are reusable, while
   action serials remain monotone.
   Browser `failedHistoryRetained` also keeps actual fixed history until End. The
   configured/audio peak already fills its group envelope. Fresh retry therefore needs
   measured prepayment and, where necessary, exact failed-history collection/compaction
   while retaining terminal floors; a nominal round allowance cannot create that room.
   Retained actual owners may legitimately prevent reuse; deleting an index entry cannot
   make them free. No arbitrary-size full mesh or indefinite retry guarantee is established.
   State exactly when future-round allowance is consumed: a definite unstarted Busy/probe
   must not cost an ambiguously committed round. Signed requests keep their own original
   deadline/identity/tombstone; expiry is not a general refund or renewal.
6. New message/profile support must be explicit at ingress and helper selection. Preserve
   existing canonical bytes and fail closed with old endpoints. Legacy traffic must never
   write into an upgraded round or become an automatic downgrade path.

## Candidate implementation shape reviewed on 28 October

This is a design direction, not an implemented wire contract or activation approval.
The following resolves the incarnation/replay and ownership questions; measured capacity,
failed-round collection and the end-to-end protocol still need implementation and tests.

- Derive `RoundRef32` with a new domain from the canonical COMPLETE bilateral agreement:
  group/policy/room/epoch/profile, both DeviceIds/joins/key references, both public admission
  incarnations and generations, independent LOW/HIGH nonces, the exact Request digest and
  the fixed validity contract. Root Derive/KDF uses a distinct upgraded domain/profile
  together with RoundRef; a control nonce cannot renew the legacy transcript.
  It is never a caller-chosen label. Replace the upgraded native selector with this reference;
  do not append it. Prefixes105/122/123/138/154 become33/50/51/66/82 bytes, and the largest
  ACK is82+80+16=178 ciphertext bytes. This arithmetic does not establish the whole12KiB
  handler graph, allocation overlap or Source inventory.
- Root mints an unpredictable public admission-session incarnation using its sanctioned RNG.
  Actual native Session/selected-Grant registration owns its private current gate. Restart or
  replacement gets a fresh incarnation; remount/lost replies recover the same original
  registration. A reset generation integer alone is insufficient. Never advertise channel
  secrets or raw private Session identity. Signed capability/generation publication is a new
  explicit endpoint/helper profile; existing presence renewal does not promise free capacity.
  The native-owned generation advances monotonically on cancellation/terminal closure before
  slot reuse. EVERY upgraded creation requires the exact current local incarnation,
  generation and stored pending agreement; labels or a previous grant alone cannot recreate it.
- The actual Actor candidate supplies identity/orientation. LOW reserves affine inert capacity
  before Request; HIGH does so before Grant. Request carries LOW's nonce, Grant adds HIGH's
  independent nonce and the exact Request digest, and Commit binds both. One original slot
  retains one agreement. A definite unstarted Busy is distinct from ambiguous committed work;
  a refillable group/process admission budget bounds new attempts independently of each
  original Source submission allowance. A bounded fair cursor/cooldown policy is still needed.
- Capacity includes actual claims for the group progress cell,12 Action positions,
  role-specific packet/pair positions, crypto/lower lanes and native row position, as well as
  bytes. Consume the SAME claims at Commit. Do not install a dummy Workflow or begin the
  transcript clock while waiting. Another trusted same-group owner must be unable to take a
  reserved cell. Weak strong-count zero does not establish actual old-owner return.
- Commit consumes the original agreement/backing once under the original native/Root fence
  and creates an immutable committed `RoundGate`. Later free-slot generation advancement
  must leave established sibling media live. Old close/finally handlers compare original
  slot identity, generation and committed round before changing the slot. Full End revokes
  admission and established rounds; peer-targeted closure affects only that original round.
- Cancellation fences the original preclaim before browser/native refund and retains Closing
  ownership until actual return. Lost Commit recovers the same committed round or a permanent
  original cancelled/expired refusal; it never reconstructs an expired Grant. Upgraded Root
  creation requires a consumed current agreement and cannot fall back to legacy
  `admit_protected_pair` recovery/creation. Keep legacy cleanup separately compatible.
- Explicit failed-round Forget needs a paid terminal anchor and actual collection, including
  Source/Lower/IO/SDK owners. Retain exact accepted/rejected results until explicit Forget
  acknowledgement. Unknown old RoundRef after that boundary is always refusal, never
  collected success inferred from a generation floor. Canonical agreement hashing prevents
  reference reuse without a lifetime RoundRef set. Held actual owners may prevent reuse;
  more than32 sequential retired rounds must progress when those owners/callers return.

The minimum seam is `register_admission_session` -> `admission_candidate` ->
`reserve_round_capacity` -> `accept_request`/`grant` -> `commit_round`, with separate
`cancel_preclaim`, `terminal_status`, `collect_failed_round` and `forget_ack` paths.
These names describe the proposed ownership boundaries, not APIs already present.
Prepay agreement storage separately and carry a small affine/Arc handle through the Actor:
current AppCommand/Envelope are336/352 bytes, whole Control12,244/12,288 bytes, Root
metadata2KiB, Ready metadata4KiB and Source bookkeeping16,360/16,384 bytes. Hidden boxes,
large inline messages or extra pools are not a substitute for measured payment.

An alternative early Source-retirement path needs an authenticated, exact peer-bound native
Applied acknowledgement and actual lower-owner joins. Local media Current, SDK connected,
a renderer flag and helper receipt are insufficient. Faster retirement alone does not solve
admission while the counterpart is busy.

## Failed-round retirement: reviewed requirements, not implementation

The read-only review confirms that a failed plan without an indexed media entry cannot use
the existing live-lease media-history collection path. That path in native
`ledger/collection.rs` requires a retiring media entry. `ledger/media_index/history.rs`
validates that entry's exact original parent, action, Session
and issued/ACKed Plan proof. Browser `failedPlanCompletion` retains failed fixed history
until End, and `prepareChainCompaction` requires `mediaIndexed`. Do not manufacture a media
entry or remove that guard to permit retry.

A future explicit failed-plan Forget operation needs an independently prepaid terminal
anchor. It must bind the original round and exact Session, grant/executor/lease, browser
preclaim generation, group/policy/room/epoch, endpoint joins/keys and agreement. Original
Return/settlement, closed children, exact Plan issuance and ACK, and actual Claim/key/storage/
SDK/Source/Lower/IO return remain required. Native scheduling availability alone grants no
Forget authority.

Install the paid anchor and permanent negative replay floor before unlinking rows. Extend
the existing affine collector and exact-subtree removal rather than bypassing them. Rows
must drop outside Index/Row guards. Only actual removal-batch destruction may request the
original Root `PlanActionHistory` collection; positive completion additionally requires
`is_collected()`, including actual fixed-credit return. Dead-owner/End cleanup remains a
separate path.

Browser failed-history compaction must follow the failed worker's actual finally without
awaiting that worker from itself. It needs the exact generation, original final ACK and
authenticated native collection receipt. Preserve negative publication before callbacks,
dispatch-cell detachment, physical joins, graph-edge clearing, synchronous commit and refund
after the heavy frame returns. Advancement still requires the full `negotiationQuiet`
barrier. Old exported handles stay inert; a detached handle and its closures do not cease
to occupy memory merely because their heavy target has been removed.

Retain separate issued/rejected and actually-collected floors. A terminal ordinal may
include never-admitted or cancelled gaps, so `ordinal <= floor` cannot establish collection.
An old opaque RoundRef cannot be authenticated after its original anchor disappears merely
by comparing a number. Retain the exact paid completed receipt until authenticated browser
forget acknowledgement or an exact successor transfer makes recovery unnecessary. A
stateless certificate would need its own specified and reviewed authentication contract.
Lost ordinary terminal replies must remain recoverable until explicit Forget begins.

Funding and overlap need executable inventories before publication. A failed anchor lacks
the successful media entry's complementary 2-KiB funding. Its whole allocation, candidate
overlap, Session/provenance and collector frames must be prepaid; the existing 2304-byte Row
fee cannot return while the old Row or handle remains. Browser compaction currently retains
2 KiB per plan until Session collection, rather than one marker per peer. The configured
audio peak already fills 256 KiB. Floors, completed receipts and old caller-held handles need
a bounded transfer strategy, not an extra pool or a nominal counter reset. A bounded
peer-floor table also needs eviction semantics that preserve old signed authority and
actual outstanding owners. Distinct-device churn is not covered by the current design.

These are design blockers for new-round recovery, not newly demonstrated bypasses in the
existing refusal behavior. No failed-plan Forget, floor/receipt protocol or new key domain
is implemented by this branch. The review does not approve an unspecified replacement.

## Acceptance required for the implementation

Keep the positive all-three test; it must pass without scripted pairing or manufactured
permission. Pin the first native admission/terminal cause with bounded, non-creating metadata
observations. Add deterministic tests for crossed intents and3--8 peer wait graphs, reordered
or lost Proposal/Accept/Commit/replies, duplicate and expired intents, busy/retiring browser
preclaims, peer fairness, full tables and group pressure, epoch/lock/unlock/restart, old round
traffic after replacement, held actual transport/SDK owners, incompatible peers, and progress
of a different group. Assert exact peers and endpoint receipt over forbidden-edge real TCP.
Also cover more than 32 sequential retired rounds without monotonically retained resources;
held original Action/Row/SDK/Source/Lower owners; lost Forget, Collected and forget-ACK replies;
floor gaps; delayed old Begin/Input/Result/confirmation; cancelled preclaims; retained browser
handles; full peer-floor tables; and native/Control/Sourcebook/configured-audio inventories.

An aggregate multi-peer test bound must follow its proven number of serial protocol phases;
it cannot hide an unresolved first-attempt failure. Do not change the original per-attempt
window or the existing two-peer bound. Actual RTC/audio and physical tests remain separate.

## Validation status

The later combined integration at `fa3107d2` passes complete local native validation:
1,011 library tests plus six ACL tests, with both earlier fixture failures corrected.
Root passes 3,960 / 25 existing ignores on unchanged compiled Root inputs; the current
native-source construction scan passes separately. Frontend passes 3,539, and native
check/strict Clippy/build/startup pass. This does not include #73's deliberately red
all-three reproduction or implement the admission/Forget design above. V2 remains OFF.
Both refreshed dependency audits now pass on the combined candidate; the older red
audit statements below describe their earlier checkpoints only. Hosted CI is pending.

- Published Broker45 checkpoint ecfd3cd3:897 native library+5 ACL pass; native format/check/
  strict Clippy/construction scan pass. Complete Root3,691/24 existing ignores apply to its
  unchanged Root Rust/Cargo inputs. This does not cover this new composition test.
- Published Broker46 checkpoint adf74d41: its complete native run was interrupted across
  the27 October restart, with no final result. Its38 explicitly mirrored inputs and725
  byte-equal tracked Root Rust/Cargo inputs were verified before the new focused run.
  It excludes this failing test and must not be reported as validating it.
- This composition worktree: full frontend3,205 pass62.66s, including all11 helper
  regressions; strict helper/interop TypeScript passes. The non-creating native diagnostic
  unit passes. The original two-peer positive passes40.19s; all-three remains RED, most
  recently60.67s with the actual negative-latch diagnostics. Svelte check has zero diagnostics
  and production build passes36.04s. Read-only re-review has no remaining finding in this
  reproduction. Complete native/Root handoff gates are not claimed for this unfinished,
  deliberately red investigation checkpoint.
- Read-only design/adversarial review found no blocker in the reproduction/oracles/cleanup;
  it confirms immutable failure suppression and the need for a distinct admission/round
  design. That is not a review approval for an unimplemented protocol.
- Dependency audit remains red on inherited RUSTSEC-2026-0387/crypto-bigint0.5.5; no waiver.

Capture/playback wiring, production App lifecycle/renewal, automatic multi-peer admission,
member media relay, combined acceptance and physical acceptance remain open. One desktop is
available. External-download resume and dependency upgrades are separately owned.
