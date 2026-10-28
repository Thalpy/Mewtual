# Two-computer communication acceptance

Status: prepared 26 October 2026; not executed. App V2 remains disabled. The native/browser
fixtures use controlled RTC objects and do not establish audible WebView or physical-network
behavior. This checklist is a test plan, not a release or activation approval.

## Record the build and test conditions

Use disposable accounts and a test server. Record the exact commit and binary hash on each
computer, OS/WebView versions, network type, enabled protocol mode, and UTC start/end times.
Keep the same binary on both computers for the initial run. Retain diagnostics locally;
review them for private addresses, identities and message content before sharing them.

Two computers can establish an endpoint-to-endpoint baseline across separate networks. They
cannot by themselves prove an independently hosted member bridge, bridge replacement, or
fairness for another group under bridge load. Those cases need additional isolated member
processes and enforced, recorded network rules. Do not infer a forbidden A-C path from a
diagram, a failed direct dial, or a selected candidate label alone.

## Entry conditions

- The chosen head has matching complete root, native and frontend results, static/build
  checks and adversarial review. Record failures and dependency advisories without waivers.
- For V2 cases, production App setup/cleanup, native-authorized SDP/ICE, actual RTC adoption,
  microphone tracks and immutable legacy exclusion must exist. A test-only broker script or
  the contract call driver does not satisfy this condition. Do not manually turn on the
  production switch to bypass missing implementation.
- For member-media cases, the authenticated relay, scoped authorization, expiry/revocation,
  bounded transport and endpoint key isolation must be implemented. The loopback TURN
  experiment is useful feasibility evidence, not this implementation.
- Have a rollback build and a fresh test profile available. Preserve the original profiles
  when testing upgrades; never substitute a fresh profile for a claimed migration result.

## Cases and evidence

| Case | Procedure | Pass evidence |
| --- | --- | --- |
| Existing call baseline | Join the same room on the two computers, speak in both directions, leave and rejoin. Repeat after switching the active server. | Actual audible audio both ways; no old peer or audio after leaving; exact build and protocol mode recorded. This is legacy evidence while V2 is off. |
| Direct V2 | After its entry conditions pass, join simultaneously, exchange speech and supported data-channel traffic, then leave/rejoin one endpoint. | Native-authorized SDP/ICE reaches actual RTC, audible bidirectional audio, and fresh join/offer ownership. Delayed old signals cannot close or mutate the replacement. |
| Reordering and interruption | Delay/duplicate signalling with the test transport; briefly disconnect one network; reconnect without pressing repair controls. Lock/unlock and switch servers while work is in flight. | Bounded automatic recovery, responsive chat, no cross-session effects or duplicate peers. No success is reported solely because a local request was submitted. |
| Forced TURN | Apply rules that forbid direct endpoint media and retain the permitted TURN route. Speak both ways, interrupt that route, then restore it. | Recorded rules and selected candidate pair, received packet progression and audible audio; truthful unavailable state during interruption; automatic recovery or a clearly recorded bounded failure. |
| Member bridge | Put A and C on separate networks, forbid their direct signalling/media edge and allow only B as a member helper. B does not join the room. Replace B with D while endpoints remain joined. | Endpoint receipt and audible traffic through the enforced graph, recovery via D, relay-only members receive no endpoint media keys, and revoked/expired scope cannot forward new traffic. Requires additional isolated member processes. |
| Peer isolation | Keep A-B media active while A negotiates with C. Expire or fail only C, then negotiate another permitted peer. | B's actual Current/audio progresses throughout; failed work remains bounded and paid until real drainage; no global teardown or silently stranded pending row. |
| Combined load | During a call, send chat in another group and transfer a file through its permitted providers. Interrupt and restore the bridge. | Measured progress for the other group, exact chat ledger recovery and verified file bytes. Record durable transfer/restart evidence separately; verify that the tested binary includes the external-download integration from #75. |
| Version and rollback | Pair the candidate with the supported older build, then restore the candidate. Repeat with an interrupted call. | Explicit compatible behavior or refusal; no legacy packet mutates an upgraded session and no silent protocol downgrade. Record profile and persistence compatibility. |

For each case retain the timestamped steps, outcome, sanitized diagnostics, actual selected
network path, audio observation from both users, and any reproduction. An empty error log,
queue acknowledgement or matching UI participant list is insufficient evidence of delivery.
Do not record microphone content unless both participants consent.

## Current blockers and ownership

The primary voice implementation still owns operational Ready/admission and negotiation
retirement, production App/native/RTC composition, peer-local failure recovery, member media
relay and combined acceptance. Native two-peer TCP acceptance is under active validation.
External-download resume is now included in draft #75 and its stacked Score candidate #76;
their combined gates/merge are still pending. The refreshed dependency audits pass on that
candidate. Verify the tested binary's ancestry rather than assuming another branch changes
it. Operational admission, production audio composition and member relay remain entry
blockers; a passing controlled two-peer fixture does not remove them. Physical acceptance
remains unrun because only one desktop is currently available.
