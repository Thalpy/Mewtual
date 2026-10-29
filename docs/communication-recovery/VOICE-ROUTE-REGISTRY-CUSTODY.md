# Voice route registry custody prerequisite

This isolated prerequisite corrects an inherited route-metadata custody defect before
new Control7 Query/Support transport reuses the registry. No Query/Support producer,
wire allocation, Source enum, routing limit or rollout activation is added by this fix.
V2 remains OFF. The implementation and eleven focused regressions are unrun; narrow
formatting/static inspection and the early read-only design review are complete.

The previous `RouteOwners.active` stored raw Weak<RouteGate>. The original Lower or
verification payment lived only in RouteGate's keepalive. Dropping the last strong
route could refund that payment while the registry Weak still retained the allocation.
Testing Weak::strong_count==0 before pruning did not close the race: zero is visible
when final strong destruction starts, before body/Arc allocation destruction finishes.

The shared registry now holds a strong RouteGate and the SAME actual ledger Flight or
verification Credit outside it. No Weak<RouteGate> escapes. Under exclusive Root
mutation, registry-sole strong ownership permits detaching a row; its Arc then drops
before its outside payer, with all destructors outside Group/Policy/native guards.
The ledger keepalives contain no route backedge. The outer transport FlightAuthority
does own RouteGate and must never be substituted as the row's payer.

Root retirement can remove registry rows before outside preparations/driver replies
return. PendingVoiceForward therefore retains its SAME ledger payer as a last field,
after its authority; CompletedVoiceForward already did. Sync reply custody destroys
authority before its original verification payer. The public rt RequestCancellation
and ResponderReply wrappers likewise destroy authority/effect before keepalive/custody.
Successful and refused reply admission use the same ordered owner from function entry,
including capacity-callback refusal; no early parameter-drop path can refund first.
This changes custody ordering, not wire bytes, authority checks or driver APIs.

Aggregate capacity stays64 across Source, helper and reply routes. Source/helper Lower
payments already include512 bytes of route metadata before attempt debit; that amount
does not increase. Their same original payment remains held until the route allocation
is actually destroyed. Logical retirement, an ACK or cancellation alone never refunds
owned work. Sole registry rows are pruned before ingress/ACK Crypto reservation and
before Source/helper Forward reservation, including while a Root is unjoined and idle.
Otherwise four retained verification payments could block the next request before
anything reached a pruning pass.
Actual quiet housekeeping also prunes idle rows before Clock callbacks. Two unrelated
group Roots sharing a Runtime can otherwise retain all eight process Crypto lanes
while a third Root's ingress can prune only its own registry.

The fixed64-row Vec backing and RouteOwners inline/construction metadata have a
separate4KiB payment. The already-fallible ChannelSync::prepare_voice_actor knows the
exact group and shared Resources; its first attachment reserves/guards that payment
before VoiceRuntime construction/publication or Vec allocation. Existing attachment
reuses the original owner. Runtime::new stays infallible and unchanged. Ordinary
Capacity/stale-ticket refusal returns normally, dropping actual rows/Vec before the
plain Credit. Empty retained capacity stays paid through Root destruction. This adds
one explicit4KiB cost per attached Root even when voice routing is OFF; group512KiB,
process2MiB, Crypto4, Forward4 and route64 ceilings remain unchanged.

Production preallocation guards cover the full fixed backing, complete owner metadata,
construction overlap and original per-route512-byte metadata graph. The focused growth
test also reports the actual enclosing VoiceRuntime size and RouteOwners inline size.
These declared application-owned inventories exclude allocator/RSS overhead. Actual
measurements and runtime validation are pending; earlier complete suites do not cover
this changed input.

Draft regressions use a real prepared Source, pause its actual final route destructor
after internal custody destruction and check payment during concurrent pruning; retire
Root before dropping an unexecuted Source, an awaited public driver cancellation, a
real ResponderReply, a retained Bytes clone and a refused prepared response; drive real
kind28 requests/responses through an unjoined Root with all four verification lanes
retained; check lane rollover and quiet cross-Root process-cap recovery; exercise actual
factory capacity64 with distinct real metadata sponsors; and refuse first attachment
under pressure/stale ticket without publishing VoiceRuntime or retaining the new fee.
Two rt tests pin authority-before-payer ordering on cancellation and successful/refused
reply channels, including budget callback refusal. Compiled guards cover the actual
opaque execution/completion/scheduler wrapper graph. The64-row oracle is
structural, not a claim of64 feasible native calls or Lower flights. Fixtures establish
real P2P policy and bound only their outer harness waits through sanctioned Clock.

The next operational Query/Support slice still needs separately typed Control7 bytes,
Source/ingress/forwarding and immutable current query owners. Agreement, neutral/native
consumption, Ready, SDP/ICE/RTC and production audio/relay remain separate gates. The
primary owns focused execution, strict lint and complete integration suites.
