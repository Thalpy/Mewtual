# Zero-config reachability, switchboards, and server modes

Status: **design, v2, not yet built.** Written 2026-08-18 after a confirmed field failure,
then substantially rewritten the same day after three adversarial reviews (privacy /
protocol / abuse-and-adoption) refuted several claims in v1. Sections marked
**[v1 RETRACTED]** record what the first draft asserted and why it was wrong, so the mistake
is not re-made.

Extends [`ARCHITECTURE.md`](ARCHITECTURE.md), [`design-6e-rendezvous.md`](design-6e-rendezvous.md),
[`design-6e-relay.md`](design-6e-relay.md). Touches the boundary tracked in
[`THREAT-MODEL.md`](THREAT-MODEL.md), see section 7.

## 1. The problem

A remote user pasting a valid invite gets **"timed out connecting to the server"**. The
invite decoded fine, the MLS token was good, the joiner dialled, nothing answered. This is a
reachability problem, and the product has no path through it that does not require the
founder to understand networking.

### 1a. Bugs in the invite path (fix pass, in progress)

| # | Bug | Effect |
|---|-----|--------|
| 1 | `build_tcp_swarm()` uses `with_new_identity()`; every launch mints a new PeerId | Every invite issued before a restart is addressed to a machine that no longer exists. Cached peer addresses die too. |
| 2 | Everything binds `/ip4/0.0.0.0/tcp/0` | New random port each launch. Port-forwarding unconfigurable, UPnP mappings never persist. |
| 3 | `reload_one` rebuilds `bootstrap` as loopback only | After the first session, invites work only on the same machine. |
| 4 | UPnP gets a 4s window and is skipped when any other field is filled | The one free zero-config path usually loses the race. |
| 5 | IPv4 and TCP only, in transport and in listen addresses | No IPv6 (no NAT at all) and no QUIC (hole-punches better than TCP). |
| 6 | The desktop join path never calls `invite.verify_self()` before dialling | A **forged** invite makes the client dial N attacker-chosen hosts, leaking the user's IP and giving a liveness oracle. The hardening landed in `catcoms-sync` only; the product path bypasses it. |
| 7 | `PeerDescriptor.seq` is not persisted across restarts | Republishing from `seq = 1` after a restart is **permanently rejected** by every peer holding the old record. Latent today, activated by fixes 1 and 2. |

### 1b. Pre-existing defects the reviews surfaced

Not caused by this design, but load-bearing for it. Several are user-visible today and at
least three block any public deployment.

- **P1. Member PEX and `AddressCache` are dead code.** `publish_self_record`, `request_pex`
  and `known_peer_records` have no callers outside `catcoms-sync`'s own tests, and
  `AddressCache` is never constructed. So `peer_records` is **permanently empty in the
  shipping app**. Consequences: `peer_addrs_from_snapshot` always returns `[]`, so the 9g
  cross-session re-dial does nothing; `connected_member_fingerprints` is always empty, so
  **live presence and the roster online dots always read zero** despite being listed as
  shipped; and `observe_eclipse` computes `reachable = 1`, making the eclipse CAUTION fire
  **unconditionally for every group of 4 or more**, forever. Highest-value pre-existing fix
  in this list, and a prerequisite for rungs 2 and 4.
- **P2. Every relay is built with `relay::Config::default()`**: 128 KiB per circuit, 120s
  duration, **16 concurrent circuits total**, 128 reservations. An avatar is 128x over
  budget; a voice call dies at 32 seconds; the 129th group ever created is refused. Also
  `RelayBehaviour` has **no `connection_limits`**, contradicting the stated "connection
  limits on every swarm".
- **P3. The rendezvous server can be filled by one laptop.** Registration is refused once the
  table is full (16,384 total at 128 per peer, so 128 free keypairs fill it), minimum TTL is
  2 hours, and there is no eviction, fairness or per-IP accounting. Separately, `Discover`
  with **no namespace matches everything**, so any anonymous caller can dump the entire table
  (a global membership-and-address census), with roughly five orders of magnitude of
  amplification and a per-request cookie allocation that exhausts memory in a few thousand
  requests.
- **P4. `KIND_CALL_SIGNAL` has no rate limit and no per-sender fairness**, just one global
  256-entry FIFO. Any member sending 257 signals evicts everyone else's ICE candidates and
  **kills voice group-wide**. Unlike PEX and blob fetch, which both have limits.
- **P5. Connection limits cap per-peer but not in total**, and peer ids are free. Harmless
  while nodes sit on random ports behind NAT; exploitable the moment the ladder makes them
  deliberately reachable.
- **P6. There is no eviction primitive.** No `Command::Disconnect`, no allow/block list
  behaviour. A removed member's established connections and granted circuit reservations
  survive removal indefinitely.
- **P7. Invite `bootstrap` addresses get no validation** (unlike `rendezvous`, which is
  carefully validated), up to 64 of them, all dialled.
- **P8. The eclipse detector's source count is attacker-supplied.** `observe_eclipse` sets
  `trust_roots = rendezvous_nodes.len()`, i.e. the number of *configured rendezvous strings*,
  which come from the inviter-chosen `rendezvous` vector in the invite. It is not a
  corroboration measurement. A hostile inviter naming two nodes it controls satisfies
  `min_sources` and the suspect predicate can never fire. The predicate also requires **both**
  low reach and low sources, so an attacker who relays honestly keeps reach high and stays
  silent regardless. Fix: count roots that actually returned a distinct, tag-verified peer, and
  make a drop to a single discovery root its own alarm independent of reach.
- **P9. The pre-dial membership tag is never carried on the wire.** `Candidate.tag_verified`
  is hard-coded `false` for every rendezvous-discovered candidate, so `SCORE_TAG_VERIFIED`
  never fires in production and all discovered candidates score flat. The primitive itself
  (`membership_tag` / `verify_membership_tag`) is correct and constant-time; it is simply not
  plumbed through `Discovered`. Currently defensible because the policy only ranks and never
  drops, but it means the design's "member-tag-verified" ranking tier does not exist.
- **P10. No padding or size quantization.** Listed in `ARCHITECTURE.md` as a locked
  adversarial-review fix and never implemented. A forwarder sees exact per-message ciphertext
  lengths with publisher attribution, and blob fetches reveal exact file sizes, enough to
  fingerprint a shared file against a known corpus without breaking any encryption. Pull
  forward, because rung 2 deliberately puts a member on the path.
- **P11. Discovery is unjittered and dials unconditionally.** The per-server discovery timer is
  a bare 60s interval with no jitter, and each tick issues a dial plus a register and a discover
  for every namespace in the grandfather window. `Swarm::dial` is reached with
  `PeerCondition::Always`, so **an existing connection never suppresses a dial**. After an infra
  outage every member converges in one 60s window. Fix before a shared node is deployed: jitter
  the interval and gate the dial on connection state.
- **P12. `run_relay` advertises `0.0.0.0`** as an external address when no external address was
  supplied, so reservations carry an undialable address and fail silently. Make the
  misconfiguration an error rather than a doc comment.

## 2. The constraint, stated plainly

If two peers are both behind NAT they **cannot** meet without some mutually reachable third
party. No protocol removes this. What is under our control is who that party is, whether the
user ever hears about it, and whether the dependency expires.

Four mechanisms, two of which are frequently conflated:

- **Direct reachability** (UPnP/PCP, a real IPv6 address, a manual forward). No third party.
- **Hole punching** (DCUtR). Needs an address-exchange channel but **not** a traffic path.
  Fails against symmetric NAT.
- **A noticeboard** (rendezvous). Carries no traffic. Near-zero cost.
- **A relay.** Carries real bytes including files and voice. The expensive one.

A noticeboard is needed nearly always; a relay only for pairs that cannot punch through.
Conflating them overstates infrastructure cost by an order of magnitude.

**Signal is not a counterexample.** Signal is client-server: clients dial out and never accept
inbound connections, so NAT never arises. This is strictly harder.

### What is actually solved

**Authorization is permanent.** A device added to the MLS group holds a leaf in the ratchet
tree and is a member from then on. Invites expiring does not eject it.

**[v1 RETRACTED] Re-finding the group is NOT half solved.** v1 claimed member PEX and the
address cache mean "reaching any one member reveals where the others are, so a group heals
itself". That machinery is written but **unwired** (P1), so steady-state re-finding does not
work in the product either. v1 used this false claim to scope the whole document to "how the
first connection gets made". Both problems are open.

## 3. Two server modes

Reachability strategy and moderation capability are the same decision, so ask it once, at
creation, in product terms.

### Friend circle (default)

Peer to peer. Members connect directly where they can; the group's own reachable members act
as switchboards for the rest. No operator, nothing to run or pay for.

- Members can learn each other's IP addresses. Inherent to a direct connection.
- Moderation is **policy-layer only**: every member receives every operation directly, so a
  modified client can ignore a mute or a ban. Already recorded honestly in `HANDOVER.md`.

### Hosted (community)

Traffic is routed through a node the operator runs.

**[v1 RETRACTED] Both of v1's selling points were false.**

- v1 said *"members cannot see each other's IPs"*. False by four independent paths, three of
  them unconditional in today's code. **(a)** Voice is a WebRTC mesh in the webview that never
  touches libp2p, with no relay-only ICE policy, so every participant ships their LAN and
  public candidates to every other participant the moment a call starts. **(b)** libp2p
  `identify` hands a peer's full listen-address set, including private addresses, to anyone who
  completes a handshake. **(c)** `dcutr` is a hard field of `MeshBehaviour` with no config,
  feature gate or runtime flag; "direct upgrades are disabled" is unimplemented and there is no
  hook to implement it against. **(d)** `serve_pex` hands out the address book to any roster
  member with no role gate. UPnP is unconditional too.
- v1 said *"bans actually work... the only place a rule can be enforced against a modified
  client"*. False. A relay sees only source peer, destination peer, byte counts and timing:
  everything above Noise is ciphertext, which is the zero-knowledge property working as
  designed. So a ban can only be keyed on PeerId, and a PeerId is **self-minted client-side and
  self-asserted** in `PeerDescriptor`, with no binding to a DeviceId or an MLS leaf (that
  binding is a documented deferral). A banned user restarts with a fresh identity and walks back
  in. Fix 1 above makes identities stable, but that is a *client-side* choice a modified client
  simply declines to make.

**The honest version of hosted mode**, which is what may be built:

- It makes **removal fast and operator-triggered**, using MLS Remove, which is cryptographic and
  already implemented. That is real and valuable.
- It **reduces incidental IP exposure**. It does not hide your address from a determined member,
  and voice discloses it outright.
- The operator learns the membership set at peer-id level, the traffic graph, and via identify
  members' private addresses, public addresses and ports.
- The operator also gains **silent censorship and partition** power: selectively withholding or
  delaying a member's operations is undetectable from inside the group, because there is no
  path-diversity check and no delivery-acknowledgement invariant.

Making bans genuinely device-bound requires every circuit reservation to carry an
MLS-leaf-signed token verified against the group roster. That makes the node an **admission
gatekeeper** holding live roster material, which is O4's second branch and needs its own design
and review. There is no version that is both cheap and true.

**Open question (O1), restated:** with both selling points reduced, is hosted mode still worth
asking a new user about on their first screen, or does it become a Settings-level option for
groups that have outgrown friend circle? This is now a product call, not a technical one.

## 4. The ladder

**[v1 RETRACTED] v1 specified a serial escalation. That is the wrong shape.** Running the rungs
in order means the most reliable one runs last, so the user experiences the ladder as latency: a
UPnP window, then a detection pass, then a 30-second punch attempt, then finally the thing that
works. **Race the rungs concurrently**, take the first that connects, and let a direct
connection preempt a relayed one when it lands (`next_direct_upgrade()` already exists for
exactly this). The ladder below is an ordering of *preference*, not of *time*.

It is still not a menu. A choice is surfaced only when everything in flight has failed.

| Rung | What | User effort | Third party | Beats CGNAT | Beats symmetric NAT |
|---|---|---|---|---|---|
| 0a | mDNS (same LAN) | none | none | n/a | n/a |
| 0b | UPnP on a per-install port, IPv6, QUIC | none | none | via IPv6, if a pinhole opens | yes |
| 0c | AutoNAT (the sensor everything branches on) | none | **yes, a dial-back peer** | n/a | n/a |
| 1 | Two-way invite code | one paste back | STUN only | yes | no |
| 2 | Switchboard members (transport relay only) | none | none | inherits | inherits |
| 3 | Guided port forwarding | router config, once | none | yes | yes |
| 4 | Bootstrap node, including a TCP/443 listener | none (a toggle) | yes | yes | yes |
| 5 | Self-hosted node / hosted mode | operator setup | own | yes | yes |
| 6 | Public DHT | none | public network | yes | yes |

### Rung 0a: mDNS

The most common first invite is someone in the same house. v1 buried mDNS in a later step, so
that user walked the entire ladder out to a public server to reach the next room. One
behaviour, zero third parties, zero latency. It belongs at the top.

### Rung 0b: direct reachability

A **per-install** fixed port (see O6), IPv6 listening, QUIC alongside TCP.

**[v1 RETRACTED] v1 called IPv6 "the highest-value item here".** Overstated. Consumer routers
generally ship a default-deny inbound IPv6 firewall, and the `libp2p-upnp` behaviour is
IGD-based and **IPv4-only by construction**: there is no code path that opens an IPv6 pinhole.
PCP would do it and is not implemented. IPv6 also **does not compose with an IPv4-only peer**,
and the model has no notion of *pairwise* reachability, only per-node. Keep IPv6, downgrade the
claim to "free when it works, silently absent otherwise", and make reachability pairwise in the
model, which is also the right input to switchboard selection.

### Rung 0c: AutoNAT

**[v1 RETRACTED] v1 wrote the entire ladder around AutoNAT and AutoNAT does not exist**: no
feature, no code, nothing. It is the sensor rungs 1 and 2 branch on and the eligibility test for
switchboards. It is a **prerequisite**, not a follow-on.

It also **needs a peer willing to dial you back**, and a brand-new founder has no peers, so the
only candidate is the bootstrap node. **Rung 0 therefore depends on rung 4**, which inverts v1's
framing that dependency is only incurred on failure. Say so plainly.

Without it the escalation trigger degenerates to a timeout, and a timeout cannot distinguish
"the founder is unreachable" (escalate) from "the founder is asleep" (escalating is useless, and
rung 3 then asks the user to reconfigure a router to fix someone else's problem).

### Rung 1: two-way invite code

The founder sends an invite; the joiner's app emits a short **reply code** carrying its own
public address; the joiner pastes it back into the same chat; both dial repeatedly until the
punch lands. **The humans are the signalling channel**, and that costs nothing.

**Simultaneity is not required, and v1 wrongly said it was.** Offline Add-request queuing is
already implemented and reviewed: a signed request parks on the control topic until the other
party is next online. The same pattern removes the "both at their keyboards" constraint, which
was the single largest human failure in this rung.

**The reply code must be authenticated.** The founder holds no key for the joiner before the
join, so the only pre-existing shared secret is `invite_nonce`. Minimum binding: a MAC keyed on
`HKDF(invite_nonce)` (the construction already used for `join_namespace`) over a canonical
length-prefixed encoding of `(domain, group_id, invite_nonce, joiner_ephemeral_pubkey,
claimed_addresses, joiner_nonce, expires_at_ms)`, with the founder accepting one reply per
invite and keeping a seen-nonce set. This closes replay, cross-group redirect, and invite/reply
binding. It does **not** close substitution by someone who read the invite in the chat: they
hold the nonce too. Say that plainly rather than implying the MAC fixes it. An invite is a
bearer token and that person could already redeem it.

**The 5-minute validity is wrong in both directions.** For security it is decorative. For
function it is too long: the payload is a **NAT mapping**, which typically dies after 30 to 120
seconds idle, so a 5-minute-old reply code names a recycled port. Give the reply code a
**60-second life bound into the MAC** and keep the mapping alive with keepalives during the dial
window. This resolves **O5**: invite and reply code get different lifetimes because they carry
different things.

**Address validation is mandatory here**, not optional. The founder is pasting content from
someone who is by definition not yet a member, and invite bootstrap addresses get no validation
at all today (P7). Cap the reply at 2 to 4 addresses, reject private, loopback, link-local,
multicast and reserved ranges, cap outbound pending dials, and back off exponentially. Without
this, a crafted reply code broadcast to many people turns their machines into a distributed
connect flood against a target, sourced from clean residential addresses.

### Rung 2: switchboard members

A directly-reachable member serves as the group's relay and noticeboard.

**A switchboard is a transport relay only. It never admits a join.** `serve_join` hard-rejects
any request that did not name *this* device as inviter, so a joiner routed to a switchboard that
is not the inviter is refused outright, reproducing the original symptom with a different error
string. Making switchboards admit would require the joiner to accept a Welcome signed by any
current member, and the joiner **has no roster before joining**, which is the entire premise of
the 6c fix. Accepting any signer **reopens the group-substitution HIGH in full**: a hostile
admitter adds the joiner's (non-secret) KeyPackage to a group it controls, and `group_id` does
not save you because it is plaintext in the invite. If a pinned multi-admitter set is genuinely
wanted it is a separate design with its own adversarial pass, and it must not become an argument
for raising `max_committer_rank` above 0.

**[v1 RETRACTED] The switchboard set must NOT ride in the invite.** `InviteToken` binds every
field under one signature, which is good crypto and exactly the problem: the set is **frozen at
mint time with no update or revocation path**. A switchboard that goes offline, changes ISP, or
is *removed from the group* is still named in every outstanding invite, still on the joiner's
only path, still able to see the joiner's IP and silently drop the join. Removal rotates the
routing secret but touches no outstanding invite, and changing the set needs a new nonce, hence
a new invite, hence revoking the old one.

Instead: switchboards **register at the rendezvous under the member-only namespace**, which is
live, self-updating, TTL-bounded by the server, and automatically excludes a removed member the
instant the routing label rotates. The invite keeps only the *infrastructure* rendezvous set,
which is operator-stable. This also fixes liveness: a `PeerDescriptor` capability bit is a
**monotonic counter with no expiry**, so a claim never ages out and a member whose router closed
the mapping keeps advertising until it notices and republishes.

**A switchboard is a gossipsub hub, not just an IP observer.** In the topology this rung exists
to serve, it is the group's mesh peer, and gossipsub is signed, so for every message it forwards
it sees publisher, topic, sequence, timestamp and exact size. Payloads stay sealed; the activity
graph does not. That yields per-message attribution, **selective censorship by publisher and
topic**, and one escalation worth naming: dropping a Remove commit toward a victim keeps that
victim sealing under a pre-removal key the removed party still holds, turning availability
control into a forward-secrecy break. The existing missed-commit probe rides request/response,
which limits this to a race rather than a permanent break.

**The cost disclosure matters more than the privacy disclosure.** Relay bytes are invisible to
every rate limit in `catcoms-sync`, which key on authenticated DeviceId at the application
layer, while a circuit is a transport-layer pipe between two *other* peers. And the limits that
do apply are per-requester with no aggregate ceiling: twenty members at the per-requester blob
budget is a large multiple of any home uplink. Requirements: an **aggregate** egress budget with
a user-set monthly cap, auto-demotion when it is hit, never auto-offering switchboarding on a
metered or mobile connection, and consent copy that **leads with cost**.

**Removal does not revoke a switchboard's position.** There is no `Disconnect` command and no
deny-list behaviour (P6), so established connections and granted reservations survive removal,
and the grandfathered topic window keeps an ex-member on the wire for two more removals. Both
need fixing before this rung ships.

**Disclosure is consent, not a badge**, and v1's version was defeatable three ways: post-join
promotion (nothing binds the capability to join time, so a member can become the switchboard a
week after you joined), an inviter-chosen pre-join view, and asymmetric consent (the volunteer
gains a capability and consents; the member whose IP is exposed gets no choice). Treat a *new*
switchboard as a consent event, prompt affected members before their traffic is first routed
through one, and offer "never route my traffic through a member".

### Rung 3: guided port forwarding

Offered, never required, framed as helping your group. Detect the gateway, read the router's
make and model from its UPnP description even when it refuses to open a port, deep-link to its
admin page, prefill the port number. One person doing this once fixes their whole circle. The
only rung that defeats both CGNAT and symmetric NAT with no third party.

### Rung 4: bootstrap node

**Capacity, honestly.** P2 and P3 mean the node as currently configured serves 128 groups and 16
simultaneous relayed connections, and kills every circuit at 128 KiB or two minutes. It cannot
carry a file or a voice call, and one laptop with 128 free keypairs can deny registration to
every user worldwide for two hours at a time, indefinitely. **These are blocking. Nothing is
deployed publicly until they are fixed**, and fixing them means real per-peer byte accounting
and a registration-admission story, not a config tweak.

Once sized, state the bandwidth bill. A shared relay carrying voice for the users who need it
most is not "one small always-on machine".

**It must also listen on TCP/443** with TLS or WebSocket. Corporate and university networks
filter outbound to arbitrary high ports, and today **every rung fails identically** there with
the same useless timeout. This is the single highest-yield addition to the ladder and costs one
listen address plus two cargo features.

**Default on, disclosed, one click off.** A user whose first server creation silently fails does
not stay long enough to have an opinion about decentralisation, and the alternative (default
off, in a collapsed Advanced section) is strictly higher friction *and* higher failure. But v1's
justification was weak: it claimed the dependency **expires** once the group gains a
switchboard, and that expiry does not fire for the CGNAT-without-IPv6 or symmetric-NAT users,
who are exactly the population that falls through to this rung. The mitigating property mostly
applies to groups that did not need the node in the first place. The better answer is
**concurrent racing plus a per-group relayed-bandwidth cap**, so the node provides bootstrap and
signalling for everyone and bulk relay only briefly.

Expiry must be a **live, hysteretic condition, not a latch**: a latch traps a group whose
switchboard went offline an hour later, and an unhysteretic condition flaps the dependency at
the discovery cadence.

**What it sees. [v1 RETRACTED]** v1 said "never which group, because namespaces stay blinded".
That misreads the primitive. Blinding stops *outsiders* computing a namespace; the rendezvous is
the party the namespace is *presented to*, and the protocol is queried by namespace, so its
registry is literally a partition of peers into groups. The label is stable, rotating only on
member removal, which for most friend circles is never. And the **invite tree is
reconstructible**: the same peer registers the per-invite join namespace and its member
namespace back to back at the same node, so the operator learns who invited whom, and with
stable identities that survives restarts. Mitigations: register the join namespace and the
member namespace at *different* nodes, or from a throwaway identity for the pre-join role, and
rotate the group label on a schedule rather than only on removal.

Run it on a cheap VPS, never on hardware at home: a home node has a rotating address, a fraction
of the upload, and puts the operator's **home IP into every shipped copy** of the software.

### Rung 5: self-hosted node / hosted mode

The binaries exist. This is both the escape hatch for a group wanting zero external dependency
and the substrate for hosted mode (section 3).

### Rung 6: public DHT

Dominated: worse than rung 4 on privacy **and** on reliability. Kept documented, built last if
at all. The user-facing warning must not be "this is less secure", which will be misread as
"your messages become readable". They do not. It is: **who you talk to becomes publicly
traceable.**

## 5. Failure messaging

This document exists because a user saw an unactionable error, so the error strings are in
scope, not deferred. The problem is structural: `join_server` branches to either the discovery
path or the direct path, never both, and on timeout it knows only that a socket did not open. It
never asks the noticeboard whether the peer is even registered.

**The code must collect the evidence**: always query for a last-seen record even when dialling a
direct address, so the failure branch can distinguish "this server was last online three days
ago" (message your friend) from "it is online but we cannot reach it" (escalate) from "the
shared helper is full" (retry later). That distinction is the difference between a user
messaging their friend and a user uninstalling.

UPnP already reports gateway-not-found and non-routable-gateway into a channel nobody surfaces.
Symmetric-NAT detection is two STUN queries to different servers, which is cheap, and is what
rung 1 needs in order to know whether to bother.

**Two peers at incompatible rungs deadlock silently.** A founder reachable only via the
bootstrap node, and a joiner who clicked it off, produce the exact original symptom with no
signal about why. The invite must declare which rungs it depends on, so the joiner can say "this
invite needs the shared helper, which you have disabled".

**Pre-flight self-test.** Today "Copy invite" is enabled having verified nothing, and the first
bootstrap entry is unconditionally loopback, so the remote joiner's first dial target is always
their own machine. The app should confirm its own advertised address is reachable from outside
before offering the invite, and escalate silently if not, rather than minting a known-dead
invite.

## 6. Where this appears in the UI

Rungs 0 to 2 need no interface beyond a status line. The rest live in an **Advanced** section,
collapsed at creation and mirrored in **Server Settings / Connectivity**.

From the UI design pass, three things the option set could not survive:

- **The port-forwarding wizard and the public DHT are not creation-time choices.** A wizard is
  an action taken after a diagnosis exists. Both belong in Settings only.
- **Hosted mode is not a symmetric peer of friend circle at creation**, because it needs a node
  address before the server can exist.
- **"Change your mind later" is not designed.** Switching a live group between modes is a
  topology migration with O1 consequences, not a setting. Do not promise it until it is.

Pre-join switchboard disclosure also costs a click: join becomes paste, preview, join.

## 7. Threat-model deltas

Each needs a line in [`THREAT-MODEL.md`](THREAT-MODEL.md):

1. **Switchboards see other members' IPs, carry their traffic, and as gossipsub hubs get
   per-message publisher/topic/size attribution and selective-drop capability.** IP visibility is
   **not revoked by removal** for connections already established (P6).
2. **Hosted mode**: the operator learns membership at peer-id level plus, via identify, private
   addresses, public addresses and ports; and gains silent censorship and partition power.
3. **The bootstrap node** sees the membership partition under a slowly-rotating label and can
   reconstruct the invite tree, not merely "a traffic graph".
4. **STUN** reveals your public address to its operator. Already true of voice.
5. **A stable per-server identity** makes a node linkable across restarts (the point) and makes
   past observations **retroactively attributable** if the device or a backup is later obtained.
   Today's churn makes past sessions unattributable; that is destroyed permanently. The vault
   tier matters: under a non-auth-bound service key, a seized locked device yields the identity.
   **[v1 PARTIALLY RETRACTED]** v1 claimed per-server scoping "preserves the property that two
   servers cannot be correlated to the same user". It does not: every party that sees a peer id
   also sees your address, all of a user's per-server swarms share one IP and go online together,
   and identify publishes an identical private-address set for each. The N-tuple co-presence
   signature is arguably a *better* cross-network tracker than a single identity would be.
   Per-server scoping is still right, but the honest claim is narrower: it stops the peer id
   itself being a join key.
6. **A fixed port plus default `identify`** is not "a weak fingerprint". Identify runs after
   Noise, which authenticates but does not authorize, so any host that connects receives the
   stable PeerId, a distinctive protocol string, the implementation version, and the full
   listen-address set. One port scan yields a global directory of installations, and it works in
   reverse for anyone who has seen a peer id in a pasted invite. Mitigations (hide listen
   addresses, neutral agent string, per-install port) are folded into the fix pass.
7. **IPv6 sharpens every disclosure**: a global address identifies a *device*, not a household,
   and a stable interface identifier tracks it across networks. It also re-links all of a user's
   per-server identities, since they share one address.
8. **The reply code puts the joiner's public IP into a third-party chat log**, a new and
   symmetric exposure of the party who previously exposed nothing, and after fix 1 it becomes a
   durable binding of chat account to IP to long-lived peer id, rather than an ephemeral one.

## 8. What this design does not claim

- Not zero-infrastructure connectivity in the general case.
- **Not** that hosted-mode bans are enforceable against a modified client, by cryptography or by
  topology. The topology's only handle is a self-minted identifier.
- **Not** that hosted mode hides your address from a determined member.
- Not that the two-way invite code works everywhere. Symmetric NAT defeats it.
- Not that the bootstrap node is trustless. It is low-trust and swappable, which is weaker.
- Not that IPv6 is reachable by default. Nothing here opens an IPv6 pinhole.

## 9. Build order

| Step | Work | Blocking? | Review |
|---:|---|---|---|
| 0 | Fix pass 1a: identity, port, reload pipeline, UPnP window, IPv6+QUIC, `verify_self` in the join path, persisted `seq`, identify hardening | prerequisite for everything | yes, key persistence |
| 1 | **Wire PEX and `AddressCache` end to end** (P1). Also fixes presence and the permanent eclipse false positive. Attempts P8; **P9 is NOT included** (it lives in `catcoms-net`), and without `tag_verified` the P8 corroboration count is only partial: see the note below | prerequisite for rungs 2, 4, 5 | **yes**: discovery and membership surface |
| 2 | AutoNAT into `MeshBehaviour`; mDNS; pairwise reachability in the model | prerequisite for rungs 1, 2 | light |
| 3 | Concurrent rung racing, status line, failure messaging, pre-flight self-test | needs 0-2 | none |
| 4 | Create-server flow, Advanced, Settings / Connectivity | needs UI pass | none |
| 5 | Two-way invite code: MAC binding, 60s life, address validation, async join via the existing offline queue | needs 0-2 | yes |
| 6 | Node capacity fixes (P2, P3), TCP/443 listener, jittered discovery (P11), relay external-address misconfig (P12), bootstrap address validation (P7) | **blocks any public deployment** | yes |
| 7 | Switchboards: rendezvous-registered capability, relay-only never admit, aggregate egress budget, `Disconnect` plus deny list (P6), consent flow | needs 1, 2, 6 | **mandatory** |
| 8 | Bootstrap node deployed, default on, live hysteretic expiry | needs 6, 7 | yes |
| 9 | Port-forwarding wizard | needs 0 | none |
| 10 | Hosted mode | **blocked on O1 and O4** | **mandatory** |
| 11 | Public DHT | last, if ever | yes |

Independent of the ladder, worth fixing on their own: P4 (voice DoS), P5, P10 (padding).

**Note on P8 and P9.** The design specifies counting roots that returned a **tag-verified** peer.
`tag_verified` is P9 and is not implemented, so a corroboration count built on unverified
discovery records is still attacker-influenceable: a hostile inviter naming two rendezvous it
controls need only serve one fabricated record from each to pin the source count at the
threshold forever. Counting a root only once a peer it surfaced has survived
`ingest_peer_record` (roster plus self-signature) is the interim measure; the real fix is P9.
Do not describe P8 as closed until P9 lands.

## 10. Open questions

- **O1.** With both of its selling points retracted, is hosted mode worth a first-screen
  question, or a Settings option for groups that outgrow friend circle? Product call.
- **O2.** Resolved in favour of the third option: the switchboard capability is a
  `DiscoveryPolicy` **ranking input**, never a promotion, and it lives in a TTL-bounded
  rendezvous registration rather than an unexpiring record. Require live corroboration (an actual
  successful inbound connection observed by another member) before counting a member toward the
  health number.
- **O3.** Bootstrap node default on. Argued in rung 4, with the honest note that v1's
  justification was weaker than stated.
- **O4.** If a node holds a ban list it is a **second authority**, and the two provably diverge,
  because ownership is not sticky: it follows the lowest live MLS leaf, and a new owner starts
  with an empty admin roster. Alice bans Mallory at the node, ownership migrates to Bob, Bob
  re-admits Mallory in MLS, and the node still refuses her: a fully valid member who cannot reach
  the group, indistinguishable from being offline. The reverse also holds. Any deny list must be
  **derived from owner-signed replicated state**, or be named explicitly as a second authority
  with specified divergence semantics.
- **O5.** Resolved: invite 1 hour, reply code 60 seconds, because they carry different things.
- **O6.** New. A fixed port and per-server identities are **mutually exclusive**: two servers
  means two listeners, which cannot share one port. Either one transport identity per device
  (breaking the scoping in delta 5) or a port per server (breaking the wizard's "one number,
  once" pitch). Current lean: derive a per-install base port from the vault so it is stable and
  unpredictable, and accept a small per-server offset, with the wizard showing the user their own
  numbers.
