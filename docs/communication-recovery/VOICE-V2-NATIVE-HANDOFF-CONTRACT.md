# V2 call and native handoff contract

The primary/native handoff owner accepts the `VoiceV2Native` interface in
`apps/desktop/src/voice-v2-call.ts` as the integration contract. This records agreement;
it does not claim an implementation or enable V2. The native adapter, complete confirmation
continuation, SDP/ICE factories and real WebView/RTC acceptance are still required.

`VoiceV2Native` is the renderer adapter interface. It is not a set of stock Tauri commands
carrying plaintext SDP, keys or crypto results. Its implementation must use the native
authorization and prepaid encrypted handoff for byte-bearing work. The browser can perform
authorized WebCrypto; Root still owns identity, membership, reservation, signatures, replay,
confirmation and final application. A browser projection never replaces those owners.

The inactive child adapter `nativeConfirmationFromChannel` retains one original purpose and
prepaid claim. Supplying both row/action hints continues a known child; omitting both performs
first/lost Begin on that same descriptor. Authenticated Pending has no action identity. Only
same-Session history may adopt its original row/action. End stops Begin(true), drains actual
local SDK/I/O, and transfers unresolved cleanup to the original channel driver. Begin(false)
plus exact Return handles existing children; refusal alone never proves absence. Exact native
parent Plan-final issuance/ACK must cover any still-unknown child before channel collection.
Fixed history/row custody lasts through actual unlink and owner retirement in either order.
These draft adapters do not activate production dispatch or implement `VoiceV2Native`/RTC.

- `start(server, room, attempt)` selects one current native join. A lost reply repeats the
  same attempt and returns its original result. Server incarnation, UI-session generation,
  membership, epoch and local protocol policy must remain current at first publication.
- `events(lease, after)` is a bounded, non-dequeueing pull of native-authorized items. Exact
  retries recover the same cursor/items. Peer presence requires challenge/proof; a candidate
  announcement cannot produce `paired`. Plaintext cannot reach RTC before its original native
  incoming token and exact peer/OfferId transition commit successfully.
- `submit(lease, action)` retains the original action identity and bytes across uncertain
  replies. Reusing an id with different content refuses. An OfferId is native-issued and binds
  the original group/room, epoch, devices, joins and keys. `submitted` means native admission,
  signing and local queueing; it never means remote receipt or working media.
- `end(lease)` permanently revokes that exact join. Old cleanup cannot stop a replacement.
  A V2 bye is best effort: local End does not wait for a remote response. Peer expiry must
  handle a lost bye. Cleanup never sends a legacy bye or creates a replacement join.

The call driver alone owns RTC negotiation. Its `VoiceV2Rtc` implementation must bind every
continuation, media attachment and data channel to the actual peer object, call generation
and OfferId. Native admission cannot authorize a stale renderer continuation. Pausing routing
or receiving an empty reply never selects legacy for an upgraded endpoint.

Native additionally excludes all opaque legacy kind-8 call signals for the affected server
while an actual V2 participant is live, including a routing pause. It clears prequeued legacy
signals at V2 publication and fences legacy egress. This is deliberately coarser than the App's
room gate: other-room and legacy companion/juke signals are excluded too. Presence-only
observers and unjoined helpers do not select a call protocol. Existing App fences are still
necessary for events emitted before native selection. Companion traffic needs an explicit V2
channel; parsing opaque legacy JSON is not an authenticated carve-out.

## Remaining composition gate

The earlier224KiB SDP reserve already exceeded the provisional256KiB group ceiling once
native Session/bootstrap/pair owners were included, before discovery/history. Removing a
hedge or refusing incoming authentication while old Lower custody survives is not acceptable:
a legitimate Answer can exhaust its original attempts during the10s transport retirement.

The current candidate removes redundant source copies and declares187,966B inside192KiB
while retaining both Lower lanes and full incoming verification. The native group policy is
explicitly revised to512KiB, keeping the2MiB native process pool and work/rate/record limits.
One group may occupy one quarter rather than one eighth of that pool; pressure/fairness tests
must cover the revised boundary. The browser ledger remains independently256KiB. See the
threat model for rationale and exact limits; these are declared signalling costs, not total RSS.

Neither change proves composition. Retained confirmation rows, channel metadata, actual input/
result/frame copies and browser/RTC strings still need measured phase transfer and custody.
Never refund live owners, erase uncertain history or reset Session counters to make it fit.
Preserve original authority, exact retries and actual last-owner collection through the transfer.

Native confirmation continuation must use the existing SINGLE chain-wide handoff loan. Each
child can have its own native row/Phase, but its loan can bind only after the previous actual
Return completes. Reserve the exact factory purpose before Actor work and retain a published
child across local attachment refusal or lost replies. Root role indexes are native metadata.
Child rows must keep the original parent alive and compare its actual opaque plan allocation;
equal public pair fields do not establish provenance.

The parent now retains that original plan and a separate close capability after compaction.
End between children must not depend on a successor loan. Trusted whole-plan cleanup freezes
new adoption, closes the exact original control outside native locks and stores its actual
retirement witness. A missing control does not prove drainage. Late input/lower owners still
delay the original publisher, and ordinary Phase Return never becomes terminal Phase Retire.
This capability and the inactive child factories/adapters are implemented. Production work
delivery and the complete native/browser WebCrypto key/phase transfer still need integration;
framed mocks and a fixed-key native provider do not establish that combined path.

## Group-call integration gates

The existing phase machinery is a narrow progress prototype. Resources permits one original
confirmation Workflow and one SDP Exchange per group; PacketLedger also holds one SDP exchange
for the whole LocalJoin. Its32-action queue retains published terminal records until End.
Closing unused12-row claims does not remove those records. Native32-row and browser32-action/
per-chain fixed-history limits also need a supported multi-peer lifecycle. A one-pair TCP test
cannot establish the advertised sixteen-peer upper bound.

The continuation candidate shares two actual outgoing Forward positions per native group
between confirmation and SDP phases. Their independent1KiB metadata payment is outside both
phase variable envelopes. Each actual Lower retains the SAME shared owner; closing a phase
takes only that phase's handle and drops it outside its state fence. Retained closed inputs
cannot pin idle process positions. Public/helper work can also borrow an idle shared position,
with its own ordinary byte payment and unchanged authentication, slot and attempt limits.

Focused Root voice and native handoff/layout checks pass; full validation of this increment
is in progress. Protected outgoing packets now
retain their original typed Workflow; the exact Source activity identity includes its
allocation. Source scratch, records and Lower preparation borrow that original phase. The
private retained ACK view binds envelope/hash and borrowed progress together. Actor admission
compares the supplied owner against the current native index, pool/group and live phase;
public ingress cannot select a Workflow. Busy original Crypto capacity retains the SAME
completion for retry after actual SDK settlement, with no replacement budget or Source
attempt reset. Actual protected publication/native pump/Source/authenticated Refused ACK/End
integration now passes under full ordinary-pool pressure, including late actual Lower custody.
It does not prove positive remote decrypt, browser SDK or RTC execution.

A permanently ended actual join/watch now permits early Source map collection once its
active Flight count reaches zero. A live call's packet/pair/chain retirement does not erase
attempt history. Map removal is not drainage proof: retained SourceActivity, FlightAuthority
and actual Lower owners still retain their original payments. Native activation must issue
fresh serialized join/watch incarnations; equal public metadata cannot authorize adoption.
The collection and signed Source/ACK/End regressions pass the495-test focused voice gate.

Both endpoint origin/requester rows are prepaid before protected pair admission. Scheduling
also prepares the selected immediate responder's row before spending a Source attempt; an
unseen helper's row-pressure refusal preserves the original packet/deadline/attempt budget.
No traffic bucket is reset. The production endpoint pump now borrows only its32-row native
Published cache and pays Source's own copy before cloning it. Four NEW insertions are allowed
per pass; up to32 bounded failed preparations are possible. Cached metadata and stale rows
cannot starve a later live row. It borrows the paid Actor member map after outer MLS/directory
observation; no new roster scratch is constructed. Broader process contention, complete native/
browser composition and multi-peer history/phase transfer remain open. Preserve the one original Source
attempt budget and select real capacity before spending an attempt; weak-zero, timeout and
cancellation never refund an outstanding actual Lower.

Successful confirmation-to-SDP transfer must also preserve the final Finish packet. A local
seal, result ACK/settlement or HelperQueued is not evidence that the peer opened Finish. End
may intentionally cancel; successful continuation must retain the original retry authority
until its explicit causal/native transfer condition holds. Test reordered Finish/SDP, silent
or disappearing helpers, late lower retirement and actual peer completion before enablement.

The Derive pilot owns one immutable terminal descriptor per channel. A second distinct plan
refuses before payment. This is an explicit pilot limit, not the eventual multi-peer adapter.
Production action/final dispatch and desktop activation remain closed until the complete
supported chain, actual WebView handoff and physically blocked A-B-C negotiation pass.
