# Zero-config reachability, switchboards, and server modes

Status: **v2, partially built.** Written 2026-08-18 after a confirmed field failure, then
substantially rewritten the same day after three adversarial reviews (privacy / protocol /
abuse-and-adoption) refuted several claims in v1. Sections marked **[v1 RETRACTED]** record
what the first draft asserted and why it was wrong, so the mistake is not re-made.

**Where it stands (2026-08-28):** the invite path is fixed, the two deployment-blocking
CRITICALs are closed, and **every defect P1 to P14 has been worked to a conclusion**. Nine are
closed outright; P3 is deferred by decision (census rate-limited, not prevented); P5, P6 and P10
are partial with their remaining gaps named in the board; and P9 is **closed as a decision**, its
premise having turned out to be false (the tag is keyed on the same secret as the namespace, so
it never defended P8's attacker, and "P9 blocks P8" was wrong).

**Section 1c is the status board** and is the answer to "is P-whatever fixed". **Section 9**
tracks the ladder. Stable direct listeners, UPnP/PCP/NAT-PMP mapping, guarded AutoNAT v2,
two-way 60-second reply codes, and opt-in member switchboards are built with live diagnostics and
regression coverage. AutoNAT is still only **partial as a product rung**: Mewtual deploys no owned
public infrastructure. Post-join recovery now drives bounded PEX/cache retries, SWIM-style helper
observations and reciprocal direct dialing; pairwise evidence remains intentionally session-only,
not a durable presence model. A successful direct join now retains one bounded, vault-sealed,
Noise-authenticated local route to the named inviter under explicit durable admission provenance,
and attempts PEX before the first post-join snapshot, so an established same-LAN pair can reconnect
after close/reopen without publishing the retained private address. Helper/reply/switchboard joins
cannot acquire that authority later; legacy migration is two-member-only. This is not discovery: it
cannot find a never-contacted peer or a peer whose listener address changed. mDNS, concurrent rung racing, the port-forwarding wizard, hosted mode and a
public DHT remain. **Section 11** carries the loose ends that are not defects. No Mewtual-operated
service is deployed or required by default.

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
  forward, because rung 2 deliberately puts a member on the path. *(Now partial: sealed CRDT ops
  and sealed file chunks are bucketed; avatars, banners and control traffic are not. See the 1c
  row for the schemes, the measured costs and what is left.)*
- **P11. Discovery is unjittered and dials unconditionally.** The per-server discovery timer is
  a bare 60s interval with no jitter, and each tick issues a dial plus a register and a discover
  for every namespace in the grandfather window. `Swarm::dial` is reached with
  `PeerCondition::Always`, so **an existing connection never suppresses a dial**. After an infra
  outage every member converges in one 60s window. Fix before a shared node is deployed: jitter
  the interval and gate the dial on connection state.
- **P12. `run_relay` advertises `0.0.0.0`** as an external address when no external address was
  supplied, so reservations carry an undialable address and fail silently. Make the
  misconfiguration an error rather than a doc comment.
- **P13 (NEW, opened by the rung-4 client work). Invite-supplied *rendezvous* addresses are not
  range-validated, and client-side DNS now resolves.** `validate_rendezvous_addrs` checks three
  things: not a circuit, exactly one `/p2p/`, distinct peer ids. It does **not** reject private,
  loopback, link-local or CGNAT literals, and it does not reject `/dns4`, `/dns6` or `/dnsaddr`.
  The bootstrap path is now validated (`dialable_bootstrap`), so the two halves of the same invite
  are treated inconsistently.

  Adding the WebSocket and DNS transports to the client for rung 4 changed the exposure: a
  `/dns4/...` address in an invite previously failed at transport selection and now **resolves and
  dials**. So a malicious inviter can point a name at `192.168.1.x`, rotate the A record per query,
  and have every joiner sweep its own LAN, which is the attack the peer-record validator was added
  to stop, reached through the one path that has no validator.

  **Fixed by splitting the validator by trust.** The obvious fix (reject DNS here) collides head-on
  with rung 4, whose entire point is that clients dial `/dns4/<name>/tcp/443/tls/ws`, and that
  address is legitimately a rendezvous or relay address. The distinction that actually matters is
  **operator-configured** (trusted, may be a name) versus **invite-supplied** (attacker-controlled,
  must be a literal in a routable range), and `validate_rendezvous_addrs` served both callers
  without distinguishing them. There are now two functions with those two names; see the 1c row for
  the exact rules and which call site got which.
- **P14 (NEW, found by the product-layer suite). Missed-commit recovery asks the one member who
  cannot answer.** A member that misses a membership commit *does* detect it: the newcomer's first
  op arrives sealed under a future epoch, `ingest_future` enqueues a commit catch-up, and it drains
  on the next tick. But `remember_peer(from)` runs on that same inbound gossip event, so the sender
  is the most-recently-seen peer, and `pick_catchup_peer` prefers the most recent. There is no
  proven `member_peers` entry to prefer instead, because only a commit catch-up promotes and a
  document catch-up does not. So the peer asked is **by construction the member whose arrival
  caused the gap**, and a member that joined at epoch N holds an empty `commit_log` and answers
  with an empty bundle. An empty bundle returns `Ok(0)` and marks nothing failed, so every
  subsequent op repeats the identical choice indefinitely.

  It heals only when a member that *was* present for the commit posts a document op after the
  newcomer. A discovery or PEX tick from the founder is **not** enough (verified): the next
  future-epoch op re-orders the newcomer back to the front before the queue drains.

  Was latent, because the joiner control-topic fix means a commit is normally received live and
  recovery is not reached; it went live whenever a commit was genuinely missed, which is exactly
  the case the recovery path exists for.

  **FIXED.** Two composing changes in `catcoms-sync`, both in catch-up **selection** only:

  1. `CatchupTask::Commits` now carries `gap_at`, the epoch something proved a peer had reached.
     With a gap proven, a catch-up that returns **no progress** (an empty bundle included) marks
     that peer failed and re-queues the chase, so the next attempt goes elsewhere. `gap_at` is
     what makes this safe: a `PeerConnected` probe carries `None`, and every honest up-to-date
     member answers a probe with an empty bundle, so marking those would have emptied the pool
     that document and blob fetches draw from too. A peer that advanced us but did not finish
     (a bundle truncated to the response budget) is *not* marked failed.
  2. `ingest_future` now records the peer whose op revealed the gap, and that peer is skipped
     when picking a source for that gap. It is a first-choice exclusion: every re-queue drops it,
     so a member whose only reachable peer is the newcomer asks it once rather than never
     chasing, and two different peers revealing the same gap cancels the exclusion (one of them
     may be an older member that can actually serve it).

  The third candidate, preferring a peer known to have been present at the target epoch, was
  **not** built. It needs per-peer epoch-presence state that nothing currently records, and 1 and
  2 already send the first request to a peer that can answer; `member_peers` (populated only by a
  roster-verified signed catch-up) is the existing, security-meaningful version of "known good
  source" and is already preferred. Revisit only if a real deployment shows repeated misses.

  Regression-tested at both layers: `catcoms-sync`'s test module covers the empty-bundle failure
  marking, the exclusion, the degenerate case where the excluded peer is the only one reachable
  (retries, does not wedge or spin), and the property itself; `catcoms-app/tests/product_e2e.rs`
  covers the property where a user would see it. All four fail against the unfixed code.

### 1c. Status board

Legend: **[x]** done · **[~]** partial, with what is left · **[ ]** open. Keep this current: it is
the answer to "is P-whatever fixed", and a stale board is worse than none (see the corrections
block in `HANDOVER.md` for what happens otherwise).

| | Defect | Commit | Left to do |
|---|---|---|---|
| **[x]** | 1a.1 identity regenerated every launch | `0af1583` | |
| **[x]** | 1a.2 random listen port | `0af1583` | |
| **[x]** | 1a.3 reload rebuilt loopback-only invites | `0af1583` | |
| **[x]** | 1a.4 UPnP crippled (4s, skipped) | `0af1583` | |
| **[x]** | 1a.5 IPv4 and TCP only | `0af1583` | |
| **[x]** | 1a.6 no `verify_self` before dialling | `0af1583` | |
| **[x]** | 1a.7 `seq` not persisted across restart | `0af1583` | |
| **[x]** | P1 PEX and `AddressCache` dead code | `32dab2a` | |
| **[x]** | P2 relay sized for a lab | `e35b1b2` | |
| **[~]** | P3 rendezvous fillable / census / cookies | `e35b1b2` | **DEFERRED by decision (2026-08-19), documented not fixed.** Occupancy, TTL, cookies and per-prefix quotas done. The census is **rate-limited, not prevented**: rejecting a namespace-less `Discover`, clamping the caller's `limit`, and evicting a registration all need the upstream `Registrations` store vendored (~600 lines), which is a fork in a security-critical path. Revisit before any public deployment; see the note below |
| **[x]** | P4 call-signal FIFO kills voice group-wide | `32dab2a` | |
| **[~]** | P5 connection limits | `0af1583`, `0ec438a` | Both missing caps are now set. Inbound was already capped (64 pending / 256 established / 8 per peer); added `max_pending_outgoing(32)` (the bound on being made to dial a large address set, from an invite's bootstrap list, a PEX record or a rendezvous response) and `max_established(320)` (a *total* cap, inbound and outbound together; the inbound cap bounded none of this node's own dials, relay circuits or rendezvous links). Each carries a doc comment saying what it buys and what it costs, in the `relay_node.rs` style. **Left:** neither limit is proven to bind. The mesh swarm's limits are constants inside `MeshBehaviour::new` with no config seam to turn down, and `connection_limits::ConnectionLimits` exposes no getters, so the `infra_limits.rs` discipline (turn a limit down until the operation fails) needs a knob that does not exist yet. Stays `[~]` until it does: an unexercised limit is a limit nobody knows the units of |
| **[~]** | P6 no eviction primitive | `0ec438a` | Built, then reworked twice after hostile reviews. `MeshTransport::evict_peer`/`unevict_peer` (default-inert, so the memory transport is unaffected) → `Command::Evict`/`Unevict` → an `Eviction` gate behaviour keyed on the **phase-0** peer id, declared **first** in `MeshBehaviour` so it refuses before `gossipsub` allocates anything (`to_peer` is a forward hash, so the deny is computable at connection time), plus `allow_block_list` to close connections that were already live and a `ConnectionEstablished` check to close one that raced past the gate before the deny landed. Driven from every path that merges a removal via `note_removal_applied`: `commit_remove_now`, `note_commit_applied` (roster diff around `process_incoming`) and the fork-contest winner branch. Deny entries do not expire on a timer; they are bounded at 256, oldest-first, and are lifted only by an **explicit** owner/admin action (`Server::readmit_evicted_peers`, wired to the desktop's "Generate new invite" and nothing else). Readmission alone cannot drive it: at the inviter the roster cannot change until the joiner's request is served, and that request needs the very connection the eviction refuses. Minting deliberately does **not** lift, because minting is also automatic; `get_invite` re-mints whenever the node has gained an address the stored invite lacks, so tying the lift to minting re-admitted every removed member the next time anybody opened the invite panel. **Left, and why it is `[~]`:** (a) the `DeviceId → transport peer` link is still the self-asserted `PeerDescriptor.peer_id`, and the signature binds that value to its signer without binding it to *naming* its signer, so it is attacker-chosen; three checks bound the damage but do not close it (see the section 11 entry for the honest reading of the residual, which is worse than a race). (b) Infrastructure is out of reach only where **this node's own configuration names it**: relays and rendezvous it dials, reserves on, registers at or routes through. An infra node this process has never been told about is not protected. (c) The lift cannot be narrower than "everyone currently evicted", because an `InviteToken` is not bound to an invitee device, so one deliberate re-invite re-admits every outstanding eviction at once. (d) The deny list is process-local and not persisted, each member evicts only when *it* applies the removal (so a lagging member stays attached), and no member grants circuit reservations today (`relay_client` only), so that half is closed by construction rather than tested |
| **[x]** | P7 invite bootstrap addresses unvalidated | `32dab2a` | |
| **[x]** | P8 eclipse source count attacker-supplied | `32dab2a`, `126c606` | Closed, and **not** the way the P9 row used to promise. `32dab2a` made `S` count roots that actually returned a peer (with decay) and added the source-collapse alarm that fires independently of reach. What was left was the fabricated record, and it turned out the membership tag would not have fixed it (see the P9 row). Two rules finish it, both in `effective_discovery_roots`. **(a) A rendezvous root counts only once a peer it named is *confirmed*:** some device still on the roster has signed a `PeerDescriptor` claiming that transport peer. Serving a registration is free and unauthenticated, so "answered with something" is not corroboration; the evidence is a *device* signature checked against the MLS roster, which is a stronger key than the tag's group-shared `ns_secret_L` and needs nothing new on the wire. This also closes the self-echo residual `ingest_discovered` documented (own device excluded), so a rendezvous handing our own registration back is worth nothing. **(b) All rendezvous roots together count at most one**, because the set of them comes from the inviter-chosen `rendezvous` vector: two entries in it are one trust decision, and nothing observable from inside distinguishes two independent operators from two hosts one party rents. That is what actually kills "a hostile inviter naming two rendezvous it controls". Member roots (a roster member that answered PEX, keyed on the device its response was signed with) still count one apiece, because being two of those takes two admissions through the group's own owner-serialized gate, which the inviter does not own. **Residuals, named:** confirmation reads `PeerDescriptor.peer_id`, which is self-asserted (the section 11 entry), so a member could claim a fabricated transport peer and let a colluding rendezvous "confirm" it; that buys the attacker the single root one honest rendezvous would have given anyway, since the class is capped. And no measurement at this layer can establish that two rendezvous are *independently operated*; refusing to count more than one is the honest response to that, not a proxy for it |
| **[~]** | P9 membership tag never carried on the wire | `126c606` (closed as a decision) | **CLOSED AS A DECISION (2026-08-19): not built, and not to be built. It does not block P8, and the claim that it did was wrong.** Three findings, any one of which is sufficient. **1. It defends the wrong attacker for P8.** The tag is a MAC keyed on `ns_secret_L`; the namespace is *derived* from `ns_secret_L`. Same secret, so the only party the tag separates from a member is one that learned the namespace *string* without the secret, which means the rendezvous operator (it is presented to them by construction) and anyone they tell. P8's own attacker is a hostile **inviter**, which is a member, holds the secret, and can mint a valid tag for any `peer_id` at any of its rendezvous. Verifying tags would have left P8 exactly where it was. **2. The libp2p API cannot carry it, and forcing it through would be a new disclosure.** `rendezvous::client::register` builds the `PeerRecord` from the swarm's *global* external-address set and mints `seq` inside `PeerRecord::new` from the wall clock, so a registrant cannot know the `seq` the tag preimage binds and cannot scope an address to one registration. Getting a synthetic component in means `add_external_address`, which feeds **identify**: every namespace's tag would ride in every other namespace's record *and* be broadcast to every peer this node connects to, handing strangers a stable group-linked token. Fixing that means vendoring `register`, the same fork-in-a-security-critical-path objection that deferred P3's census fix. And the component itself has nowhere safe to live: multiaddr rejects unregistered protocol codes, so it would have to masquerade as `/dns*` (exactly what P13 hardened against) or as an address something will try to dial. **3. No call site could act on it.** `SCORE_TAG_VERIFIED` only orders a list. The one shipping path that ranks several candidates together is the **pre-join** discovery in `apps/desktop` and `catcomsctl`, which holds no group secret and so cannot verify a tag at all; the **post-join** path (`ChannelSync::ingest_discovered`, the one with the secret) calls `plan` with a one-element vector, where a score orders nothing; and `dial_cached_peers`, which does batch, is all `Source::Cache`, where no tag exists. So even a perfect carrier would change no behaviour, unless the policy started *dropping* on it, and the standing property is that `DiscoveryPolicy` only ranks and never drops. **The PEX `PeerDescriptor` was considered as an alternative carrier and is not one.** It is signed, extensible and already exchanged between members, but it travels between peers that are *already connected and already roster-verified*, so it cannot pre-authenticate a rendezvous record it does not contain. What it can answer is the question the tag was a proxy for; "is this transport peer a real member's"; and it already answers it, under a device key bound to the MLS roster rather than a secret every member shares. That is why the P8 fix lands there instead. **What was left**: the primitive stays (correct, constant-time, and the operator-injection attacker it names is real if a carrier ever appears); its doc comment no longer claims it is carried; `Candidate::tag_verified` and `SCORE_TAG_VERIFIED` are documented as permanently inert rather than as a live tier; and what P8 needed from it is served by the roster-backed confirmation in the P8 row |
| **[~]** | P10 no padding or size quantization | `d19fec0` | **Built for the two paths that carry content; not built for the third.** The scope question was real, because the two sources disagreed: `ARCHITECTURE.md` locks *blob-fetch* padding, P10 broadened it to messages. Measurement settled it. A sealed CRDT op is `~250 + text_len` bytes, so the wire size *was* the message length plus a constant, attributed to a publisher by a signed gossipsub message, which is the leak rung 2 makes worse by design and it is also the cheapest one to close. So **both** were built, as two ladders over one frame, and a third case was rejected with numbers.<br><br>**The frame** (`catcoms_storage::pad`): `body ‖ zero fill ‖ u32 big-endian length`, padded to a bucket, sealed **inside** the AEAD so a forwarder cannot strip it. It is *canonical*: `unpad` recomputes the bucket the declared length maps to and refuses a frame that is not exactly that length, refuses a declared length that does not fit, and refuses a single non-zero fill byte, so there is one valid encoding per body, no covert channel in the fill, and a hostile pad is an error rather than a truncation. It is *deterministic*, so it draws nothing from the injected RNG seam (`check-no-ambient.sh` is unaffected) and cannot perturb the locked byte-identical-compaction property, which in any case operates over the **unpadded** `SignedOp` log: `SignedOp::encode`, the op hash, the signature preimage and the `EncryptedDoc` snapshot are all byte-identical to before, because padding is applied at `SealedOp::seal` and peeled at `SealedOp::open`.<br><br>**Ladder 1, sealed ops** (floor 512 B, ceiling 1 MiB, powers of two). 512 covers every message up to ~190 characters plus every reaction, pin, edit, topic set and profile tweak, so the bulk of ops become **one** size class. Measured wire sizes: a reaction 326 → 590 (+81%), `"ok"` 395 → 590 (+49%), a typical 37-character message (with or without an emoji and a mention) 431 → 590 (+37%), a `cid:` embed 472 → 590 (+25%), a 200-character message 595 → 1102 (+85%, the octave step).<br><br>**Ladder 2, sealed file chunks** (floor 4 KiB, ceiling 8 MiB, powers of two), inside `seal_file`/`open_file` so `plaintext_cid` and `size` still describe the **true** plaintext and the whole-file content-address check, the index's dedup and the UI byte count are untouched. The ceiling is **exactly** `catcoms_app::CHUNK_BYTES` (pinned by a test), which is the whole design of the large-file case: a full chunk is a fixed point costing only the 4-byte footer, while a short tail pads up to it and stops being the file's exact size. Stored-blob sizes: a ~6 KB custom emoji 6,040 → 8,236 (+36%), an 18 KB image 18,040 → 32,812 (+82%), a 200 KB image 200,040 → 262,188 (+31%), a 1.5 MB photo 1,500,040 → 2,097,196 (+40%), a full 8 MiB chunk 8,388,648 → 8,388,652 (**+4 bytes**), a 7 MiB tail 7,340,072 → 8,388,652 (+14%).<br><br>**Rejected, with the numbers:** padding every chunk to 8 MiB (a 1 KB avatar would cost 8 MiB, and a full chunk is *already* uniform, so it buys nothing at 100% cost); a finer ~19%-step ladder (cheaper, but it leaks file size to ±19%, which against a known corpus is most of the fingerprint back); and a 1024-byte op floor (it would collapse messages up to ~700 characters for ~1 KB each, affordable on gossip, but it lands on every op in a catch-up bundle too and turns a 5,000-message backlog from 2.1 MB into 5.5 MB instead of 2.95 MB).<br><br>**Relay arithmetic: unchanged, deliberately.** `RelayLimits::nominal_window_bytes` sizes `max_circuits` against `node_budget_bytes` using `NOMINAL_CIRCUIT_BYTES_PER_SEC` = 16 KB/s, and that number is a **voice** figure (32 kbit/s each way, charged twice by the meter). Voice is WebRTC DTLS-SRTP in the frontend: not a sealed op, not a blob, untouched here. It also remains the pessimistic yardstick, since a member gossiping a padded op every second draws 1.2 KB/s and bulk blob traffic was already bounded by `max_circuit_bytes` and the per-peer/per-prefix byte budgets rather than by the nominal rate. So `validate()` still asserts a true statement and was not changed. What padding *does* move is how far a budget goes: 1,000 members at 100 ops/hour cost a relay ~40 MB/hour of extra ingress+egress, 0.45% of the 8 GiB node budget; small-file fetches cost up to one bucket step more against the 1 GiB per-peer budget.<br><br>**Left to do**, and why it is `[~]`: (a) **avatars and banners are not padded**, and they are exactly the sub-chunk blob the leak is about. They do not go through `seal_file`; `set_profile` puts the raw image bytes and the profile doc carries the **plaintext** cid, so there is no application AEAD and no plaintext-cid arbiter to make an in-band frame unambiguous, and every scheme that fits rests on a heuristic or on an additive profile-doc key. Bounded, not closed: `MAX_AVATAR_BYTES` is 64 KiB and `MAX_BANNER_BYTES` 256 KiB, and both are UI-produced JPEGs at a fixed pixel size, so the size distribution is narrow to begin with; but a fetch still reveals it exactly. (b) **Per-session outer re-encryption**, the other half of the `ARCHITECTURE.md` §2.8 item, is not built, so a forwarder can still link repeated fetches of the same blob by ciphertext identity even though it can no longer read the size. (c) **Membership and discovery control traffic is unpadded**: commit records, the owner-signed roster, PEX bundles, DM invites, call signals and the join handshake all carry their natural sizes. (d) The doc catch-up ceiling moved; see the section 11 entry |
| **[x]** | P11 unjittered discovery, unconditional dials | `923a4eb`, `e35b1b2` | |
| **[x]** | P12 relay advertised private addresses | `e35b1b2` | |
| **[x]** | P13 invite-supplied rendezvous addresses unvalidated | `7c46833` | Split by trust. `validate_operator_rendezvous_addrs` keeps today's behaviour (a DNS name is allowed; rung 4 needs one). `validate_invite_rendezvous_addrs` adds: no `/dns*`, and every literal globally routable, with loopback permitted only when the **whole set** is loopback, the same rule `dialable_bootstrap` uses. The range predicates moved into one shared `catcoms_net::addr` module that the desktop bridge and `is_advertisable` now both import, so there is no second classifier to disagree. Call sites: desktop `discover_and_connect` and `catcomsctl` join take the invite variant; desktop `connect_rendezvous`, desktop `mint_and_store_invite` and `catcomsctl serve --rendezvous` take the operator one. Also found: the pairing-grant path (`join_one_grant`) had no validation at all and now takes the operator variant |
| **[x]** | P14 a joiner never saw later members | `36ca2df`, `8b97afb` | Both halves closed. Control-topic cause fixed in `36ca2df`; the recovery cause is fixed here: a commit chase carries the epoch that proved the gap and the peer that revealed it, an **empty** bundle answering a proven gap now marks that peer failed instead of counting as success, and the peer whose op revealed the gap is skipped for that gap's first attempt (a first-choice exclusion only, dropped on every re-queue, so it can never wedge). Selection only: response verification, the roster check, the nonce/epoch anti-replay and the two-pool separation are untouched |

**Note on P3, and why invite limits do not apply.** The instinct to charge for registration is
right; the mechanism has to sit where the attacker actually is. The rendezvous is public
infrastructure *below* any group: a squatter is an anonymous host that has never held an invite
and is registering namespace strings it made up. The node cannot distinguish a real blinded
namespace from a fabricated one, and that inability is the zero-knowledge property, not a bug.
So there is no membership to gate on, and the existing owner/admin gate on minting invites
operates at a layer the attacker never reaches.

Levers that work without giving the node what the design withholds: **source addresses** (built:
the per-prefix quota) and **proof-of-work per registration**, which is superlinear for a squatter
and a few milliseconds once for an honest client. The lever that would work but should not be
used: requiring proof of group membership, which hands the node exactly the membership knowledge
blinded namespaces exist to deny it.

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
| 0b | UPnP/PCP/NAT-PMP on a per-install port, IPv6, QUIC | none | none | only where the controlled gateway can map the upstream path | yes |
| 0c | AutoNAT (the sensor everything branches on) | none | **yes, a dial-back peer** | n/a | n/a |
| 1 | Two-way invite code | one paste back; both apps overlap for 60s | human chat only | no | no |
| 2 | Switchboard member (admission bridge) | joiner consent; host opt-in | reachable current member | only when that member is reachable | inherits the member's route |
| 3 | Guided port forwarding | router config, once | none | yes | yes |
| 4 | Bootstrap node, including a TCP/443 listener | none (a toggle) | yes | yes | yes |
| 5 | Self-hosted node / hosted mode | operator setup | own | yes | yes |
| 6 | Public DHT | none | public network | yes | yes |

### Rung 0a: mDNS

The most common first invite is someone in the same house. v1 buried mDNS in a later step, so
that user walked the entire ladder out to a public server to reach the next room. One
behaviour, zero third parties, zero latency. It belongs at the top.

**Still not built.** The narrower established-peer case is now covered by a vault-sealed route
captured only after an outbound direct Noise handshake and rechecked against the current roster.
That does not scan the LAN, advertise the group, or solve first contact. A future mDNS design must
bound observations and avoid letting unauthenticated advertisements bypass the application dial
scheduler; it must also disclose the LAN peer-identity privacy cost.

### Rung 0b: direct reachability

A **per-install** fixed port (see O6), IPv6 listening, QUIC alongside TCP.

**[v1 RETRACTED] v1 called IPv6 "the highest-value item here".** Overstated. Consumer routers
generally ship a default-deny inbound IPv6 firewall, and the `libp2p-upnp` behaviour is
IGD-based and **IPv4-only by construction**: there is no code path that opens an IPv6 pinhole.

**Built for IPv4 mappings and IPv6 firewall pinholes (2026-08-21).** Separate actor-owned `portmapper` clients probe and
maintain PCP or NAT-PMP mappings for both the stable TCP port and UDP/QUIC port, while libp2p keeps
owning UPnP IGD so the implementations do not duplicate one another's leases. A unified bounded
snapshot labels mechanism + transport, rejects non-public/CGNAT results as unusable while preserving
the reason, offers public mappings to AutoNAT, and folds them into the live bootstrap/peer record.
Lease ownership is reference-counted across UPnP/PCP/NAT-PMP and manual forwards; a late/slow
consumer sees authoritative current state rather than an unbounded event backlog, and failed probe
or MAP attempts retry after 60 seconds. The next displayed invite is re-minted
when the live address set changes, although an already copied signed code is immutable. `portmapper`
0.18 is pinned because it matches the
workspace's Rust 1.89 baseline; its high-level gateway/address path is IPv4-only. A narrow internal
PCPv6 MAP client supplies the missing firewall-pinhole path: it discovers the scoped default router
for the exact global listener address from the operating system IPv6 route table, binds that
address as the PCP client, validates the full request identity and Global Unicast result, and
maintains independent TCP and UDP/QUIC leases on monotonic deadlines. It requests five minutes but
honors the router-assigned lifetime, sanity-capped to 24 hours. Listener/interface loss withdraws
lease evidence and mapping-derived/NPTv6 addresses immediately and attempts a lifetime-zero delete;
an identical baseline GUA remains an unverified listener candidate after its pinhole expires. Task
generations make late worker events inert. The client accepts aligned response options it does not
understand, as RFC 6887 requires. Its rapid-recovery handling is deliberately per worker rather
than a complete gateway-wide ANNOUNCE coordinator. This is a mapping candidate, not a
reachability proof: the host/upstream firewall or the remote peer's IPv6 path can still fail, so
only address-scoped AutoNAT may mark it tested. IPv6 also **does not compose with an IPv4-only peer**,
and the model has no notion of *pairwise* reachability, only per-node. Keep IPv6, downgrade the
claim to "free when it works, silently absent otherwise", and make reachability pairwise in the
model, which is also the right input to switchboard selection.

### Rung 0c: AutoNAT

**[v1 RETRACTED] v1 wrote the entire ladder around AutoNAT when AutoNAT did not exist**: at that
point there was no feature, code or result path. It is the sensor rungs 1 and 2 branch on and the
eligibility test for switchboards. It is a **prerequisite**, not a follow-on.

**Partially built (2026-08-20).** `MeshBehaviour` now runs the libp2p AutoNAT **v2 client**;
relay and rendezvous swarms can run the v2 server after explicit `--enable-autonat`; a result travels through `MeshService` into
the desktop connectivity record; and the UI distinguishes a nonce-verified callback from a failed
address test, a router mapping and a relay path. Explicit public direct addresses (manual forward,
public IPv6, UPnP/PCP/NAT-PMP) are offered as candidates. Although upstream identify can suggest
additional candidates, product evidence is retained only while the exact address has a current
configured or mapping owner; a successful relay-circuit callback is never called direct
reachability. Two end-to-end memory transport tests prove both callback success and scoped failure
through the actor boundary.

The scope is deliberately narrow. Ordinary members do **not** serve dial-backs: making every
stable member listener an anonymous public probe service would add a resource and metadata surface.
Only explicitly enabled relay/rendezvous infrastructure serves. V2 is used instead of the legacy
aggregate-status protocol: the client accepts success only after receiving the callback nonce from
a fresh connection, and the requester uploads 30--100 KiB before the smaller callback, reducing
false positives and reflection amplification. A first-declared pre-socket guard now requires one
canonical direct public target at the request connection's exact source IP, rejects relayed/DNS/
circuit/private targets, and charges peer, source-prefix, whole-node and concurrency limits before
the transport dial. Serving remains experimental and **off by default** because same-public-IP or
CGNAT co-tenants can still request bounded probes of other ports and the observer learns metadata;
the missing target/rate-policy deployment blocker itself is closed.

A result remains **per address, server and moment**. A bounded snapshot retains the newest
observation for each address/server pair (with a fixed global cap), ranks public success above
public failure above local success, and prunes all observations when their route is withdrawn.
“Public” there classifies the candidate address, not the observer: operator configuration permits
a private/LAN relay or rendezvous. The UI therefore says a direct callback succeeded, not that the
node is universally reachable from the internet. Success does not prove every observer can
reach every transport; failure does not prove every candidate is private. The desktop therefore
keeps listening across candidates and reports those qualifiers rather than collapsing the result
into a permanent boolean.

It also **needs a peer willing to dial you back**, and a brand-new founder has no peers, so the
only candidate is the bootstrap node. **Rung 0 therefore depends on rung 4**, which inverts v1's
framing that dependency is only incurred on failure. Say so plainly.

Without it the escalation trigger degenerates to a timeout, and a timeout cannot distinguish
"the founder is unreachable" (escalate) from "the founder is asleep" (escalating is useless, and
rung 3 then asks the user to reconfigure a router to fix someone else's problem).

That paragraph still describes the product's escalation logic today: the measurement is surfaced
but does not yet drive concurrent rung racing or pre-flight invite gating. And with no relay or
rendezvous in the configuration--the exact shape of the 2026-08-20 field reports--there is no
dial-back server, so the honest result remains unknown. A public bootstrap deployment is still the
dependency this rung cannot remove.

### Rung 1: two-way invite code

The founder sends an invite; the joiner's app emits a short **reply code** carrying its own
public address; the joiner pastes it back into the same chat; both dial repeatedly until the
punch lands. **The humans are the signalling channel**, and that costs nothing.

**Built (2026-08-21), with an important limit:** both applications must keep overlapping
60-second sessions. The joiner retains its listener/transport after direct failure and emits a
`mewtual-reply-v1` code; the inviter validates at most four direct public TCP/QUIC candidates and
runs a bounded backoff dial session. A Noise connection is not enough: every callback peer must
prove possession of the invite-derived reply channel before it receives the bearer invite or
KeyPackage. Exact replay is idempotent, changing the joiner identity requires confirmation, and
the local verifier never authorizes longer than 60 seconds even when clocks differ. This is human
signalling, not STUN and not a relay. It mainly helps QUIC/NAT punching; no TCP simultaneous-open
guarantee is claimed, and symmetric NAT/CGNAT can still defeat it.

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

A directly-reachable, opted-in member can bridge the bounded admission exchange to the invite's
named inviter. This is now built as two explicit consent depths: a one-time reply-code helper grant,
or a per-server standing role. Standing offers are self-signed, live for two minutes, and remain
separate from strict `PeerDescriptor` v1. A fresh, explicitly prefixed `mewtual-invite-v3` envelope
carries each helper's complete signed offer and lets the inviter endorse at most three without
altering its identity, routes, sequence or expiry. The joiner always tries direct/rendezvous
routes first and must consent before helper dials. The helper verifies that the plan endorses its
exact device/transport identity, that it has a current record-bound live connection to the named
inviter, and that the short route has not expired. It forwards only bounded `JOIN`/`WELCOME` frames,
never chooses/adopts the Welcome, and catches up the exact MLS Add before becoming the joiner's
first member sync path. Pre-signature, per-requester, aggregate-node and frame-size limits bound the
public surface.

This is deliberately **not** a general circuit relay or long-term traffic switch. Once admitted,
the retained helper connection is an ordinary encrypted member link and may carry normal sync;
the helper already has member access, learns the joiner's IP/timing and spends bandwidth. Fresh
invite recipients learn the helper's stable device/transport identities and candidate addresses.
Turning hosting off refuses new forwards immediately, but cached or already-copied offers remain
dial-visible until their signed short expiry. Signed public candidates are endorsements, not proof
of address ownership/reachability. The feature helps an established group only: if the founder and
first joiner are mutually unreachable and no third party/public route exists, no signalling format
can carry their traffic.

**[v1 RETRACTED]** The original section described a general transport relay/noticeboard and then
placed discovery only at rendezvous. The shipped zero-owned-server slice instead uses inviter-signed
short offers inside a separately labelled assisted invite, because a pre-member with no reachable
rendezvous otherwise has no live helper-discovery path.

**A switchboard never admits a join.** The implemented helper path forwards the bounded admission
request to the exact named inviter and returns only the inviter-signed Welcome. The joiner verifies
that original signature; the helper cannot substitute itself as an admission authority. Accepting
a Welcome signed by any current member would still **reopen the group-substitution HIGH in full**:
the joiner has no roster before joining, and `group_id` is plaintext in the invite. If a pinned
multi-admitter set is genuinely wanted it is a separate design with its own adversarial pass, and
must not become an argument for raising `max_committer_rank` above 0.

**[v1 RETRACTED] The switchboard set must NOT ride in the invite.** `InviteToken` binds every
field under one signature, which is good crypto and exactly the problem: the set is **frozen at
mint time with no update or revocation path**. A switchboard that goes offline, changes ISP, or
is *removed from the group* is still named in every outstanding invite, still on the joiner's
only path, still able to see the joiner's IP and silently drop the join. Removal rotates the
routing secret but touches no outstanding invite, and changing the set needs a new nonce, hence
a new invite, hence revoking the old one.

The discarded rendezvous-only alternative had switchboards register under the member namespace.
That cannot bootstrap a pre-member when the rendezvous itself is absent/unreachable, which is the
zero-owned-server case this slice targets. The shipped short signed offer plus explicit v3 envelope
accepts a bounded two-minute address-disclosure window; it does not add a never-expiring capability
bit to `PeerDescriptor`.

**General transport switchboards remain unbuilt.** If the role later becomes a gossipsub/circuit
hub rather than the shipped admission bridge, it is not just an IP observer. In that topology it
would be the group's mesh peer, and gossipsub is signed, so for every message it forwards it sees
publisher, topic, sequence, timestamp and exact size. Payloads stay sealed; the activity graph does
not. The original analysis follows because it remains the gate for that deeper opt-in role.

In the topology this deeper rung exists
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

**Removal now detaches a switchboard, but only best-effort.** P6 is built: applying a Remove
commit asks the transport to evict the removed peer, which closes every live connection to it
(and with it anything scoped to that connection, which is what a granted circuit reservation is)
and refuses the next. Three things this rung still has to reckon with.

First, the eviction is aimed at the peer id the removed member **asserted about itself**, and the
signature on that record binds the value to its signer without binding it to *naming* its signer.
So a switchboard that published a peer id that is not its own keeps its connections, and a member
that published *somebody else's* can aim the group's disconnect at that somebody. The transport
refuses to evict any relay, rendezvous or bootstrap **this node's own configuration names** (which
now includes one it merely routes through, not only one it reserves on), and the sync layer refuses
a peer id a remaining member also claims. Neither closes the case where the squat lands before the
victim's own record does; only the deferred device-key binding does. A switchboard is exactly the
member with both the motive and the position, so this rung cannot treat the primitive as more than
best-effort.

Second, eviction is per node and fires when a member *applies* the removal, so a member still
lagging on that commit is still attached to the ex-member; the grandfathered topic window is
narrowed by that much rather than closed, and remains subscribed and derivable for two more
removals.

Third, a switchboard that is removed and later re-invited must be able to reconnect, so the deny
is lifted by an **explicit owner/admin action** rather than expiring. That keeps an eviction exactly
as durable as the removal itself, but it is coarse: an invite names no invitee, so one lift releases
*every* outstanding eviction, including for ex-members nobody meant to re-admit. Anyone re-inviting
one removed switchboard is, for that moment, re-admitting all of them. And the lift must stay bound
to a person's action: it was briefly attached to invite *minting*, which is also reached
automatically whenever the node gains an address its stored invite lacks, so every eviction in the
group was silently released the next time anyone opened the invite panel.

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

UPnP/PCP/NAT-PMP now surface gateway-not-found, non-public/CGNAT and probe failures in the shared
Connectivity assistant; symmetric-NAT classification below remains unbuilt.
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

### 5a. Built (2026-08-19): the operator's join log, real logging, the connectivity panel

Prompted by a live session where a founder sent an invite, the joiner got "join request
rejected", and **neither party could find out why**. Three causes, all closed:

1. **`serve_join` discarded its own reason.** Five distinct rejections all returned a bare `None`.
   It now classifies every exit into a `JoinOutcome` (`catcoms-sync`): `admitted` / `relayed` /
   `staged`, and `undecodable` / `wrong-group` / `not-this-inviter` / `bad-signature` / `expired`
   / `revoked` / `already-used` / `not-authorized` / `admission-failed`. Each attempt lands in a
   bounded (32) ring of `JoinAttempt { at_ms, outcome, peer_prefix, nonce_prefix }`, stamped from
   the injected `Clock` and **transient** (never persisted: it is a live diagnostic, not a record
   of who tried to reach this node). Surfaced `Server::join_attempts` → `AppCommand::JoinAttempts`
   → `get_join_attempts` → **Server settings / Join Log**, with copy-as-text.
   **The wire protocol is unchanged**: the joiner still receives an opaque rejection, because
   telling an unauthenticated caller which of the causes applied turns any stale token into an
   invite-ledger oracle. This is the *operator's* half only.
2. **The desktop app installed no tracing subscriber at all**, so every `tracing::warn!` in the
   whole stack was discarded and no log file existed anywhere. `run()` now calls
   `catcoms_log::init_debug_with` in `setup`, gated on a flag file under the app data dir,
   **off by default**, toggled in Settings / Diagnostics which also states the folder. The file
   filter is `APP_FILE_FILTER`, narrower than the CLI's blanket `debug`: the transport crates
   stay at `info`, because at `debug` they narrate every address and connection the node sees.
   `catcoms-log`'s module docs now say plainly what a debug log may contain (addresses, peer and
   device ids, activity metadata) and what it never contains (message text, file contents, names,
   key material), because the file exists to be shared.
3. **Nothing surfaced what an attempt actually did.** A `Connectivity` record in the bridge
   captures, per found/join: the advertised addresses, the UPnP result (distinguishing
   no-gateway from timed-out from an address), the AutoNAT v2 result, an ordered step log
   (`listen`/`advertise`/`relay`/`rendezvous`/`discover`/`dial`/`connect`/`join`/`invite`) each
   `ok`/`failed`/`unknown`, and the last error **verbatim**. Read by `get_connectivity`, rendered
   on the create/join screen and in Settings / Diagnostics, copyable as text.

   It now answers the narrower question AutoNAT can actually test: **could this connected public
   server reach this address at this moment?** A nonce-verified callback is labelled direct and
   tested; failure stays scoped to that address/server pair. Without both public infrastructure
   and a public candidate, `reachabilitySummary` remains `unknown`. A relay reservation is a
   separately usable route, while UPnP without a callback remains evidence rather than proof.
   Per-address *outbound dial* outcomes are still reported as `unknown` rather than invented:
   libp2p dials the set concurrently and only the first connection surfaces.

Still open here: the pre-flight self-test above, the last-seen rendezvous query on a failed dial,
symmetric-NAT detection, and the invite declaring which rungs it depends on.

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

1. **Admission switchboards see the joiner's IP/timing, spend bandwidth, and disclose their own
   stable identities/candidate addresses to invite recipients.** The current feature forwards the
   admission exchange and then retains an ordinary encrypted member connection; it is not a
   general gossipsub/circuit hub. Opt-out refuses new admission forwards immediately; a later MLS
   removal closes the retained ordinary member path best-effort once that commit propagates. An
   already-observed IP cannot be forgotten, and signed candidate addresses remain dial-visible
   until their two-minute expiry.
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
- Not that IPv6 is reachable by default. PCPv6 now requests a pinhole when the exact listener and
  scoped gateway can be discovered, but a grant is not proof of end-to-end reachability.

## 9. Build order

Status legend as in section 1c. **Review is per slice, before the commit that lands it**, never a
phase at the end: batching it is how unreviewed work reached `main` twice on 2026-08-18.

| | Step | Work | Blocking? | Review |
|---|---:|---|---|---|
| **[x]** | 0 | Fix pass 1a: identity, port, reload pipeline, UPnP window, IPv6+QUIC, `verify_self` in the join path, persisted `seq`, identify hardening | prerequisite for everything | yes, key persistence |
| **[x]** | 1 | **Wire PEX and `AddressCache` end to end** (P1). Also fixes presence and the permanent eclipse false positive. Started P8; P8 is now **closed** without P9, which is closed as a decision rather than built: see the note below | prerequisite for rungs 2, 4, 5 | **yes**: discovery and membership surface |
| **[~]** | 2 | AutoNAT v2 client in `MeshBehaviour`, guarded opt-in server on relay/rendezvous, scoped live diagnostics, UPnP/PCP/NAT-PMP mapping, PCPv6 firewall pinholes, recurring/pairwise repair, and sealed authenticated same-LAN re-dial for established peers are built; mDNS and concurrent escalation wiring remain | prerequisite for rungs 1, 2 | adversarial review complete |
| **[~]** | 3 | Shared live status line/readout/diagnosis is built; concurrent rung racing, failure messaging and pre-flight self-test remain | needs 0-2 | none |
| **[~]** | 4 | Settings / Connectivity and onboarding share the live evidence panel; the create-server mode/Advanced redesign remains | needs UI pass | none |
| **[x]** | 5 | Two-way invite code: MAC binding, local 60s cap, four public direct candidates, retained pending join, proof before bearer disclosure, idempotent admission/replacement | needs 0-2 | adversarial review complete |
| **[~]** | 6 | Node capacity fixes (P2, P3), TCP/443 listener, jittered discovery (P11), relay external-address misconfig (P12), bootstrap address validation (P7) | **blocks any public deployment** | yes |
| **[~]** | 7 | Opt-in admission switchboards: two-minute signed offers, inviter-endorsed v3 plan, direct-first joiner consent, exact live inviter binding, bounded forwarding and Add convergence are built. General circuit relay, monthly host budget and proactive one-time-help popups remain separate work. | needs 1, 2 | **mandatory; adversarial review complete for admission slice** |
| **[ ]** | 8 | Bootstrap node deployed, default on, live hysteretic expiry | needs 6, 7 | yes |
| **[ ]** | 9 | Port-forwarding wizard | needs 0 | none |
| **[ ]** | 10 | Hosted mode | **blocked on O1 and O4** | **mandatory** |
| **[ ]** | 11 | Public DHT | last, if ever | yes |

Independent of the ladder, worth fixing on their own: P4 (voice DoS), P5, P10 (padding; the op
and file-chunk halves are built, the avatar/banner half is not).

**Note on P8 and P9 (rewritten 2026-08-19; the earlier version of this note was wrong).** It used
to say that counting a root only once a peer it surfaced had survived `ingest_peer_record` was an
*interim* measure and that "the real fix is P9". It is the other way round. The membership tag is
a MAC keyed on `ns_secret_L`, the same secret the namespace is derived from, so it separates a
rendezvous **operator** from a member and nothing else; the hostile inviter in P8's own scenario
is a member, holds that secret, and can mint a valid tag for any `peer_id` at any rendezvous it
controls. Verifying tags would have left P8 exactly where it stood.

What closes P8 is the roster-backed confirmation (a *device* signature, which no member can forge
for a peer that does not exist) **plus** treating the whole inviter-chosen rendezvous set as **one**
trust root. The second half is the part that actually answers "two rendezvous it controls": their
independence is not observable from inside the node, and the invite picks them both, so counting
more than one of them was the error. P9 is separately closed as a decision (it is unbuildable
through the libp2p `PeerRecord` as designed, would broadcast a group-linked token over `identify`,
and no call site could act on it); see its 1c row. P8 may be described as closed; do **not**
describe the tag as a pending prerequisite for anything.

## 10. Open questions

- **O1. RESOLVED (2026-08-19): hosted mode keeps its first-screen question**, reframed around
  moderation rather than IP privacy. The product judgement that a group of strangers needs an
  enforcement point stands; what changes is the copy, which may no longer promise either of the
  two things v1 promised. The card may claim **immediate operator-triggered removal** (MLS Remove
  is real, cryptographic and already built) and **reduced incidental exposure**. It may **not**
  claim that members cannot see each other's addresses (voice discloses them outright, and DCUtR
  has no kill switch), nor that a ban holds against a modified client (the chokepoint's only
  handle is a self-minted peer id). It must disclose that the operator learns the membership set
  and the traffic graph, and that the operator can silently withhold or delay a member's messages.
  The "you can change your mind later" line must go until a mode migration is actually designed.
- **O2. RESOLVED (2026-08-21).** Standing assistance is a separate two-minute self-signed offer,
  never a `PeerDescriptor` bit or membership promotion. The inviter endorses fresh offers inside
  the explicitly labelled v3 join plan. The helper accepts a forward only while its local opt-in is
  on and it has a live, exact record-bound route to the named inviter; no rendezvous is required.
  Offer addresses remain best-effort candidates rather than health/reachability proof.
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

## 11. Open avenues

Things surfaced by the reviews, the field test or the work itself that are **not** tracked as
P-defects and would otherwise be lost. Not a roadmap: a list of loose ends with enough context
to pick up cold. Keep it current; delete an entry when it lands or is deliberately dropped.

### Blocks an honest answer somewhere

- **AutoNAT (rung 0c) is only a scoped sensor so far.** The v2 client, guarded opt-in infrastructure
  server and diagnostics exist, but there is no default deployed server, recurring/pairwise model
  or automatic escalation into relay/hole-punch rungs. The panel can prove one address from one
  server at one moment; it correctly remains unknown without that candidate/server pair. Turning
  those observations into the escalation trigger remains high-leverage work. Serving stays off by
  default because the guard bounds but cannot erase probe metadata, egress, or same-NAT port probes.
- **Per-dial outcomes are not observable.** `MeshService` races a dial set and only the winner
  surfaces, so the panel shows per-address results as unknown rather than inventing them. Needs
  per-dial event plumbing out of the transport.
- **The `DiscoveryPolicy` "member-tag-verified" ranking tier does not exist and will not.** P9 is
  closed as a decision, not as a build (see its 1c row for the three reasons), so nothing sets
  `Candidate::tag_verified` and every discovered candidate scores flat on that axis. This is much
  less consequential than it reads, because the one shipping path that ranks several rendezvous
  candidates against each other is the **pre-join** one, which has no group secret and could never
  have verified a tag; the post-join path plans one candidate at a time. The old note here said
  "P9 blocks P8"; that was wrong, and P8 is now closed without it.

### Hardening residuals, deliberately deferred

- **Eviction (P6) rests on a self-asserted peer id, and that value is attacker-chosen.** The only
  `DeviceId → transport peer` link is the `peer_id` field a device signs into its own
  `PeerDescriptor`. The signature binds the value **to** its signer; nothing binds it to *naming*
  its signer, so a member can publish a record carrying a third party's transport peer, be removed
  in the ordinary way, and have every member disconnect and refuse that third party. Three checks
  bound this (ingest refuses a duplicate claim; the eviction refuses a peer id a remaining member
  or this node claims; the transport refuses to evict infrastructure this node's own configuration
  names). **Only the deferred device-key-to-transport-identity binding closes it**, which makes
  that deferral load-bearing rather than cosmetic. Until then, eviction is best-effort and no
  property may be made to depend on it.
- **The residual is not a coin-flip, and calling it "a race the attacker has to win" was
  flattering.** Three things stack in the attacker's favour. A newly joined member starts with an
  **empty** record map, so nothing is there to collide with. Adds are announced to everyone on the
  control topic, so an attacker knows exactly when a new member appears and can push its squat
  immediately. And the duplicate check runs **before** the `seq` comparison, so a device can
  retarget its claimed `peer_id` at any moment with a higher `seq` (which it must be able to do,
  since a node's network identity can legitimately change). An attacker that publishes on every
  observed join therefore wins on essentially every new member. Worse, the payoff does not need a
  removal at all: while the squat stands, the victim's genuine record is refused on those nodes, so
  its PEX addresses and presence dot are suppressed there for as long as the attacker stays in the
  roster. The removal-driven disconnect is the escalation, not the entry price.
- **The duplicate-claim rule costs the victim a record.** First claim wins at ingest. Bounded (a
  removed device's record is dropped, so the claim does not outlive the membership) and far smaller
  than the harm it prevents, but it is a real cost and it is the mechanism the entry above abuses.
- **Eviction is not persisted and does not reach a lagging member.** The deny list is process-local
  (deliberately: a restart brings up a fresh swarm with no connections, and the ex-member is then
  an unauthenticated stranger holding none of the group's keys), and each member evicts when *it*
  applies the removal, so a member still behind on that commit stays attached to the ex-member.
- **Lifting an eviction is coarser than the removal that caused it.** An `InviteToken` names no
  invitee, so one deliberate lift releases every outstanding eviction rather than one.
  Owner/admin-gated and bounded at 256, and the alternative was that remove-then-re-invite
  deadlocks at the inviter, but a narrower lift needs an invite bound to an invitee device, which
  is its own design and is the same missing binding that makes eviction best-effort at all.
- **The lift must never be attached to an automatic path, and nearly was.** It was first wired to
  `mint_invite`, on the reasoning that minting is the owner declaring willingness to admit. That
  reasoning holds for the button and not for the call graph: `get_invite` re-mints on its own
  whenever the node has become reachable in a way the stored invite does not mention (UPnP
  answering at startup, a relay circuit reserving, a rendezvous registering), so opening the invite
  panel released every eviction in the group with nobody deciding it and no trace. It is now
  `Server::readmit_evicted_peers`, a separate owner/admin command wired to `mint_invite_fresh`
  only. Worth remembering as a shape rather than an incident: a security control whose *release*
  is folded into a convenience action gets released by convenience. The defect lived in the
  interaction between two files with different owners, and neither diff showed it alone.
- **A time-boxed admission window was considered instead, and declined.** The idea: rather than an
  explicit lift, allow a removed peer back for as long as an invite is outstanding. It is narrower
  in *time* but not in *who*, and it fails on the same ground the automatic lift did: the window
  would be opened by the automatic re-mint, so it would restore precisely the silent re-admission,
  with a timer on top. It also re-introduces "a decision nobody made, because a clock advanced",
  which is the argument against expiring deny entries in the first place, and it would need a
  `Clock` plumbed into the mesh actor's deny path (which, unlike `admission.rs`, has none). An
  explicit lift is auditable: one action, one moment, and it can carry UI copy naming what is
  being re-admitted, which is what rung 2's consent story needs. The genuinely better answer is
  neither: bind the invite to an invitee device so the lift can be narrow in *who*.
- **The establish/deny race closer is not deterministically exercised.** Whether an eviction
  reaches the actor before or after an in-flight connection's `ConnectionEstablished` is
  `select!`'s choice, so the socket test asserts the property under either ordering rather than
  forcing the interesting one.
- **The fork-contest winner path's eviction is untested.** It is one call on a branch that needs
  `max_committer_rank >= 1`, which the project deliberately does not enable, so it is covered by
  construction (all three merge branches now funnel through `note_removal_applied`) rather than by
  a test.

- **The served PEX set is chosen in randomised map order.** Local retention is 512 records, the
  wire cap is 64, so `serve_pex` picks 64 of up to 512 by `HashMap` iteration order. It spreads
  rather than pins and converges over ticks, but the selection is not deliberate.
- **No idle-connection reaper**, so the per-prefix connection lockout is mitigated rather than
  closed: an attacker holding idle sockets moves no bytes and makes no registrations, so neither
  budget notices.
- **The file-descriptor check reads but cannot raise** the soft limit (`setrlimit` needs `unsafe`
  and the workspace denies it), and it reads `/proc/self/limits`, so it is Linux-only and a
  documented no-op on Windows and macOS, where the default limit is also low.
- **P3's census prevention** is deferred by decision: rate-limited, not prevented. Revisit before
  any public deployment.
- **Corroboration cannot measure operator independence, and the eclipse count no longer pretends
  to.** `S` now credits the whole inviter-chosen rendezvous set with **one** root, because nothing
  observable from inside this node distinguishes two independent operators from two hosts one
  party rents, and the invite chooses them both.

  The consequence, stated so nobody re-derives it as a surprise: `S >= min_sources` now requires
  at least one **member** root, and a member root requires having reached a member. So in suspicion
  term 1 (`reach < min_reach && S < min_sources`) the two conjuncts have become close to the same
  measurement, and the term is carried by reach. The corroboration signal does its independent
  work in term 2, the source **collapse**, which is where it was always meant to live: a group that
  once had a member vouching and now has only infrastructure, while still reaching several members,
  is the eclipse shape.

  This is not a new false-positive class. A group with a **single** rendezvous already sat at
  `S = 1` and already tripped term 1 whenever reach fell near zero (an ordinary quiet night in a
  group of five or more). What changed is that adding a *second* rendezvous no longer silences it,
  which is precisely the silencing P8 was filed about. If quiet-night noise proves to be the
  practical problem, the honest lever is `roster_floor` / `min_reach` / `grace_ms` tuning against a
  real deployment, not re-inflating `S` with roots nobody verified.
- **Rendezvous confirmation rests on the same self-asserted `peer_id` that eviction does.** A root
  counts once a roster member's signed record claims the transport peer it named, and that claim
  is bound *to* its signer rather than to *naming* its signer (the entry above). So a member can
  manufacture a confirmation for a peer that does not exist. It is bounded by the one-root cap
  (the attacker gains at most the root a single honest rendezvous would have supplied) and by
  `ingest_peer_record`'s first-claim-wins rule, and it is closed by the same missing
  device-key-to-transport-identity binding that would close eviction.
- **P14's third refinement** (prefer a peer known present at the target epoch) was declined with
  reasoning. Revisit only if a real deployment shows repeated misses.
- **Padding shrank the un-resumable document catch-up ceiling by about 27%.** `export_catchup`
  re-seals the **whole** op log from op zero every time and `size_capped_ops` serves a contiguous
  prefix that fits `MAX_CONTROL_RESPONSE` (16 MiB); there is no offset and no resumption, so that
  cap is a hard ceiling on how much history one document can ever transfer. At ~434 encoded bytes
  per op that was roughly 38,700 ops; at the padded ~594 it is roughly 28,200. The wall is
  pre-existing and is the deferred "resumable chunked anti-entropy" of `ARCHITECTURE.md` §2.8;
  padding moved it closer, and both numbers are the same order, which is the honest reading. It
  cannot be bought back by raising the cap, because 16 MiB is also `MAX_FRAME` in `catcoms-net`.
  The alternative considered and **not** taken was to skip padding when re-sealing for a bundle,
  on the ground that a forwarder sees one response total and per-op sizes inside it are hidden by
  aggregation. It was declined because it needs a dual wire format whose reader must accept an
  unpadded payload, which reintroduces exactly the ambiguity the canonical frame removes, and
  because an incremental catch-up after a brief disconnect is often one or two ops, where the
  aggregation argument does not hold.
- **Avatars and banners are still exact-size on the wire** (P10's `[~]`). They are the canonical
  "small file whose blob size is its file size", and they are the one content path padding did not
  reach, because they are unsealed blobs addressed by the **plaintext** cid: nothing on that path
  can tell a padded frame from image bytes without either a heuristic or a new profile-doc key.
  The named fix is an additive marker key beside `avatar_cid` (the shape `banner_cid` already
  used), and it belongs with the per-session outer re-encryption slice rather than folded into a
  padding change. Bounded meanwhile by the 64 KiB / 256 KiB caps and by both being fixed-size
  re-encodes produced by the uploader's own UI.
- **The padding primitive lives in `catcoms-storage`, which is not where it belongs.** It is a
  byte-frame codec and its natural home is `catcoms-wire`, which both call sites already depend
  on. It sits in `catcoms-storage` because that is the lowest crate `catcoms-replication` and
  `filecrypto` share without editing a crate outside this pass's ownership, and because one
  implementation of the unpadder is worth more than a tidy module tree. The cost is a
  `catcoms-replication` → `catcoms-storage` dependency edge that is acyclic but reads oddly.
  Move it to `catcoms-wire` when something else opens that crate.

### Product gaps the copy now exposes

- **Mode migration is not designed.** Switching a live group between friend-circle and hosted is
  a topology change, not a setting. The "you can change your mind later" promise was removed from
  the UI rather than kept, so a founder steered into friend circle has no stated path out.
- **Hosted mode is blocked on O4**: a node-held ban list is a second authority that provably
  diverges from the group's own, because ownership follows the lowest live MLS leaf and migrates.

### Housekeeping

- The **desktop workspace is not rustfmt-clean** (6 pre-existing diffs) and is not covered by the
  build ritual, so `cargo fmt` there reformats unrelated code. Every agent has had to hand-match
  its own hunks and revert the churn. Worth either fixing once or adding to the ritual.
- Desktop clippy has a baseline of 2 lib and 4 lib-test warnings.

