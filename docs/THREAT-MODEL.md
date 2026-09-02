# Threat model; client integrity & enforcement boundaries

This document tracks **what a modified ("hacked") client can and cannot do**. Mewtual is
peer-to-peer and open-source: anyone can run a build that ignores the UI's rules. The question
this document answers, action by action, is *which rules are enforced by cryptography / the
protocol (a modified client cannot break them) versus enforced only by the honest client (a
modified client can ignore them)*, and what we plan to do about each gap.

It is a **living backlog**. When a residual is closed, move it to the "Protocol-enforced"
table with the commit that closed it.

## Trust assumptions

- **Cryptographic core holds against modified clients.** End-to-end message confidentiality,
  membership authentication, forward/post-compromise secrecy, the owner anchor, and admin-grant
  authenticity are enforced by MLS + signatures, not by client behavior. A modified client
  cannot read messages it isn't entitled to, forge membership, forge an admin grant, or forge
  the owner identity.
- **A colluding malicious *majority* is out of scope** (as elsewhere in the design). We defend
  against a single modified client / member, not against the committer (owner) itself being
  malicious; the owner is the root of governance authority by construction.
- **Governance/policy actions vary.** Some are protocol-enforced; some are enforced only by the
  honest client today. The tables below are the source of truth.
- **Public reachability infrastructure is low-trust, not invisible.** A relay/rendezvous that
  serves AutoNAT v2 learns the requester's source address, candidate addresses, peer id and probe
  timing, and can withhold service or cause a false negative. It cannot forge a v2 positive merely
  by claiming success: the client accepts success only after the fresh callback returns its nonce.
  Ordinary members do not serve anonymous probes. The relay/rendezvous wrapper tags the upstream
  callback and a first-declared pre-socket guard accepts only one direct public TCP/QUIC literal
  whose IP exactly matches the request connection's source; it charges peer, source-prefix, node
  and concurrency limits before opening a socket. Exact-IP matching still permits bounded probes
  of other ports on the requester's shared NAT/CGNAT address, and the service retains metadata and
  egress cost, so it remains **experimental, disabled by default, and operator opt-in**.
  A successful callback proves only that the configured observer reached that exact candidate at
  that moment. Operator configuration also permits LAN/private infrastructure, so the product does
  not turn that result into the broader claim “reachable from the internet”.
- **Member switchboards are consented, bounded admission paths, not new authorities.** A standing
  host explicitly opts in per server; its complete two-minute self-signed offer is carried under
  the invite's named inviter endorsement, so the inviter cannot substitute routes or lengthen the
  helper's consent. A joiner explicitly consents before any helper address is dialled.
  The helper must be a current member with a live record-bound connection to that exact inviter.
  It forwards only bounded `JOIN`/`WELCOME` frames, applies the resulting Add before serving as the
  joiner's first sync path, and cannot forge the inviter-signed Welcome. It already has normal
  member plaintext access; helping grants no additional content authority. The host learns the
  joiner's IP/timing and spends bandwidth, while invite recipients learn the host's stable device
  and transport identities plus candidate addresses. Opt-out refuses new forwards immediately,
  but cached/already-copied signed offers remain dial-visible until their short expiry. A malicious
  member may sign an arbitrary *public* candidate: canonical terminal-peer binding plus endpoint,
  prefix, server and process caps bound the resulting scan surface, but the signature does not
  prove address ownership or live reachability.
- **Two-way replies authenticate possession of the bearer invite, not a person.** Candidates are
  public direct literals, capped at four and live for at most 60 seconds from receipt. Every callback
  contact proves the invite-derived reply channel before seeing the invite/KeyPackage; replacing a
  different joiner needs confirmation. Anyone who obtained the original invite can still form a
  valid reply or redeem it. Both apps must remain open during an overlapping window, and symmetric
  NAT/CGNAT can still make punching impossible. Each callback socket pass spends the shared
  endpoint budget; proof retries use a connected-only actor command, so a proof queued before the
  first connection completes can still use that live connection but cannot consult the ordinary
  recent-peer cache and silently redial after the scheduler denied a new socket attempt.
- **The local router is trusted only for a mapping candidate.** UPnP/PCP/NAT-PMP and PCPv6 firewall
  pinholes can expose this app's stable TCP and UDP/QUIC listeners and can return a wrong or stale
  public socket. PCPv6 accepts only a request-matched Global Unicast result from the exact scoped
  default router, but the pinhole remains open to arbitrary Internet sources because invite peers
  are not known in advance. Noise and connection limits still protect the listener; AutoNAT is the
  independent test before the UI calls the route verified. A malicious gateway can still cause
  denial of service or dead invites. The client requests five minutes but honors a larger
  router-assigned lifetime up to a 24-hour sanity cap; a crash can therefore leave the pinhole
  until that grant expires. Publishing a global IPv6 privacy address also makes that
  device/address visible to invite recipients, peers and the configured AutoNAT observer.
- **Discovery retries are availability aids, not proof of presence.** The current member roster and
  self-signature gate every cached record. Every route accepted from PEX/cache/rendezvous is parsed
  by one canonical grammar, must terminate in the exact signed/discovered transport peer, and is
  revalidated immediately before submission; the network actor refuses a peer-less dial instead
  of falling back to `Swarm::dial(address)`. A signature authenticates who chose a public socket,
  not ownership of that socket, so the local policy charges every address and one desktop-owned
  scheduler caps process, server, canonical Phase-0 peer, attempt, and IPv4 `/24`/IPv6 `/48`
  dial-command submissions.
  The parser embeds the principal into the opaque endpoint, preventing cache, rendezvous, and
  pre-join callers from selecting different byte representations for the same transport. Direct
  attempt/prefix/process keys do not include descriptor sequence or terminal PeerId, preventing
  those rotations from resetting the main scanner bounds. Relay attempts are keyed by relay and
  terminal target so unrelated circuits sharing a relay do not consume one two-attempt bucket; the
  relay host is instead bounded by prefix and process caps. Scheduler state is bounded and transient;
  restart resets it. It accounts submitted dial commands rather than actor-confirmed socket starts,
  and relay outer sockets have no separate exact-socket permit. Discovery endpoint permits are now
  actor-consumed, single-use, deadline-checked, and accounting-generation-bound. Reservation and
  commit share one injected monotonic clock; queued work cannot start after its window merely because
  no later reservation rolled the counters. Cancellation after command enqueue cannot refund queued
  work; actor-owned connected, member-duplicate, and pending-infrastructure checks refund before
  commit; and a late old-window drop cannot decrement current counters. A process-wide
  in-flight/concurrency lease remains future hardening.
  Direct routes carried by companion grants use the same canonical invite policy, must name one
  unambiguous contact, and spend this process budget across every grant in a bundle. Pre-join
  rendezvous uses at most two validated seeds so infrastructure routes cannot consume the entire
  per-server window before the actual inviter route.
  The sync classifier refuses DNS and dangerous local/private/transitional ranges but deliberately
  retains non-routed documentation/benchmark literals for deterministic tests; such a record can
  waste only bounded retry tokens and is not proof of a live public route.
  Failed current epochs retry with monotonic exponential backoff and jitter; a newly signed epoch
  is tried immediately. Each discovery pass also asks the local kernel which IPv4/IPv6 source it
  would route toward documentation-only destinations (UDP `connect` sends no packet), then
  republishes a changed raw route set. A process-wide native route/interface monitor normally
  triggers that operation after a short debounce; the discovery-period poll repairs a missed event
  or unavailable monitor. Exact ownership preserves an identical manual/mapping/relay route.
  The cache intentionally replaces rather than permanently unions withdrawn
  public IPs: an ISP may reassign an old address, and Noise authentication prevents impersonation
  but not the metadata/connection cost of probing its new holder. PEX success proves one live
  member connection at that moment; failure means only “not reachable from this device by this
  path,” never offline, removed, or malicious.
- **Typed member-route health describes a self-asserted claimed peer.** A member's device signature
  authenticates the `PeerDescriptor` fields it chose; it does not prove control of the transport
  key named in `peer_id`. A malicious member can claim an unrelated already-connected peer and
  make its own row inherit that peer's current coarse path evidence. The backend therefore uses
  `ClaimedPeer*` health variants, exports `binding=self_asserted`, attaches observations only to
  the signer/member's own row, clears history when the claimed transport identity is replaced or
  removed, and the UI preserves the caveat. A fresher address epoch for the same stable transport
  keeps that peer-scoped history; it is still labelled historical and carries no address. This is
  diagnostic evidence, never authorization, membership proof, “online,” or
  public reachability. The implemented reciprocal-dial repair is not a device↔transport dual-key
  challenge and does not strengthen it; a future explicit proof is required. Raw active snapshots and retained history are capped, deduplicated, session-only,
  monotonic-aged, and hidden after 24 hours. Transport churn can still backpressure the actor's
  bounded event queue; connection limits and duplicate-close suppression bound amplification but
  do not make it free. Unclaimed transport peers do not invalidate the member-only UI projection.
  A dial-scheduler row proves only that a policy-approved batch was submitted, not that every
  candidate reached the transport or failed, and the UI does not infer outbound IPv6 capability
  from inbound/public candidate observations. Current switchboards forward admission only and are
  not presented as a post-join repair path. Operational availability instead uses request-bound
  signed catch-up proof. That transient cache retains the current roster `DeviceId` that answered
  on each peer connection, so a removal prunes the departed signer without invalidating unaffected
  live member paths; every later request still authenticates independently.
- **Member-assisted repair is bounded signalling, not delegation of authority or traffic.** A
  helper answers SWIM-style probes only from its current session-proven connection table and never
  dials the target for a probe. Forwarding requires a current authenticated requester, exact
  current requester/target descriptor references, live proven paths to both, a short expiry and a
  per-device budget. The helper signs the original frame and queues connected-only delivery so an
  A→C→B cycle cannot nest actor waits. The target rechecks roster, descriptors, helper proof,
  signature, replay and rate limits, then submits at most two direct routes through the ordinary
  discovery and process scheduler. A descriptor replacement/removal cancels pending work; already
  submitted transport dials cannot be recalled. Signed negatives create only time-bounded
  suspicion and never change presence or MLS membership. The control plane reveals attempted peer
  pairs/timing to the selected helper, but no plaintext; ordinary participation does not make that
  helper a public listener or general relay.
- **The desktop webview is trusted only while the UI session is unlocked.** An explicit lock keeps
  native actors online for background sync but closes all non-bootstrap Tauri commands. CSP and
  the main-window capability reduce injection reach; the native command gate is the enforcement
  layer, not the fact that Svelte hid or cleared a control.
- **Diagnostics have separate local-capture and public-disclosure boundaries.** Safe capture
  destructively replaces literal addresses, arbitrary runtime prose, runtime field names, and
  non-allowlisted targets before bounded ring storage; Enhanced and Full intentionally retain more
  local detail and still require review before sharing. Webview-origin trace ids are normalized
  under the process-local diagnostic salt; a native trace crossing back through the renderer is
  accepted unchanged only with its session/trace-bound proof, which is never stored or rendered.
  The optional raw debug file is a different sink and does not pass through Safe admission. Any
  native tracing or frontend console/error value that passes its target/level filter can reach disk,
  including names, message fragments, paths, URLs, tokens, serialized objects, and stack traces.
  Its size/rate bounds are availability and retention controls, not a privacy filter; the user must
  inspect the actual file before sharing it.
  After Tauri decodes an invoke body, structured UI commands retain at most 256 events and 32 fields
  per event. This does not impose a byte cap on Tauri's already-materialized JSON request: a compromised unlocked
  webview can still spend native parsing time/memory with an oversized request body, so a transport-
  level command payload cap remains an availability hardening item. An unlocked or compromised webview may ask
  native code to open the fixed public-diagnostics issue, but supplies no report, title, or URL:
  native code renders its closed allowlist, validates it, constructs the bounded tracker URL, and
  launches that exact value. A truncated URL does not claim the later clipboard attempt succeeded;
  the UI reports copy success only when the clipboard promise resolves. Ordinary user-authored
  feedback remains a separate exact-tracker launcher path and is not presented as automatic safe
  diagnostics.
- **Delivery receipts prove receipt by a member device, not attention by a person.** Only a newly
  applied, signature-verified op can queue a receipt, and it is sent as a connected-only
  member-authenticated request. The author records it only when its exact document/change hash is
  already in the bounded recent-target set; arbitrary hashes, duplicates and unauthenticated
  frames cannot allocate confirmation state. A recipient can withhold receipts or a modified
  current member can claim receipt without displaying content, so the UI says delivered/held and
  never read. Receipt traffic adds message-timing metadata to peers already participating in the
  encrypted group; it is not broadcast outside the group.
- **Adaptive stream resolution reveals only a coarse call preference.** A call participant sends
  one of four height buckets, not its exact window, monitor dimensions, or pixel ratio. The sender
  treats it as an advisory upper bound and independently caps each WebRTC encoding. Screen tracks
  stay parked until resolution, bitrate and frame-rate parameters apply; rejection pauses the edge
  or stops sharing if parking fails. A malicious
  receiver can request 4K but cannot raise the sender above the sender's own capture/bitrate cap.
  H.265 and other efficient codecs remain runtime-negotiated capabilities, not security
  guarantees. WebRTC encryption protects media in transit, while endpoint capture, decoding and
  the who-is-watching/timing/bitrate metadata remain trusted-endpoint and traffic-analysis risks.
  Shared audio is off by default. Surface audio is whatever the platform chooser grants and can
  accidentally include Mewtual's own playback; separate sources require a fresh chooser grant per
  source/session, discard the picked picture, and mix only their audio into one WebRTC track. A
  bounded 0--200% master and bounded per-source gains change that shared track only; overlapping
  boosts can clip and never grant a new capture source. At most eight explicit application-audio
  grants can be live, and the graph/output track is released when the last grant ends. The call microphone is a separate sender. The
  WebView/OS decides whether window/application audio is available, so this is not an OBS-equivalent
  native process-capture guarantee.
- **Jam events are untrusted member input rendered inside a receiver-owned, bounded synthesizer.**
  The jam engine does not authenticate peers itself: the identity used to mint its source-channel
  capability must come from the already authenticated call connection, never a sender-controlled
  frame field. Each callback must present the opaque capability minted for that exact generation;
  reopen/removal invalidates older queued callbacks before they can recreate sender state. Every data-channel
  delivery is size- and rate-charged before parsing; valid patches pass the same strict, unknown-
  field-rejecting validator regardless of whether they came from the wire, local storage, import,
  or take playback. A patch selects values inside one fixed topology and cannot introduce graph
  edges, code, samples, assets, or feedback. Per-peer held limits, a global tail-inclusive voice
  limit, release/hold watchdogs, and receiver-owned gates/limiter bound honest-renderer work and
  output; a malicious member can still make bounded noise and a modified receiver can discard or
  reinterpret events. Remote events are not queued into a suspended audio context. A content hash
  promises the same recipe, not identical platform audio. Exact legacy note frames have a separate
  receive-only compatibility domain; they gain no patch, drum, clock, or recording authority.
  Deafen and member removal are security-relevant integration points: shipment requires Deafen to
  gate the instrument master and release remote voices, and roster revocation to close the removed
  member's call/data-channel connection. Recording consent and performer labels are honest-client
  coordination until the planned group-bound, domain-separated durable per-participant aggregate
  lane commitments exist; another endpoint can always record audio or events without announcing it.
- **A shared file is authenticated bytes, not trusted content.** Explicit save streams into a
  non-overwriting `.part`, checks the declared size and whole-file content address, sanitizes the
  peer-provided name to one leaf, atomically publishes only after verification, and reveals it in
  the file manager without executing it or invoking a shell. This blocks path/argument injection,
  overwrite races, corrupt-provider substitution and automatic execution; it does **not** make the
  payload benign or protect an external application the user later opens it with from its own
  parser vulnerabilities. Inline image/video/audio loads through a CSP-limited, `nosniff`,
  range-bounded custom scheme but is still parsed by the platform WebView/media stack. The scheme
  emits a body only when a bounded container signature matches the exact allowlisted MIME; SVG,
  mismatch and unknown inputs receive a bodyless denial because an octet-stream body may still be
  media-sniffed. Cached heads/chunks bind to the exact uniquely resolved current manifest rather
  than only the member-claimed plaintext CID. Cache access and the synchronous URI-responder
  publication are generation-gated, so a disk/network read that finishes after explicit lock
  cannot refill plaintext caches, reveal a stale size, or publish its already-built body. Storage
  inventory separately authenticates every exact encoded chunk reference and joins verdicts to an
  atomic manifest/listing snapshot. More than four distinct exact references for one ciphertext
  CID makes that CID and its dependent manifests unreadable without attempting unbounded repeated
  decryption. Native cache publication is additionally bound to the process-local server
  incarnation, while webview continuations require their exact still-unlocked view. A plaintext
  export's verified staging rename and reveal hold the same exact unlock-generation commit guard,
  so an older export cannot become visible after lock or a later unlock. Before a
  server is founded/joined, the user chooses local on-demand, specific-member, or everyone trust;
  the bounded per-server policy is vault-sealed. On-demand is the default. It leaves passive shared
  media, custom emoji, card thumbnails, event images and call-jukebox tracks inert;
  specific-member trust checks the file's authenticated full device identity. The short 32-bit
  fingerprint remains display-only. An explicit Load/Play/Open/Download click overrides the
  passive-fetch policy. Bytes fetched into the blob store occupy one authenticated,
  XChaCha20-Poly1305-sealed vault copy; explicit export creates a separate plaintext Downloads copy.
  This reduces unsolicited decoder exposure but does **not** sandbox a decoder or make a trusted
  member's file benign. Automatic whole-share mirroring remains unimplemented because the sealing
  disk store has no quota; enabling it first would permit storage exhaustion even by an opted-in
  trusted member.
  Third-party HTTP(S) images always require a click, even under everyone mode: they have no file
  attestation, disclose the client address, and may target loopback/private-network services.
- **A sealed local reconnect route is a narrow continuation of a completed direct join, not LAN
  discovery or durable presence.** After direct admission, the joining installation may seal at
  most two literal-IP TCP/QUIC routes to the named inviter that were actually used by an outbound
  Noise-authenticated connection. Inbound ephemeral ports, DNS, relay circuits, untried invite
  candidates, WebSocket wrappers, and admission-only helper/reply/switchboard contacts are not
  retained. Durable `ReconnectPolicy` provenance prevents a new non-direct admission with no route
  from later being mistaken for migration consent. On every later dial the sync layer reparses the
  terminal peer binding, requires exactly one current roster member's signed descriptor to claim
  that transport peer, rejects removed/replaced/ambiguous claims, and charges the shared
  process/server/peer/endpoint/prefix budget. Direct admission therefore makes one bounded
  best-effort PEX request while the authenticated connection is still live, so an immediate
  post-join snapshot normally contains the inviter descriptor required by reload.

  The route remains self-asserted at the device-to-transport boundary: Noise proves control of the
  transport key, while the member's signed descriptor is the only link from that key to a device.
  It can be stale or unreachable and does not prove the person is online. The private address is
  vault-sealed, never put into PEX/rendezvous/the UI, and reconnect dial logs expose only address
  family and transport shape. Broader same-LAN discovery remains absent because naive mDNS would
  disclose peer identities and create an unauthenticated address-injection surface. Pre-v3 records
  decode as `LegacyPending` and can learn this hint only after one successful overlap in a group
  with exactly one other member and one unique signed transport claim. That one-time migration
  accepts private or loopback routes only, so public rendezvous and ambiguous/helper-bearing groups
  do not become durable by accident. If both peers are already isolated with no old hint, no local
  state can reconstruct the missing address; one fresh invite, rendezvous/relay path, or future mDNS
  discovery is still needed.

- **An out-of-band member recovery code is narrow dial consent, not new membership or route
  proof.** The code is group-bound, signed by a device that must still be in the current MLS roster,
  expires after ten minutes, and contains at most four terminal-peer-bound literal-IP TCP/QUIC
  routes. DNS, relay, WebSocket, unsafe host classes, foreign peer ids, wrong groups, future,
  expired and self codes fail before dialing; accepted candidates spend the shared endpoint,
  prefix, peer, server and process scheduler. Private/loopback addresses are intentionally allowed
  because the user explicitly transports the code and same-LAN address churn is the recovery case.
  The signer's exact transport peer must also be the unique peer in that device's current signed
  record; re-signing another member's peer is rejected. The signer can still direct bounded attempts
  at sockets it does not own, and the code exposes its candidate addresses to the recipient.
  Applying seals only expiring permission before dialing and preserves the prior proven contact.
  No pasted route becomes durable until outbound Noise authentication proves the peer; bounded
  process-local success evidence survives a short close edge long enough for the vault worker, but
  still requires the pending/authorized peer and a unique current roster claim. Expiry is rechecked
  under the final write lock. A code cannot repair an expired first-contact reply or make two
  NAT-isolated devices reachable without some viable route.

## Protocol- / crypto-enforced (a modified client CANNOT bypass)

| Guarantee | Mechanism | Where |
|---|---|---|
| Message confidentiality / integrity | MLS group encryption; only current members hold the key | `catcoms-mls`, `catcoms-replication` |
| Membership authenticity | MLS commits; joins admitted only via a signed invite + committer | `catcoms-sync::serve_join` |
| **Invite admission** | The admitter must be the **named inviter** *and* an **authorized committer** (leaf rank ≤ `max_committer_rank`; at rank 0 that is exactly the owner). A non-committer member **cannot get anyone admitted**, even with a self-minted invite. | `catcoms-sync/src/lib.rs:3779-3795` |
| Owner identity | Owner = MLS **designated committer** = lowest leaf index; cryptographic, not stored | `catcoms-mls::designated_committer` |
| Admin-grant authenticity | Admin = an **owner-signed** capability (`owner_pubkey ‖ sig` over `domain ‖ len(group_id) ‖ group_id ‖ target_fp`), verified at read against the *current* owner's full device id. A modified client **cannot forge** an admin grant. | `catcoms-app::read_admins` |
| Member-removal authorization | Removal is **owner-only**: `request_remove` rejects a non-owner, and the committer ignores any inbound remove request whose requester isn't the owner (signature-verified, so a forged owner-claim fails). A modified member cannot get anyone removed. | `catcoms-sync` (on_remove_request gate + `request_remove` Unauthorized) |
| Forward secrecy on removal | A removal is a real MLS Remove commit → epoch advance + routing-secret rotation; the removed member is genuinely cut off | `catcoms-sync` removal path |
| Blob integrity | Content-addressed; served bytes are re-hashed against the requested CID before storing (no cache poisoning) | `catcoms-sync::request_blob` |
| File-at-rest encryption | Per-group file-wrap key; sealed at rest under the vault key | `catcoms-storage` (Phase 9h) |
| UI continuity and backup confidentiality | Drafts/read positions are vault-sealed and bounded; sealed records use destination-specific create-new siblings, file sync, rename and parent-directory sync on Unix, so concurrent record types cannot alias, pre-planted staging symlinks are rejected, and abrupt termination exposes a complete predecessor or replacement rather than a partial record. A failed post-rename directory flush is disclosed as committed-but-not-durable. Offline backup copies only the already-sealed vault tree without following links. Export creates another offline guessing target and exposes filesystem metadata; it does not weaken record encryption. Catastrophic filesystem/hardware failure remains outside the guarantee. | `catcoms-app::ServerStore`; desktop `create_backup` |
| Vault creation and secret rotation | A non-blocking OS-backed sibling lock serializes first creation and rewrap across processes; contention returns `VaultBusy` for retry instead of hanging behind a suspended process. The current wrapper is authenticated; the same root DEK is published through a unique create-new, file-synced staging sibling with rename and Unix directory sync, so concurrent app instances cannot return mismatched first-run DEKs or both report a conflicting rewrap. New/replacement secrets are capped at 4096 bytes. A v1 wrapper with a legacy 4097..65536-byte secret receives one bounded compatibility open and is atomically migrated to fixed-input v2; larger inputs are rejected. This intentionally loses downgrade compatibility with v1-only builds, not ciphertext confidentiality. Wrapper reads accept exactly 89 bytes and never allocate from a hostile file length. | `catcoms-storage::{open_or_create_vault,change_vault_passphrase}`; desktop `change_vault_secret` |
| Vault single-writer lifetime | `ServerStore::open` acquires a separate non-blocking OS session lock before unsealing and retains it until drop/process exit. A second desktop cannot start duplicate MLS, registry, invite-ledger or transport writers from the same snapshot; it receives `VaultBusy`. Normal exit and abort release the OS lock. Explicit UI lock keeps the native mount but closes IPC; re-unlock performs verify-only authentication against `vault.bin`, and a wrong secret cannot reopen the session. This is same-host installation exclusion, not distributed consensus or protection from malware with the user's OS authority. | `catcoms-storage::{acquire_vault_session,verify_vault_passphrase}`; `catcoms-app::ServerStore`; desktop `unlock` |
| Desktop explicit-lock IPC boundary | Every non-bootstrap Tauri command requires both a mounted vault and an open UI session. Lock atomically saves bounded continuity state then closes the command boundary; actor events are dropped, late frontend/native cache publications are exact-generation gated, and plaintext export publication holds the exact-generation commit guard while actors continue native background network/persistence work | desktop `require_unlocked_session`; `require_ui_session_generation`; `lock_session`; `forward_events` |
| Moderation-record attribution and field integrity | Each event/vote has a canonical group-bound Ed25519 signature; the reader verifies signer fingerprint and linked-device origin, so records cannot be altered or replayed into another server without detection | `catcoms-app::moderation` |
| Path traversal | Virtual file paths normalized (drops `.`/`..`/empty) so a path can't escape the share | `catcoms-app::normalize_path` |

## Honest-client / product-layer only (a modified client CAN bypass); the residual backlog

| # | Action | Honest-client gate | What a modified member could do | Severity | Status |
|---|---|---|---|---|---|
| ~~R1~~ | **Member-removal requests** |; | ~~The committer honored a removal request from any member without a role check.~~ **CLOSED:** removal is now owner-only at the protocol layer; `request_remove` rejects a non-owner, and the committer ignores any inbound remove request whose requester isn't the owner (verified by signature, so a forged owner-claim fails too). |; | **Closed**; `crates/catcoms-sync` (on_remove_request owner gate + request_remove Unauthorized) |
| R2 | **File deletion** | `Server::delete_file` gates on owner/admin role | A modified member could post a raw `FileIndex` delete op directly, unlisting any file. Low stakes; the content-addressed blob survives on every peer that holds it; nothing is destroyed. | **Low** | **Open**; close with the same committer-side role re-check, or accept (lowest stakes) |
| R3 | **Invite-minting permission** | `require_invite_permission` → `can_invite()` (Owner/Admin) gates `mint_invite` | A modified member can *mint* an invite token, **but it is useless**: admission is rank-gated (see the protocol table), so a non-committer can't admit the joiner. Not exploitable in the default single-committer config; the rank check backstops it. | **None today** (single-committer) / **Medium** if multi-committer is enabled | Backstopped by R-protocol; the role re-check makes it explicit for multi-committer |
| R4 | **Local role display** | `my_role()` drives which controls the UI shows | A modified client can paint itself as "admin/owner" **in its own UI**, but this grants **no real capability**; grants are owner-signed (unforgeable) and admission is rank-gated. Cosmetic only. | **Cosmetic** | Accepted; documented in-app |
| R5 | **Invite rate-limit / server policy** (planned) | An owner-set server-settings doc, respected by honest clients | A modified admin can ignore a mint rate-limit / expiry policy. It is a guardrail against *accidental* over-sharing by honest admins, **not** a control against a malicious admin. | **Soft guardrail** | Document the limitation in the UI when shipped |
| R6 | **Message edit / delete / react / pin** | `edit_message`/`delete_message` gate on `author == self` (own messages; delete also allows owner/admin moderation); `set_pin` gates on the owner/admin role; `toggle_reaction` keys the reaction by the caller's own fingerprint | A modified member could post a raw channel op editing/deleting/pinning **any** member's message, or forging a reaction under another member's fingerprint; the per-op inner signature signs the *delta*, not the semantic `author`/reactor/role (the same property that already lets a member forge a message's author on send). Low stakes; message content is not authenticated, by design. | **Low** | **Open**; accept (same posture as R2), or a later per-message author-binding hardening |
| R7 | **Moderation-log semantic authorization and completeness** | Product APIs gate warning/case creation to current owner/admin, resolution to owner, bind evidence to a currently visible message and same-target warning, and readers ignore invalid/unattributed/currently unauthorized records | A modified member can still submit raw Automerge changes that delete/overwrite moderation keys. A modified current admin can sign an invented evidence snapshot because the original message is not author-signed. Signatures make alteration and attribution failures detectable, but do **not** make the CRDT an append-only audit log or prove the signer's role at the historical instant. Votes never authorize removal; owner-only MLS removal remains enforced. | **Medium** (accountability), **None** for removal authority | **Open and disclosed**; historical role certificates plus a countersigned/hash-chained append-only log are the hardening path |

### Notes on the key residual (R1) and the "admin invites" entanglement

Reading the admission code surfaced a structural fact worth recording, because it reframes the
roadmap:

- **In the default single-committer model, only the owner can effectively invite *and* admit.**
  An admin can pass the `can_invite()` UI gate and mint a token, but when a joiner connects to
  that admin, `serve_join` rejects the admission (the admin is not an authorized committer at
  rank 0). So "admins can invite" is, today, a **UI affordance without a working end-to-end
  path**; not a security hole, but a missing feature.
- Making admin invites actually work means **connecting the roles model (Owner/Admin) to the
  committer/admission model (leaf rank)**; i.e. letting the owner-signed admin set be the
  authority for who may admit, which is a deliberate protocol decision (it interacts with the
  single-committer fork-freedom guarantee; multi-committer needs the staged fork-resolution
  path). The **committer-side role re-check ("option b")** is exactly the mechanism that makes
  admission *role*-based rather than *rank*-based, so it is the same primitive that (a) closes
  R1/R2 and (b) unlocks real admin invites.

## Hardening backlog (the fixes)

1. **Committer-side requester/inviter role re-check (option "b")**; ✅ **done for removal**
   (owner-only, see R1). Still **open for multi-committer invite admission (R3)**: before the
   committer admits via an invite in any `max_committer_rank ≥ 1` config, verify the *inviter*
   is Owner or Admin per the owner-signed roles doc and reject otherwise. The inviter's identity
   is already cryptographically recoverable from the invite (`inviter_device_id` + signature),
   and the roles doc is reachable at the admission layer; so it is feasible without changing
   the invite format. This is the same mechanism that would make admin invites functional.
2. **File-delete protocol gate (R2)**; optional; same re-check applied to `FileIndex` deletes,
   or accept the residual (lowest stakes).
3. **Replay-proof grant revocation**; ✅ **DONE + REVIEWED** (`design-grant-revocation.md`).
   Rather than a CRDT epoch/nonce, the authoritative admin set is kept **owner-local** (a
   persisted `admin_roster`); since only the owner admits (Option C), the admission gate
   (`inviter_is_authorized`) reads that local set, which a malicious member cannot write; so a
   demoted admin replaying or deleting its grant in the shared CRDT can no longer re-authorize
   itself. The CRDT now carries a single **owner-signed** `roster` value for display only
   (readers verify the owner's signature; a tampered copy is at worst cosmetic). Adversarial
   review: no blocking/should-fix findings. **Residual:** the guarantee rests on single-committer
   admission; under `max_committer_rank ≥ 1` a second committer would re-introduce the replay
   surface (it would need a per-reader high-water on the signed published roster). Do not enable
   concurrent committers. This **closes the GA gate** for admin invites (item 4).
4. **Make admin invites functional (R3)**; **IMPLEMENTED (slices 2a–2d) + REVIEWED; UI gated on
   item 3.** Design: Option C, "owner-serialized admin invites." An admin who wants to invite
   broadcasts a *signed Add-request* on the control topic (mirroring the R1 remove-request
   pattern); the **owner alone** runs the MLS Add after re-checking the inviter is Owner/Admin per
   the live roles doc. This keeps `max_committer_rank = 0` (single committer → **no fork**),
   reuses the most-tested membership code + the existing two-phase Welcome-push. Shipped slices:
   (0) `read_admins`/grant logic moved into `catcoms-sync` (live-doc gate, zero staleness) ✓;
   (1) the inviter-role re-check at admission ✓; (2) the `CTRL_ADD_REQUEST` op + `on_add_request`
   + the **Welcome-authentication chain** ✓; the load-bearing new crypto passed a focused
   adversarial review with **no blocking findings** (verified: no non-owner commit path; the
   no-substitution property holds; the admin re-signs the *identical* transcript only after
   verifying the owner's signature over it, and the joiner's `group_id` pin + MLS KeyPackage bind
   reject any substituted group; exactly-once admission across the hop). (3) actor/desktop wiring
   is the only remaining step and is **gated off in the UI until item 3** (see the residual).
   - *Rejected:* **Option A** (admins are committers / `max_committer_rank ≥ 1`); forces
     concurrent committers and the staged fork-resolution path's **I1 is still open**, so two
     admins admitting at once can permanently split the group. **Option B** (owner-admits-on-
     behalf via point-to-point forward); routing is a blocker without a new forward protocol +
     a rewritten Welcome trust anchor.
   - *Chosen variant: Option C + **offline Add-request queuing**.* The admin's signed
     Add-request sits on the control topic until the owner is next online; the joiner waits and
     is admitted when the owner finalizes. So an admin can create + hand out invites **with the
     owner offline** (admin-independent), and admission completes whenever the owner next syncs.
   - *Product implication:* admission finalizes **when the owner is next online**; admins are
     independent for minting/handing out invites, but the final MLS Add is owner-serialized so
     the group can't fork. Not a regression (today only the owner can admit at all). Owner-
     **never**-online concurrent admission is explicitly **out of scope**: it needs safe
     concurrent committers, i.e. closing **I1**, which is fundamental consensus (FLP/CAP); a
     multi-week-to-month redesign whose failure mode is a *permanent group split* (forward-
     secrecy/PCS defeated on the losing branch). Recommended against; do **not** enable
     `max_committer_rank ≥ 1`.
   - *Residual:* the re-check makes a *non-admin's* minted invite useless at the protocol layer,
     but a **demoted** admin can still replay their old grant op until item 3 (grant epoch/nonce)
     lands; so demotion is "current-doc, honest-client," not yet replay-proof.
5. **Invite rate-limit as server policy (R5)**; owner-set, honest-client-enforced; ship with
   the limitation stated in the UI.
6. **Moderation-log completeness (R7)**; design a monotonic, hash-linked event log with historical
   owner-signed role evidence and a protocol-side append/delete policy before calling the timeline
   a tamper-proof audit trail. Preserve the existing invariant that a tally is never executable
   authority and only the owner removal path can mutate MLS membership.

## How to use this document

- When you add a governance/policy action, add a row to the right table and state its
  enforcement layer honestly.
- When the UI implies a rule is enforced, make sure the rule is either in the protocol table or
  carries an in-app note that it is advisory (as the roles panel already does).
- When a residual is closed, move it up with the commit hash.
- When a Tauri command is added or removed, update `apps/desktop/src/tauri-command-security.ts`;
  the frontend suite checks the ledger against the native handler and every literal invocation.
