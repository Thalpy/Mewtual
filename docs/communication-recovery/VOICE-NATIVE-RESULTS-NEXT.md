# Next voice milestone: native result recovery

Design-review checkpoint, 18 October 2026. **Planned, not implemented or accepted.** Live V2,
normal kind28 dispatch and general SDP action IPC remain disabled. This is the next small slice
after the PR48 review corrections, rather than a claim that the communication programme is done.

## First slice: recover the exact native outcome

The existing CryptoAction engine fence serializes first effects but retains no native terminal
result. Confirmation currently consumes an incoming token before closed-result validation and
transition. A lost accepted or rejected reply therefore needs history independent of pending
token authority. Repeating ordinary token admission cannot supply that history.

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

The current 9,232-byte SdpResult phase owns one native Vec. It does not account for native IPC
serialization, browser quarantine or RTC UTF-16 strings. The narrow simultaneous native peak
leaves only 5,082 spare bytes in its 229,376-byte reserve, so adding those copies alongside an
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
