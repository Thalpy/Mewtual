# Next media milestone: endpoint TURN gateways over a member bridge

19 October 2026. Architecture candidate for implementation, not a live relay or completed
Package 5 acceptance. Live V2 is still disabled. Only one physical desktop is available.

## Selected experiment and what it establishes

Use a loopback TURN gateway at each endpoint, connected through the selected member bridge.
Each browser uses `iceTransportPolicy: relay`. Its gateway advertises a synthetic allocation
tuple; the other endpoint sends to that tuple through its own gateway. The gateways carry the
opaque datagrams over a bounded connected member path. The bridge never terminates WebRTC.
This avoids requiring the bridge's TURN port to be reachable from both browsers and keeps
STUN/TURN parsing at the consenting endpoints rather than at a general network-facing relay.

The manually invoked experiment in `experiments/member-media-probe.mjs` passed in headless
Edge and the existing Mewtual debug executable's actual WebView2 host (version 155). In the native
run both selected candidates were `relay`; generated audio decoded with a measured peak of
0.10077 and 27 received audio packets before the cut. Jam and drawing fixture messages crossed
the same relay path. Cutting the local TCP bridge stopped new audio and message delivery;
restoring it resumed delivery. The bridge accepted 134 bounded writes / 15,165 bytes in 6.23 seconds.
Checkpoint log/results are in `target/member-media-probe-native-19oct-checkpoint.log` and
`target/member-media-probe-native-19oct-checkpoint.json`; the retained script is the reproducible source.
The assertion also requires audio packet progress after restoration, not just a message.
The retained Edge run passes the same assertions in 6.21 seconds (134 writes / 15,115 bytes), in
`target/member-media-probe-edge-19oct-checkpoint.{log,json}`. Enqueue counters are local evidence;
selected candidates, decoded audio and channel receive/echo checks establish endpoint receipt.

A repeat against the freshly rebuilt80927a85 native host also passes: relay/relay, decoded
generated-audio peak0.10104,28 received audio packets,129 accepted bridge writes /14,618 bytes
in5.83 seconds, with jam/drawing delivery and the same cut/resume assertions. Its frozen log and
JSON are `target/member-media-probe-native-rebuilt-19oct-3.log` and
`target/member-media-probe-native-rebuilt-19oct.json`. Two preceding attempts failed before any
TURN packet because the native document did not mount. The first had no frontend dev server;
the second's exact document state was not captured. The runner now records only URL/readiness/
app-child-count/Vite-overlay diagnostics on that failure. Readiness and deadlines were unchanged.

This is a feasibility result for synthetic tuples in this WebView2 implementation. It uses a
small loopback-only Node TURN fixture, fixture credentials, two peer connections in one window,
and an ordinary local TCP bridge. It does not exercise Mewtual membership, V2 signalling, its
mesh driver, consent enforcement, helper replacement,
microphone/speaker audibility, independent hosts or NAT/firewall diversity. The fixture parser
and its fixed credentials/role handshake must never become the application server or authority.

Run from the repository root with Node 22 and installed Edge. Native mode needs the debug binary
and the existing frontend dev server on 1420, opens a temporary app window and uses an isolated
WebView profile. It never opens a vault or asks for a microphone. Generated playback is attenuated
to one-millionth amplitude; decoded waveform measurements occur before attenuation.
The native probe waits for the actual app document to mount and retains its origin/CSP. Closed
test-runner controls acknowledge cut/resume and observe the bridge counter independently of
page fetches. Earlier provisional runs lacked that protection and are superseded by the retained
script: startup could replace the blank document, or a relative cut request could reach the app's
dev server instead of the bridge. No production navigation, CSP or IPC policy was relaxed.
Review also found incomplete setup cleanup, uncapped silent bridge sockets, and counters that
included refused writes. Setup now belongs to one cleanup scope, every external wait shares an
80-second absolute deadline, accepted sockets are capped at two with a two-second role deadline,
and counters advance only after bounded enqueue. Four injected failures (missing page, failed
child startup, stalled debugger, absolute deadline) each fail and close owned listeners/sockets.
Silent-socket expiry/cap and high-water counter refusal checks pass separately. These fixture
checks do not establish production media-resource or authorization enforcement.

```powershell
node docs/communication-recovery/experiments/member-media-probe.mjs
node docs/communication-recovery/experiments/member-media-probe.mjs --native
```

## Production boundaries before implementation can be enabled

Endpoint DTLS/SRTP/SCTP must terminate at A and C. B can be a full MLS member and still must
receive no endpoint media key. The existing server-wide `media_secret` cannot establish that
separation. V2 must authenticate each endpoint's exact SDP fingerprint and negotiation owner;
substituting the bridge's fingerprint must fail before RTC work. A transport forward or queue
ACK is neither remote signalling acceptance nor an audio connection.

Native must bind each gateway/allocation and bridge grant to the full group, endpoint device
identities, call/join incarnations, pair/negotiation generation, chosen helper and original
connection leases. Consent and current membership apply at both endpoints and B. End, lock,
room/member/policy replacement, expiry and connection replacement revoke queued and actual
effects. A later helper D requires fresh scoped ownership and an explicit ICE restart policy;
it cannot revive B's old allocation. No call state, TURN credentials, SDP, audio or synthetic
allocation addresses are saved across restart.

The only allowed destination is the exact paired synthetic allocation. Enforce it for
permission creation, channel binding, Send indications, inbound datagrams and every queued
write, including after awaits. TURN's ordinary IP permission is insufficient for a group/call/
device/allocation/port boundary. The implementation must have no generic UDP send, address
forwarding, DNS resolution, public listener or private-network probing path. An allocation
credential authorizes only its existing native call; it cannot create a call or change consent.

Carry the complete opaque ICE transport: STUN consent checks, DTLS, SRTP, RTCP and SCTP.
Jam/drawing channels must remain usable under the same negotiated ownership. Do not introduce
an audio-only shortcut or send PCM through the signalling actor. A library decision still needs
licence/advisory/MSRV review and proof of per-packet policy enforcement. The TURN crate offers a
custom relay-address/connection seam, but that alone proves none of these authorization rules.
[TURN relay interface](https://docs.rs/turn/latest/turn/relay/trait.RelayAddressGenerator.html).

Use a dedicated bounded media stream/datagram capability in the mesh driver, separate from
Root request/response handling. Captured current connection ownership must be checked at the
actual lower handoff, with no automatic dial or protocol downgrade. The public
`libp2p_stream::Control::open_stream` API attempts a connection when disconnected, so it cannot
be substituted directly for that capability. Its cloneable controls also need an explicit
process cap. [Stream control contract](https://docs.rs/libp2p-stream/0.4.0-alpha/libp2p_stream/struct.Control.html).

Bound allocation count, datagram size, partial frames, queued bytes, rate and retained lower
work per original endpoint/group/process. Timeout and cancellation must retain actual buffer/
stream charges until retirement. Measure TCP head-of-line blocking, queue age and loss before
choosing production budgets. Chat, membership, V2 setup and another group must continue under
sustained media/file load. Expired audio is dropped rather than replayed after a blocked queue.

## Remaining acceptance

Replace the fixture bridge with actual permitted native A–B–C paths and block direct A–C.
Prove selected relay candidates, generated audio decode, jam/drawing, exact traffic counters,
wrong-destination/call/group refusal, fingerprint substitution refusal, expiry/End/lock/removal,
held-write revocation, reconnect and B-to-D replacement. Exercise saturation, loss, delay and
another group progressing. Re-run against the rebuilt current native executable. Only after
those pass should this candidate become an enabled relay implementation.

Separate physical acceptance still requires two devices on separate networks, audible
microphone/speaker checks, forced-relay selection, reconnect and an unavailable-relay case.
The one-desktop experiment cannot close those requirements.
