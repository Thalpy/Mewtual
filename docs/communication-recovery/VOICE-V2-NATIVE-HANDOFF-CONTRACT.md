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
This capability is implemented; child factories and browser key/phase transfer remain open.

## Group-call integration gates

The existing phase machinery is a narrow progress prototype. Resources permits one original
confirmation Workflow and one SDP Exchange per group; PacketLedger also holds one SDP exchange
for the whole LocalJoin. Its32-action queue retains published terminal records until End.
Closing unused12-row claims does not remove those records. Native32-row and browser32-action/
per-chain fixed-history limits also need a supported multi-peer lifecycle. A one-pair TCP test
cannot establish the advertised sixteen-peer upper bound.

Do not stack the current reservations and assume that512KiB solves progress: confirmation
reserves two Forward lanes and SDP reserves another two, exhausting all four group slots.
Confirmation Source currently borrows ordinary slots, so it cannot retry Finish after that
overlap. Public discovery and member helper forwards also need a scheduling opportunity.

The next design must share or safely lend original outgoing progress lanes across native
phases, while leaving authenticated public/helper work able to progress. Every actual Lower
must retain the original lane-owner/payment through transport retirement; weak-zero, timeout
and cancellation cannot permit a replacement reservation to escape those counts. Phase close
must not close capacity that another phase still owns. Preserve the one original Source attempt
budget and select real capacity before spending an attempt.

Successful confirmation-to-SDP transfer must also preserve the final Finish packet. A local
seal, result ACK/settlement or HelperQueued is not evidence that the peer opened Finish. End
may intentionally cancel; successful continuation must retain the original retry authority
until its explicit causal/native transfer condition holds. Test reordered Finish/SDP, silent
or disappearing helpers, late lower retirement and actual peer completion before enablement.

The Derive pilot owns one immutable terminal descriptor per channel. A second distinct plan
refuses before payment. This is an explicit pilot limit, not the eventual multi-peer adapter.
Production action/final dispatch and desktop activation remain closed until the complete
supported chain, actual WebView handoff and physically blocked A-B-C negotiation pass.
