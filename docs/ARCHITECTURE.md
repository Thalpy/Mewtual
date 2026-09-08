# Mewtual; Architecture

This document is the in-repo source of truth for the design. It records the locked
decisions, the corrections that came out of an adversarial design review (run before any
code was written), the honest residual risks, and the phased build plan.

## 1. Locked decisions

| Area | Decision |
|------|----------|
| Stack | Rust core (shared `rlib`+`cdylib`) + Svelte 5 UI, packaged via **Tauri 2** to Linux/Windows/Android from one codebase. |
| Group crypto | **MLS (RFC 9420)** via `openmls`, ciphersuite `0x0003` (X25519 + ChaCha20-Poly1305 + SHA-256 + Ed25519), `PrivateMessage` wire format only. One MLS group == one server/connection. Per-**device** identity (a human with N devices = N leaves). |
| Channels | NOT separate groups; each channel/wiki/status/calendar/moderation document derives an independent key via the MLS exporter secret + a canonical, injective `(doc_type, doc_id)` context. |
| Delivery | Encrypted **CRDT documents** (`automerge`) synced P2P. Chat logs are append-oriented; policy documents are not assumed append-only without protocol enforcement. New Studio/index/reply/registry types use the owner-receipted bounded epoch-close protocol in [`design-epoch-close.md`](design-epoch-close.md); this is distinct from per-operation delivery receipts. A newly applied remote op queues an authenticated, connected-only delivery receipt for its exact document/change hash; causal descendant evidence remains the compatibility fallback. Delivery receipts prove delivery, never reading. |
| Networking | **rust-libp2p** (QUIC + TCP + WSS for the application mesh). Direct reachability uses stable ports plus best-effort **UPnP IGD, IPv4 PCP/NAT-PMP, and IPv6 PCP firewall pinholes**; zero-knowledge **circuit-relay v2 + DCUtR** hole-punching and authenticated rendezvous cover harder networks; **AutoNAT v2** performs scoped dial-back testing through explicitly enabled relay/rendezvous nodes. PCPv6 binds the exact global listener address to that interface's scoped default router, requests short leases for TCP and UDP/QUIC, and honors the router's assigned lifetime up to 24 hours. AutoNAT serving is experimental and off by default; its pre-socket guard enforces exact source/target matching, direct public-address shape, global/per-prefix/per-peer rate buckets and concurrency caps. Invites embed bootstrap multiaddrs. A 60-second `JoinReply` lets the inviter (or one explicitly authorized current-member helper) dial a joiner's validated public routes back. Separately, an opted-in current member can publish a two-minute signed **switchboard offer**; a fresh, explicitly labelled assisted invite may endorse up to three such members, and the joiner must consent before they are contacted after direct routes fail. A switchboard forwards only the admission exchange to the invite's named inviter and must catch up the exact MLS Add before becoming the joiner's first member path; it never signs/adopts the Welcome or becomes a general circuit relay. An already-admitted but isolated member can create a ten-minute, member-signed `MemberRecoveryCode` containing at most four direct literal-IP, terminal-peer-bound candidates. The recipient verifies its exact current device→transport binding and seals expiring consent before the bounded dial; only a later outbound Noise-authenticated route is promoted, without destroying the last proven contact on failure. |
| Live media | Calls and screen shares use WebRTC separately from the libp2p application mesh. A receiver advertises only its nearest 720p/1080p/1440p/2160p display bucket (with resize hysteresis); a screen sender parks each edge, applies per-peer resolution/bitrate/frame-rate limits, and only then attaches the screen track. Client-wide presets are exposed both in Settings and from the call-stage cog. Screen audio defaults off; the user may request the selected surface's audio or grant multiple independently chosen application/window sources, which are mixed locally into one audio sender and forgotten when sharing stops. Its 160-kbps-per-peer cap is reported as planned unless the WebView applies it. Permission-prompt results carry lifecycle leases so stop, leave, lock, mode changes, and competing requests stop late tracks rather than resurrecting capture. It reports the estimated aggregate mesh upload and prefers H.265/AV1/VP9 with compatible fallbacks when the WebView exposes them. Runtime stats, not preference order, are the source of truth for the negotiated codec. |
| Invites | Strictly **single-use, device-bound** (one device per invite); revocable/expirable. |
| Files | Content-addressed over **ciphertext**. New listings carry a group-bound device signature over author/name/path/file-ref so per-uploader local trust cannot be bypassed by rewriting the CRDT `author` scalar; unsigned legacy listings remain usable but cannot satisfy specific-uploader auto-load. Circulation expiry is metadata; automatic eviction is not wired. Explicit kept copies use a separate quota-reserved sealed local store. Index publication does not prove remote possession. Health checks authenticate storage seals/CIDs and decrypt file refs; repair may overwrite a corrupt local record only with authenticated, CID-valid peer bytes. |
| Moderation | One group-bound, independently signed moderation document per server. Warnings attest to bounded snapshots; kick votes are advisory; only the owner-only MLS removal path changes membership. The log is not yet protocol-enforced append-only (threat-model R7). |
| Local continuity & backup | Drafts/read positions are bounded and vault-sealed. Backup is an opaque, non-overwriting copy of a freshly snapshotted sealed vault. Secret changes atomically rewrap the same DEK; automated restore is not implied. |

## 2. Corrections from the adversarial review (must hold in the implementation)

Creative immutable blobs are independent of the P1 document lifecycle. The C0c native seam uses
the existing blob staging/store and authenticated fetch protocol, with PIX1 validation and bounded
reads/response decoding added at the relevant boundaries. Publication returns a real CID only after
promotion and flush; saving a Studio reference is a separate subsequent operation. P1 does not replace
Automerge or sync: it adds permission to retire their retained history into owner-receipted,
verifiable checkpoints with bounded recovery. Existing snapshots alone do not provide that.

Studio's first Rust schema layer (`catcoms_replication::studio`) decodes the closed IndexOp and
FlipnoteOp bodies, preserving stable element ids and the frontend's canonical JSON bytes.
Contextual decoding checks the whole P1 envelope and expected type/key; Index creation additionally
binds `created_by` to a caller-supplied, independently verified full author. This decoder grants
no membership, causal-mutation, storage or publication authority. The first read-only projection
is StudioIndex: flat immutable insertion/deletion records retain full provenance and mutable
title/expiry registers use Automerge's actual winner with all concurrent alternatives retained.
The smallest insertion op id wins a same-object collision; the first 64 live object ids are
visible, with explicit overflow and deleted content kept for later typed recovery. Every live
root value is checked, including losing headers; primitive and value-byte limits bound the reader.
The art-only Flipnote frame projection now likewise retains insertion/replacement/deletion
evidence. Stable insertion-node origins and a right-origin forest preserve sequential placement
and same-gap concurrent ordering without rewiring losing/deleted anchors. All live alternatives
remain available even when a frame is deleted or beyond the 999-frame / 8 MiB playable prefix.
Sound/score/export state rejects until its projection support lands. Neither reader authenticates
its records. A separate Index delta validator checks canonical record/actor
binding, the sender's dependency-frontier target existence, immutable headers/evidence and exact
same-property predecessors for mutable registers. It is a pure semantic callback: P1 must still
authenticate membership/server/physical scope and preflight the exact checkpoint. The art/frame
callback supplies corresponding checks, deriving left/right insertion origins
from the sender's full causal projection, including hidden nodes. Its private historical reader
uses one frontier for keys, all values and register winners; the public current reader retains
its original pristine-history rule. `StudioTarget` combines these callbacks with existing P1
gated edit/ingest and exact next-checkpoint AND complete recovery preflight. It supports epoch
zero and verified typed seeds; it does not own durable intents, storage, publication or settlement.
The compact baseline is one immutable `_studio/seed` bytes property alongside the original
headers, not a new operation log. It preserves selected values/authorship and up to four values
per conflict field across 1024 fields. Frame origins normalize to the live playable chain and
are explicitly marked `checkpoint`; all original/hidden/losing evidence and current operation
bodies remain in typed recovery. Ordinary post-seed registers override fallback values, with
same-property predecessors only; inherited provenance ids are not reusable operations/markers.
No user operations, markers or tombstones enter the compact seed. Generic P1 hash/receipt
verification and canonical typed rebuilding both run before installation. Recovery uses the
existing bounded envelope with a complete typed payload, not the lossy generic summary arrays.
Historical reads still repeat Automerge clock work; boundedness is not measured production
latency. These projection helpers themselves provide no actor/native or user-interface binding.
`StudioEpoch` now privately owns the Index/art document, signed log, gate, opening receipt and
receipt book. Its bounded vault-only restart format revalidates signatures, causal mutations,
typed seed and gate/log coherence; removed authors remain historical evidence, not current edit
authority. The `ServerStore` adapter saves the exact intent before this whole unit and returns
prepared ciphertext only after both durability barriers. Exact retained-envelope retries flush
unchanged files, including at the ordinary content ceiling. Receipt sealing retains the complete
source and persists faults; it does not settle, prune or install a replacement.
Opt-in five-family inventory adds Studio records and temporaries to the existing storage/intent
budgets. A mount-local generation rejects stale scans and duplicate Studio budget handles. The
coordinator must still exclude interleaved raw registry/recovery/owner writes; the token is not
a global filesystem transaction or blob-retention guarantee.
Explicit Index/art Save/Load now runs through `ServerActor::studio_begin` and five native commands.
The bounded queue carries no vault guard: the actor advertises Ready before native attempts all
locks fail-fast. The transferred lease retains mounted vault, numeric-server persistence, UI
commit and exact registry-incarnation guards. One blocking worker owns the sole live Server
(not a cloned MLS/device) plus that lease; invoke/actor cancellation cannot release its custody
mid-write. Guards drop before reply/event awaits. A worker panic stops the actor rather than
resuming an absent or stale Server. Live channel/member and complete-envelope checks precede
mutation I/O; current MLS/device snapshot persistence precedes the intent/source write.
Local frame references require exact held, validated 192x144 PIX bytes promoted and flushed in
the same vault namespace. Create writes the object before the Index and is exact-retryable, not
atomic across files; a different nonce cannot rename an existing object through Create.
Native projections retain conflict/deletion evidence and three-state expiry. Local update events
invalidate views, while responses explicitly label local publication and provisional edits.
Current Create targets epoch zero. Automatic sync/settlement and the rotated-index installer
remain integration work, with dense-source latency still unqualified. UI is unchanged.
Persistent blob handles now share one mount-local deletion guard outside the kept-copy adapter.
The derived reference union is keyed by full group bytes (including numeric-server aliases).
Studio source/intent/recovery writes add holds before I/O; only a complete, generation-current,
exclusive five-family reference scan replaces them. Unknown/corrupt/unsupported/partial state
refuses held-blob deletion. A cheap bounded absence-only filename check can initialize an empty
new mount; a restored P1 mount starts Unknown. Studio access refreshes before pre-holding new
PIX CIDs, then verifies/promotes bytes. Its later budget scan is deliberately reference-neutral.
Source holds include the verified seed-only projection as well as current projection and signed
operations: successor edits can hide a seed's replacement pixels. Pending intents and BOTH
retained and staged typed recovery also hold pixels, including deleted/superseded versions.
The guard stays locked through synchronous unlink; dropping the store revokes old blob handles.
There is no durable pin ledger or expiry engine. Full scans have existing work rails plus a
65,536-reference cache rail; overflow holds bytes rather than truncate. Existing file unlisting
and upload cleanup use the guarded delete, but still do not promise freed disk space. Explicit
kept-copy release and unreferenced staging cleanup retain their separate ownership semantics.
Future sound/export/doodle record support must extend enumeration before enabling reclamation
for those formats; current unsupported records cannot be interpreted as an empty hold set.
Expiry mirrors FileExpiry's absent/null/timestamp states, not the fixture's numeric-only view.
Jam descriptors are bounded/validated and retain their existing declaration-order identity hash
even inside the sorted-key Studio body. No audio renderer or game/avatar work is added.

P1 owner authority now also has independent local tenure evidence in the MLS synchronizer.
Locally founded epoch zero is known; Welcome joins and legacy snapshots are Unknown. Every
applied owner-changing MLS transition records its resulting epoch, including Adds into recycled
low leaf slots. Same-owner commits preserve Unknown rather than inventing a start. A shared
synchronous mutation seam observes the actual group before propagating a helper result, including
an error after a successful merge. The observation is saved in the same strict snapshot as MLS,
not in a separately advanced sidecar. A received receipt never establishes its own tenure.
This supplies local evidence only: fresh owner proofs require the publication barrier described
below. An upgraded or newly joined owner with Unknown tenure may remain
unable to authorize rotation until independent evidence or a witnessed transition is available.

Explicit owner registry rotation now uses that same durable MLS/tenure snapshot permit. Under
exclusive sync/store borrows it flushes the checked source, checks the owner inventory, and either
resumes the exact saved decision or derives a new close from all current heads. More than 64 heads
refuses without truncation. The actual authenticated closure must meet the lower/upper budgets and
typed checkpoint validation before signing; a live projection alone is insufficient. First-of-tenure
inheritance comes from the installed opening seed, and later decisions repeat the journal baseline.
The owner vault record saves the exact close alongside the selected receipt in one atomic replacement,
so a crash before sealing cannot change the decision when more Open edits arrive. Such later edits
enter recovery through the existing adjacent installer. Legacy pending receipts without a saved close
hold explicitly. Installation returns publication-pending status and does not mark the receipt
published. Kind-21 serving now records exact completion only after a current-owner proof is
accepted by the local reply-forwarding channel. Its private one-shot handoff is consumed under
the same synchronous Server/store borrow with fresh authority and accounted journal persistence.
This is not transport-driver admission or peer delivery. A crash or write failure after handoff
can require exact republication; uncertain disk state blocks accounting until reconciliation.
Automatic scheduling and durable repair remain open.

Keyed registry receipt-head discovery is now a cooperative authenticated kind-21 exchange.
It registers logical buckets, not concrete epochs, and returns provisional hints or a one-shot
current-owner selection proof. Explicit local preparation saves the whole-server MLS/tenure
snapshot and mints a runtime/epoch/mount-bound permit; remote queries never trigger that large
legacy serialization. A proof additionally requires complete inventory agreement, a nonfault
saved registry, and exact equality with the pending-preferred owner journal. Source flush and
journal re-save precede signing. Disagreement/stale preparation remains a hint; lost indexed
files, faults and corruption refuse. A selected receipt is not proof its seed is available or
verified. Expected-seed fetching uses additive kind 22. Explicit recovery-first registry
installation is now connected to that scoped selection; automatic actor scheduling remains unwired.

Checkpoint fetching keeps discovery provenance rather than converting public receipt fields into
authority. The private selection is minted only while checking the fresh kind-21 owner response;
runtime, MLS, requester, owner and a per-bucket supersession token accompany it. Four non-Clone
handles retain at most one verified 2-MiB seed each for bounded attempts. Any newer authenticated
owner selection for that bucket, MLS transition, runtime replacement or receiver timeout revokes
use without refunding memory still held by a handle. This is not a current-head lease.

The independently proven seed provider may be any current member. It serves the installed
opening seed from a checked, inventory-matched vault unit, not the latest receipt's prospective
successor; during Closing the latter is normally unavailable. Fault refuses. Kind 22 binds full
identities, current MLS, actual transports and the exact query/answer. Raw seeds use the existing
512-byte to 1-MiB padding ladder inside group AEAD; above that ceiling the encoded size remains
visible. The receipt's exact Automerge hash is checked before parsing, then the canonical registry
schema. Fetching never writes the receiver's vault, retires intents or replaces provisional work.
The registry core has explicit checkpoint-adoption state: a distant selected receipt seals
the entire held source, with no pruning, and a typed plan preserves the whole previous version.
Retargets keep the same content-derived recovery id; seed-only and terminal-epoch versions count.
The outer restart v2 / adoption-only receipt-book v3 keep nonadjacent selections separate from
ordinary adjacent settlement. Opening/prior-target equivocation is checked before stale filtering.
Fault is a successful typed outcome which the store transaction saves before seed or recovery work;
bad seed or recovery must not suppress that evidence. Constructing a separate successor
does not install it, retire intents, or provide a transferable network-authority permit.

`Server::install_registry_seed_step` now performs this explicit newcomer transaction under exclusive
sync/store borrows. It rechecks runtime, MLS, full local identity, owner/tenure, superseding selection,
receiver-clock expiry and physical mount/server at entry. The accounted store saves the full source
Closing (or Fault) even before a seed is available, then validates and saves whole-source typed
recovery before atomically selecting the successor. Invalid inventory or uncertain I/O fails closed;
an indexed source cannot be replaced by invented epoch zero. Recovery warnings keep their original
ids/deadlines across retargets and restart. Resuming after expiry requires fresh discovery, not a
receipt reconstructed into a permit. Adoption retires no intents; exact installation retries flush
the actual successor and preserve later edits. Old concrete watches/queued pages are not rebound:
the caller explicitly watches the installed epoch before normal paged catch-up. This is cooperative
registry integration, not an actor worker, current-head lease or universal progress at full quota.

The registry's adjacent, locally proved checkpoint transaction orders durable barriers as source flush,
typed recovery, included-only intent retirement, then atomic successor selection. Until selection
the complete Closing source remains the restart proof; afterward the verified seed and preserved
receipt book do. Exact retries flush the actual successor without replacing newer work. Local
Save requests carry a concrete epoch id so retired intents cannot silently reappear on retry.
Delayed opening-receipt conflicts fault even a successor already sealed by a newer receipt,
preserving both accepted content and high-water evidence. This is a tested backend adapter,
not live actor/transport orchestration. A bounded replay step now selects a saved intent by id,
checks its actual local author/current Open epoch and all typed recovery evidence, then uses the
existing durability path. New authoring is held on deletion or a superseding pointer hint; an
exact current-log retry reseals unchanged so failed-flush deletions remain retryable. Holds never
retire intents. Deletion screening is conservative for stable keys and best-effort after bounded
recovery eviction. A cooperative replay pass now snapshots one author's bounded id set and paces
one attempt at a time. It waits for an exact local submission acknowledgement, pauses failures at
the same id, and never retires intents on traversal/submission. The cooperative Server sender now
binds that pass to the exact sync instance and holds exclusive sync/store borrows from checked
preparation through one-shot dispatch. Actual full local identity, MLS epoch and current blinded
routing are checked before sending. Only Submitted advances; Duplicate, errors, cancellation and
unwind preserve the saved id for a fresh reseal without resetting pacing. This is not an autonomous
worker: aggregate scheduling, native lifecycle cancellation, catch-up, settlement-wide
capacity headroom and discovery remain to be integrated. No universal progress at full quota is
claimed yet.

The transport offers one-shot publication, now used by that cooperative backend sender. Legacy
`publish` acknowledges actor enqueueing and can hold ciphertext for later retries. `publish_once`
instead waits for one driver attempt and never enters that application retry queue. Its bounded
commands retain their capacity after caller cancellation until drained. Cancellation observed at
the last driver check suppresses an attempt, but cannot retract work admitted before it. Normal
gossip caches/handler queues may retain bytes even after some refusal results; cache duplicates
and local submissions are never delivery proofs. The sender owns saved-id retries and known-state
checks, not native UI-lock policy, driver deadlines or live actor wakeups. These additive seams do
not change existing chat publication or wire/persistence formats.

Managed registry gossip now has an opt-in receive path separate from the legacy document map.
A synchronous watch captures the checked current concrete epoch (or deterministic epoch zero
when absent), numeric server, physical vault mount and fresh sync/watch generation. Network ticks
authenticate into a 16-packet queue; an explicit Server drain persists at most one packet through
the existing typed gate and accounted store. Only the returned Admission describes the saved
outcome; queuing is not acceptance, and no delivery receipt or legacy accepted-op counter is earned.
Current local/full author membership, MLS scope, bucket and blinded topic are checked; delayed
packets recheck authority at drain. Same-id watch replacement revokes old queued work. Current-MLS
only reception and drop-on-cap/failure require author retry or future catch-up, not silent recovery.
Global pre-auth and per-full-author/document limits bound work; the shared pre-auth allowance does
not promise per-peer fairness. Subscription reconciliation retains a single uncertain topic across
an interrupted subscribe/unsubscribe and establishes it as unsubscribed before retry, including
after the same topic was rewatched. Revoking a watch needs only its exact generation, so an old
mount can discard its own queued traffic without gaining permission to ingest. Desired
watch installation itself never awaits. Actor/native ownership, automatic scheduling of the
cooperative discovery/catch-up/installation paths remain next.

Registry catch-up has a cooperative read-only page provider over checked vault history.
Vault reconstruction checks dependency/duplicate presence through Automerge's applied change-graph
metadata, without reconstructing raw operations merely to test membership. It avoids historical
visibility scans for a change whose dependencies exactly equal the entire current frontier.
The trusted restore loop derives that equality after signature,
actor, scope and dependency checks, with no mutation before semantic validation. Other branches
retain historical reads, and live edit/ingest never asserts this optimization. The exact-frontier
path also skips the semantic validator's already-proven dependency-presence predicate. This changes no
wire/snapshot format, membership rule, cap or request deadline. `P1-PERFORMANCE.md` records measurements;
off-executor scheduling and source reuse still need lifecycle/version fences before automatic use.
Its HMAC cursor freezes the provider's dependency-complete accepted-log prefix
and advances by position, so appends and large head sets cannot force the same prefix forever.
Every page freshly seals at most 32 operations / 512 KiB of framed bytes; byte-identical reloads
preserve cursors, changed prefixes and provider restarts do not. Source seeds must already be
verified by a conforming requester; missing removed-author history explicitly needs historical
authorization rather than weakening live operation admission. Already emitted cursor history
counts as claimed held dependencies. Requester claims and prefix completion prove no remote
possession, finality or currency. The app binds provider state to the exact runtime/mount and
validates bounded request/MAC/current membership before source I/O. Kind 20 now routes authenticated
page requests into a bounded opt-in queue, and a mount/watch-bound app drain serves saved history
under independent requester and aggregate source-read limits. Both transport endpoints are bound
in the exchange; a client needs a current bound-member proof before disclosing private identifiers.
Its four outbound permits follow actual transport termination after caller cancellation. Replies
are unadmitted claims, not currency or delivery evidence. A bounded receiver now derives the
frontier from checked durable state, fetches without borrowing the vault, and advances only after
one accounted all-or-none page save. Independent unknown heads permit one empty-head fallback;
verified seed, provider and charged limits stay fixed. Four watch-bound passes each retain at most
one page and expire on the receiver's monotonic clock. Duplicate and terminal-empty pages still
cross the storage/inventory barrier; a receipt seal or MLS advance cannot turn them into stale
success. Automatic actor/native scheduling remains; the adapters
do not move vault ownership into sync or change the legacy document map.

The naive "one group, every device commits, replay old ciphertext to latecomers" design
is broken. The load-bearing fixes:

1. **Proposal/commit split.** Concurrent MLS commits fork the group. Devices replicate
   *proposals* via the CRDT; a single designated committer per epoch packs them into one
   commit; deterministic fork-resolution (lowest `commit_hash`) + loser re-issues.
2. **Snapshot-only catch-up.** Never replicate raw old-epoch frames between members (it
   contradicts forward secrecy). Latecomers receive a self-contained Automerge snapshot
   re-sealed under the current epoch. `max_past_epochs` only covers in-session reordering.
3. **Inner per-op signature.** Every `LogEntry` is signed by the author's Ed25519 leaf
   over `(doc_id, deps, mls_epoch, payload, author_device)`, verified independently of MLS
   sealing; so re-sealing is a pure transport re-wrap and history cannot be forged/omitted.
4. **Invites bound in MLS.** The invitee's KeyPackage carries a `(GroupId, invite_nonce)`
   extension validated at `Add`; the leaf is reserved in the InviteLedger before commit.
   Stops cross-group KeyPackage replay and partition double-claim.
5. **Routing topic decoupled from content epoch.** Gossipsub topic + rendezvous namespace
   derive from a **separate metadata exporter label**, rotating only on member *removal*
   (with grandfathering); not on every commit.
6. **File crypto.** Per-file wrap nonce (no nonce reuse); re-wrap content keys under each
   new epoch so a removed member cannot lazily fetch post-removal files. Already-fetched
   files cannot be un-shared (documented limit).
7. **One key hierarchy.** Platform-wrapped root → one DEK → HKDF subkeys (SQLCipher DB key,
   openmls value sealing, blob-at-rest). On Android, **two tiers**: a non-auth-bound
   SERVICE key (background sync of ciphertext) + an auth-bound CONTENT key (decrypt
   plaintext for display). `SecureKeyStore` reports a tier enum; a downgrade forces a
   passphrase before the store opens.
8. **Also:** no device key ever leaves a device (multi-device is origin-rooted
   certificates at chain depth 1; `design-multi-device.md` v2; with origin-signed
   revocation; the earlier account-key cross-cert chain design was superseded and its
   module deleted); blob-fetch padding/quantization + per-session outer
   re-encryption; eclipse resistance (≥2 rendezvous + member peer-exchange + roster-size
   check + cached addresses); decorrelated cache eviction (jitter) + holder liveness probe
   + mandatory archive-pin role; bounded deterministic owner-receipted checkpoints for P1
   document types (with provisional edits and two-version recovery, not an all-acked consensus
   claim); resumable chunked anti-entropy under relay caps; panic-revoke (any
   sibling device removes another) and `mint_invite` gated behind the auth-bound key;
   per-server "relay-only / hide my IP" mode; pure-Rust crypto on the hot path;
   metadata-index aging/re-keying.
9. **Discovery egress is peer-bound and process-bounded.** Every peer/invite/switchboard/companion
   grant route
   accepted for dialing uses one canonical supported multiaddr grammar with a non-zero TCP or
   UDP/QUIC socket and a terminal `/p2p/<PeerId>` matching the signed/discovered transport
   identity. Untrusted records use IP literals; DNS and dangerous local/private/link-local,
   multicast and transitional ranges fail closed outside the explicitly local invite case. The
   sync classifier deliberately retains non-routed documentation/benchmark literals as
   deterministic test stand-ins; those can only consume bounded retry tokens. `DiscoveryPolicy`
   charges addresses rather than peers, then
   one desktop-owned `EndpointDialScheduler` applies monotonic per-process, per-server, canonical
   Phase-0-peer, attempt, and IPv4 `/24`/IPv6 `/48` caps before submission. The parser embeds that
   peer principal in each opaque endpoint so cache, rendezvous, and pre-join callers cannot select
   different accounting identities. A direct attempt is keyed by its physical socket, excluding
   the claimed terminal id; a relayed attempt is keyed as the authenticated relay/target circuit,
   while the relay's outer host remains bounded by the prefix and process caps. PeerId/sequence
   rotation therefore cannot reset a direct socket, prefix, server, or process bucket. The transport
   refuses a dial command with no terminal peer rather than falling back to an address-only socket
   dial. Two-way reply retries use the same scheduler for each new socket pass; their proof request
   may continue only over a connection that is live when the network actor handles it, without
   consulting the ordinary recent-peer redial cache.

## 3. Honest residual risks

- **Metadata** is the dominant weakness: who-talks-to-whom, timing, group sizes, and; once
  two members hole-punch; each other's IP, are partly observable. Mitigated, not eliminated.
  AutoNAT additionally reveals a candidate address and probe timing to its configured observer;
  a positive is scoped to that server/address/moment and is not a universal reachability claim.
  No nation-state-grade metadata protection is promised.
- Automatic router mapping and IPv6 firewall pinholes intentionally expose the stable TCP and UDP/QUIC libp2p listeners to
  the internet when the local gateway grants a lease. Noise still authenticates/encrypts libp2p
  sessions and connection limits cap strangers, but a hostile/local gateway can deny mapping or
  report an unusable address. PCPv6 may also disclose a device-specific/privacy address, and a
  granted pinhole does not prove the host or upstream firewall permits traffic. A mapping is
  therefore only a candidate until AutoNAT calls that exact address back.
- Two-way reply codes and member switchboards improve signalling/routing, but cannot create a path
  from nothing. The first two mutually unreachable users still need a public IPv6/manual/router
  mapping, a configured relay, or a reachable third party. Reply punching requires both apps'
  60-second sessions to overlap and is primarily useful for QUIC; TCP simultaneous-open is not
  claimed. Switchboard offers disclose the helper's stable identities/candidate addresses to the
  invite recipient and disclose the joiner's IP/timing to the helper. Signed candidates are not
  proof of address ownership or reachability.
- Post-join discovery is self-healing only while some route survives: the desktop polls a bounded
  set of peers with authenticated PEX, retries cached/current signed address epochs with monotonic
  exponential backoff and jitter, and resets that delay on a new signed epoch or connection
  lifecycle. Native route/interface notifications trigger a debounced re-sample of the kernel's
  route-selected IPv4/IPv6 sources, while the roughly-minute pass remains a fallback. A changed
  sample republishes one address epoch; exact route ownership prevents raw-interface removal from
  withdrawing an identical mapping/manual/relay route. It intentionally does not merge
  withdrawn public IPs forever because an ISP can reassign them; a newer zero-route descriptor
  also removes the prior sealed cache row, so restart cannot resurrect it. Route signatures and matching
  peer ids still do not prove ownership of an IP/port before the bounded first packet is sent.
  Scheduler counters are transient and reset with the process. The scheduler owns the one injected
  monotonic clock used for both reservation and commit. A generation-bound, non-cloneable permit
  moves into the network actor with each discovery dial command; commit rechecks the window deadline
  under the scheduler lock, so queued old-window work cannot start merely because no later
  reservation happened to roll the generation. Actor-side duplicate, already-connected, or
  already-dialling suppression drops/refunds the permit before commit. The peer-wide check covers
  member dials later reclassified as infrastructure; constructor TCP/QUIC routes for one peer are
  grouped into one known-peer address race and enter the pending ledger by exact libp2p
  `ConnectionId`. Constructor peers are protected from remote eviction but are not thereby
  classified as infrastructure, so an inviter reached over a bootstrap/relay route may still race a
  newly learned direct member address. Pending infrastructure attempts are released only by the
  matching connection/error id; an unrelated inbound connection cannot clear them. The actor commits only immediately before the endpoint enters the
  pending/socket-start path. Caller cancellation after enqueue therefore cannot refund work the actor
  will still perform, and an old-window command cannot decrement a replacement window. A failure
  after commit is conservatively spent even if libp2p rejects before a socket completes. A relay circuit
  has its own attempt key so unrelated targets at one relay do not starve each other, but the shared
  outer relay socket is not separately leased at the exact-socket scope; it is bounded only by the
  relay-host prefix and process caps. Libp2p's pending-outgoing cap is still per swarm, so this slice
  does not yet provide a process-wide in-flight/concurrency lease. A fully isolated device whose
  current address is unknown to every peer still needs
  out-of-band signalling, rendezvous/relay infrastructure, or a reachable member; swarm sampling
  cannot manufacture a route from no contact.
- Connected current members now provide a bounded baseline repair control plane. A node pushes at
  most two session-proven active members an authenticated probe for one exact descriptor; each
  helper answers later with a separately authenticated, exact-attempt-bound result. One positive
  observation may queue a short reciprocal request A→C→B; every hop is a connected-only
  asynchronous push, and B revalidates current roster, exact descriptor hashes/sequences, helper
  signature, expiry, replay and rate limits before submitting
  at most one IPv4 and one IPv6 direct route through the shared endpoint scheduler. Two signed
  negatives are suspicion only. Helpers never dial during a probe and never carry application
  traffic; switchboards remain opt-in admission-only, while general circuit relay hosting remains
  a separate opt-in role. HyParView/CYCLON-like active/passive selection and local age/source
  metadata improve which peers are tried but cannot repair a partition with no surviving edge.
- Pairwise route evidence is a bounded, session-only refinement of aggregate transport liveness.
  Libp2p reports a sorted/deduplicated IPv4/IPv6/DNS/memory + TCP/QUIC/WebSocket/circuit snapshot
  after connection edges, including relay-to-direct DCUtR upgrades and partial closes. The sync
  layer accepts detail only while `PeerConnected` remains live, caps custom transports again,
  retains historical success for at most 24 monotonic hours, and never serializes it. Admission
  paths that temporarily inspect pushed proof/Welcome requests coalesce any lifecycle
  edges they dequeue into a bounded final-state handoff, which the new sync owner adopts once;
  ordinary pre-owner connection waits use a non-consuming watch. Connectivity exposes typed
  claimed-peer health/actions, refreshes on path changes even when aggregate presence
  is unchanged (plus a bounded visible-view refresh for time-derived expiry), and says “no claimed
  path,” never “offline.” Dial counters describe scheduler submissions/cooldown, not
  unobserved per-address failures, and IPv6 candidate shape is not presented as an outbound-route
  test. A
  signed peer record is still a self-asserted device-to-transport binding, so this evidence is not
  proof that the member controls that live transport key or is reachable from another network.
  Reciprocal repair is intentionally not a dual-key ownership proof and does not strengthen that
  binding.
- A **fully compromised device** exposes its current keys and plaintext; PCS only heals
  *after* the device is removed.
- **Already-fetched files cannot be un-shared.**

## 4. Engineering seams (every layer is written against these)

- **`Clock`** (in `catcoms-rt`): all time flows through it; no layer calls the OS clock
  directly. Enforced by `scripts/check-no-ambient.sh` in CI.
- **`MeshTransport`** (in `catcoms-rt`): pub/sub + addressed request/response. Node logic
  runs over the deterministic in-memory `MemNetwork` in tests and over rust-libp2p in
  production, unchanged.
- **Canonical wire format** (`catcoms-wire`): length-prefixed, fixed-width, injective
  encoding; the basis of collision-free key-derivation contexts.

## 4a. Network join handshake (6c)

A fresh device joins over the wire by sending its invite-bound KeyPackage to the
inviter (request/response); the inviter validates and returns a Welcome. Adversarial
review (run before commit) hardened it:

- The joiner's KeyPackage init key is **not secret**, so a malicious inviter/relay
  could otherwise add it to a group *they* control and return a valid Welcome. The
  invite therefore carries the **inviter's public key**; the joiner authenticates
  the invite (`verify_self`), the admitter **signs the Welcome**, and the joiner
  verifies that signature against the invite's inviter and re-checks `group_id`.
- Only the invite's **named inviter admits** over the network (so the joiner can
  authenticate the response). `serve_join` runs cheap invite checks *before* the
  expensive KeyPackage validation, and oversized control requests are dropped.

## 4b. Membership commit propagation (6d-1a)

The Add commit (previously discarded) is now captured and fanned out on a per-group
**control topic**; every member applies it via `process_incoming` and advances to
the same epoch; so a multi-member join converges and a non-admitting member can
decrypt the new joiner's ops. A design+adversarial-review pass (verified against
the openmls 0.8.1 source) showed the "safe by construction" claim was only *assumed*,
so safety is **enforced**: only the **designated committer** (lowest leaf index) may
admit, which prevents concurrent commits from forking the epoch chain. Tested with a
3-member join + a non-committer-admit rejection.

## 4c. Missed-commit recovery + past-epoch key window (6d-1b)

A member that misses a control-topic commit (unreliable delivery / brief offline)
self-heals: an out-of-order commit is buffered (`pending_commits`), a new
`KIND_COMMIT_CATCHUP` request fetches the gap from any member's bounded `commit_log`,
and the missed MLS commits are **replayed in epoch order** through `process_incoming`.
Every retained buffer/queue is hard-bounded so an untrusted peer cannot force
unbounded allocation; a forged future commit is gap- and size-capped and fails MLS
verification at apply time (its only residual cost is one deduped catch-up request).
Separately, a bounded **past-epoch channel-key window** (`snapshot_epoch_keys` before
each advance → `Zeroizing` keys, evicted past `max_past_epochs`) lets an op sealed
just before an epoch boundary still decrypt (`ingest_with_key`, inner signature still
verified) instead of being dropped as `EpochUnavailable`; deeper gaps fall back to
auto-queued document/commit catch-up. Peer discovery is by remembering inbound
`Gossip.from`/`Request.from` (no `DeviceId→PeerId` directory yet).

An adversarial review (background `Workflow`) hardened this before commit. The
load-bearing fix: the **catch-up serve endpoints are members-only**. A requester
proves current membership by signing `("catcoms/catchup-auth/v1" ‖ group_id ‖ kind ‖
body ‖ requester_pubkey ‖ timestamp)` with its MLS leaf key; the server serves only
if that key content-addresses a current member, the timestamp is fresh, and the
signature verifies; so an outsider cannot harvest a group's id, member device ids,
or history from these endpoints. (Residual: within-freshness-window replay of a
captured signed request, closed by the Noise transport in production; a server nonce
challenge or authenticated-peer binding is the full fix, with 6e.) Also folded in:
hard response-size bounds on the *serving* side, `committer_device` validated against
the designated committer on the inbound apply path, and explicit caps on every
recovery buffer/queue.

**Still deferred, with the data model already in place (no rewrite):**
- **6d-2**; concurrent-commit fork resolution + the full RFC 9420 proposal/commit
  split (designated committer packs replicated proposals; deterministic lowest-hash
  tie-break; openmls `clear_pending_commit` rollback / `fork_resolution` heal),
  plus the replicated InviteLedger (single-use across members) and joiner-bound
  nonces. Until then network admission is single-committer only.
- Per-peer rate limiting / off-actor offload of join work.

## 4d. Product operations and moderation plane

The desktop keeps operational observations separate from protocol claims. **Storage health** walks
the current file index and validates every manifest and referenced chunk through the storage seal,
CID and file-layer key. One report per server is cached for the process session and augmented with a
deduplicated category/pin/largest-file inventory; category local-byte totals remain estimates, while
the verified ciphertext total is exact. **Repair** is an explicit network action: it re-fetches only
missing or unreadable content from authenticated members, re-runs verification, and replaces that
cache. Upload deduplication also requires complete locally authenticated possession, including
whole-file hashing. A metadata-only re-upload retains fresh staged ciphertext and publishes an
attested repair; only an exact verified local-device listing can be replaced. Other uploaders'
attestations remain intact. Downloads and media share a bounded resolver for at most four encrypted
manifests with identical ordered plaintext chunk identities, lengths and MIME fields. Media cache
identity binds the complete sorted set; every alternative is independently authenticated before
use, with local alternatives tried before any network wait. **Connectivity assistant**
reports the live peer/path evidence already available to the node; it is diagnostic and never a
proof that every remote member or future network path is reachable.

Moderation history lives in `DocType::Moderation` (stable tag 14, document id 0), not in a chat
channel. Each event/vote has a canonical Ed25519 signature binding its semantic fields and group id.
The app also binds the signer device to its certified member origin and interprets authority through
the current owner-signed role state. Warning evidence therefore survives a live message edit/delete
and is attributable to the moderator who observed it. It does not retroactively authenticate the
message author, prove historical role state, or prevent a modified member from deleting a CRDT root
entry; those are the explicit R7 residual. A vote can never invoke membership mutation. The owner
must resolve a case, and removal reuses the protocol-enforced MLS Remove flow.

Frontend continuity state (currently composer drafts and per-channel read marks) is stored as a
bounded vault-sealed record rather than browser plaintext. An offline backup first snapshots live
actors and persists the registry, then copies the already-sealed vault tree to a new destination
without following links or overwriting an existing backup. Restore remains deliberately staged work:
it needs a locked-vault import path, full verification, atomic swap and rollback before it can be
safe to expose. Export creates another permanent offline guessing surface and leaves filesystem
metadata visible. Changing the live secret atomically rewrites only the DEK wrapper; it cannot revoke
an older export and never rotates the server/blob encryption keys.

Normal window close is a native-owned lock/snapshot/destroy transaction. Close attempts serialize
inside `AppState`; once a continuity failure has been returned, another already-queued caller must
also defer until a later request explicitly acknowledges losing that latest screen snapshot. This
prevents duplicate WebView calls from replacing the failure with a newer success and destroying the
only surface capable of warning the user. The decision is bound to the close request's own native
generation because another lock caller may acquire the shared commit mutex first and consume that
newer snapshot. Plaintext jam-sheet export uses the same exact-generation commit guard and holds it
through both Downloads publication and OS reveal.

Bounded whole-file reads may reserve a native cancellation registration before invoking
`download_file`. The signal crosses the Tauri bridge into the server actor's chunk-fetch `select!`,
because dropping a webview promise or reply receiver does not cancel an actor command already in
progress. Registrations are process-capped and generation-bound. A shared opaque keepalive crosses
the sync/transport seam and remains in libp2p's pending-request row until that exact request
responds, fails or times out, even when the JavaScript/native waiter has already been cancelled.
Their begin step commits under the exact UI-session guard. Locking signals active operations and
drops unclaimed permits; transport work stays counted until its terminal event, while an unclaimed begin-only
permit may also be displaced instead of stranding capacity after a webview reload. Legacy callers
that omit a cancellation token receive a reserved native token, so they share the same cap and
lock cancellation without changing the IPC shape.
`MeshTransport::request_cancellable` intentionally has no default: every transport must state the
accounting lifetime. Production libp2p and the in-memory transport transfer the keepalive to their
actual pending/queued owner; wrappers delegate to that implementation, and non-transferring test
fakes own no detached lower work.

## 5. Roadmap (test-gated, block by block)

0. Workspace + `Clock`/`Transport` seams + canonical wire format + CI/lint gate.
1. Device identity + unified key hierarchy (`SecureKeyStore` tiers; desktop impls).
2. MLS `ServerGroup` (local, no network) + channel-key derivation.
3. Invites (MLS-bound, single-use, device-bound) + InviteLedger.
4. Replication engine: inner-signed ops, proposal/commit split, snapshot catch-up. P1 adds
   owner-receipted deterministic checkpoints only for its new document types; see its own staged
   implementation status rather than treating compaction as already shipped globally.
5. Storage & retention: CID blockstore, 3-scope expiry, clock-injected GC, file crypto.
6. Real mesh: libp2p, relay-v2/DCUtR, rendezvous, scoped gossipsub, eclipse resistance.
7. End-to-end local integration over real sockets + consolidated security suite.
8. Product model + Tauri desktop UI (channels, fileshare browser, status, wiki).
9. Android (Tauri 2 mobile): JNI keystore, foreground service, two-tier keys.
10. Hardening: calendar, cover traffic, supply-chain attestation, security review.

### File reliability and explicit retention

[File reliability](design-file-reliability.md) separates actor-owned authorization/storage from bounded
network tasks, projects conflicting first file-index lists consistently, and adds explicit local kept
copies. Kept-copy ownership never comes from replicated expiry. One exact manifest is reserved and
verified before durable visibility; normal cache deletion cannot remove it. UI availability reports
local evidence and leaves remote copies unconfirmed.
