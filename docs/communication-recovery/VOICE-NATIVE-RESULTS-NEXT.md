# Next voice milestone: native result recovery

## Round wire isolation implemented; SDP effects still next (24 October)

New Ready body6/profile1 and Offer8/Answer9 authenticate the actual native Ready message ID.
Original body5 and encrypted kinds1-7 retain their exact bytes. Protected mode is pinned before
capacity admission and survives round cleanup; ordinary verifier/namespace paths cannot admit
old/wrong-round SDP or separate ICE. See INTERFACES for the exact closed grammar.

Current maxima: AAD579B, exchange188,521/192KiB, signed Ready692B, Ready peak32,644/32KiB.
The original one-shot Finish/Ready deadlines are unchanged. This completes wire/admission
isolation, not fresh long-lived round creation, browser SDK prepayment, seal/open, remote-SDP
delivery tickets, automatic application scheduling, multi-peer compaction or member media relay.
Those remain the next implementation work. Older status/size sections below are historical.


## Reviewed operational handoff implementation (24 October; design, not code)

The next implementation uses separate prepaid native SdpHandoffPhase and browser SdpSdkPhase
subtrees, with64KiB each as measured candidates. Native group cap is512KiB; the separate browser
broker cap is256KiB. Do not assume256KiB native or spend the existing8087B core slack on a16KiB
UTF16 string. Do not multiplex the Crypto stage in the first integration: result HTTP/actor
commit still overlaps the actual AEAD engine owner. Any new limit must be pinned to the whole
largest graph, including real two-Lower custody, old fixed history, channel/row payments,
plaintext, sealed request, BYOB body, opened reply, UTF16 strings and SDK continuation objects.

A fixed native KeyEffectState belongs under the original KeyClaim. It owns closure/drain
state and native round/PC-generation counters, with no strong parent Row or Exchange backedge.
Begin/recovery binds the same authenticated Session, parent RowId/action, key receipt,
SelectedHandoffOwner and UI generation. Final admission follows UI -> Coordinator -> Route ->
try-Index/row. Key Release closes effects first; it does not refund actual owners. Preserve a
possibly admitted intent across lost replies/capacity refusals. Plan::can_adopt_child remains
confirmation-only; add a distinct SDP purpose rather than weakening that existing boundary.

For the first operational round, retain the current finite contract. HIGH prepays its sidecar
after actual old Workflow drainage and before operational Ready; LOW may record Ready first,
then transfer/drain and prepay before any PC/SDK effect. Later prepare_established_round needs
the exact original EstablishedKeyCustody plus a native KeyClaim effect lease and exact native
intent. Its EstablishedRoundProof is distinct from finite SuccessorProof. Do not update old
Claim/Finish/Ready deadlines or issuance bits. A single atomic lifecycle fences preparation
against closure, with actual last-owner publication included in whole-plan retirement.

Add closed versioned SDP Data operations and a funded phase lookup after Header MAC but BEFORE
body allocation/counter debit. Existing v1/v2 Input/Result2048B and fixed12KiB Control handling
stay unchanged. A separate boxed funded worker owns the new maximum8192B body plus AAD579B
and exact descriptor. Native body ownership must move into Action/Packet/Result without a
refund/reacquisition gap. No generic Data fallback or JSON plaintext SDP command is permitted.

Typed seal/open inputs bind Session/key receipt, Ready ID/sequence/profile, OfferId,
PC generation, action serial/version/tag and exact incoming token/AAD/sequence/ciphertext.
LOW first receives InitiatorPendingOffer before its OfferId exists. Gather complete ICE, freeze
one exact bounded UTF8 owner, then mint one native packet/action; exact retry recovers it.
HIGH plaintext stays private until native open commit and a fresh exact delivery permission.
Ordinary protected-key encrypt/open still refuse; add a module-private SDP computation seam.
The RTC driver must obtain its native PC-generation permit before constructing the PC, build
its intended audio/jam/drawing topology before gathering, wait for actual ICE completion and
check8192 UTF8 bytes without truncation. SDK helpers return Promise<void> and affine results;
never cache a resolved SDP-bearing promise/string and then refund it. End/timeout/PC close is
not proof that a still-pending SDK promise drained.

Repeated rounds need actual prior input/SDK/Lower/result drainage, the same key/nonce/replay
domain, and authenticated compaction beyond32 native/browser actions. HIGH issues explicit
positive monotone sequences, refuses wrap and preserves the same intent through failure. LOW
retains a paid committed sequence/ID/hash floor: lower and equal-different input refuses;
equal-exact recovers; higher cannot discard still-busy work. LOW recovery/ICE restart also
needs a bounded signed request with its own serial and last Ready identity; duplicates recover
the same HIGH intent. Post-Workflow control verification needs its own explicit funding.
Pump completion must compare full RowId/serial, not slot bits. Do not evict unresolved history.

Acceptance must hold SDK work over End/Release, lose every intent/data/result/delivery reply,
reorder old/new Ready, use exact UTF8 boundaries, run more than32 actions without resetting
packet counters, preserve another peer/group's progress and refuse an old funded broker on
HMR after any ABI version change. Separate native/browser phase accounting and real WebView
RTC/audio still need implementation and evidence. Physical two-device/network testing remains
unavailable with the single desktop; synthetic/local media evidence must be labelled honestly.

## Original key adoption now implemented (24 October)

Distinct Root EstablishedKeyCustody, exact authenticated opcode11 native adoption/recovery/
release and browser23 pending quarantine now preserve the original key domain. Normal old
confirmation expiry does not erase it; old Claim/action/Ready/Exchange deadlines are unchanged.
Actual local old work returns independently while fixed key/history custody remains paid.
The reviewed design below is partly historical: actual captured opaque Finish proof removed
the need for another Finish-proof lookup in key retention. Normal Actor context preflight
still awaits Root before final synchronous admission. No SDP/RTC permission follows from this leaf.

NEXT: native long-lived key-session/round admission;
prepaid native/browser SDK/IPC/UTF16 coexistence before any receive promise; typed seal/open and
private decrypted-result commit; fresh exact round/Offer/PC-generation RTC delivery; old-round
compaction after real custody drainage. Then multi-peer scheduling, member media relay and
combined acceptance. The current one-shot Ready/exchange is still bounded by original Finish;
it cannot be reset or lengthened to stand in for a live SDP round. HANDOVER records evidence
and remaining low native late-recovery/first-adoption-negative coverage gaps.

## Current LOW Ready boundary and next consumer (24 October)

Exact protected LOW Ready/Root admission, immutable fixed receipt, original Workflow verifier,
registered inactive-Exchange exact retry lender and delayed original-lane extraction now exist.
Only native Ready stops LOW Finish; actual ticket/reply/handoff/Source/Lower owners still drain
through real custody. The1600B plain history fee uses no action row; ten confirmation rows keep
both original SDP positions. Namespace, original Finish/epoch/joins/keys/helper route, strict
first deadline, native clock/revocation, ordinary pressure and fixed-history cleanup are pinned.

NEXT: operational native/application progression and authenticated original-key browser SDP
phase adoption. That proof must bind the actual original Claim/session/parent action and current
native LOW Ready or HIGH Offer, without extending old Claim/action authority. It must prepay
HTTP/IPC/body/UTF16 SDK strings before SDK awaits, keep actual owners through lost replies/End,
and issue typed SDP/result/RTC permissions. Current528/2048-byte confirmation data grammar and
192KiB SDP accounting do not establish that composition. The complete-SDP wire profile is implemented; browser gathering and RTC delivery remain open. Multi-peer, member relay, combined and physical acceptance follow.
Older sections below describe their checkpoints; they are not a claim of live availability.

### Reviewed adoption/round dependencies (design only; not implemented)

Separate key retention from SDP effects. A native established-key custodian must adopt the
SAME actual protected Finish, original Claim/session/parent action, executor, call, pair and
epoch while original custody is current. Requiring HIGH's first Offer before retaining keys
would strand a gather/bridge failure before that Offer at the old30s expiry. LOW local Finish
may justify key retention but still cannot stop Finish retries or grant SDP preparation.

Before its adoption RPC, browser must install a paid PendingAdoption owner of EXISTING keys.
An uncertain reply may quarantine them for exact same-session recovery/cleanup after old
expiry, with NO SDK/packet/RTC authority. It must never derive/reconstruct keys or renew the old
Claim. Native latches the same pending identity before its Root await, retains actual returned
publication before replying, and repeats final UI/selection/epoch/native gates. End first closes
effects, drains actual SDK/RPC owners, recovers the original adoption and releases every actual
obligation. Permanent history holds fixed observations, not Exchange/large weak/input custody.

Each later SDP round needs a new bounded owner after actual preceding round input/SDK/Lower/
result drainage. Keys, nonce domain and burned sequence counters persist independently of its
Crypto lane. Bind the exact native round (for example Ready message ID) into signed Offer/Answer,
AAD, actions, fixed receipts and PC generation using an explicit closed mode/version grammar.
A delayed, previously unadmitted old Offer must not activate a newer Ready even if its global
sequence/freshness still passes. Never reset old attempt/action issuance to manufacture a round.

LOW starts with an InitiatorPendingOffer preparation lease: OfferId currently comes from the
native packet MessageId when ciphertext length is known, AFTER complete SDP gathering. Freeze
that exact string/bytes once, then create one original draft/action/OfferId. HIGH already has its
native admitted token/OfferId. Gathering listeners precede completion checks; after completion
read localDescription.sdp once and enforce8192 UTF-8 bytes, never truncate. Separate ICE is
rejected at both boundaries in this initial complete-SDP profile. Preserve audio/jam/drawing.

Introduce typed SDP seal/open actions and retained-only successor handoff custody; old
protected_plan/borrow_handoff depend on closed confirmation Workflow. Keep decrypted text private
through exact native token/AAD/sequence/ciphertext/round commit. Only a fresh same-session/PC/
Offer/profile delivery ticket may authorize setRemoteDescription. Pay UTF-8, UTF-16, IPC/native/
SDK copies and keep them through real finally. An unresolved SDK promise cannot be refunded by
expiry. The192KiB native declaration leaves8642B; one maximum UTF-16 SDP string alone needs
16384B. Browser's current96KiB variable+32KiB fixed+28KiB retained channel/key composition also
cannot simply add192KiB under its256KiB group ceiling. Measure complete coexistence and review
any new bounded envelope/limit; do not use ordinary fallback or silently discard a hedge.

Multi-peer/repeated rounds additionally require explicit terminal compaction after real ACK,
settlement and copy drainage. Root ActionQueue/native rows currently retain32 claims; browser
NativeWorkPump has one parent and slot-only completed bits. Use full RowId/generation/round
identity, with paid closed-round tombstones/replay floor, rather than resetting bits or growing
history forever. Established peers retain paid keys/PC state while the sole variable negotiation
position moves fairly to another peer. This boundary is required for production recovery, not
just an optional extension of a successful first SDP exchange.

## Current Ready Source implementation (24 October)

Trusted native Root prepare/queue, exact inline cache/receiver/position provenance, original
stamping clock, frozen helpers and causal retry completion now exist. ACKs cannot complete
Ready or refund its three attempts; exhausted live records stay retained. Native first-Offer
publication activates only after token/event/history and consumes the position that wakes
Ready cancellation. Dedicated pending waits avoid generic inactive Exchange cancellation.
Ready Source4267B plus metadata/cache/receipt/two Lower/ACK totals32,599B within32KiB. Final
ACK tickets sample outside nonblocking Group/Policy guards and reject bound incoming tickets.
Focused evidence is recorded in HANDOVER; application scheduling stays off.

NEXT: LOW Ready bounded original-Workflow verifier and exact native fixed receipt admission;
shared immutable-ID classification; original signed Finish identity/hash/epoch/helper adoption;
retry delayed transfer only after actual verification/reply owners drain; retire LOW Finish
Sources only after causal Ready. Then implement original-key browser SDP adoption and typed
SDP/RTC profile, serial multi-peer composition, relay and combined acceptance. Older sections
below describe the then-missing Source boundary, not current application availability.

## Current first-Offer implementation (24 October)

Root HIGH Ready preparation now reserves its shared endpoint receipt position; encrypted Offer
intake uses the original receiver slot/bytes and publishes an exact token/input/fixed receipt
through final native origin admission. Lost replies, consumed input, immutable ID/kind collisions,
full ordinary capacity, final native close/unlock and frozen-helper enforcement have focused
regressions. Fixed receipt funding adds1536B, making Ready's declared peak30,551B within32KiB.
Preparation/committed graph guards include the native event, token/role, map allowances and
namespace allocation. Quiet active cleanup takes native holders while actual external input
retains custody through last Drop. SDP remains capped by original Finish pending browser phase.

NEXT: causal Ready Source using the original Producer and strict deadline (transport ACK cannot
stop retries); LOW Ready exact admission and delayed transfer after actual verifier drainage;
original-key native/browser SDP adoption; typed bounded SDP/RTC integration; multi-peer and
relay composition; combined acceptance. Application Ready scheduling and V2 UI activation stay
off while these consumers are missing. The ordered preparation notes below describe the prior
checkpoint; their absent first-Offer token/receipt and cleanup constraints are now implemented.

## Current transition boundary and next implementation (24 October)

Original browser/native key custody is implemented by opcode10 Claim/Release and Broker22;
the older proposed browser boundary below is historical. Whole-plan retirement now includes
actual prepared/ticket/Exchange capacity, independently of old-Workflow drainage. Ready32KiB
is prepaid before extraction. Independent Producer quotas, original member-row prepayment
and a private first-stamped/signed artifact now exist; obsolete HIGH confirmation caches
retire after native Finish-open/old closure. Maximum signed Ready683B, declared29,015B
capacity composition and actual owner layouts are pinned. This still issues no Ready
Source or SDK/RTC authority. The next receive/intent preparation leaf is recorded in HANDOVER
with 608 focused Root passes; normal endpoint admission remains absent.

The reviewed implementation order is:

1. Independent Producer, private immutable unsigned/signed cache, native endpoint/helper
   origin-row payment and separate quotas are implemented. Producer retains Exchange LAST;
   Exchange never retains Producer/cache. A native intent marker exists, still unscheduled.
   Preserve the first native monotonic deadline and stamp across every later refusal/lost reply.
2. HIGH now has an original-slot/prepaid-byte receive preparation owner and immutable native
   Ready intent marker. Restricted first-Offer verification is implemented; generic Exchange
   stays inactive. Implement the actual recoverable native token/receipt admission before
   scheduling this capability or treating the intent as an operational readiness promise.
3. Give native Ready Source causal-Offer completion. Direct/helper transport ACKs cannot stop
   its retries. Terminal record pruning waits for actual flights and cannot reset attempts.
4. Admit LOW Ready through a bounded peek into the original protected Workflow loan, normal
   work budgets/signatures/native final admission and shared immutable-ID classification.
   Store exact paid receipt without Workflow custody; do not transfer inline while the real
   verification/reply owner still holds the lane. Maintenance retries the same preparation.
5. Native first-Offer publication must retain the exact recoverable token/event outside
   Exchange. Prepare all fallible gates before origin debit, then publish token/replay/OfferId,
   Active phase and Ready terminal state infallibly. Lost ACK recovers the same token.
6. Add authenticated browser key-phase transfer, typed SDP/ICE actions/results and fresh RTC
   authority. Preserve original keys/counter domain. Release is cleanup, not phase transfer;
   old confirmation expiry must remain unchanged. Only the new phase may have its own
   once-admitted bounded work deadline. Multi-peer/relay integration follows this path.

Ready/receive/adoption stays bounded by the original Finish window. A Ready publication or
packet header does not prove SDP decryption, RTC connection or audible media. Full current
evidence and the open dependency audit are in HANDOVER and the communication acceptance ledger.

## First-Offer admission constraints reviewed on 24 October (not implemented)

Root's combined public/encrypted receipt namespace has its OWN256-entry limit. The incoming
PacketSlot does not reserve it. Before a transmitted Ready promises full-pressure reception,
reserve one combined receipt position while ordinary admissions respect that reservation, or
prove that saturation cannot coexist with the paid receiver. Check this before origin debit
and cover an actually full Root receipt table; never bypass shared immutable-ID classification.

The next consumer must take the receiver's SAME slot and paid bytes into native token/event
preparation; normal PacketLedger incoming allocation would acquire both twice and fail.
Prepare all replay/token/OfferId/current-owner gates before EndpointOrigin's single debit.
Its apply tail must atomically publish token/event, fixed recoverable receipt, active role and
causal completion. Signal Ready completion through a pure marker; do not enter SourceLedger
from Group/Policy, which would invert SourceState -> Group -> Policy ordering.

Fixed admission history must own neither Exchange/large Weak nor Receiver/event/WorkLane.
Use an exact original non-authorizing PlanBinding observation plus separately prepaid plain
retained bytes, paid before Ready. Revisit the declared Ready peak before choosing that fee.
Do not split a Producer/PhaseLoan-backed credit: split_retained preserves its progress custody
and would keep Exchange/lane alive. Obtain plain reserved bytes before adding such custody.
Old PlanBinding/confirmation liveness is closed after extraction and cannot serve as the new
receipt's current predicate. Exact historical token identity is not another decrypt grant.
After the original progress lane drains, history alone cannot promise ACK verification under
permanently full ordinary Crypto capacity; that would need another explicitly paid verifier.

Until authenticated browser SDP key-phase adoption exists, every SDP deadline must be capped
at min(first-Offer native time +20s, original Finish.until, applicable packet bounds). A late
Offer cannot silently receive20s beyond the browser's original key custody. Do not change the
old Finish/Claim deadline. This conservative cap can be replaced only by the explicit new
native/browser phase receipt and its separately reviewed once-bounded lifetime.

Activation also needs actual cleanup, not just an atomic flag. The current pending collector
ignores activated owners, and PacketLedger's old prune closes but retains its SDP owner.
Native Pair/cache/receiver and exact closed ledger.sdp must be taken outside final guards on
active expiry/revocation, while actual lower/input/SDK owners keep custody through last Drop.
A fixed recoverable receipt remains separately bounded; it must not block serial next-peer
registration or whole-plan Final after actual progress has drained.

## High-side affine successor continuation (24 October; unscheduled)

Native HIGH can now retain Reserved capacity across busy-lane refusal and consume the
original lane/reserve/action claims/Lower owner into an inactive Exchange after actual
protected Finish-open. Pair owns pending before any fallible final fence or registry attempt;
terminal expiry removes native closed custody. Actual external observations remain charged
through last-drop. Pair's permission-independent housekeeping covers unregistered conflicts
too. There is no activation method, browser key phase transfer, causal Ready admission or
application scheduling in this leaf. LOW extraction remains absent so original Finish
retries cannot be stopped before HIGH has received them. HANDOVER records current checks.

The next browser boundary must preserve keys BEFORE a Finish that committed near the old
deadline finishes asynchronous SDK/result/Return drainage. Split custody lifetime from
original confirmation-action authority; never lengthen generic chain.expires. An exact
authenticated native transition receipt must bind the retained channel/selected key/parent
plan and original epoch/Finish/window, with conservative relative remaining time. LOW may
obtain bounded AwaitReady key custody after actual native local Finish sealing, but it may
not extract SDP or retire Finish Source until exact signed Ready. Public historical evidence
or a browser boolean is not that receipt. Lost replies cannot rearm either lifetime.

## Automatic browser consumption (24 October; validation in progress)

Broker21 now implements an explicit single-pair existing-work consumer on the exact Ready
channel/selected key. It pins original Derive/confirmation handles through actual Return and
joins metadata/attempt work before End cleanup. Focused browser170 and strict Node TypeScript
pass, as do complete frontend2,671, Svelte checks and production frontend build;
automatic blocked-TCP passes19.42s and complete current Root/native suites remain in progress. Replayable native
events, causal Finish-to-SDP transfer and all later integration listed below remain open.

## Current production continuation (24 October; validation in progress)

Application-bound authenticated v1/v2 effects and bounded native maintenance now exist in
ordinary code. The Host schedules original Derive/Pending and actual confirmation work through
separately paid native-position tasks; incoming actions come from the existing paid Root
snapshot. Browser19 has an exact coalesced SDK/settlement/ordinary Derive Return, preserving
keys/counters/deadline before native Start. End fences/drains that flight before final cleanup.
HANDOVER records current focused results and unresolved validation; earlier disabled-dispatch
descriptions below are historical. UI V2 activation and the complete call adapter stay off.

Broker20 now discovers unseen work during cleanup after actual SDK/RPC drainage. Its original
metadata payment precedes setup ACK; close-first stops new effects while preserving Control.
Children close before a retained parent retries; stored final proofs survive lost replies.
Normal attached-parent closure requires actual issued/ACKed final delivery, independently
of Root End. HANDOVER records reviewed boundaries and pending current native/full gates.

Remaining implementation order after the single-pair consumer: complete browser orchestration and replayable native events;
explicit causal Finish-to-SDP transfer; separate typed SDP/ICE factories and byte grammar;
exact native receipt commit plus fresh RTC dispatch authority; measured browser/RTC custody;
serial multi-peer progress and terminal-history reclamation; scoped media relay and combined
acceptance. Low-side Finish sealing cannot retire its retry custody or start an Offer budget
before the responder can causally be known ready. Native ICE must resolve the active OfferId/
pair/direction/generation before publishing replay/token work. The current single Exchange and
32 retained rows cannot be advertised as a supported16-peer lifecycle. Physical two-device/
separate-network audible acceptance is still unavailable from the one desktop.

### Reviewed SDP transition design (native Finish custody implemented; adoption open)

The follow-up review identifies two additional prerequisites. `Source::finish` currently
retires targeted retries on `EndpointAdmitted`; Ready instead needs an internally selected
causal-completion policy, retaining its original allowance until the authenticated exact
Offer witness. Do not renew the finite Source budget. Its verification can borrow only the
exact activated Exchange lane, with independent Ready byte payment and real custody.

Move the original Workflow Crypto lane affinely rather than close/re-reserve it: freeze old
confirmation adoption/effects, require the actual lane borrower to be idle, take it from
protected Workflow state, and install it into a separately prepared Exchange. Old inputs,
SDK/IPC and Lower owners remain charged; later Workflow close must not close the moved lane.
Prepared Exchange cannot lend verification/crypto or admit SDP before explicit activation.
The existing confirmation deadline remains unchanged. A separately bounded transition must
preserve the confirmed pair while old owners drain, because late Finish plus actual Lower
retirement can outlast that deadline. Capture this phase deadline once, without new Finish
attempts. Browser key transfer requires the same explicit lifetime treatment.

Ready uses closed Body tag5 while preserving tags1–4 and signing domains. Bind the original
protected epoch, both endpoints/joins/key references, handshake, and native signed Finish
message ID/object hash (not the WebCrypto output fingerprint). Require high-to-low direction.
Helper selection needs one immutable compatible canonical list, at most two members, rather
than independent endpoint choices. The native accepted Finish ID/hash and one immutable
transfer-custody window now commit atomically. This smaller boundary preserves confirmation
under full ordinary-pool pressure without promising SDP capacity. Later native maintenance
must prepay the actual successor and independent Ready owner before issuing any Ready;
allocation refusal must not repeat Finish, consume another token or spend a Source attempt.
Signing/RNG/payment remain outside the irreversible Ready transaction. Incoming
Ready uses a shared immutable message identity namespace and exact synchronous pair admission,
not a queued public ACK. A protected AwaitReady peek selects capacity only; normal signature,
requester, policy/rate/freshness checks remain. These changes are not implemented by the pump.

The next increment should give high a small independently prepaid Ready/control Source
owner, prepared within the ORIGINAL native Finish window. Its ID/deadline and canonical
helper choice are published once after actual capacity exists. The Ready deadline must fit
the original window; retries cannot renew either one. High publishes Ready only after its confirmation owners actually drain
and its exact SDP receive owner is installed. Low accepts that original signed Ready only for
the exact Finish ID/hash, scope/current protected epoch, endpoint devices/joins/key references
and handshake, then retires only Finish retry authority and transfers its original keys.
This avoids consuming low's finite Offer attempts while high is unable to retain the Offer.
Ready needs its own Source slot/payment: borrowing Exchange's single Source stage would
block the Answer, and retaining Workflow to fund Ready would prevent the intended transfer.

Keep Ready's exact signed bytes, first issue/expiry and Source allowance through lost replies.
High may retire Ready retries on a fully authenticated canonical low-issued original-pair
Offer header; this is causal retry evidence only, never plaintext/token/RTC authorization.
An authenticated malicious low can sabotage only its own pair with bad ciphertext. Compare
the actual original protected epoch to current context: Scope and the Offer header do not
carry that epoch. Third-party, other-pair/join and high-issued Offers must not advance it.

Ready verification needs a narrowly selected existing protected loan under ordinary-pool
pressure. Untrusted peek selects capacity only; normal signature/requester/rate/current-view
checks remain mandatory. Release actual verification Crypto custody before awaiting transfer.
Use explicit browser/native transfer owners preserving the same key objects and burned
sequence domain; closeChain/Plan-final/Return are not transfers. The new closed Body tag must
leave existing signing domains/tags unchanged and update target/codec/validation/peeking,
helper routing, Source activity and exact current-owner enforcement together.

If Ready is instead published before high has SDP capacity, high must retain one fully paid
original Offer inbox and its exact first deadline, not only a hash or a refused transport ACK.
That alternative also needs protected pre-Exchange Offer verification and must release the
verification ticket before waiting for old Lowers. The independent Ready owner is preferred
because it removes those extra receive states. Tests must hold Finish and two real Lowers,
fill ordinary capacity, lose replies, change every binding, and End/lock/replace at each
transition; no new deadline, attempt budget, key or allocator may be manufactured.

## Existing work projection continuation (24 October; validated inactive integration)

`read_voice_channel_work` and `voice-ipc-work.ts` now implement a bounded metadata-only
snapshot seam for existing native rows. Canonical original registry provenance, a serial
paid projection, full original purpose/parent, fixed JSON and one exact Row/parent lifetime
are enforced. Pending/Closing/ClosedPending/attached stay distinct after End/lock; slot hints
require rescanning and are not VoiceV2Native event cursors. Empty never proves no producer.
Read-only review corrected process-ledger retention and impossible decoder references.
Browser/security13 focus, complete frontend2,642/check/build/ten flows and strict Node
TypeScript pass. Native handoff focus passes292; corrected TCP passes all seven (88.73s),
including actual Node/WebCrypto and forced local preflight contention before two mandatory
exact known-publication refusals. HANDOVER preserves the earlier6/1 run and unrecorded
original Capacity site. Complete Root3,320 (24 existing ignored), native779 and frontend2,642
pass, zero failures. Native check/build/startup, Root/native strict Clippy/formatting, dependency
and ambient gates, strict Node TypeScript and all ten browser flows pass. HANDOVER records
exact commands/logs and measurements; shipping effects and V2 activation remain disabled.

The Node fixture now reads its Derive descriptor through this native handler and actual
decoder; it no longer receives row/action identity from the orchestration step. Production
orchestration still must create work independently, account for bounded incoming-action
snapshot hints, coalesce browser pulls under original channel custody, and clean up unseen
Pending only after admitted native production actually drains. Then finish event replay,
SDP/ICE factories/RTC, Finish-to-SDP transfer and supported multi-peer composition. Shipping
effect dispatch/activation remain off. HANDOVER contains exact validation scopes and limits.

## Native/Node interoperability continuation (23 October)

Runtime afcbee27 on bfe8439e now passes a genuine native/Node single-pair cycle: separate actual
WebCrypto brokers, non-extractable selected keys, authenticated native HTTP/BYOB, real A-C
blocked TCP via unjoined B, Derive and all six Hello/Proof/Finish receipts and exact final
collection. It deliberately forces actual local Input Capacity and requires retry of the
original intake before native Derived success. Focused native285, browser141 and all seven
real TCP scenarios pass. Complete native772 and frontend2,638 pass; Root3,314 (24 existing
ignored) covers identical unchanged Root inputs. Strict native static/build/startup and browser
check/build/ten flows pass. HANDOVER records exact scopes, measurements and failure history.

This proves neither independent established-pair authority nor packaged WebView/ACL/CORS,
production work delivery or RTC. Remaining work below must keep those boundaries explicit;
shipping effects and V2 activation are still disabled. The native/Node fixture supersedes
the earlier statement that all native TCP confirmation uses fixed AES keys, not the remaining
implementation prerequisites. One desktop still cannot provide separate physical-network
and two-device audio acceptance.

### Native query boundary (continuation implemented; validation pending)

The continuation exposes a fixed native `Confirming/Established` observation for the actual grant, original
PairSelector and handshake through ChannelSync and the bounded Actor mailbox. Resolve only
the existing participant/pair: never call a pair factory or clone its transcript to answer
this query. Require the original grant allocation, live call/watch and exact scope, peer,
join/key reference and protected epoch equal to the current context epoch. LocalJoin's
existing `pair_established` alone does not enforce that protected-epoch comparison. Fresh
current-owner/room/watch checks must still precede return, with Clock callbacks outside final
native gates. The observation grants no SDP/RTC authority and must create no action, packet,
deadline renewal or healthy revision wake. Existing control validation, reconciliation and
context eligibility retain their normal roster allocations; the member predicate copies only
two booleans. The query adds no persistent owner. The Actor wrapper
retains its ordinary bounded mailbox/oneshot overhead.

Local confirmation is asymmetric: native signing Finish establishes low locally; high
establishes only after committed Finish open. Tests query both real endpoints independently and pin
low Established/high Confirming with Finish held, plus wrong grant/selector/handshake,
End/replacement/lock, epoch/policy/member/room change and ordinary discovery-expiry cases.
Do not retire Finish retry custody from a local Established observation.

Subsequent production work delivery needs its own prepaid bounded metadata projection tied to
the actual selected owner, parent and Row/action. The coalesced revision watch is a wake, not
a replayable cursor. Preserve Unknown/Pending/attached and lost-reply/output custody; neither
this query nor a renderer `paired` item replaces final native SDP admission.

## Current implementation boundary (23 October; combined continuation)

The child-v2 grammar and Begin/Input/admission/result/ACK/settlement/Return adapters now
exist on the combined continuation, as does the browser continuation on the original
Derive chain. The native owner accepts the `VoiceV2Native` interface. Earlier statements
below that these pieces are absent describe their named historical checkpoints.

HANDOVER and CHAT-FILES-VOICE-ACCEPTANCE record the current source and validation. The
deadline-admission correction and Tokio patch are published; runtime9109f20d passes complete
native759, Root3,314 (24 existing ignored) and frontend2,638 on the documented exact input
scopes, plus static/build/startup and ten browser flows. The separate browser and native
tests do not yet establish an actual native-to-WebCrypto confirmation cycle. The native
TCP provider uses fixed test AES keys, while browser tests use framed endpoint mocks.

Native producer 7edf9192 adds the private paid origin needed by that integration. It checks
the canonical Route, owns a separate native Session position and reserves 8KiB before
preflight. The physical factory keeps its existing 4KiB task payment. Focused handoff tests
pass 282 cases and five real blocked-TCP confirmation scenarios pass using the actual
uncommitted native parent. The complete measured graph is 4,456B; strict native Clippy and
checking pass. Full native validation passes 768 tests (763 library plus five command-policy),
and native build and actual startup pass. A separate native/Node HTTP interoperability draft
is uncommitted and excluded from these passes; its native and combined layout checks are pending.

The next implementation must cross the browser boundary explicitly:

1. Connect the existing private native producer to bounded native-owned work delivery.
   Preserve its original payment, native position and publication history through output
   collection. Do not fabricate an authenticated browser Header/Incoming to start it.
2. Use the broker's actual non-extractable selected P256 key and original channel for
   P256/HKDF, then all six Hello/Proof/Finish seal/open operations. Byte-bearing Input,
   results and future SDP must use the prepaid authenticated encrypted channel. Ordinary
   command IPC may carry only its bounded public selection and work metadata.
3. Expose the authenticated child row needed by a Respond purpose as bounded original
   metadata, without making renderer hints into action, role or parent authority. Keep
   Unknown, Pending and attached states separate across reply loss and End.
4. Require real native receipts before proceeding, preserve accepted and rejected results,
   exact ACK/settlement/Return, and the original Plan-final proof/ACK after actual browser
   and native owners drain. A complete single-pair interop test must block A-C, route via
   unjoined B, and preserve original authority across bridge replacement.
5. Complete production work/event delivery, native SDP/ICE factories and RTC integration,
   protocol isolation, Finish-to-SDP causal transfer, and supported multi-peer history/
   resource composition before enabling V2. A successful single-pair Node interop test
   would not prove packaged WebView/ACL/CORS/RTC behavior or physical audio/network results.

Member media relay and combined acceptance remain separate unfinished work. One physical
desktop is available. Production effect dispatch remains disabled.

## Historical checkpoints

## Latest checkpoint and next work (23 October)

The integrated confirmation/Plan-final runtime `f1f2e48c` passes native724 and frontend2,538;
Root3,310 (24 existing ignored) and strict Root Clippy cover identical unchanged Root inputs.
Current native check/strict Clippy, formatting, ambient gate, Svelte/build and unchanged-dependency
audit pass. HANDOVER and the acceptance ledger record exact scopes and corrections. Native
Hello/Proof/Finish traverses the actual A-C-blocked TCP fixture through unjoined B with fixed
test-provider AES keys. Original-parent Plan-final intent, physical GC, exact final ACK and
retained browser cleanup after ordinary Return are now implemented as inactive adapters.

Next code is isolated in `target/communication-confirmation-ipc-worktree`: separate child-v2
request grammar with one direct-written prepaid browser buffer. Its61 focused browser tests
pass; native tests are written but not yet compiled. It is excluded from this checkpoint.
Next required implementation remains actual child Input/admission/results, browser key/Phase
transfer, SDP/ICE/RTC, VoiceV2Native/App composition, supported multi-peer histories and real
blocked-route negotiation/replacement. Member media relay and physical acceptance remain open;
only one desktop is available. Production activation and action dispatch remain disabled.

## Current integration status (23 October)

Published Source/chat runtime `5600fecd`, documented/pushed at `9004cb0a` in draft PR #53, passes
complete local Root3,304 (24 existing ignored), native702 and frontend2,531 tests plus strict
static/build gates. HANDOVER records exact scope and logs. This supersedes the earlier pending
Source-validation statements below; production V2 remains disabled.

The next draft is isolated in `target/communication-native-children-worktree`: retained known
publications, original-parent confirmation factories, atomic normalized role claims, retained
accepted decrypt triggers after Return, and an affine paid Root/Actor action snapshot. Focused
Actor21, Root pressure1, native handoff/layout230 and native TCP confirmation2 pass; complete
combined suites are still in progress. The TCP check uses test-provider AES and actual native
Source/helper/intake with A-C forbidden; it does not execute browser P256/HKDF. These results
do not cover child IPC, actual browser key/phase transfer,
chain-final Session delivery after ordinary Returns, SDP/ICE/RTC or member media relay. No
physical audio/network acceptance is available from the single desktop.

## Source continuation implementation

The accepted native/browser contract is recorded in `VOICE-V2-NATIVE-HANDOFF-CONTRACT.md`.
Published Derive checkpoint `583794de` (draft PR #53) passed the full Root/native/frontend gates;
its old pending-validation statements below are historical. The newer continuation is in
`target/communication-v2-progress-worktree`. HANDOVER records its source, current results and
remaining implementation boundaries; its complete validation and publication are recorded above.

The native scheduler now transfers already-published encrypted packets to Source automatically,
borrowing the actual cache and paying Source's copy before allocation. It preserves the original
Workflow, activity/deadline/attempt budget and current paid member map. Confirmation/SDP share two
actual outgoing group positions; actual Lower custody delays reuse/retirement. Permanent native
End permits idle Source collection without waiting for signed expiry, while retaining all real
late owners. Root495 focused voice and native221 handoff/layout tests pass. Read-only re-review
resolved the added roster-copy/bound-wording findings; production remains disabled.

Next implementation remains: original-parent-bound native confirmation-child factories and lost
reply recovery; canonical seal/open Input and SDK result adapters; established browser-key/phase
transfer; measured native/IPC/SDK/RTC composition and supported multi-peer histories; actual native
SDP/ICE factories/ingress; VoiceV2Native/App wiring; real blocked-TCP negotiation and combined
acceptance. Signed Refused ACK/late-Lower evidence is not successful remote decrypt or RTC. Member
media relay and physical audio/network acceptance remain open; only one desktop is available.

## Current native Derive continuation (23 October; validation checkpoint)

Branch `communication-native-live` in `target/communication-final-proof-worktree` continues
from accepted integration `18ac9585`. The implementation now supplies canonical Input and
actual admitted-permit replies, atomic native/Root admission, original input/physical custody,
prepaid browser intake, exact result/ACK/settlement and post-End final issuance/ACK with the
original channel keys retained. These adapters remain **inactive in production**. The seven
items in the historical sections below describe their earlier checkpoints, not the present
absence of the newly implemented Derive pieces.

Actual blocked TCP discovery through unjoined B, replacement by restored D under the SAME
join/key, then native factory/Input/admission/Actor result/ACK/settlement/End/final ACK passes.
The last focused run took22.17s before the latest plan-control increment; full native validation
is still required. A retained actual Root input keeps final retirement pending. This
does not execute browser WebCrypto inside a WebView or exchange SDP/ICE. Separate browser
tests exercise real WebCrypto against an independent framed endpoint fixture.

Original protected role indexes and plan allocations now survive action compaction. A fixed
opaque plan identity shares Root's original paid metadata without retaining an Action or
Workflow. Native reconciliation recovers actual Root ACK even if native's earlier cache/latch
was lost; receipt presence, settlement and retirement never substitute for ACK.

The original parent now preserves that opaque plan and a separate native-only close capability
after ordinary Return/compaction. A caller can retire the exact plan even when End happens before
any successor exists. Root's actual original publisher, including late inputs, gates channel/GC;
Phase Return remains immutable. The native2KiB guard includes the larger of the mutually exclusive
Pending-retirement and compacted-plan-retirement tickets, with all other owners still counted.
Focused Root473/native221 and complete frontend2,531 tests pass; Root full is running, native full
and Rust static/build/startup checks remain pending. Frontend check/build/ten Edge flows pass.

Remaining work includes shared native confirmation-child custody/factory cancellation,
confirmation source scheduling, explicit transfer of the established browser keys without
renewing old tickets, composed SDP/IPC/browser/RTC capacity, actual native SDP/ICE factories
and ingress, the VoiceV2Native adapter, App RTC wiring and combined acceptance. Production
activation remains unavailable. External download resume stays in its other agent's area.

Do not shrink the SDP reserve by removing a hedge or refusing large incoming authentication:
real Mesh lower custody survives the upper3s cancellation until response/failure (up to the
10s request timeout), and a legitimate Answer could exhaust its original attempts during that
wait. The current candidate instead removes redundant source ciphertext/framing copies,
uses exact-capacity canonical parent encoders and a private original-job cached-hash view for
small ACK verification. The Root candidate now declares187,966B with two actual Lowers inside
a192KiB reserve; pressured packet/cancellation/canonical-capacity regressions pass. The native
group ceiling is explicitly revised to512KiB, retaining the2MiB native
process ceiling, work/rate/record limits and real last-owner accounting. The independent browser
group ceiling remains256KiB. Revised-bound pressure and two-group authenticated source/helper
progress/cleanup pass. Complete native composition remains unresolved: actual setup-ACKed Derive
uses226,752B in its original group, but no maximum-SDP/native/browser/RTC composition is established.
The accepted handoff contract records reserved-slot overlap and single-peer/history limits that
still block full group calls. No physical audio/network acceptance is claimed.

## Validated target integration (23 October)

Combined runtime `3f584d6049ef139492f8fa4146b5181bdc080575` includes target `2f08bde0` and passed complete
Root 3,262, native 685 (including five ACL) and frontend 2,515 tests, with zero failures.
Focused Root 458/native 208, static checks, production frontend build, all ten Edge flows,
native development build and actual startup pass. HANDOVER records exact commands, durations,
current-source fingerprints and the excluded stale 450-test run. Both reviewed runtime source
sets and both documentation histories are preserved. A read-only integration review found no
blocker/high/medium. The stale driver integration note is now historical; its status-only
source comment remains a recorded LOW wording follow-up in the V2 owner's area.

Production activation and action/final dispatch remain disabled. The seven remaining native/
browser implementation steps below are unchanged. Do not interpret combined local suites as
an actual Actor/MLS/WebView action cycle, live SDP/ICE, member relay or physical acceptance.

## Validated terminal delivery; remaining handoff work (23 October)

Frozen runtime `193f220f4761844946740ee7b7d32f870b50722f` passed Root 3,254, native 685 (including five ACL) and frontend
2,464 tests with zero failures, plus native static/build/startup and unchanged-input frontend
check/build/ten-flow gates. HANDOVER records exact source, cache provenance, durations, review
and failed iteration logs. This is a bounded terminal-delivery slice, not a complete action flow.

Implemented but inactive: immutable Return/Retire, original-row/Session terminal intent, separate
100-byte final replies, stable stored issuance/exact ACK, normal close gated by ACK and independent
physical-identity GC. Production still accepts Status9 and existing-history Begin8(false) only.
Historical sections below describe their original checkpoints; they do not reopen these fixed
native terminal-state gaps or establish current production action dispatch.

Remaining implementation order:

1. Add canonical Input/admitted-permit replies binding exact row/action, protected context,
   native points/transcript and engine bytes. Scope.room is u128; u64 browser values stay exact
   decimal strings. Root exposes no public chain serial: binding.chain is an original Phase's
   scoped adapter identity, never renderer-created authority or a fresh confirmation-row ID.
2. Prepay one affine browser intake before reading Data. Claim existing chain/row/group/channel
   capacity before fetch, coalesce duplicates and transfer that same claim into the chain without
   a refund/recharge, renewed deadline or new execution position. Publish before callbacks.
3. Keep the actual native CryptoInputLease, paid Phase and physical Incoming alive through
   serialization, sealing, socket write and output collection. Payload custody must drop before
   the last header worker; measure the composed wrappers under existing limits.
4. Add a synchronous, non-escaping native/Root engine-admission commit. Current admit_engine
   samples injected Clock under Engine; it cannot be called under Coordinator/Route guards.
   Sample the original Clock outside locks, then let trusted native code hold its short final
   UI/selection/route fence while immediately consuming a no-Clock Root step. No await, reusable
   observation or renderer-supplied timestamp may bridge that commit. Reentrant Clock tests must
   find Engine/native gates available, and lock/revocation in either callback must refuse admission.
5. Implement the remaining result/receipt ACK/settlement adapters using the original native
   action and receipt, not a renderer digest as authority. Emit a permit only after actual native
   admission. Exact action settlement waits for that action's SDK/result RPCs, never chain/final
   drainage: driveAction clears chain.running only after settleAction returns.
6. Add authenticated original-Session cleanup after End. Preserve keys/counters through final
   issuance/ACK; do not weaken fresh-work owner checks. Cleanup already owns the execution lane
   and must not recursively acquire it. Add bounded Clock/availability coordination for action
   cleanup without overlapping pending RPCs or treating cancellation as actual drainage.
7. Prove actual Actor/MLS discovery and a complete native/WebView action cycle before enablement.
   The existing protected_start fixture calls private observers and is not normal signed ingress.
   Coordinate that positive fixture with the production V2 owner without taking over its lifecycle.

Production V2 signalling/call lifecycle and external download resume remain other agents' work.
No live V2/RTC, member relay, combined or physical audio/network acceptance follows from this slice.

## Current proof retention and cleanup retry candidate (22 October)

The isolated final-proof checkout follows `e560d319`. Compacted final-intent history retains
the same already-paid Phase and original Root witness. Native snapshots, channel preflight
and independent history GC preserve unready proof; compaction still drops Live outside locks
before requiring Root readiness. Ordinary continuation retains none and ClosedPending cannot
invent Root retirement. All199 focused handoff tests and current complete native676/frontend2456 gates pass,
with static/build/startup/packaged public-metadata checks. Root3254 retains identical-input
sourcee560d319 provenance. HANDOVER records exact boundaries; no action-flow acceptance follows.

Broker v12's existing setup/channel cleanup retries temporary native refusals using one
injected-Clock timer and the same availability Signal/paid driver. Large jumps, stale timer
callbacks and pending RPCs cannot create duplicate work. The focused browser/request/reply
suite passes 139/139; HANDOVER records source-specific evidence. General action retries remain
availability-driven, and older retained brokers refuse HMR adoption.

Still required: authorized factory/input/admission/result/receipt ACK/settlement/phase/final
adapters and dispatch; authenticated post-End final exchange and original keys through final
issuance/ACK/retry; normal production Actor/MLS pair success and combined real-WebView/native
action acceptance. Final dispatch stays disabled. No live V2/RTC, relay or physical acceptance
is established by these local custody/cleanup improvements.

Before enabling final dispatch, add native original-row issuance/ACK state to channel preflight.
Its current local-Drained/Root-ready condition is not proof of final delivery: setup cleanup could
otherwise close Control before the final reply/ACK. Preserve the independent actual physical-owner
collection proof. ClosedPending with actor_handoff=true cannot mint whole-Workflow proof: Pair
may have been published before Derive/attachment failed, and this local tombstone owns no Root
closure capability. Keep final proof separate from the242-byte read-only history response.

The browser action pump must settle its exact action before waiting for whole-chain/final
retirement. driveAction.finally awaits settleAction before clearing chain.running; making that
settlement wait for chain drainage would deadlock. Authenticated post-End cleanup must preserve
original keys and avoid reacquiring the execution lane recursively from current cleanup.

Implement the next derivation delivery slice in this order:

1. Define canonical Input, admitted-permit and separate final-proof replies. The242-byte history
   response is observation, not an engine-admission permit; it cannot carry another32-byte proof
   inside its256-byte allowance. Define browser binding.chain as a scoped adapter identity:
   Root ProtectedCryptoBinding has no public chain serial. Distinguish ordinary16KiB phase
   return from terminal whole-plan retirement explicitly in the request grammar.
2. Prepay browser intake before materializing the Input response, then bind its exact action,
   protected context, points/transcript and engine bytes. Existing prepareChainFromChannel
   receives input before charging its chain, so it cannot alone fund the earlier Data read.
3. Compose native factory/Input/result/metadata adapters within the existing listener/codec/
   task payments. Retain actual Incoming/physical hold through detached factory completion,
   and actual CryptoInputLease/paid Phase copies through Input response collection. ACK must
   compare the native digest and pass the actual retained receipt to Root. Metadata cleanup
   must not hold Work/copies while waiting for their own drainage. Actor refusal stays a refusal.
4. Prove the complete original-Session cycle under lost replies, End, cancellation, full group/
   process pressure and completed-but-uncollected output ownership. Keep action settlement
   independent of whole-chain finalization and add bounded action-cleanup retry coordination.
5. Establish successful actual Actor discovery before enablement: voice_endpoint/pair.rs requires
   the actual participant grant, original implicit watcher and proven peer record. The current
   protected_start fixture reaches private observer state; synthetic grants/selector IDs cannot
   establish normal ingress. Native adapter/issuance tests can proceed independently, but a
   production successful pilot needs the actual signed-discovery Actor fixture.
6. Enable only the exact derivation shapes after those contracts, composed memory guards and
   acceptance pass. A terminal derivation-only pilot ends its confirmation plan; it cannot
   claim completed Derive->Hello/Proof/Finish continuation, SDP phase transfer or RTC readiness.

Historical checkpoints below retain their own source scope and do not supersede current status.


## Current read-only history integration (22 October)

Unpublished39fb329c supplies original Workflow retirement observation/native snapshot; the
following WIP connects original-session existing-history recovery end to end. Native production
now serves Status9 plus context-backed Begin8(false), never factory admission/retry. Browser
v11 recoverChannelHistory uses its exact established channel/owner, original keys/counters and
shared paid driver, with canonical242B metadata/3B refusals. Same-query Promise coalescing,
competing-query busy, authenticated malformed-reply RX spending and retained actual work after
End are covered. Native focused192 and browser/request/reply130 tests, Svelte check, Root/native strict
Clippy/format, ambient/deny and native check pass; actual-diff review has no remaining
blocker/high/medium. Fresh complete local gates also pass: Root3254, native669 including5 ACL, frontend2447;
Svelte/build/tenflows, native builds/startup and packaged public-metadata probe pass. This
establishes no normal Actor/MLS/WebView action or live V2/media acceptance. See HANDOVER. HANDOVER is the source/evidence ledger.

Still required: authorized factory, input, admission, result, exact receipt ACK, settlement,
phase/final adapters and dispatch, final-proof retention across compaction, bounded coordination
and normal production MLS-pair Actor/browser success. Generic sealed/opened/rejected codec
variants do not implement their effect adapters. localDrained is not whole Workflow drainage.
No live V2/RTC/relay/combined/physical acceptance follows. Older checkpoint sections below are
historical source-specific status and do not supersede this current candidate.

## Original variable-retirement observation candidate (22 October)

The latest unpublished Root/native slice separates ordinary16KiB return from final96KiB plan
retirement. Original consuming-loan provenance, a LAST field publisher and a non-Clone snapshot
witness replace missing-row/weak-count inference. Root focus4 passes30 tests with2040/2048 retained
bytes and8/8 caller witness. Native Phase retains the SAME witness in paid State before local
completion; its separate snapshot has no wake/scheduling guarantee. Native focus11 passes179 tests
including unchanged native phase/codec layout guards. Root/native strict Clippy, both format
checks, ambient/deny and native check pass; full-suite gates remain pending.
HANDOVER.md records the exact boundaries. Existing compaction may drop this witness; final-proof
capture/transfer and bounded retry coordination still need implementation. Production remains
Status only, with no normal successful Actor/browser handoff, live V2 or media acceptance claim.

## Explicit native Pending closure candidate (22 October)

The latest unpublished checkpoint adds an inactive exact authenticated ClosePhase adapter for
native Pending only. Closing/ClosedPending explicitly terminate local preparation without
asserting Root publication absence, engine settlement or Phase/96KiB Workflow drainage.
Indexed native identity, actual owner destruction and physical last-owner GC remain distinct
boundaries. Native handoff177 focused tests and strict Clippy pass; full gates remain pending.
HANDOVER.md records source/provenance and unchanged capacity measurements. Browser closure
wiring, actual action dispatch and normal proven-pair Actor acceptance are still unfinished;
the production service continues to serve Status only.

## Active action transport checkpoint (22 October)

See the current opening section of HANDOVER.md for exact source and focused/full gate scope.
PR48 published7dfbb797 contains the fully validated ownership work; the isolated dispatch
checkout adds physical shutdown collection, fixed shared Rust/TypeScript request vectors,
in-place result-body transfer, inactive typed derivation-result execution and Drained-only
native history collection. The latest focused handoff run passes168 tests. It does not enable
an opcode or establish normal successful Actor/MLS or browser action delivery. Fresh complete
native/root gates and rebuild/startup remain pending for this unpublished implementation.

Completed compared with the historical design below: one original browser execution lane is
shared by the linked provider/RPC, full confirmation retirement uses the original consuming
HandoffLoan capability, actual Route closure is fenced at factory/result admission, detached
native result ownership is retained through physical shutdown, and compacted native history
has a private physical-collection proof and monotonic bounded slot reuse. Explicit channel
closure preserves Control until every original action is compacted under the shared UI commit.

Still open: truthful Pending closure (Root publication may be uncertain), actual opcode and
browser adapter wiring, exact phase/final-delivery acknowledgements, normal proven-pair Actor
success and complete workflow/input/lower/copy drainage evidence. The production service
continues to serve Status only. Historical sections below keep their original source claims;
they do not supersede the current checkpoint or imply full live V2/media/physical acceptance.

## Native weak-context and lost-reply checkpoint (22 October)

Frozen runtime source4db3f8249ef7960e11cc40febd2b81f6c3a6018e in
target/communication-browser-action-worktree follows published PR48 checkpoint8773ac6b.
The native/browser handoff remains this agent's assignment. This checkpoint does not enable
an action opcode, live V2, SDP/ICE/RTC or member media relay.

The paid application IO state now holds nine Weak references to the actual native owners.
Per-call upgrade is all-or-nothing; ordinary commands and the native factory borrow the SAME
UI session gate, real store, atomics and original commit mutex. Broad native state drops outside
guards before detached work, which retains only actual actor/selected/grant/commit/work/payment.
Factory selection accepts the original native Arc; pure early/final Coordinator checks cannot
materialize a local snapshot when rejecting a foreign equal-label owner.

Weak references retain original inline Arc allocations after value destruction. The fixed IO
payment increased from24KiB to32KiB, within the same2MiB process limit. The current Windows
layout is32,256 bytes, including11,352 bytes for weak-retained allocations and context wrappers;
tail padding is measured using the pinned Arc layout. Context drops before its payment.
Other platform layouts continue to refuse if the measured peak cannot fit.

Ledger::lookup_original now recovers the same paid native row after a lost Begin reply supplied
no RowId. It checks original grant allocation/executor/lease/full purpose, runs no Clock/charge/
grant upgrade/factory action, and remains usable after End and actual grant destruction.
None means no retained matching row, never drainage. This historical lookup is separate from
currently authorized retry: idle handed-off Pending work still resumes on its original row/serial.
No caller or transport uses historical metadata as permission to re-arm a factory.

Focused current gates: all123 native handoff tests pass (2.06s); the preceding172 voice-filter
tests pass before the three additional lookup tests. Nine actual native command-security tests
and the full2,348 frontend tests pass (62.24s). Actual-diff adversarial design/review/re-review
has no remaining findings. The earlier24KiB layout refusal and test-only Context name collision
remain in failed logs; corrected focused runs and measured-layout check pass.
The restricted browser-flow attempt lost CDP before flow assertions; the unchanged gate was
rerun with normal Windows process access. Failed attempts are retained, never counted as passes.

Complete fresh native passes600 tests including five ACL tests, zero failures
(512.28s gate). Check27.17s, strict all-target/all-feature
Clippy40.94s, format0.84s and the fresh Root construction
scanner0.77s pass. Fresh complete Root passes3233 tests, zero failures,
24 existing ignored across59 result summaries (1920.34s). Root Rust/Cargo/
dependency-policy inputs remain byte-identical to validated f1c58519; prior Root static/ambient/
dependency checks retain their own provenance. Native and Root full suites ran serially.

Fresh frontend Svelte check57.41s has zero errors/warnings; production
build45.14s and all ten rendered flows55.89s pass.
Development executable build131.47s, actual startup3.62s,
packaged custom-protocol debug build119.19s and actual packaged Windows
document/reload/CSP/CORS/public-metadata/normal-close probe2.67s pass.
Probe binary SHA256:32e07da86119b0cc764d719fb9ec0c3e172a5443d062096b9eafc969470be8ab. It requests no unlock/credentials and verifies
only public metadata, exact-port policy and normal native collection. It does not establish
selected crypto/action delivery or audible/physical networking. Only one desktop is available.
Logs: target/voice-native-context-*; exact commands/times: voice-native-context-gates.jsonl.

Still required: authenticated factory/action input/result/receipt ACK/settlement/phase closure
and final delivery over the same Session; final actual-channel closure under the original UI
commit; production proven-pair registration; full96KiB Workflow retirement and copy funding;
sharing the original browser execution position; safe collection of the separate32-row action
history. The existing16KiB Phase loan is not the whole Workflow. Live V2/RTC/isolation, relay,
combined acceptance and independent physical audio/network acceptance remain open with their
assigned owners. Separate native/experiment dependency-audit findings remain open.

## Next action transport sequence (design only,22 October)

The next change must connect the actual channel and selected key, rather than enable an opcode
around the current factory unchanged. Before any first actor effect, repeat the actual Route's
closed-state fence under the SAME original UI commit. Use original-purpose history before
new-work eligibility for lost Begin replies, while preserving an explicitly authorized same-row
idle-Pending retry. Historical lookup is not authority to perform that retry.

BrowserChain and RetainedIpcChannel live in voice-crypto.ts. A broker-only channel-chain entry
must resolve the exact retained channel/SelectedKeyRecord/key and borrow its already reserved
browser position. Do not increment the two-position pool or allow a third provider position.
Keep chain.running for logical action serialization, but acquire the shared physical execution
lane only for provider/digest work, releasing it BEFORE admit/commit/settle RPCs over the same
channel. Holding that lane around the whole action pump would deadlock on its own RPC.

End/retire must fence first effects without destroying cleanup transport. Current channel
cleanup knows only its own setup/exchange: it cannot yet know a linked chain still computes or
needs exact receipt ACK/settlement. Retain original framing keys/control position until linked
SDK/RPC/ACK/settlement, native phase close and final reply owners actually drain; collectChannel
must be last. A timeout or current-owner refusal cannot replace that observation.

Native full-chain closure needs an original capability, never a handshake-ID lookup or a copied
browser assertion. A narrowly named consuming HandoffLoan method could upgrade its existing
Workflow.control Weak and close that original PreparedChain (or close its original Workflow if
control is gone), then drop outside native State/Index/Coordinator/Group/Policy/Engine guards.
This is a derivation-only pilot proposal, not implemented. Phase.close currently returns only
16KiB to an OPEN96KiB reserve. Even chain close must not report full drainage while credited
input/engine/lower/copy owners remain. Whole-call End is a fallback, but its reply alone is not
96KiB drainage evidence: EndpointOwners.prune/quiet housekeeping still owns pair-chain closure.

Required regressions: two retained channels sharing the existing two positions; provider work
and same-channel RPC mutually excluded without self-deadlock; End during provider work followed
by exact historical ACK/settle/phase/final cleanup; lost replies at each phase; actual channel
close winning or losing final admission; original Workflow held by delayed last copy/input/
lower owner; and more than32 safely collected action generations without uncertain eviction.
Normal proven-pair Actor ingress remains a separate required integration, not a fixture claim.

## Previous action-row ownership checkpoint (21 October)

Source b0b303ebcc140f6414e89ae4b06ee8a46ece5175 follows the published edb00dda app-owned
Status checkpoint in target/communication-browser-action-worktree. The native/browser handoff
remains the active assignment. All raw RowId-taking operational helpers are now private;
Ledger::resolve requires the original native-selected grant allocation, executor, lease and
complete peer/join/key/epoch purpose. A native-only ResolvedRow retains the actual paid Row.
Repeated resolution is not a once-only work permit; physical callers still must account and
bound their handles. The complete layout remains inside the original2KiB Row payment.

Historical lookup/receipt ACK/settlement remains usable after End and actual CallGrant drop.
Data work checks the original lease under State before copy/worker admission, including when
End won during a wait. Four new regressions cover foreign equal-field owners/all selector
fields/invalid IDs, each key component, exact post-End recovery and the actual held-State race.
All112 focused handoff tests pass (2.03s after49.66s compile). Actual-diff adversarial review/
re-review has no remaining findings. Full frontend2,348 passes (68.32s). Full native583 library
plus five ACL tests pass, zero failures (523.69s library;526.92s gate). Native check48.50s,
strict all-target/all-feature Clippy50.34s, format0.76s and the fresh Root construction scanner
(one test,0.97s gate) pass. These ran serially; no validation process remains active. Failed
test-Barrier compile and interrupted focus3 are retained; the final barrier samples
credits after End's legitimate independent generation retirement, without weakening work,
receipt, copy or worker assertions. Logs: target/voice-scoped-row-*.

This adds no action opcode, new command, live V2 driver or Root/wire/storage change. The passing
packaged probe/build/startup evidence below belongs to unchanged bac90 application lifetime;
it does not prove action delivery. Native/browser action/result/ACK/settlement/phase closure and
final delivery remain open. Native context, full Workflow/copy funding, sharing the original
browser execution position and safe32-row action-history collection still require integration.
Root Rust/Cargo/policy and frontend/build inputs are unchanged; preserve their recorded exact
provenance rather than treating previous gates as fresh runs. One physical desktop is available.

## App-owned status checkpoint (21 October)

Source bac90cfa is isolated in target/communication-browser-action-worktree, following published
PR48 checkpoint 9218d179. Application host, exact pre-window CSP, fixed main-only bootstrap/cleanup
commands and a retained production metadata adapter now connect the previously internal service.
Only authenticated Status is enabled. Native action/receipt/result transport remains open.

All 108 focused native handoff tests and 35 browser adapter/Session tests pass. Full frontend
passes 2,348; Svelte has zero errors/warnings, build and all ten rendered flows pass. Native
strict all-target/all-feature Clippy/check/format, complete native579+5ACL, development build/
startup, packaged build and actual Windows document/reload/CSP/CORS/metadata/normal-close probe
pass. The probe requests no vault unlock and does not establish selected crypto exchange or
action transport. Actual-diff review/re-review has no remaining blocker/high/medium
finding after command-ledger, serializer-capacity and non-Ready projection-custody corrections.
A missing close-first initializer failed compilation once; its log is retained and corrected
focus4 passes. Root's prior 3,233/full-static validation is on unchanged Root Rust/dependencies,
not a fresh native-source run. HANDOVER records exact final gate provenance.

Closed channel collection now tracks real independent owners and preserves monotonic serials;
68 successive actual selected generations reuse capacity without reviving old Sessions. Browser
v9 cleanup awaits all SDK/IO, then exact setup settlement, buffer disposal and final collection.
Every public reply variant retains one original paid serialization position through framework
handoff. Framework/WebView public-metadata copies beyond that boundary remain excluded; private
or action bytes must never use stock IPC. Only one physical desktop is available.

Implemented by the current checkpoint above: retain only Weak references to actual native state in the
application host. Per-call context upgrades must fail closed, use the SAME borrowed UI gate as
AppState (including actual store presence), then repeat original selection/registry/generation
checks after awaits under the existing commit. Drop broad context references before detached
work; workers retain only the actual actor, selected owner, grant, commit and paid work owners.
Weak references prevent persistent ownership cycles; they are not a revocation mechanism.

Lost BeginDerive replies need original-purpose lookup before new-work/current-owner eligibility:
a requester may have no RowId yet while the original actor handoff remains uncertain. Resolve
historical native identity without rearming a factory or treating unknown as drained. Any such
API must retain the same grant/executor/lease/purpose checks as ResolvedRow and distinguish
history from admission. The current checkpoint above implements the native-only lookup. Action transport still does not use it; it must precede first-effect admission without suppressing an explicitly authorized idle-Pending retry.

The existing Phase.close returns a16KiB HandoffLoan, not the original96KiB Workflow. Full-chain
closure needs an exact native original-chain retirement capability or explicit original-call
End for a derivation-only pilot; Phase.drained must never be described as full Workflow release.

Next implement original factory/action input, result, exact receipt ACK, settlement, authenticated
browser phase closure and final delivery collection over this same immutable Session. Resolve
native context without an AppHandle cycle; admission still needs actual selected owner/actor,
original UI commit/generation, native clock/deadline and current room/epoch/pair authority.
Pending RowId is metadata, not action readiness. Normal proven-pair actor registration remains
required; fixture-created pairs cannot establish that production path.

Compose the original Workflow funding with actual native/browser serialization, SDK and retained
copies before enabling action transport. The existing 16 KiB derivation HandoffLoan is not proof
of a complete chain. Channel control already owns one of the existing two browser positions;
protected work must share/lend it explicitly, never queue cleanup behind the chain being settled.
The separate 32-row action ledger needs safe historical collection too. SDP phase/key transfer,
live RTC/mode isolation, relay and combined/physical acceptance remain open.

## Historical native bootstrap candidate (21 October, preceding complete gates)

Active checkout is target/communication-bootstrap-channel-worktree, based on published
`c5c52fd8`; Root resource primitive is separately committed `a695470d`. The current candidate
adds internal selected-owner ECDH setup, with no renderer command, application listener,
CSP change or browser Session. Pending is retained before RNG; final generation, actual
selected Arc and lease checks cover Ready installation under Coordinator. Error/unwind closes
that original purpose. Exact setup replies share one immutable descriptor and its original
credit even after registry removal. Close-first is independent of join-key settlement.

The lazy64-slot index prepays4KiB process capacity; each row/output prepays8KiB real group
capacity, separate from the existing12KiB control/2KiB Session payments. Native arithmetic
prepays an ordinary8KiB Crypto owner. The concrete future, named setup owners and pinned
primeorder16-point table have layout regression guards. All13 new native bootstrap tests pass
within61 passing handoff tests, including actual ServerActor selected-key fixtures. Complete
native passes532 library plus five ACL tests, check/strict Clippy/format/build/startup. Exact
counts/timings and source provenance are in HANDOVER.md. No complete allocation/RSS bound is claimed.
There is no uncertain-row eviction; this finite pilot requires lifecycle collection before
live activation. `Closed.native_setup_drained` currently describes native computation only, never actual
browser/provider drainage. Before browser crypto is connected, retain/transfer a corresponding
ordinary engine owner through real setup, outgoing sealing/MAC and authenticated setup ACK.

The canonical transcript includes version/suite, monotonic native session ID/slot, original
generation/server/incarnation/raw group/scope/policy/room, actual local device/join, selected
owner/executor and both65-byte points in fixed roles. The pure browser parser creates no key
or counter allocator. Seven new real-WebCrypto/metadata regressions and the eight existing
frame regressions pass, including fixed-width leading-zero ECDH and UTF8 BOM identity refusal.
Shared fixtures use deterministic public test keys only. Fresh complete Root passes3,233
tests with zero failures; format, strict Clippy, ambient and Root deny pass. Complete frontend
passes2,312, Svelte zero errors/warnings, production build and all ten rendered flows pass.
Read-only actual-diff re-review has no remaining findings. Hosted c5c52fd8 still fails Linux
exact-history recovery and a Windows file-range actor bound; current native diagnostics do
not fix the Linux failure. No live V2/media/combined acceptance follows.

### Newly reachable native ECDH source review

The direct pinned `p256=0.13.2` ECDH edge adds only that dependency to the native package's
lock entry; no resolved version changed. `default-features=false` limits this edge's request,
not features unified by other dependants. This is a new secret-arithmetic consumer, not
assurance inferred from a pre-existing lockfile entry.

Reviewed exact registry sources: p2560.13.2 ecdh.rs and arithmetic/scalar.rs;
elliptic-curve0.13.8 ecdh.rs/public_key.rs/scalar/nonzero.rs; primeorder0.13.6 projective.rs.
Native prepays before parsing the full uncompressed SEC1 point and doing scalar work. The
borrowed Sized RNG adapter delegates only to injected catcoms-rt randomness. Scalar generation
uses rejection sampling; fixtures pin that contract. Both ephemeral public-key construction
and ECDH use the pinned arithmetic, whose multiplication table is considered in the guard.
`EphemeralSecret` and `SharedSecret` drop zeroize their own stored scalar/field bytes; extracted
seed/framing keys have their own zeroizing owners. Arithmetic temporaries, browser providers,
thread stacks, SDK/GC heaps and compiler copies are not covered by that erasure statement.

The p256 README explicitly reports no independent arithmetic audit or thorough generated-
assembly constant-time assessment. This source review is not such an audit. ECDH alone is
unauthenticated: the immutable native point/transcript must come from trusted main-window
IPC and the actual selected browser private key. Local keys confer no remote member or Root
effect authority. Compromise of the retained browser join private key can recover recorded
local exchanges; this construction does not claim forward secrecy against that compromise.
Existing separate native/experiment dependency-audit findings remain open; Root deny passing
does not clear them, and this change adds no advisory exemption.

## Historical framed-handoff continuation (21 October)

Published checkpoint is `71d11284`; reviewed file-fixture correction is `7609b66b`. Active work is in
target/communication-framed-handoff-worktree. The native fixed framing codec and pure browser
metadata counterpart now pass13 actual-native and8 real-WebCrypto tests, including shared
fixed bidirectional vectors at maximum Data sizes and exact u64 sequences. Full frontend
passes2,305 tests, Svelte zero errors/warnings and production build36.06s. Codec-only static
re-review has no remaining findings. Complete Windows native passes515 library plus five ACL
tests, check/build/startup and strict native Clippy/format pass; all100 focused native voice
tests pass. Current-delta complete Root and hosted Linux native gates are not yet established;
the hosted71d11284 Docker context audit passes.

The current internal native factory worker wires actual selected-owner/actor registration,
original UI-generation ordering and separately paid completed-output custody. Its six focused
actual-native regressions pass (0.78s;4m41s compile); read-only re-review has no remaining
findings after a test-only panic-wake correction. Full native validation passes on that correction.
Its pending RowId is not readiness. Positive proven-pair Actor registration is still unproved
through normal ingress.

Published-head Linux membership recovery and the native Studio mutation gate have since failed.
The continuation strengthens original admission to wait for the actual sealed3->1 route needed
by the membership restart topology, adds bounded saved-route/dial observations, and corrects
the mutation's expected assertion marker without loosening security checks. Focused Windows
chat-matrix scenarios pass; the corrected mutation gate passes389.75s. A subsequent full run
exposed C's file directory arriving before B's queued copy was applied. Requiring both exact
catalogs within the original30-second barrier preserves all bridge/byte/restart oracles. All
seven file scenarios pass44.56s; final native passes515+5ACL (521.62s), check/strict Clippy/format
pass. Fresh complete Root is running serially; Linux native confirmation remains required.

Next connect immutable native/browser bootstrap ownership and actual framed action input,
result, exact receipt ACK, settlement, browser closure and final delivery collection. Do not
copy the experiment's resettable seed constructor into app code. The future browser Session
must spend TX before WebCrypto and spend authenticated RX before body intake; failed body or
lost reply never refunds either. Its sole prepaid position survives fetch/read cancellation,
crypto settlement and explicit returned-buffer disposal. The current pure codec owns no keys.
Whole-chain/SDP/RTC activation and relay/combined/physical work below remain open.

### Original bootstrap design: selected-key ECDH (candidate above)

Use the actual selected browser join key and a fresh native local-channel P-256 ephemeral key.
Return only the immutable native public point/session metadata through the trusted main-WebView
bridge. Native retains the original Session/counters before any publication; browser retains
one constructor promise before its first await. Retries recover that identity; they never
install another allocator or substitute another native point. A hard remount with a different
executor must replace its original controller/join explicitly. Generic same-owner compatibility
key handles do not qualify: require the exact accepted selectedKeys record and executor.

Current native selected-key checks validate the SEC1 prefix/reference, not the curve. The new
consumer must require exact65-byte uncompressed encoding and successful p256::PublicKey parsing
before scalar multiplication; the browser independently imports the native point as P-256.
Bind an injective versioned transcript to native session ID/slot, original native binding,
owner/executor, selected point and native point in fixed roles. Preserve all32 big-endian bytes
of WebCrypto's256-bit ECDH result, including leading zeroes. Derive a32-byte seed under a new
bootstrap domain/label, then use the already tested directional handoff key schedule.

Prepay setup bytes and an ordinary Crypto permit before native RNG/ECDH, outside Coordinator
and ledger locks. Cache Pending identity first and repeat current generation/selection at final
native installation. Settlement-first latches closure against a delayed admission. Actual
browser setup and outgoing seal/MAC must settle and release setup buffers before a first frame
can acknowledge that setup phase; physical response verification/decryption has its own custody.
Cancellation before any frame needs exact cleanup/settlement, with no timeout refund.

This adds direct native secret-ECDH reachability. Pinned p256's README reports no independent
audit/assembly constant-time assessment of its arithmetic; record a fresh dependency/source
review rather than treating existing lockfile presence as assurance. Owned ephemeral/shared
secret types zeroize their own storage, without a broader provider/SDK erasure claim. Local
channel keys confer no remote signalling/media authority, and this exchange does not provide
forward secrecy against later compromise of the still-retained browser join private key.

Required tests include shared Rust/WebCrypto ECDH vectors with leading zeroes; valid-prefix
off-curve points; transcript/point/session substitution; distinct native session IDs under
repeated RNG fixtures; wrong actual selected handle; accepted reply loss; concurrent startup;
End/lock during computation; and settlement before delayed first admission. Start with a
harmless authenticated Control exchange before adding Root effects. This design is not a
validated operational bootstrap, application listener or live V2 call. The candidate above
implements the internal native setup and awaits application/browser integration.

## Validated handoff prerequisites (21 October)

An actual native action can now lend one chain-wide HandoffLoan from its existing96KiB
confirmation envelope and expose metadata from the original paid fixed PlanBinding. Eight new
cases are included in434 passing focused Root voice tests, with unchanged layout/budget guards.
End/expiry/settlement cannot return an actual outstanding loan. The adapter must retain the exact
action independently; capacity is not effect authority.

The candidate reader passes ten regressions and an actual packaged Windows BYOB response check.
It requires authenticated length, exclusive response and enclosing copy prepayment; it is not
channel authentication/registration or complete flight custody. Full-suite status is in handover.
The disabled pilot now implements the original native ledger and fixed drained history, with
row/phase prepayment, original selected-key/action/pair binding and metadata-only reconcile/
ACK/settlement after phase closure. Its process-owned pending/live/drained rows share 32 slots;
there is no eviction. All 60 included harness cases pass in both the smaller harness and the
complete native gate (496 library plus five ACL tests, zero failures). Root3,226, frontend2,297,
native check/build/startup, static/build and rendered flows pass; exact provenance and failed
attempts are in the handover and acceptance ledger.
Ledger re-review has no remaining blocker/high/medium. Snapshots retain Weak grants; tombstones
retain no strong Action/Phase/Grant. Reviewed cleanup dead ends and observer wake races now
have deterministic regressions, including cancellation and late exact ACK.

Next implement actual Actor registration and framed derivation/result/ACK/settlement delivery,
independently prepaid terminal replies, authenticated browser drainage and final delivery ACK/
collection. The retained metadata alone does not prove a physical response was drained. Application
listener/CSP/dispatch stays off; live V2/media/combined/physical work remains open.

## Validated selected join-key bootstrap (21 October)

The new selected-key path reaches actual native endpoint installation, with original owner/
executor/permit/join identity and one browser-held nonextractable key. Exact uncertain retries
share original computation/public point. Definitive foreign-executor refusal is typed history,
not work that the new broker must settle. End/remount wakes bounded original cleanup, including
when a settlement rejection arrives after retirement. Current execution/review status is in
the handover and acceptance ledger. Complete native465+5ACL, frontend2,287, static/build/startup
and rendered flows pass. No live V2 call follows from this bootstrap alone.

Next complete the original action registration and real native/browser result/ACK/settlement
transport. The proposed bounded loopback pilot below still needs precharged physical-flight
copies, original Root/actor/action ownership, exact accepted/rejected history recovery and actual
drainage proof. The fixed public-metadata bootstrap commands are not that byte-bearing channel.
Then implement measured confirmation-to-SDP transfer, epoch-driven replacement, SDP/ICE/RTC and
live mode isolation; media relay and combined/physical acceptance remain separate requirements.

## Current bootstrap correction and next disabled transport pilot (21 October)

Accepted-current native activation recovery is isolated in communication-activation-recovery-
worktree on319db214. It preserves the actual original controller/call allocations, join nonce,
UI generation and server incarnation after lost replies, and rechecks current identity after
asynchronous queries/commit waits. Concurrent early misses recover before another nonce. End,
lock/replacement and revoked actor grants cannot recover historical authority. Temporary fresh
grants remain with the bridge through all refusal paths and drop after native guards. All40
focused native voice tests pass, including thirteen new regressions; static re-review and full
frontend checks pass. Root full passes 3,218 on byte-identical root code. The cumulative native
suite passes 456 library plus five ACL tests; check, build, formatting and actual startup pass.
The handover records exact provenance and logs. Earlier pending notes below are historical.
Exact join-key generation admission and native action/settlement delivery are still separate.

The next transport candidate is the application-owned bounded encrypted loopback channel tested
in the [packaged experiment](experiments/loopback-ipc/README.md). A response-only Wry hook would
leave its growing request-body allocation before Tauri/ACL, and would also need Windows/Linux/
macOS response-owner plumbing. No production listener, CSP change or dependency is selected or
enabled by this note. Other-platform packaged origin, secure-context/CORS/CSP and private-network
behavior remain unproved; do not replace exact origin checks with wildcards or security bypasses.

Scope the first disabled pilot to one real native derivation/confirmation transaction. Separate
physical request sequences from the immutable logical action so a fresh physical retry recovers
the original output/receipt without another AEAD, nonce or commit. Prepay a measured serial IPC
loan from the existing confirmation envelope, not a second independent reserve. Cap the complete
physical flight, including input snapshot, encryption, fetch, reader and cancellation, rather than
readers alone. Keep native permits through actual worker/result collection and browser custody
through actual crypto/read/cancellation settlement. Clear/drop application references before
returning the loan. Bootstrap copies and known outstanding framework queues/copies count too;
the fixture's finite successful-grant count is not complete bootstrap-copy accounting.

This custody contract covers application-reachable buffers, known framework copies/queues and
observable outstanding API work. It cannot prove physical deallocation of unreachable SDK/GC
heap, total WebView/network-service RSS or library zeroization. Those are explicit residual
risks after observable owners drain, not an indefinitely live application loan. This distinction
does not excuse a concrete still-owned Wry Vec/COM stream or an unfinished cancellation. Prove
the maximum application-owned peak and one-at-a-time lane through request/result/settlement loss,
delayed native preparation, End/remount and a queued next action. Preserve a single settlement
retry owner and non-reusable drained receipt. Actual native proof of drainage remains required;
the current Coordinator recognition boolean cannot provide it.

SDP still needs an explicit measured phase transfer after engine/RPC drainage. Existing
split_retained forbids converting Crypto/Forward/lane credits; do not stack96KiB confirmation
and224KiB SDP reservations or silently enlarge limits. Live V2/RTC, media relay, combined rollout
and separate-device audio/network acceptance remain open. Only one physical desktop is available.

## Production pair continuation (21 October, focused Root checks passed)

communication-pair-origin-worktree now implements the native pair factory and passes 426 focused
Root voice tests,10 actor tests, external API and2,268 frontend tests, zero failures. Full gates
remain pending. The older
actor full run was interrupted for the reviewed context correction. This supersedes the missing-
factory description below, without a live integration/enablement claim. The production proof
selector is bound to actual implicit watch plus immutable peer/join/key/epoch. Both tables and
serial share final native publication; lost replies reuse original pair metadata without fresh
discovery or wake loops. Creation epoch cannot migrate into later chain authority.

Next still requires registered action/IPC copy accounting, exact history/ACK/settlement custody,
automatic join/phase replacement after epoch invalidation, authorized SDP/key transfer, live
dispatch/RTC and legacy isolation. Media relay and combined/physical acceptance remain open.

## Native actor bridge (21 October, full gates pending)

The body-free trusted ServerActor bridge now reaches Root endpoint/watch/derivation/confirmation
factories with original affine owners and typed uncertain-stop errors. Terminal lifecycle paths
retire local voice before ACK/Closed backpressure; actual external crypto/input/transport owners
remain charged. This is not the production IPC adapter. No serialized projections, byte-bearing
result commands, action registry or live V2 dispatch have been enabled.

Next orchestration gap: production Root pump proves JoinProof observations but does not create
LocalJoin pairs. Positive derivation fixtures create those pairs privately. A native pair driver
must select from the actual implicit participant watch, preserve original call/session/room/
epoch and exact retry identity, and prepare Clock/RNG/allocations outside final admission.
Do not expose a caller-supplied FreshPeer as authority or present the fixtures as live integration.
Then implement the registered result/ACK/settlement and copy-custody contract below, authorized
SDP/key transfer and normal V2/RTC isolation. Relay and combined/physical acceptance remain open.

## Active Root derivation and expiry correction (21 October, focused checks passed)

communication-derived-origin-worktree now supplies native Root derivation producer/result
consumers. It preserves exact action/row-zero identity, original actor/session/room authority,
affine result preparation custody and historical metadata without key reconstruction. Final
checks use the original native clock/deadline after callbacks. The shared first-seal fence also
rejects source/pair/SDP expiry when a late output still has construction time left. Static
adversarial re-review is clear after deadline and test reservation corrections; all 409 focused
voice and 2,268 full frontend tests pass. Strict Clippy, formatting, ambient and root dependency
policy also pass. Complete Rust/native gates remain required. The earlier full fe9 run was deliberately interrupted
for this correction; do not report it green. Published c153 evidence remains separate.

The next production boundary remains registered historical result/ACK/settlement and exact retry
delivery, plus native/browser IPC copy ownership. The settlement contract below still applies.
Then implement authorized confirmation-to-SDP key/phase transfer, current SDP/ICE/RTC and live
dispatch isolation; relay and combined/physical acceptance remain open.

## Prepared Root seal/open continuation (21 October, uncompiled)

The active next checkout now adds Root seal/open result consumers, actual call/action identity,
private signature staging, final original actor/session admission, deferred token/slot/promotion
cleanup and an affine preparation hold through settlement. This implements the planned native
transaction shape below, and all 389 focused voice tests pass, including unchanged layout guards. Static re-review has no remaining
blocker/high/medium. Formatting and ambient checks pass. Complete validation still belongs to
the frozen earlier checkpoint; do not claim current result readiness before complete gates.

Next after this slice: Root derivation integration; registered historical-result/ACK/settlement
and exact retry delivery; native/browser IPC copy ownership; authorized confirmation-to-SDP phase
and key transfer; current SDP/ICE/RTC and live dispatch isolation. Reconcile native terminal rows
quietly after lost replies without another computation or allocator. Explicitly cover lost
settlement requests, not just lost commit replies: a browser terminal outcome must not strand
a native engine allowance or let later protected rows spin on its capacity. The adapter/broker
settlement owner must retain original identity and actual RPC custody until confirmed drainage.

### Settlement adapter contract before enablement

The selected broker already awaits `settleAction` inside its finalizer, retaining the serial
protected lane while that promise is pending. Its terminal handle must keep returning the
immutable outcome, not rerun AEAD/commit or drive a second settlement owner. The future stateful
native IPC adapter must coalesce one settlement per original admitted controller/permit/action,
allow at most one RPC in flight and retain actual reply custody. Lost request/reply is retryable:
keep the promise pending and retry only on bounded concrete availability/driver events or an
explicit retry, with the same identity. No polling loop or replacement grant is justified.

Only exact native proof of actual drainage may resolve that promise. `settle_engine` latching a
refusal is insufficient while ResultPreparation still owns running work; await native drained
status/completion. An unknown identity is not evidence of settlement without the original
non-reusable tombstone proving drainage. Irrecoverable identity/protocol rejection must first
retire/quarantine the original owner, retaining uncertain native custody. End, lock and remount
must not cancel that settlement owner or bind it to a current replacement call.

Required actual IPC tests: request lost before native; ACK lost after native; native preparation
still running; End/remount during retry; permanent wrong identity; and a queued next protected
row. Prove one AEAD/commit, original exact identity and no early lane advance, input retirement
or credit return. Current broker fault seams and Coordinator test recognition booleans do not
prove this production contract.

## Selected browser consumer and next Root boundary (21 October)

Protected WebCrypto seal/open now consumes the original prepaid chain rows and serial execution
position after accepted derivation. Real-crypto/fault-seam checks pass105 cases; full frontend
passes 2,268, Svelte reports no errors/warnings, production build and ten rendered flows pass.
This is not production native IPC or RTC. The unselected native-provider experiment stays isolated.

Next implement a prepared Root result transaction, not a thin wrapper around LocalJoin commits.
Directory::call_lease registers the actual child in BOTH actor policy and room; validate_call_grant
recognizes that same child. Match the original participant grant allocation and actual ActionQueue
allocation. Exact registered-action terminal lookup may recover only metadata after retirement;
a miss cannot adopt an equal-ID replacement. Current-context checks belong to first effects.

Crucially, Runtime::revoke_session flips the original session atomic before ActorGate::revoke can
take Policy and revoke children. An actual child CallLease alone can briefly remain live while
Policy is held. Prepare all Clock/signer/RNG/allocation work outside the final Group/Policy fence,
then recheck the original actor/session, room, call, epoch and action/token owners after the LAST
callback immediately before infallible packet/token/state publication. Existing result methods
enter Engine and Clock: wrapping them wholesale under Group/Policy creates a lock-order/reentrancy
hazard. Preserve actual input/engine/retirement owners across local Capacity and drop them after
guards. Pin the final boundary with blocked-Policy session-revocation and final-callback tests.

The future driver must also retry original browser rows refused before derivation readiness and
uncertain earlier result handles; otherwise a lost result reply occupies the sole serial lane
ahead of later inputs. Keep those retries quiet and bounded, without another AEAD or fresh row.

## Protected Proof/Finish (21 October, implementation awaiting validation)

The next checkout now implements the reviewed role-inbox and response-lane design below through
protected intake and `Root::respond_voice_confirmation`. Incoming role identities are independent
of arrival attempts, with an explicit typed ProtectedInbox owner. Closed native echoes alone may
progress the current nonces. Rejections retain original token/pair/chain history without borrowing
an unrelated current attempt. High Proof and low Finish outputs follow the actual incoming receipt
lane; a malformed earlier input does not shift that mapping. Exact produced factory history
recovers the original action before allocations/current-trigger checks; a superseded unproduced
trigger refuses. Old unstarted responses retire outside the current decrypt Engine fence.

All **365 focused voice tests pass**, zero failures in 15.76s (`voice-followup-all-3.log`),
including original 2 KiB metadata/1,536-byte row guards. Bounded links and exact response pointers
live in their paid input records. AEAD is mocked at the same trusted
output seam as existing state tests; interoperability and physical media are separate. All fixed
layout guards remain unchanged and pass. Complete combined gates are pending. Normal dispatch,
production crypto/IPC/RTC,
SDP transfer and live V2 remain unfinished. Native signalling crypto ownership is under design
investigation only; no provider dependency or ADR selection has changed.

## Protected high-side Hello (20 October, private native seam)

The first protected consumer is implemented in LocalJoin and the Root endpoint/transport seam.
It publishes original origin allowance, incoming token/replay, one decrypt action and credited
admission history in a single Group -> Policy -> try-History transaction. Samples are taken before
nonblocking locks. All fallible preparation precedes publication; outer unpublished owners are
destroyed after guards return. Three high-side Hello lanes stay spent after malformed plaintext.
The receipt's actual EffectAuthority allocation is included within the unchanged 1,536-byte row.
Compact history and a fixed weak-action array keep the unchanged 2 KiB metadata allowance.

Root public/encrypted issuer/MessageId/hash classification precedes token effects and shares one
256-entry limit. Exact admission retries survive consumed tokens and terminal crypto outcomes,
without another action, candidate or original quota debit. Their original room/call/pair/chain/
member/actor custody cannot move to a replacement call with equal join bytes. Actual Root reply
routes separately enforce current epoch and connection. Selected protected pairs refuse ordinary
factories even after capacity refusal; ordinary-active pairs cannot migrate. First confirmation
results check the chain epoch; accepted and rejected history remains recoverable before that gate.

Focused `voice_` verification passes 315 cases, zero failures (26.56s,
`target/voice-live-integration-all-2.log`). It covers actual Root signed ACKs, lost replies after
token consumption, cross-public/encrypted ID substitution, equal-join replacement, real MLS epoch
advance with queued driver custody, document room deletion/recreation, three malformed candidates,
full-pool consumer/engine custody after a prepaid verification ticket, and final History contention
with exactly one remaining original allowance. The earlier failed test assumptions are recorded
in the acceptance ledger. Read-only review requested explicit effect accounting; that correction
fits the original row. Frozen source `53191c87` passes the full root suite (3,096 passed, zero
failures, 24 existing ignored) and unchanged frontend suite (2,254 passed). Native gates are
being consolidated with the low-side increment; its Clippy argument-count finding has a narrow
documented exception in that increment.

This is not live V2. Proof/Finish, SDP phase transfer, production
native/browser IPC, compatible helpers, RTC dispatch and legacy-session isolation remain open.
The browser protected broker still supports derivation only. The historical prerequisite sections
below describe earlier checkpoints; this section supersedes their native Hello/epoch-fence gaps.

## Low-side Hello start (21 October, focused verification passed)

The next worktree now implements the design below through actual
`Root::start_voice_confirmation` and `LocalJoin::start_protected_hello`. None always recovers the
original request; Some(previous packet ID) always recovers that exact successor, even after all
three lanes are spent. A changed native call cannot adopt the history with equal join bytes.
Outgoing packet preparation is affine: abandonment publishes no sequence, outgoing row or SDP
identity. Its packet/action/candidate and native nonce are published together under a dedicated
local Group -> Policy -> try-History fence, without an IngressTicket or incoming-origin debit.

Three bounded pending leases retain the independent packet, action and chain gates. Fixed action
and low/high history layout guards still enforce the original 1,536-byte row and 2 KiB metadata
allowances. A protected sealed packet also retains chain cancellation in queued source work.
Each original constituent clock is sampled before pure owner checks; a source job cannot use its
own monotonic origin to revive an expired packet. Busy effect preparation skips the untouched
packet while returning any earlier prepared jobs whose attempts have already been spent.

New tests cover exact lost replies after terminal compaction, immutable predecessors/fourth
refusal, full ordinary pressure, final refusal and History contention, delayed old seals after
replacement/close, original deadlines and epoch changes, actual Root call replacement/MLS epoch
addition, native owner/table substitutions and final Clock revocation. Read-only re-review has no
remaining actionable finding. All **352 focused voice tests pass**, zero failures in 15.33s
(`target/voice-low-all-4.log`), including unchanged row, metadata and Pair layout guards.
Complete combined gates remain pending. The previous checkpoint's full root/frontend results
belong to frozen source `53191c87`, not this increment. No live V2/IPC/browser/Proof/Finish/SDP
readiness is claimed.

The design constraints remain:

The next native increment must prepare an outgoing packet transaction without publishing its
sequence, row or SDP identity. Ordinary reserve can wrap the same affine preparation/publish
path. The low-side start then publishes its nonce/attempt/action, outgoing row and inactive
encrypt action together under a fresh-sample Group -> Policy -> try-History fence. It requires
actual local actor/epoch/call/room authority; it must not manufacture an ingress ticket for an
unsigned local reservation. Original origin quota belongs to signed Source admission.

Recover-current and explicit fresh-start need separate immutable native retry identities. A
lost fresh-start reply cannot cause another nonce, reservation, candidate or engine admission.
Use rows1/4/7 for at most three Hello identities, preserving the other three low-side outgoing
claims for Finish. Do not re-reserve automatically after five seconds within an issued row.
Keep action/attempt supersession, actual outgoing crypto and chain revocation independent.
CryptoAuthority now has three finite pending positions; the unchanged layout guard passes.
Keep inactive owners outside final guards and retire old engines after them.

Required focused boundaries include each local refusal before publication, exact retry without
RNG, three starts/fourth refusal/last retry, full ordinary pressure, epoch/room/call replacement,
old running seal during supersession, construction expiry, actual retained input/engine drainage,
and unchanged ordinary packet/SDP reservation semantics. The producer remains outside live dispatch.

### Next Proof/Finish consumers: reviewed delay constraints

Proof and Finish headers cannot identify their Hello candidate until plaintext is opened. Their
three incoming identities must therefore be allocated globally per role, by authenticated
issuer/ID/hash, rather than consuming the current Hello candidate's sole row. A late Proof(A)
must not strand valid Proof(B). Outgoing responses remain bound to the exact accepted native
trigger, with immutable response receipts recovered before output allocation:

| Role | Low side | High side |
| --- | --- | --- |
| Hello | Output 1/4/7, native start identity | Input 1/4/7 |
| Proof | Input 2/5/8, independent admission order | Output 2/5/8, exact accepted Hello |
| Finish | Output 3/6/9, exact accepted Proof/candidate | Input 3/6/9, independent admission order |

Incoming ordinals are not causal candidate indexes. Retain distinct original pair/chain inbox
authority for bounded opening across attempt replacement, then validate nonce echoes and exact
continuation intent after decrypt. Never label this as arrival-attempt authority. Valid
ownership plus stale/malformed plaintext consumes that exact token and records a rejection;
revoked ownership refuses before effects. A proven local capacity refusal leaves the token and
protocol state unchanged. No accepted transition may regenerate its nonce after a lost reply.

The low increment now retains at most three canonical Hello plaintext hashes in the existing
prepaid Pair record. It refuses native A -> B -> A nonce reuse before allocating another packet;
the protected high side rejects a distinct Hello identity repeating any accepted nonce before
changing the attempt or accepted sequence floor. Ordinary compatibility behavior is unchanged.
This avoids ambiguous continuation identity without changing the wire format or nesting another
action's Engine lock inside the current result fence. The Pair growth guard uses its unchanged
6,144-byte allowance; these focused regressions pass with the full 352-test voice selection.

Required next tests include all crossed Hello/Proof/Finish orderings, old running results after
replacement, repeated canonical plaintext under distinct signed IDs, lost intake/result/factory
replies after compaction, all three role identities/fourth refusal, original deadlines and full
ordinary pressure. Production Root namespace integration and native/browser IPC remain separate.

## Confirmation packet capacity (20 October, inactive)

Preparing the native chain now reserves logical packet-table positions from the same immutable
32-row pools used by ordinary incoming and outgoing packets. The actual native role selects
three incoming/six outgoing positions for the low side, or six incoming/three outgoing for the
high side, covering three confirmation candidates. These claims belong to Workflow, not fixed
history. Close returns unused claims; an actual loan retains its position until retirement.
Ordinary rows retain their own positions through removal, independently of decrypt-event byte
custody. Partial preparation failure returns its unused claims without changing replay/sequence.

Focused tests cover actual ordinary packet APIs under pressure, cross-ledger refusal, partial
reservation rollback and LocalJoin preparation/close. A structural growth guard covers the
retained native owners within the existing 2 KiB metadata allowance on this target; it does not
measure allocator slack or framework/engine memory. This is another capacity prerequisite.
Protected packet/event allocation, three candidate history lanes and the atomic high-side Hello
consumer are now implemented as described above. No live V2 dispatch has been enabled.

## Packaged loopback platform probe (19 October, experiment only)

The standalone `experiments/loopback-ipc` harness now passes on actual packaged Windows
WebView2 `Edg/155.0.4283.62`, without browser-security bypass flags. It checks secure
`http://tauri.localhost`, an exact numeric-loopback CSP addition before window creation,
actual restricted/remote ACL denials, CORS rejection and stock Tauri bootstrap fallback after
an injected lost successful reply. Two process-lifetime fixture grants remain exhausted after
hard refresh. Native workers drain, captured output reaches EOF and the owned temporary profile
is removed before the runner records success. An actual cleanup-fault run refuses publication.

The later fixture-only increment passes real WebCrypto/native AES-GCM framing at8192 plaintext
bytes. A fixed header HMAC precedes body allocation; distinct grant seeds, directional keys,
irreversible physical counters and one64-admission process allowance prevent counter rearming.
Response HMAC/AEAD includes the exact original request header. Four header and two body positions
persist through actual task collection; browser cancellation keeps its reader position until
settlement. An independent keyless responder really replaces the closed native port and its
fresh forged reply reaches/fails HMAC verification. Nine Rust and17 Node tests, strict fixture
Clippy, actual packaged framing/takeover and terminal cleanup-fault checks pass, with no remaining
read-only code-review finding.

Neither run requested Private Network Access; its permission behavior is therefore untested.
The additional standalone deny audit fails on existing pinned Tauri maintenance and quick-xml/
plist advisories; no exemption was added. See the experiment README for exact reachability and
logs. Production action authority, reservation/phase transfer and complete native/framework/
browser copy accounting are still absent. This is candidate feasibility evidence, not selection
or activation of production IPC.
Live V2 and the remaining native/browser/RTC gates below stay disabled/open.

## Unpublished token and origin transaction prerequisite (19 October, inactive)

`PacketLedger::validate_prepared_inbound` finishes fallible ledger/time checks and lends an
exclusive unpublished token transaction. Its separately returned decrypt event can move into a
future protected action without cloning its buffers. Dropping the transaction leaves replay,
serial and SDP identity untouched; `publish` has no callback, time sampling or recoverable tail.
The existing intake wrapper preserves its original final-admission ordering.

`ActorOwner::with_endpoint_origin` lends a private affine origin transaction under the original
Group -> Policy fence. Publication repeats the original actor/session/ticket/actual-driver gates
immediately before quota debit and accepts only a synchronous unit-returning publication closure.
Refusal cannot publish and does not spend origin quota. No serialized envelope can construct it.

These are prerequisites, not the atomic confirmation consumer: History/candidate/action/receipt
publication and protected role consumers remain unimplemented. A future History fence must be
nonblocking (`try_lock`, Busy without effects), or time/deadline validation must occur afresh
before lending. Keep unstarted row owners outside the callback; dropping an unpublished row can
otherwise block on History while Group/Policy are held. The future protected final fence must
run no engine, Clock or native reentry. The current `with_endpoint_origin` seam does not yet meet
that stronger requirement: `route_operation` takes Group/Policy before sampling wall time, and
its callback samples monotonic time under those locks. Use a separate short, nonblocking final
transaction with samples taken outside the locks. Contention must return Busy without effects
and retry with fresh samples; moving samples ahead of the existing blocking locks would permit
stale-deadline admission. Do not enable the protected consumer using that shortcut.
Focused verification passes 289 voice tests and strict sync Clippy.
The combined checkpoint now passes 3,035 root, 427+5 native and 2,067 frontend cases plus required
checks, as recorded in CHAT-FILES-VOICE-ACCEPTANCE. This does not implement the consumer below.

Read-only composition review selects high-side `ConfirmHello` as the first protected consumer;
low-side start/Hello encryption and Proof/Finish must remain refused until their role consumers
exist. A protected pair must explicitly refuse ordinary confirmation receive/action factories,
including after protected preparation refuses; preserve those APIs for unprotected compatibility.
Current `HistoryState` has only a derive row, and ordinary confirmation packet preparation
still charges ordinary capacity. The consumer needs three candidate lanes independent of
`pair.starts`, each retaining its Hello-decrypt/Proof-encrypt/Finish-decrypt claims, plus a private
exact-chain preparation path for both retained packet/event copies. A malformed admitted Hello
spends its candidate even when no confirmation nonce was accepted. An issuer/MessageId/hash
admission cache must precede replay/token preparation, including after terminal rejection.

Keep the prepared row, inactive action, token transaction and credited receipt in **outer mutable
owners**. Lend only borrows into Group -> Policy -> try-History; take them inside the successful
unit origin publication closure. Capturing those owners by move is unsafe even with an infallible
apply body: quota refusal drops the unused closure under the locks, and RowIssue destruction
reenters History. All refusal destruction must run after those guards return. Admission receipts
bind original call/pair/chain/room/member/epoch/lifetime, independently of consumed token and
accepted-Hello sequence gates, so a valid delayed ACK survives its successful transition.

Before admitting protected confirmation actions, also fence their first result to the chain's
original epoch. `commit_confirmation_result` currently recovers an exact stored receipt first,
then calls `with_result` without the `check_chain_epoch` that the derivation result path already
uses. Add that check inside the first-result closure, before any nonce/token/Hello transition;
keep exact accepted and rejected receipts recoverable before that check. Regression coverage
must distinguish an epoch change before the first result (no effects) from recovery after an
already recorded terminal result (the same receipt, no fresh grant). Protected confirmation
actions are not yet constructed, so this is an enablement prerequisite rather than a claim of
an active V2 regression.

## Browser derivation prerequisite (19 October, inactive)

The surviving broker v5 now has a shared closed derive/confirmation driver. Its opaque key
handles cannot adopt a caller-supplied private key. An accepted native local point/join/key and
the complete original pair setup are frozen before a derive-only plan protects twelve combined
row claims,24 KiB fixed history,96 KiB variable capacity,8 KiB control metadata and one actual
execution position. Native admission uses its own exact engine byte formula. Only row0 has a
consumer; unused confirmation/SDP claims do not imply whole-chain readiness.

The derivation pipeline runs once. Lost admission and accepted/rejected terminal replies retry
only their original native RPCs. A native historical Derived marker without locally retained
keys yields a historical receipt and no pair. Protected pairs refuse the ordinary encrypt/open
and confirmation APIs until typed protected consumers exist. Owner/plan close clears key
references but retains actual action/IPC custody through drainage. This is not production IPC.
Historical-only replies also issue no settlement report: the new browser cannot attest that an
earlier browser's engine retired. The production adapter must independently bind settlement to
the original admitted controller/permit, beyond the historical action identity.

## Pinned Windows IPC ownership investigation (19 October)

Source inspection used Tauri2.11.3 and Wry0.55.1 from the resolved Cargo cache. Moving the whole
`InvokeMessage` into a `Bytes::from_owner` backing object could preserve its owned Raw Vec through
Root command slices without cloning it. `InvokeMessage::clone` instead clones that body and must
not be used for such a lane. The command ACL is checked before the custom invoke handler, but
does not substitute for original actor/call/action authorization. This proposed handler is not
implemented by the current browser prerequisite.

That app-owned input custody is not an IPC-wide allocation firewall. Wry's Windows request
handler reads COM IStream bytes into a growing Vec before Tauri parses the command or checks
ACL. Stock Tauri JavaScript can retry a failed custom-protocol invocation through postMessage,
where binary input becomes a numeric JSON array. A closed Raw handler would have to reject that
fallback and require a separately paid lane for every physical retry, not one per JS Promise.

Responses have a separate unresolved owner: Tauri's public reply body is only Json(String) or
Raw(Vec). The async task finishes before calling the framework responder. Wry can queue that
response onto the main thread, then copies its content into a COM memory stream and hands the
stream to WebView2. There is no public last-owner callback carrying an application credit through
those copies. Future completion, resolver return, browser ACK, timeout and serialization are
therefore not evidence of final transport retirement. Receipt-only replies reduce payload size
but do not prove this lifetime. Resolve a reviewed platform/framework ownership hook or a fully
accounted alternative before claiming complete reply custody; do not refund an uncertain copy.

Relevant pinned symbols: Tauri `ipc/mod.rs` (`InvokeMessage`, `InvokeResponseBody`,
`respond_async_serialized_inner`), `ipc/protocol.rs` (`parse_invoke_request`),
`webview/mod.rs` (`on_message`), Wry `webview2/mod.rs` (`prepare_request` and the custom protocol
response callback). Tauri's optional tracing path also records IPC bodies before a custom handler;
plaintext exposure requires that path disabled or redacted, including transitive feature changes.
These are source-verified enablement constraints, not a claim that the future Raw handler exists.
The current resolved default native feature graph has no `tauri/tracing` feature
(`target/communication-v2-tauri-features.log`); that observation does not cover future feature changes.

Design-review checkpoint, 18 October 2026; first slice implemented in the 19 October worktree,
with focused verification and adversarial review in progress. **The milestone remains open.** Live V2,
normal kind28 dispatch and general SDP action IPC remain disabled. This is the next small slice
after the PR48 review corrections, rather than a claim that the communication programme is done.

## First slice: recover the exact native outcome

CryptoAction now retains a small terminal result under its existing prepaid record and engine
fence. Accepted derivation/seal/open and closed-rejected confirmation results survive consumed
tokens and actual engine settlement. Native recomputes SHA-256 over the exact bounded output,
checks the claimed tag, and classifies exact history before pending eligibility. The result
captures the original native clock and post-transition pair/attempt owners. Derivation history
is independent of confirmation nonce replacement. Retired history grants no effect authority;
the compatibility sign/commit wrappers reject it.

Confirmation prepares closed validation, checked action serials, nonce replacement and promotion
before consuming its incoming token, then applies and records the result without a fallible
tail. Signing similarly validates promotion before signing. Original engine permits remain held
until actual settlement. This implements only the inert Rust history slice; it does not enable
native IPC, browser continuations, encrypted inbox admission, SDP result factories or RTC.

Prepay terminal history on the same action owner and fence. Resolve the native-owned input,
recompute the bounded output fingerprint, and compare the caller's claimed fingerprint. Classify
an exact historical retry before testing pending authority, consumed tokens or settled engines.
Changed output refuses without replacing history. Accepted and rejected terminal outcomes both
need exact recovery; a proven pre-effect capacity refusal stays retryable.

Prepare all closed-result validation, checked counters, history/continuation capacity and
follow-up intent before irreversible effects. After the final current-owner check, consumption
or signing, state publication and receipt installation must be infallible under that same fence.
Follow-ups enqueue and return; never await browser crypto while holding it.

History is evidence of a past transition. It grants no fresh pair, signing or RTC authority.
IPC authorization still applies independently. A separate dispatch request must validate the
actual current call, room/member/policy owners, pair generation, confirmation attempt where
applicable, exchange owner and exact OfferId. Capture original signed expiry and monotonic
deadlines once; cached lookup and later grants cannot renew them. Lock/End cannot deliver
plaintext or revive an obsolete result.

## Copy ownership is a separate enablement gate

The 19 October follow-up adds Rust `CryptoAction::recover_result(bytes,tag)` for exact bounded
output retries and a separately named `terminal_status()` snapshot for a browser that has already
destroyed its plaintext at End. Both recover past metadata after LocalJoin loss without executing
a first effect or granting dispatch authority. The snapshot observes actual engine settlement
and optional terminal receipt under the same action fence. Exact acknowledgement compares action,
input/output tags and closed outcome; dynamic present-time retirement is excluded.

The native **cfg(test)** registry model now shares32 rows across pending and terminal actions and
replacement owners. End/settlement preserves unacknowledged terminal history. An exact early ACK
latches until actual settlement; unknown duplicate ACKs create no rows. Acknowledged terminal
Arcs still held by Root cannot register again. This is a Rust lifecycle model, not production
registration or serialized reply custody. Looking up a previously registered action must precede
requiring a current participant; independently authorized production IPC remains unfinished.

The next Rust slice separates each action's variable input from its fixed history record, using
one conserved two-part retained-credit partition. `input()` lends an owned lease rather than a
detached buffer. Exact result ACK and actual settlement remove only the action's input reference,
in either order; held leases keep the original buffers and charge. Fixed rows, serials, immutable
operation/output bounds, history and admission fences remain. All four current LocalJoin callers
classify terminal history before requesting input and repeat history lookup if compaction races
the first lookup. Compatibility send refuses after compaction. This is not the phase-transfer or
serialized-copy solution below, and it does not reserve the complete confirmation chain.

The subsequent prerequisite removes a settled engine lender from permanent action history.
Exchange-backed factories prepare history separately from input, retaining phase custody only
on input and actual engine loans. An affine seal/open direction permit is reversible only while
unpublished; history/input compaction never re-arms it. Proven unstarted retirement drops unused
input and the lender. Actual final settlement without any terminal result also drops input,
since no first result can pass that permanent fence; installed terminal results still require
their exact ACK. Existing input readers preserve their original buffers and credit. This does
not by itself implement the complete protected chain or browser readiness described below.

The following inactive native prerequisite now prepays twelve local fixed rows/slots and a
provisional confirmation-only 96 KiB Workflow, with one Crypto and two Forward lanes. Only
`chain_derivation_action` uses its row so far; the other eleven remain protected, not exposed
as readiness. LocalJoin binds exact owners/identities and permanently latches preparation for
that pair. Close returns unused capacity, fences pending effects and retires proven unstarted
actions without refunding actual admitted engines or readers. Fixed history and shared 2 KiB
control metadata stay charged independently; neither holds the weak group Workflow index busy
after actual variable custody drains. Quiet Runtime collection closes expired/ended plans.
This is no native/browser continuation reservation, atomic encrypted intake or SDP phase transfer.
The whole-chain design requirements below remain open; do not stack this provisional reserve
onto the existing SDP reserve and call their combined footprint admitted.

The next small prerequisite binds each prepared incoming confirmation to its actual Hello
sequence floor or attempt and original pair deadline before ledger publication. Final admission
checks require that binding, including on the generic ledger entry point, which now refuses
confirmation packets. Revocation in the final origin callback leaves replay/token/serial unchanged.
This is not the combined decrypt-action/continuation/endpoint-receipt transaction below; no new
action or engine authority escapes preparation and normal encrypted intake remains disabled.

Before exposure, prepay a finite number of metadata replies, including request-output copies,
JSON framing, browser UTF-16/decoded metadata and acknowledgement state. Retain that capacity
through explicit post-invoke retirement, independently of the serialized DTO: Tauri can move its
serialized String into HTTP/Channel ownership after serialization returns. End, timeout, a lost
callback and serialization are not retirement. Overlapping recovery attempts need separate paid
lanes; an uncertain reply cannot make its lane reusable. History collection additionally waits
for all actual reply owners, and retired browser cleanup needs status→exact ACK without plaintext.

Encrypted endpoint intake currently refuses encrypted bodies, and action factories cover only
derivation and confirmation, not SDP. Activation needs prepaid inbox/token/action publication,
native-selected offer/answer and current OfferId/ICE ownership, plus fair retained completion
and deadline scheduling. The follow-up replaces prepared unowned transport reply vectors with
typed custody-backed replies bound to their original responder, request/route gates and exact
first helper/public admission. Verification and original packet custody follow byte clones and
the actual driver's serialized lower copy. Cached helper receipts refuse replacement actor/room
authority and first-permit extension. Normal kind28 dispatch remains disabled. A first two-party
exchange must not silently imply support for the existing UI's later track changes, glare,
additional peers or ICE restarts.

The live legacy handler still changes presence and removes peers using room/device identity.
Before activation, an immutable V2 call mode must exclude every legacy kind from those effects.
V2 RTC continuations need the exact native call, remote join/key, pair generation, OfferId and
dispatch generation before and after each await. These are software enablement requirements,
independent of the separate physical audio/network acceptance that one desktop cannot provide.

The earlier SDP design measured a9,232-byte SdpResult owning one native Vec and only4,058
spare bytes in a229,376-byte exchange reserve after the route-metadata correction. Those
figures describe that earlier design, not the current192KiB Exchange declaration recorded
above. Neither measurement accounts for native IPC serialization, browser quarantine or RTC
UTF-16 strings. Adding those copies alongside an unchanged engine loan still needs a new
complete peak and explicit copy ownership.

Prefer receipt-only commit replies. The browser already has its computed plaintext; retain it
under the surviving action record instead of copying it native-to-browser again. After actual
engine and commit-RPC settlement, atomically transfer the exact engine byte loan into a measured
larger terminal/result phase, releasing the Crypto slot while retaining all actual copy charges.
Only then issue a fresh metadata-only dispatch grant. This needs a phase-transfer API and
explicit encoding/copy accounting, not an optimistic settlement report or a timestamp refund.
Keep both actual lower loans charged: their timeout can outlive the 20-second exchange window.

Native and browser continuation admission must compose. The browser's combined 32-record
registry may already be full; reserve the supported chain's browser records and result custody
before advertising readiness or admitting dependent work. Retain existing 16-pair/32-action
limits and terminal identities initially, with explicit refusal. Do not delete protective rows
to reconstruct allocators or let repeated fresh Hello packets steal continuation slots.

## Candidate sequence and required proof

The next intake boundary must prepay a complete supported confirmation chain, rather than
publishing an inbound token and then calling `confirmation_decryption_action`. One endpoint
needs derivation plus three confirmation operations for a first attempt. Three full attempts
with one shared derivation use ten native records; adding one SDP seal and one SDP open gives
twelve. The browser currently has only the confirmation-open broker, so reserving its current
one-record low-side / two-record high-side subset is not whole-chain readiness.

Protect role-specific lanes: derive once; at most three Hello, Proof and Finish operations;
then the two SDP records. A malformed fresh Hello must spend its bounded candidate lane even
when no nonce is accepted and `pair.starts` stays unchanged. It cannot consume the reserved
Proof/Finish/SDP lanes. Exact retries reuse original identities and receipts. Capacity and
proven unstarted refusal happen before replay/origin/candidate debit; ambiguous handoff never
gets a general attempt refund. Engine failure or abandoned reservations need an explicit finite
replacement policy within those lanes, not automatic allocation until the32-row table fills.

Native and surviving browser reservations must both exist before readiness. Keep a single
group progress owner through actual retired engine/IPC/input loans, across End and replacement
calls. Fixed acknowledged history may remain charged without keeping an already drained
progress lane busy forever. Do not put a strong workflow reference in permanent history or its
immutable budget accidentally; likewise, dropping LocalJoin must not release a still-owned loan.
The new input lease/compaction seam helps this composition but does not itself implement it.

Prepare one affine encrypted-intake value containing the ledger replay proposal, exact current
Hello floor or attempt binding, inactive/unpublished decrypt action, endpoint receipt/inbox
credit and protected continuation. Check an exact issuer/MessageId/hash retry before token or
replay admission. After one final native Group -> Policy transaction repeats original packet,
request, room, membership, call, pair, candidate and deadline owners, publish replay/token,
binding, action and receipt synchronously without a fallible tail; issue the metadata wake last.
No action or engine grant may escape preparation. With no actually proven FreshPeer/pair, refuse
without admission; if reordering needs a ciphertext candidate inbox, bound it separately and
release the verification slot before waiting for proof. A raw header cannot create a pair.

Measure the complete peak before choosing reservation bytes. The earlier224KiB SDP reserve
and256KiB native group premise are historical: the current Exchange declares192KiB and the
native ceiling is512KiB per group/2MiB per process. The independent browser ceiling remains
256KiB. These larger native limits do not establish combined confirmation/SDP/browser/RTC or
multi-peer admission. Return only unused capacity during an explicit phase transition;
retain protective history, actual lower
copies and unsettled engine/IPC owners. Full native/browser/UTF-16 copy handoff still needs its
own measured phases. These are design requirements, not implemented readiness guarantees.

Registration stays internal: Root creates and registers opaque typed actions before a rotating,
non-dequeueing pull exposes them. Exact admission reuses one engine permit. Exact commit either
performs the first prepared transition or returns its terminal receipt. Settlement permanently
fences delayed admission and performs any prepaid capacity handoff. Dispatch validates a fresh
grant independently of historical success. Use canonical decimal IDs and closed bounded DTOs;
do not adapt through general Pair.open or fresh-token admission.

Regressions must cover lost accepted and rejected replies; changed output with an unchanged
claimed fingerprint; full history capacity before effects; settlement/End races; pair/room/member
replacement; expiry and wall correction; maximum SDP plus two held lowers through actual copy
handoff and RTC string creation; pair/action churn; repeated Hello intake; and actual native/
WebView serialization, lost replies, rotating pulls and lost-wake closure.

One supported A-B-C/D path additionally needs bounded encrypted inbox admission, compatible
fixed helper selections between endpoints, fair scheduling and initial ICE/RTC ownership.
Glare recovery, renegotiation, broad concurrency and physical TURN/audio acceptance remain
separate work. This checkpoint does not select or implement a media relay.
