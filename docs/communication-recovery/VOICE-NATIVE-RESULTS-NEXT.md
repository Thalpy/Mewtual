# Next voice milestone: native result recovery

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
fresh forged reply reaches/fails HMAC verification. Nine Rust and16 Node tests, strict fixture
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
otherwise block on History while Group/Policy are held. No engine, Clock or native reentry may run
under that final fence. Focused verification passes289 voice tests and strict sync Clippy; full
combined validation remains required before handoff of this continuation.

Read-only composition review selects high-side `ConfirmHello` as the first protected consumer;
low-side start/Hello encryption and Proof/Finish must remain refused until their role consumers
exist. Current `HistoryState` has only a derive row, and ordinary confirmation packet preparation
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

The current 9,232-byte SdpResult phase owns one native Vec. It does not account for native IPC
serialization, browser quarantine or RTC UTF-16 strings. The narrow simultaneous native peak
leaves only 4,058 spare bytes in its 229,376-byte reserve, including the later route-metadata
correction, so adding those copies alongside an
unchanged engine loan is not justified.

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

Measure the complete peak before choosing reservation bytes. Adding a separate confirmation
reserve alongside the existing224KiB SDP reserve can exceed the256KiB group bound. Return only
unused capacity during an explicit phase transition; retain protective history, actual lower
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
