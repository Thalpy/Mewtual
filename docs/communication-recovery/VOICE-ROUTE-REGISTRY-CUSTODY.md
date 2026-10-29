# Voice route registry custody prerequisite

This isolated prerequisite corrects an inherited route-metadata custody defect before
new Control7 Query/Support transport reuses the registry. No Query/Support producer,
wire allocation, Source enum, routing limit or rollout activation is added by this fix.
V2 remains OFF. Nine sync cases and eleven rt authority cases (including two new rt
regressions) pass. Narrow formatting and independent actual-diff re-review are clear;
strict lint and complete integration suites remain pending with the primary.

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
These declared application-owned inventories exclude allocator/RSS overhead. Windows
measurements are registry1904/4096, per-route376/512, RouteOwners120 bytes versus its
previous24-byte Vec, and enclosing VoiceRuntime2112 bytes. The actual execution,
completion and scheduler wrapper graph is2056/4096, including the752-byte opaque
execution body. These measurements preserve the full guards and original per-flight
fees. Earlier complete suites do not cover this changed input.

Passing regressions use a real prepared Source, pause its actual final route destructor
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
primary owns strict lint and complete integration suites. Focused commands used the
released shared native target with-j1 and testthreads1:

- `cargo test -p catcoms-sync --all-features --lib voice_transport::forwarding::tests -j1 -- --test-threads=1 --nocapture`:9 passed,1407 filtered,2.73s.
- `cargo test -p catcoms-rt --all-features --lib authority_tests -j1 -- --test-threads=1 --nocapture`:11 passed,27 filtered,0.00s.

The first sync compilation exited101 for a missing test-only RngCore import; no test
ran in that invocation. The one-line import correction was applied before the passing
rerun. No full suite or strict lint result is claimed for this leaf.
