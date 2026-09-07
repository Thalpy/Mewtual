# Mewtual; Interfaces & Hooks Schema

A reference for the **seams** (dependency-injection hooks) and the key public APIs.
Signatures are abbreviated; see the source for exact generics/lifetimes. This is the
contract a new contributor (or agent) builds against.

---

## 1. The seams (the load-bearing hooks)

Everything above these is generic over them, so the whole stack runs unchanged over
test impls (in-memory, deterministic) or production impls (OS / libp2p).

### `Clock`; injected time  *(catcoms-rt)*
```rust
pub trait Clock: Send + Sync + Debug {
    fn now_ms(&self) -> u64;       // signed/absolute Unix time
    fn monotonic_ms(&self) -> u64; // elapsed-time leases and retries
}
pub struct SystemClock;                         // the ONLY OS-clock reader
pub struct ManualClock;  fn new(start_ms) -> Self;  advance_ms(delta)->u64;  set_ms(v);  set_wall_ms(v);
```
Rule: no other code reads the OS clock. Pass `&dyn Clock` / `Box<dyn Clock + Send>`.

### RNG; injected randomness  *(catcoms-rt)*
```rust
pub use rand_core::{CryptoRng, CryptoRngCore, RngCore};
pub struct OsCryptoRng;   // the ONLY OS-RNG source; impl CryptoRngCore
```
Rule: take `&mut impl CryptoRngCore` (or generic `R: CryptoRngCore`). `Box<dyn
CryptoRngCore>` does **not** satisfy the bound (CryptoRng isn't forwarded through
`&mut dyn`); be generic over the concrete RNG instead (see `ChannelSync<T, R>`).

### `MeshTransport`; pub/sub + request/response  *(catcoms-rt)*
```rust
pub trait MeshTransport: Send + Sync {
    fn local_peer(&self) -> PeerId;
    fn connection_snapshot(&self) -> Vec<PeerConnectionSnapshot>; // bounded, present-time; default empty
    async fn subscribe(&self, topic: Topic) -> Result<(), TransportError>;
    async fn unsubscribe(&self, topic: Topic) -> Result<(), TransportError>;
    async fn publish(&self, topic: Topic, data: Bytes) -> Result<(), TransportError>;
    async fn publish_once(&self, topic: Topic, data: Bytes) -> Result<PublishSubmission, PublishOnceError>; // fail-closed default
    async fn request(&self, peer: PeerId, proto: ProtocolId, data: Bytes) -> Result<Bytes, TransportError>;
    async fn request_connected(&self, peer: PeerId, proto: ProtocolId, data: Bytes) -> Result<Bytes, TransportError>; // fail-closed default
    async fn notify(&self, peer: PeerId, proto: ProtocolId, data: Bytes) -> Result<(), TransportError>;
    async fn notify_connected(&self, peer: PeerId, proto: ProtocolId, data: Bytes) -> Result<(), TransportError>; // fail-closed default
    async fn next_event(&self) -> Option<TransportEvent>;   // single-consumer
    async fn rendezvous_register(&self, namespace:&str, rz_node:&[u8]) -> Result<(),TransportError>;
    async fn rendezvous_discover(&self, namespace:&str, rz_node:&[u8]) -> Result<(),TransportError>;
    async fn dial_addr(&self, addr:&str) -> Result<(),TransportError>;
    async fn dial_permit(&self, permit:BoxedDialPermit) -> Result<DialSubmission,TransportError>;
    async fn dial_peer_batch(&self, peer:PeerId, addrs:&[String]) -> Result<Vec<DialSubmission>,TransportError>; // 1..=2, direct, terminal peer-bound
    async fn dial_peer_permits(&self, peer:PeerId, permits:Vec<BoxedDialPermit>) -> Result<Vec<DialSubmission>,TransportError>;
    async fn add_external_addr(&self, addr:&str) -> Result<(),TransportError>;
    async fn next_discovered(&self) -> Option<DiscoveredPeer>; // default never resolves
    async fn next_registered(&self) -> Option<RendezvousRegistration>; // exact node/ns + granted TTL; default never resolves
}
pub struct PeerId([u8;32]);   fn from_u64(n)->Self; fn as_bytes()->&[u8;32];
pub struct Topic(Bytes);      fn new(impl Into<Bytes>)->Self; fn as_bytes()->&[u8];
pub struct ProtocolId(pub &'static str);
pub enum TransportEvent {
    Gossip { topic: Topic, from: PeerId, data: Bytes },
    Request { from: PeerId, proto: ProtocolId, data: Bytes, responder: Responder },
    PeerConnected(PeerId),
    PeerPathsChanged { peer: PeerId, active: Vec<ConnectionPath>, newly_established: Option<ConnectionPath> },
    PeerDisconnected(PeerId),
}
pub struct ConnectionPath { family: ConnectionFamily, transport: ConnectionTransport,
                             direction: ConnectionDirection }
pub struct PeerConnectionSnapshot { peer: PeerId, active: Vec<ConnectionPath> }
pub struct AuthenticatedDialRoute { peer: PeerId, address: String } // catcoms-net only; local-sensitive
pub const MAX_CONNECTED_PEER_SNAPSHOT: usize = 320;
pub const MAX_CONNECTION_PATH_SNAPSHOT: usize = 64;
pub enum ConnectionFamily { Ipv4, Ipv6, Dns, Memory, Unknown }
pub enum ConnectionTransport { Tcp, QuicV1, WebSocket, CircuitRelay, Memory, Unknown }
pub enum ConnectionDirection { Dialer, Listener }
pub struct Responder;  fn respond(self, Bytes);  fn channel() -> (Responder, ResponderRx);
pub struct ResponderRx; async fn recv(self) -> Option<Bytes>;
pub enum TransportError { Unreachable(PeerId), Timeout(PeerId), Closed, NoResponse, InvalidDialBatch }
pub enum PublishSubmission { Submitted, Duplicate } // local driver/cache evidence, NEVER delivery
pub enum PublishOnceError { Unsupported, TooLarge, Busy, Closed, NoPeers, QueuesFull, Failed }
pub trait DialPermit: Send + Debug { fn address(&self)->&str; fn commit_if_current(self:Box<Self>)->Option<String>; }
pub type BoxedDialPermit = Box<dyn DialPermit>;
```
Implementations:
- **`MemNetwork`** (tests): `let hub = Hub::new(); let net = hub.join(PeerId::from_u64(n));`
- **`MeshService`** (prod, catcoms-net): `spawn(swarm)` / `new_memory(listen, dial)` /
  `new_tcp(...)`; `build_memory_swarm()` / `build_tcp_swarm()`. Maps `PeerId`↔libp2p
  PeerId, hex-encodes topics, and holds a publication for a retry **only when the failure it got
  can pass**: no subscriber yet, or every peer's send queue momentarily full. A message too large,
  an unsignable one or a failed transform is reported and dropped rather than queued behind a
  retry that cannot help it, and a duplicate is already published. Held payloads are retried both
  on a `Subscribed` event and on `PENDING_PUBLISH_RETRY` (2 s), because a fully subscribed mesh
  produces no further subscription events, and are bounded by `MAX_PENDING_PUBLISH` (256) and
  `MAX_PENDING_PUBLISH_BYTES` (8 MiB), oldest dropped first. The queue is a bridge across a
  transient failure, not durable storage: what matters past it is recovered by document catch-up.
  **`publish_once` is a separate, driver-acknowledged path:** one synchronous gossip attempt,
  never the legacy `pending_publish` retry queue. Unsupported transports fail closed rather than
  fall back to `publish`. `MeshService` admits at most 16 queued/being-attempted commands, each
  with at most 512 KiB of payload and 64 topic bytes; capacity is acquired before compact-copying
  slices, and a full command queue returns `Busy` without waiting. A cancelled queued command
  keeps its capacity until drained. This bounds owned pending payloads to 8 MiB plus 1 KiB of
  topics, excluding caller inputs and already-admitted gossip state. The gossip configuration
  may reject a payload below this API ceiling; its limits are unchanged.

  Dropping the caller future closes its acknowledgement receiver. The driver checks that receiver
  immediately before its synchronous attempt; a drop observed then suppresses work. After that
  admission boundary, cancellation/acknowledgement loss cannot retract it. Even `NoPeers` or
  `QueuesFull` can leave normal libp2p cache effects (its cache insertion precedes some refusals).
  `Duplicate` is distinct from `Submitted` and proves neither delivery nor a successful prior
  send. `Closed` may mean unknown submission, not rollback. No outcome retires a P1 intent.
  `MemNetwork` implements immediate bounded fan-out, reports `NoPeers` when no other live
  subscriber accepts it, and has no deferred retry; it does not model libp2p cache/queue behaviour.
  The cooperative registry sender below uses this seam; actor ownership, native lifecycle
  cancellation and automatic replay wakeups remain unimplemented.

  `request_connected` / `notify_connected` are deliberately narrow repair sends: the actor
  succeeds only when its current peer map and `Swarm::is_connected` both say
  the transport is live. Unlike ordinary `request_control`, it never consults `recent_peers` and
  cannot implicitly redial after the shared scheduler denied a new socket attempt.
  `dial_peer_batch` accepts one or two direct multiaddrs and revalidates inside the actor that each
  has a terminal libp2p peer whose Phase-0 id equals the addressed `PeerId`; empty, oversized,
  relayed, bare, or substituted batches fail before `dial_gated` can start work.
  Discovery- and recovery-policy dials use `dial_permit` / `dial_peer_permits` instead. Production
  transfers each non-cloneable generation-bound permit into the actor command before awaiting a
  reply. Already-connected, duplicate, or already-dialling suppression drops and refunds it there;
  the actor owns the pending-infrastructure ledger and commits only immediately before inserting the
  exact member endpoint into the pending path or calling `Swarm::dial` for infrastructure. Caller
  cancellation therefore cannot refund a queued command, and a permit whose monotonic deadline has
  passed or whose scheduler window was replaced can neither start nor alter the new counters.
  A transport using the default fallback commits before its first await and may conservatively
  spend on later failure, but cannot over-refund work that escaped the caller.
  `PeerConnected`/`PeerDisconnected` retain their legacy one-edge aggregate meaning.
  `PeerPathsChanged` follows them on the same ordered event stream and also fires when a second
  connection upgrades/refines a path without changing aggregate liveness. It carries no address or
  physical-connection count. Relay/WebSocket semantics win over their TCP carrier; IPv4-mapped IPv6
  normalizes to IPv4. Connection limits bound the actor ledger (320 global/eight per peer), and
  duplicate coarse close snapshots are suppressed. Each genuine establishment still emits a
  snapshot to timestamp historical success, so intense connection churn can consume the bounded
  256-event channel and backpressure the single swarm actor; limits bound memory but do not erase
  that availability residual.
  `connection_snapshot()` is the non-consuming handoff seam: admission/relay/rendezvous waits use
  `MeshService::wait_for_*_connected` over its coalesced watch, leaving every ordered event for the
  eventual owner. The admission routines that must inspect pushed proof/Welcome requests on that
  stream coalesce any lifecycle events they dequeue into a bounded
  `PreOwnerConnectionHandoff`; the newly constructed `ChannelSync` adopts its final live table once
  before draining later queued events. Because the legacy query snapshot and event stream have no
  shared revision watermark, `ChannelSync` never seeds from `connection_snapshot()` directly. A
  stopped actor returns an empty snapshot / `Closed`, never Tokio watch's retained last value.
  `MeshHandle::authenticated_dial_routes()` is a separate, address-bearing watch over currently
  live **outbound direct IP** connections whose remote PeerId completed Noise authentication. It
  never contains inbound ephemeral source ports, DNS or relay circuits and is not exposed through
  the address-free `MeshTransport` snapshot. The desktop may retain at most two routes for the
  named inviter after successful direct admission; these values are local-sensitive reconnect
  hints, not membership, device-to-transport binding, presence, or future reachability proof.
  `catcoms-sync` increments a session-local member-route revision only when a current member's
  path, record, dial-scheduler state, or verdict can change. `catcoms-app` compares that revision
  and emits `AppEvent::MemberRoutesChanged` / Tauri `member-routes-changed` without rebuilding the
  O(roster × addresses) projection after every sync. Unclaimed Noise-peer churn retains only its
  bounded evidence and does not bump the UI revision. Time-derived cooldown/history expiry does
  not mutate that revision, so the visible Connectivity view also refreshes at most once a minute;
  the debug console already uses a bounded poll.
  - **NAT traversal:** `listen_on(circuit)` / `next_listen_addr()` reserve a relay
    circuit; `next_direct_upgrade()` surfaces a DCUtR hole-punch. Infra nodes:
    `build_relay_swarm()`/`run_relay(...)`, `build_rendezvous_swarm()`/`run_rendezvous(...)`.
    `MeshBehaviour` also runs an AutoNAT v2 client; `next_autonat_snapshot()` or the
    single-consumer `take_autonat_snapshots()` returns a bounded `AutoNatSnapshot` containing the
    latest `AutoNatResult` for each candidate/server pair. Results remain scoped to one candidate,
    server and test, are pruned when that route is withdrawn, and are accepted only while the
    address has a live configured or router-mapping owner. Relay/rendezvous swarms can serve v2
    dial-backs only after the operator's experimental `--enable-autonat` opt-in; ordinary members
    never do. `GuardedAutoNatServer` tags every upstream callback dial before the derived
    behaviour sees it. Its first-declared guard rejects peer-less/non-direct/DNS/circuit/private
    targets, requires the target literal to equal the exact inbound-connection source IP, and
    charges bounded node/source-prefix/peer buckets before a transport socket is opened.
    Connection-scoped source observations are refreshed by requests and removed on close.
    Router mapping is coalesced current state: `next_port_mapping_snapshot()` or the
    single-consumer `take_port_mapping_snapshots()` yields a bounded `PortMappingSnapshot` of live
    leases and scoped failures labelled by UPnP, PCP or NAT-PMP, TCP or UDP/QUIC, and an optional
    exact local-address owner. `None` is the legacy IPv4/default-gateway path; `Some(IpV6)` is an
    independently managed PCPv6 firewall pinhole, so IPv4 and multiple interfaces cannot collide.
    PCPv6 uses an internal narrow RFC 6887 MAP client because pinned `portmapper` 0.18 is
    IPv4-only: it sends an exact 60-byte MAP request, accepts aligned option-bearing responses up
    to the protocol bound, binds the exact global listener address, and selects the scoped default
    router from the operating system's IPv6 route table and native interface index. It requests
    five-minute TCP/UDP leases, honors the router-assigned lifetime up to a 24-hour sanity cap,
    renews on an injected monotonic clock with the same 96-bit nonce, and sends a best-effort
    lifetime-zero delete on listener removal. A slow/late
    consumer may skip intermediate retries but cannot resurrect an expired current route. Only
    globally routable mappings are offered to AutoNAT/the swarm; duplicate mapping and
    manual-forward owners are reference-counted. Worker generations make buffered events from a
    removed/replaced listener inert. `take_relay_address_snapshots()` similarly
    exposes the live circuit-listener set so reservation expiry is withdrawn. The product layer
    updates the live bootstrap/peer record and re-mints the next displayed invite after any set
    change.
    `MeshService`/`MeshHandle::remove_external_address(addr)` withdraws one caller-configured
    owner while preserving an identical active router-mapping owner. The desktop's discovery timer
    uses this with a route-source poll: changed raw IPv4/IPv6 entries update the aggregate
    owner map, AutoNAT/rendezvous external set, Connectivity, and one new signed peer-record epoch
    before that pass's PEX. One process-wide `netwatch` monitor wakes every server after a bounded
    debounce when the native route/interface state changes. It uses platform notifications rather
    than a second fast poll; the normal discovery poll remains the recovery path if monitoring is
    unavailable or misses an event.
    PCPv6 tracks Epoch for every response and randomizes rapid renewal after a restart signal, but
    each transport/interface worker observes that signal independently; it is not a full
    gateway-wide RFC ANNOUNCE coordinator.
    Already copied signed invite strings are immutable and cannot be rewritten after lease loss.
    `next_listener_snapshot()` reports only concrete listener addresses accepted by Swarm.
    `take_mesh_observation_snapshots()` exposes bounded per-connected-peer Identify observations
    for diagnostics only. Those outbound-source observations never enter invites, peer records,
    AutoNAT candidate sets or dial plans: TCP source ports are normally ephemeral and a peer can
    lie.
  - **Discovery (6e-3d):** `rendezvous_register(namespace, rz_node)` /
    `rendezvous_discover(namespace, rz_node)`; `next_registered()` and `next_discovered()`
    surface results. Discovered records (`Discovered { peer, addresses, namespace }`) are
    **never auto-dialed**; a higher layer (`catcoms-discovery`) decides whether to dial;
    the surfaced-record queue is per-Discover-response capped. `add_external_address(addr)`
    lets a directly-reachable node register without a relay. `dial(addr)` queues a runtime dial;
    the actor refuses it unless the address has a terminal peer id (there is no bare fallback).
    Free fn `validate_rendezvous_addrs(&[String]) -> Vec<RendezvousTarget>` (reject
    `/p2p-circuit`, require exactly one `/p2p/`, distinct PeerIds).

### Peer-bound route parsing and endpoint scheduling  *(catcoms-discovery)*

```rust
pub fn parse_peer_dial_route(addr:&str, expected_peer:&[u8;32]) -> Option<ParsedPeerRoute>;
pub struct CanonicalDialPeer; // Phase-0 terminal transport id; constructible only by the parser
pub enum DialRouteKind {
    Direct,
    Relay { relay_peer:CanonicalDialPeer, target_peer:CanonicalDialPeer },
}
pub struct ParsedPeerRoute {
    pub host:RouteHost, pub principal:CanonicalDialPeer,
    pub kind:DialRouteKind, pub endpoint:DialEndpoint,
}
pub enum RouteHost { Ip(IpAddr), Dns(String) }
pub struct EndpointDialConfig {
    pub window_ms:u64, pub process_limit:u32, pub server_limit:u32,
    pub peer_limit:u32, pub endpoint_limit:u32, pub prefix_limit:u32,
}
pub struct EndpointDialScheduler; // cloneable; clones share one bounded transient counter set
  new(EndpointDialConfig) -> Self;
  new_with_clock(EndpointDialConfig, Arc<dyn Clock>) -> Self;
  reserve(&self, server:&[u8], endpoints:&[DialEndpoint]) -> Vec<String>;
  reserve_permits(&self, server:&[u8], endpoints:&[DialEndpoint])
    -> Vec<EndpointDialPermit>; // non-cloneable and bound to one accounting generation

pub trait DialPermit { // `catcoms-rt`; object-safe transport ownership seam
  address(&self) -> &str;
  commit_if_current(self:Box<Self>) -> Option<String>;
}
MeshTransport::dial_permit(BoxedDialPermit) -> DialSubmission;
MeshTransport::dial_peer_permits(PeerId, Vec<BoxedDialPermit>) -> Vec<DialSubmission>;
```

The parser accepts only canonical, non-zero raw TCP or root-path WebSocket TCP (exactly `/ws`,
`/wss`, or `/tls/ws`; non-root paths, SNI, and standalone/mixed TLS shapes are not product transports)
or UDP/QUIC-v1 routes, plus the explicit single-relay circuit form. A direct route has exactly one
terminal `/p2p/<PeerId>`; a circuit has one relay id and one terminal target. The terminal id's
Phase-0 hash must equal `expected_peer`, with nothing trailing. Syntax and identity binding are
separate from host trust: PEX/cache/member inputs additionally refuse DNS and dangerous local,
private, link-local, multicast and transitional ranges (the sync test vocabulary deliberately
retains non-routed documentation/benchmark literals); invite rendezvous and switchboards use the
stricter global-literal classifier. Direct invite bootstraps deliberately retain bounded
LAN/loopback support.

`DiscoveryPolicy::dial_budget` counts returned addresses, not peers. The desktop creates one
`EndpointDialScheduler`, installs a clone into every server before cached redial, and applies it to
untrusted post-join discovery plus pre-join invite/rendezvous/switchboard routes, repeated two-way
reply callbacks, and direct companion-grant redemption. Trusted operator-configured infrastructure
connections have separate validation/lifecycle paths and are not universally mediated by this API.
Defaults grant at most 32 endpoints per 60-second process window, 8 per server, 4 per canonical
`(server, Phase-0 peer)`, 2 per direct physical socket or authenticated relay/target circuit, and 8
per IPv4 `/24`, IPv6 `/48`, or DNS host. The parser embeds the canonical peer principal in the opaque
endpoint; callers cannot supply a device id or raw libp2p id as an accounting alias. Direct-socket
and prefix keys exclude the claimed PeerId and descriptor sequence. Separate relay-circuit keys keep
unrelated targets at one relay from exhausting each other's two-attempt cap; the shared relay host is
bounded at prefix/process scope rather than by a separate outer-socket lease. A shared denial is
refunded to the local policy because no dial command was submitted. Scheduler state is session-only
and uses one scheduler-owned `Clock::monotonic_ms` timeline. A permit is transferred into the
production transport actor before the caller awaits; duplicate/already-connected/already-dialling
  pre-commit suppression refunds it, while a deadline- and generation-current commit happens immediately before
pending/socket submission. Infrastructure targets have an actor-owned pending ledger that is
shared with member routes later reclassified as infrastructure and released on immediate refusal,
connection, or outgoing failure. Constructor routes are grouped by terminal peer into one
known-peer address race and seed that ledger before the actor starts. Immediate transport refusal
after commit is reported separately from suppression: it remains conservatively charged in both the
shared scheduler and local discovery policy, but does not manufacture a successful attempt/cooldown.
Deadline and generation checks prevent queued expired work from starting and
delayed command drops from refunding a newer window. A process-wide in-flight/concurrency lease
remains future hardening. Pre-join invite
rendezvous seeds are capped at two distinct validated nodes so infrastructure cannot exhaust the
per-server window before the discovered inviter is dialed.

### `SecureKeyStore`; at-rest DEK protection, tiered  *(catcoms-crypto)*
```rust
pub trait SecureKeyStore: Debug {
    fn tier(&self) -> KeyTier;
    fn seal_dek(&self, dek: &Dek, rng: &mut dyn CryptoRngCore) -> Result<SealedBlob, KeystoreError>;
    fn unseal_dek(&self, blob: &SealedBlob) -> Result<Dek, KeystoreError>;
}
pub enum KeyTier { Hardware{attested:bool}, OsSoftware, Passphrase, None }  fn strength()->u8;
pub fn requires_passphrase_confirmation(prev: KeyTier, cur: KeyTier) -> bool;  // downgrade guard
pub struct PassphraseKeyStore; fn derive(passphrase, salt) -> Result<Self>;     // Argon2id
pub struct InMemoryKeyStore;   fn generate(rng) -> Self;
```
(OS-native impls; Secret Service / DPAPI / Android Keystore; are platform-phase work.)

### `HolderOracle`; fresh replica count  *(catcoms-storage)*
```rust
pub trait HolderOracle { fn reachable_holder_count(&self, cid: &Cid) -> usize; }
```
Injected into GC so it never evicts the last copy. (Network-backed impl is later.)

### `BlobStore`; content-addressed bytes  *(catcoms-storage)*
```rust
pub trait BlobStore {
    fn put(&mut self, bytes: &[u8]) -> Result<Cid, StorageError>;       // cid = Cid::of(bytes); replaces a corrupt record at that cid
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StorageError>;  // integrity-checked
    fn has(&self, cid: &Cid) -> bool;                                  // cheap existence hint only; NOT proof of integrity
    fn delete(&mut self, cid: &Cid) -> Result<bool, StorageError>;
    fn cids(&self) -> Vec<Cid>;
}
pub struct MemoryBlobStore;  pub struct FsBlobStore;  fn open(dir) -> Result<Self>;
```

---

## 2. Identity & crypto  *(catcoms-crypto)*

```rust
pub struct DeviceId([u8;32]);   // = BLAKE3("catcoms/device-id/v1" ‖ ed25519_pubkey)
  fn from_verifying_key(&VerifyingKey)->Self;  fn from_public_key_bytes(&[u8])->Self;
  fn from_bytes([u8;32])->Self;  fn as_bytes()->&[u8;32];
pub struct UserId([u8;32]);     // = BLAKE3("catcoms/user-id/v1" ‖ account_pubkey)

pub struct DeviceKeypair;  generate(rng); from_seed(&[u8;32]); seed()->[u8;32];
  verifying_key(); device_id(); sign(&[u8])->[u8;64];
pub fn verify(&VerifyingKey, msg, &[u8;64]) -> bool;                 // strict
pub fn verify_with_public_bytes(pubkey: &[u8], msg, &[u8;64]) -> bool;

// Multi-device pairing (design-multi-device.md v2: origin device is the identity root,
// chain depth 1, no account key; the v1 account-rooted chain module was deleted; all
// domains here are /v2 so no v1 statement can cross-verify).
pub const MAX_DEVICE_NAME_BYTES; pub const MAX_CERT_GROUP_ID_BYTES; pub const SAS_DIGITS/SAS_MODULUS;
pub fn validate_device_name(&str) -> Result<(), PairingError>;   // bounds + control/bidi/zero-width rejects
pub fn sas(new_device_pk:&[u8;32], pairing_nonce:&[u8;32], origin_id:&DeviceId) -> u32; // 6-digit SAS
pub struct PairingRequest { new_device_pk:[u8;32], pairing_nonce:[u8;32] }  new(rng); new_device_id(); sas(origin); encode/decode;
pub struct DeviceCertificate { origin_id, origin_public_key:[u8;32], new_device_id, group_id, device_name, issued_ts_ms, signature }
  issue(&DeviceKeypair, new_device_id, group_id, name, now_ms); verify(&expected_origin);  // group-bound; carry-the-pubkey
pub struct DeviceRevocation { origin_id, origin_public_key, revoked_device_id, rev_ts_ms, signature }  issue/verify (self-revoke allowed);
pub struct MasterHandoff { origin_id, origin_public_key, new_master_device_id, master_seq, ts_ms, signature }  issue/verify (monotonic seq enforced by consumers);

// One key hierarchy.
pub struct Dek;  generate(rng); from_bytes([u8;32]); expose_bytes()->&[u8;32]; subkey(label)->[u8;32];
pub struct KeyHierarchy;  new(Dek); db_key(); mls_seal_key(); blob_key();   // HKDF subkeys
pub struct SealedBlob { pub nonce:[u8;24], pub ciphertext:Vec<u8> }
pub fn seal(wrap_key:&[u8;32], plaintext, rng:&mut dyn CryptoRngCore) -> Result<SealedBlob>;  // XChaCha20-Poly1305
pub fn unseal(wrap_key:&[u8;32], &SealedBlob) -> Result<Vec<u8>>;
```

---

## 3. MLS group + invites  *(catcoms-mls)*

```rust
pub const CIPHERSUITE: Ciphersuite;   // MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519 (0x0003)

pub struct MlsDevice;   // a device's MLS leaf identity (owns its openmls provider)
  generate() -> Result<Self>;  device_id();  public_key_bytes()->Vec<u8>;  sign(&[u8])->Result<[u8;64]>;
  key_package() -> Result<KeyPackage>;
  key_package_for_invite(group_id, nonce:[u8;16]) -> Result<KeyPackage>;   // credential bound to (group, nonce)
  parse_key_package(&[u8]) -> Result<KeyPackage>;                          // validate one off the wire
pub fn serialize_key_package(&KeyPackage) -> Result<Vec<u8>>;

pub struct ServerGroup;   // one MLS group == one server
  create(&MlsDevice) -> Result<Self>;
  join(&MlsDevice, welcome_bytes) -> Result<Self>;
  add_member(&mut, &MlsDevice, KeyPackage) -> Result<AddOutcome>;
  add_member_via_invite(&mut, &MlsDevice, KeyPackage, &InviteToken, &mut InviteLedger, now_ms) -> Result<AddOutcome>;
  remove_member(&mut, &MlsDevice, &DeviceId) -> Result<()>;
  mint_invite(&MlsDevice, nonce:[u8;16], expires_at_ms, bootstrap:Vec<String>) -> Result<InviteToken>;
  mint_invite_with_rendezvous(&MlsDevice, nonce, expires_at_ms, bootstrap, rendezvous:Vec<String>) -> Result<InviteToken>;  // pre-validate via catcoms_net::validate_rendezvous_addrs
  designated_committer() -> Option<DeviceId>;   // lowest leaf index
  is_designated_committer(&MlsDevice) -> bool;
  create_application_message(&mut, &MlsDevice, &[u8]) -> Result<Vec<u8>>;
  process_incoming(&mut, &MlsDevice, &[u8]) -> Result<Incoming>;   // applies commits / app msgs
  channel_secret(&MlsDevice, DocType, doc_id:u128) -> Result<[u8;32]>;   // per-doc content key (epoch exporter)
  metadata_secret(&MlsDevice, DocType, doc_id:u128) -> Result<[u8;32]>;  // network identifiers (separate label)
  epoch()->u64; group_id()->Vec<u8>; member_count(); member_device_ids(); contains_device(&DeviceId);
pub struct AddOutcome { pub welcome:Vec<u8>, pub commit:Vec<u8>, pub commit_epoch:u64 }
pub enum Incoming { Application(Vec<u8>), CommitApplied, Other }

pub struct InviteToken {           // pasteable, single-use, device-bound capability (INVITE_DOMAIN v2)
    pub group_id:Vec<u8>, pub inviter_device_id:DeviceId, pub inviter_public_key:Vec<u8>,
    pub invite_nonce:[u8;16], pub expires_at_ms:u64, pub bootstrap:Vec<String>,
    pub rendezvous:Vec<String>,    // 6e-3d-9: signature-bound rendezvous infra multiaddrs
    pub signature:[u8;64] }
  encode()->Vec<u8>; decode(&[u8])->Result<Self>;   // both address vectors length-prefixed + capped
  verify(inviter_pubkey)->bool;  verify_self()->bool;  verify_inviter_signature(msg, &[u8;64])->bool;
pub struct MembershipCredential { device_id, group_id, invite_nonce }  encode()/decode();  // the MLS leaf credential binding
pub struct InviteLedger;  new(); revoke(nonce); is_consumed(&nonce); is_revoked(&nonce);
  check(&InviteToken, now_ms) -> Result<(),InviteError>;  consume(nonce) -> Result<(),InviteError>;
```

`catcoms-net::JoinReply` is the short-lived return channel for a failed one-way dial. Its
`mewtual-reply-v1:` text embeds the complete signed `InviteToken` permit, the joiner's transport
PeerId, a fresh joiner nonce, absolute expiry, at most four direct public TCP/QUIC candidates and
an invite-nonce-derived MAC. Decode/verify is size-, shape-, identity- and clock-bounded before
any dial; candidate `/p2p` suffixes are reconstructed from the authenticated joiner PeerId rather
than accepted from text.

---

## 4. Encrypted CRDT replication  *(catcoms-replication)*

```rust
pub struct SignedOp { doc_type:DocType, doc_id:u128, author_device:DeviceId, author_pubkey:Vec<u8>, delta:Vec<u8>, domain_op:Option<Vec<u8>>, signature:[u8;64] }
  sign(&MlsDevice, DocType, doc_id, delta:Vec<u8>) -> Result<Self>;  sign_domain(..., &DomainOp)->Result<Self>;
  verify()->bool;  encode()/decode();  hash()->[u8;32];
pub struct SealedOp { doc_type:DocType, doc_id:u128, epoch:u64, blob:SealedBlob }
  seal(&SignedOp, &ServerGroup, &MlsDevice, rng) -> Result<Self>;  open(channel_key:&[u8;32]) -> Result<SignedOp>;
  encode()->Vec<u8>; decode(&[u8])->Result<Self>;

pub struct EncryptedDoc;   // automerge doc + signed-op log
  new(DocType, doc_id, actor:&DeviceId) -> Self;  doc()->&AutoCommit;  op_count();
  edit(&mut, &MlsDevice, &ServerGroup, rng, FnOnce(&mut AutoCommit)->Result<(),AutomergeError>) -> Result<SealedOp>;
  ingest(&mut, &SealedOp, &ServerGroup, &MlsDevice) -> Result<bool>;   // verify+apply; dedup; epoch must == current
  ingest_with_key(&mut, &SealedOp, key:&[u8;32]) -> Result<bool>;      // open with a caller-supplied (past-epoch) key; inner sig still verified
  export_catchup(&ServerGroup, &MlsDevice, rng) -> Result<Vec<SealedOp>>;   // re-sealed under current epoch
  import_catchup(&mut, &[SealedOp], &ServerGroup, &MlsDevice) -> Result<usize>;
  restore_for_actor(snapshot, &DeviceId) -> Result<EncryptedDoc>; // required before post-restart P1 edits
  from_checkpoint(&VerifiedCheckpoint, &DeviceId) -> Result<EncryptedDoc>; // one receipt-authorized seed, empty user-op log
  checkpoint_origin() -> Option<&CheckpointOrigin>; checkpoint_bytes() -> Result<Option<Vec<u8>>>;
  edit_domain_gated(&mut, &LogicalDocument, &EpochGate, ..., &DomainOp, edit) -> Result<(SealedOp,ChangeHash)>;
  ingest_domain_gated(&mut, &LogicalDocument, &EpochGate, &SealedOp, ...) -> Result<Admission>;

pub struct LogicalDocument { server_id:Vec<u8>, doc_type:DocType, logical_key:Vec<u8> }
pub struct CloseRecord;      // authority and the real dependency-closed signed log validate together
pub struct Receipt;          // owner-signed close + seed + first-tenure inherited checkpoint
pub struct VerifiedReceipt;  // opaque capability returned only by owner/tenure verification
pub struct ReceiptRepair;    // owner-signed selection after visible receipt equivocation
pub struct ReceiptHeadProof; // nonce/requester-bound owner selection; verifies to VerifiedReceipt
pub struct OwnerReceiptJournal; // persist-before-publish high-water + one in-flight decision
pub struct ReceiptBook;      // ingest_and_seal atomically updates receipt state and its epoch gate
pub struct EpochGate;        // server/document/id-bound Open -> Closing/Settled/Fault boundary
pub struct IntentLedger;     // bounded vault-sealed local operations retained until receipted
pub struct RecoverySlots;    // two retained typed snapshots + one crash-resumable staged slot
pub struct CheckpointSeed;   // deterministic raw change candidate, at most 2 MiB; building gives no authority
  build(&LogicalDocument, checkpoint_epoch, close_hash, typed_writer) -> Result<Self>;
  verify(&VerifiedReceipt, raw_bytes, typed_validator) -> Result<VerifiedCheckpoint>;
pub struct VerifiedCheckpoint; // opaque, immutable receipt/hash/schema-verified seed
// P1 v2 raw changes are pre-scanned for bounded RLE/string/predecessor work; compressed changes
// reject. New inbound P1 authors must match a current member's full signing key, not only its relay.
pub struct CheckpointOrigin; // logical scope, epoch, close and seed hashes retained in vault snapshots
// Both checked P1 paths preflight the prospective materialization before gate admission/state swap:
EncryptedDoc::edit_domain_preflight_gated(..., typed_change_validator, projection_preflight);
EncryptedDoc::ingest_domain_preflight_gated(..., typed_change_validator, projection_preflight);
// catcoms_replication::registry (first typed consumer; no sync discovery or automatic settlement yet):
pub struct PointerKey;        // type + bounded logical key; deterministic bucket()
pub enum RegistryOp { Put { key:PointerKey, epoch:u64 }, Tombstone { key:PointerKey } }
pub struct RegistryProjection; // admitted pointers, explicit overflow and tombstones
  read(...); checkpoint(close_hash); verify_checkpoint(&VerifiedReceipt, bucket, raw_bytes);
pub struct RegistryRecovery; // typed, local-only recovery; content-redacted Debug
  from_snapshot(&RecoverySnapshot, &LogicalDocument, bucket) -> Result<Self>;
  projection(); receipt_hash(); excluded_operations(); // historical evidence, not replay permission
registry_document(server_id, bucket) -> Result<LogicalDocument>;
edit_registry(..., &DomainOp) -> Result<SealedOp>; ingest_registry(...) -> Result<Admission>;
checkpoint_registry_close(...) -> Result<(CheckpointSeed, ClosureStats)>; // verified named closure, source untouched
// catcoms_replication::registry_epoch; one exclusively owned restart unit, not a disk adapter:
pub struct RegistryEpoch; // private EncryptedDoc + EpochGate + ReceiptBook + opening receipt
  new(&ServerGroup, bucket, actor:DeviceId) -> Result<Self>;
  from_checkpoint(&ServerGroup, bucket, actor, Receipt, expected_tenure_start, seed) -> Result<Self>;
  opened_by(&Receipt) -> bool; // exact opening receipt, Open/Closing only; no durability proof
  checkpoint_successor(&RegistrySettlementPlan, &ServerGroup, expected_tenure_start) -> Result<Self>;
  // Separately constructs seed-backed successor preserving the receipt book; source untouched.
  edit(&MlsDevice, &ServerGroup, rng, &DomainOp) -> Result<SealedOp>;
  validate_local_edit(&MlsDevice, &ServerGroup, &DomainOp) -> Result<()>;
  retains_local_operation(&MlsDevice, &ServerGroup, &DomainOp) -> Result<bool>; // exact signed log, not marker
  edit_or_reseal(&MlsDevice, &ServerGroup, rng, &DomainOp) -> Result<SealedOp>;
  ingest(&SealedOp, &ServerGroup, &MlsDevice) -> Result<Admission>;
  seal(Receipt, &ServerGroup, expected_tenure_start) -> Result<ReceiptIngest>; // retains full source
  prepare_settlement(&CloseRecord, &ServerGroup, expected_tenure_start) -> Result<RegistrySettlementPlan>;
  doc_id(); epoch(); phase(); op_count(); quarantined_len(); projection(); // detached/read-only
  snapshot() -> Result<Vec<u8>>; // raw seed + signed log + gate/book, bounded version 1
  restore(vault_authenticated_bytes, &ServerGroup, expected_bucket, actor) -> Result<Self>;
  validate_vault_snapshot(vault_authenticated_bytes, server_id, bucket) -> Result<usize>;
  storage_protocol_bytes() -> Result<usize>; // exact receipt-only bytes, not declared usage
// Restore is local-only, not network authorization. Caller journals intent before edit and
// atomically vault-persists the unit before publishing/acknowledging. No mutable handles escape;
// This core builder does not prune/replace the source or acknowledge recovery. The store
// transaction below does; live orchestration and fault repair are still separate work.
pub struct RegistrySettlementPlan; // private, computation-only, content-redacted Debug
  receipt(); checkpoint(); source_projection(); // immutable references
  included_operation_ids(); included_operations(); excluded_operations(); // full envelopes; no replay authority
  source_version() -> [u8;32]; matches_source(&mut RegistryEpoch) -> Result<bool>;
  source_base_close() -> Option<[u8;32]>; // close that opened the source, not its successor
  recovery_snapshot() -> Result<Option<RecoverySnapshot>>; // bounded, not persisted
```

---

## 5. Storage & retention  *(catcoms-storage)*

### Creative blob seam (C0c, independent of Studio/P1 metadata)

- Native `publish_pix({server, bytesB64}) -> {cid, bytes}` calls
  `ServerActor::publish_pix` / `Server::publish_pix`. Rust validates PIX1 (64 KiB encoded cap),
  then `ChannelSync::publish_blob_bounded` stages, verifies, promotes and flushes before returning
  the actual BLAKE3 address of the PIX bytes. This does **not** publish a Studio/file/chat record.
  The caller publishes references only after success. A promoted orphan may remain after failure;
  failures must not delete a held CID that another reference could use. Filesystem blobs are
  file-synced, plus containing directories on Unix, matching the existing vault platform seam;
  this is not a new cross-platform sudden-power-loss guarantee. Unix blob-directory creation
  also flushes ancestor entries, including on a retry after failed creation. `publish_pix` refuses
  a process-local memory store (including the production disk-attachment fallback).
- Native `request_blob_bounded({server, cid, maxBytes}) -> {bytes_b64, bytes} | null` calls the
  same-named actor/Server/sync methods. CID syntax is exactly 64 lowercase hex digits; `maxBytes`
  is an integer in 0..9 MiB, normally the reference's declared byte length. Local reads check the
  opened file's size before allocation (sealed frame overhead is 40 bytes). One selected peer is
  asked if missing/corrupt; signed response framing, declared limit, current membership,
  request-bound signature and CID all validate before storage. Refusals have no provider fallback.
  Null is unavailable, errors reject. **Consumers must also require exact byte equality and decode
  their format before rendering.** The transport still buffers its existing capped response;
  the caller-specific network bound is post-transport, before blob body copy/hash/storage.
- Both commands reuse the four native inline-download slots, lock cancellation and transport
  accounting keepalives. Response conversion rechecks unlock generation and server incarnation.
  Neither command triggers passive fetches, installs a UI cache, emits Studio updates, pins content,
  defines expiry or provides aggregate storage admission. Those remain separate integration work.
- `BlobStore::get_bounded(cid, max_bytes)` and `promote_staged_bounded(cid, max_bytes)` are required
  store methods, not default fallbacks to unbounded reads. Ordinary fetched-blob `put` dedup also
  checks an existing file using the incoming length. Legacy unbounded reads/promotions remain for
  existing consumers; callers needing declared-size protection must use the bounded seam.

```rust
pub struct Cid([u8;32]);  of(bytes)->Self; from_bytes/as_bytes/to_hex/from_hex;   // BLAKE3 over CIPHERTEXT
pub struct FileRef { plaintext_cid, ciphertext_cid, wrapped_key:SealedBlob, size, mime }  encode()/decode();
pub fn seal_file(plaintext, mime, wrap_key:&[u8;32], rng) -> Result<(FileRef, Vec<u8>)>;   // per-file content key + wrap nonce
pub fn open_file(stored:&[u8], &FileRef, wrap_key:&[u8;32]) -> Result<Vec<u8>>;

pub enum Expiry { Never, After(u64) }   pub enum BlobKind { InlineMedia, File }   pub struct ServerId(Vec<u8>);
pub struct ExpiryPolicy;  with_global(ms); set_server(ServerId, ms); set_file(Cid, Expiry); effective(&Cid,&ServerId)->Expiry;
pub struct RetentionIndex;  new(ExpiryPolicy);  insert(cid, server, size, kind, now, rng);  touch/set_pinned/get;
  deadline(&Cid)->Option<u64>; is_expired(&Cid, now); blob_state(&store,&oracle,&Cid)->BlobState;
  gc(&mut store, &oracle, now, min_holders) -> Result<GcReport>;       // decorrelated + holder-probed
  clear_older_than(&mut store, &oracle, age, now, min_holders, force) -> Result<GcReport>;
  rehydrate(&mut store, &Cid, bytes) -> Result<()>;
pub enum BlobState { Available, EvictedRefetchable, MissingNoHolder, Unknown }
```

---

## 6. Channel sync (the integration layer)  *(catcoms-sync)*

```rust
pub struct ChannelSync<T: MeshTransport, R: CryptoRngCore>;
  new(transport:T, group:ServerGroup, device:MlsDevice, rng:R, clock:Box<dyn Clock + Send>) -> Self;  // founder
  new_joined(transport, group, device, rng, clock, routing:RoutingState) -> Self;  // a JOINER: adopts the transferred routing state (6e-3d-2a)
  set_config(SyncConfig);                                  // override recovery/key-window bounds
  async subscribe_control() -> Result<()>;                 // receive membership commits (member-only topic)
  epoch() -> u64;   routing_label() -> u64;   stats() -> SyncStats;
  rendezvous_namespaces(rz_peer:&[u8]) -> Vec<String>;     // blinded namespaces to register/discover under (current + grandfathered)
  mint_invite(nonce:[u8;16], expires_at_ms, bootstrap) -> Result<InviteToken>;
  mint_invite_with_rendezvous(nonce, expires_at_ms, bootstrap, rendezvous:Vec<String>) -> Result<InviteToken>;  // 6e-3d-9
  async open_channel(DocType, doc_id) -> Result<()>;       // create doc + subscribe its ns_secret_L-keyed topic
  async post(DocType, doc_id, FnOnce(&mut AutoCommit)->Result<(),AutomergeError>) -> Result<()>;  // edit, then gossip; Ok once the EDIT applied
  async run_once() -> Result<bool>;                        // drain outbox + recovery + sub-resync; then handle ONE event
  async request_catchup(peer:PeerId, DocType, doc_id) -> Result<usize>;        // incremental where possible; see KIND_CATCHUP_SINCE
  async request_commit_catchup(peer:PeerId, from_epoch:u64) -> Result<usize>;  // missed-commit recovery (ordered replay, SIGNED response)
  // 6e-3d-6 pre-dial member tag (keyed by ns_secret_L): a discoverer rejects a Sybil/forged record before dialing.
  membership_tag(rz_peer:&[u8], slot:u64, peer_id:&[u8], seq:u64) -> Option<[u8;16]>;
  verify_membership_tag(rz_peer:&[u8], namespace:&str, peer_id:&[u8], seq:u64, tag:&[u8;16]) -> bool;
  // 6e-3d-7 member PEX: members supply each other dialable, self-signed peer records.
  publish_self_record(addresses:Vec<String>, seq:u64) -> Result<()>;  ingest_peer_record(PeerDescriptor) -> bool;
  async request_pex(peer:PeerId) -> Result<usize>;  known_peer_records() -> Vec<PeerDescriptor>;  peer_record(&DeviceId) -> Option<&PeerDescriptor>;
  // Cross-session redial: newest roster-checked cached records are policy-ranked. Equal address
  // epochs retry with bounded monotonic exponential backoff+jitter; a newer signed seq or a live
  // connect/disconnect lifecycle resets the delay. Old public IPs are not unioned indefinitely.
  set_endpoint_dial_scheduler(EndpointDialScheduler); // inject one process-shared final dial gate
  set_local_reconnect_routes(Vec<(PeerId,String)>);    // sealed desktop hints; direct literal IP only, transient in ChannelSync
  async dial_local_reconnect_routes() -> usize;        // exact current roster claim + shared scheduler rechecked before dial
  mint_member_recovery_code(Vec<String>) -> Result<MemberRecoveryCode>; // current member signs <=4 current direct listener routes
  verify_member_recovery_code(&str) -> Result<MemberRecoveryVerified>; // pure group/member/time/exact-device-peer/address validation
  async apply_member_recovery_code(&str) -> Result<MemberRecoveryApplied>; // verifies group/member/time/peer binding, then scheduler-charged dial
  cache_known_records() -> usize;  async dial_cached_peers() -> usize;
  async drive_mesh_repair() -> usize;               // one bounded target: ≤2 connected-only probes, optional reciprocal request
  async drive_pending_reciprocal() -> usize;        // target-side exact-descriptor direct batch submission
  async manual_fallback_redial() -> ManualRedialOutcome; // anti-click cooldown; preserves policy/process scheduler
  async drive_discovery();                          // periodic discovery + TTL-aware registration renewal
  async next_postjoin_discovery_event() -> Option<PostJoinDiscoveryEvent>;
  note_rendezvous_registered(RendezvousRegistration) -> bool;
  track_delivery_target(DocType, doc_id, ChangeHash); // bounded exact targets eligible for an explicit receipt
  peers_with_changes(DocType, doc_id, &[ChangeHash]) -> Vec<Vec<String>>; // causal authors union authenticated receipts
  authorize_join_helper(joiner:PeerId, invite_nonce:[u8;16], inviter_device:DeviceId,
                        target:PeerId, expires_at_ms:u64) -> bool;
  doc(DocType, doc_id) -> Option<&EncryptedDoc>;  local_peer() -> PeerId;  transport() -> &T;  // transport(): the discovery/dial layer above ChannelSync

// 6e-3d-9 free fn: the pre-join rendezvous namespace, derivable from the invite ALONE
// (BLAKE3-keyed off derive_key(invite_nonce), bound to group + rz_peer); so a joiner
// discovers the inviter with no group secret and no hard-coded address.
pub fn join_namespace(group_id:&[u8], invite_nonce:&[u8;16], rz_peer:&[u8]) -> String;
// A member's self-signed, dialable peer record (PEX entry / discovery candidate). Every address
// must be canonical, public-IP based, and terminate in the libp2p id whose Phase-0 hash is peer_id.
pub struct PeerDescriptor { pub device_pubkey:Vec<u8>, pub peer_id:[u8;32], pub addresses:Vec<String>, pub seq:u64, pub signature:[u8;64] }
  verify_self() -> bool;

// Bounds (all hard caps; Default suits a desktop node). past:8, commit_log:256,
// pending:256, gap:1024, peers:64, catchup_queue:256, outbox:256.
pub struct SyncConfig { max_past_epochs:u64, max_commit_log:usize, max_pending_commits:usize,
                        max_commit_gap:u64, max_known_peers:usize, max_catchup_queue:usize, max_outbox:usize }
pub struct SyncStats { commits_applied, commits_buffered, commits_served, commit_catchups_requested,
                       ops_ingested, ops_recovered_past_epoch, ops_dropped_future_epoch, ops_dropped_old_epoch,
                       doc_catchups_requested, requests_rejected: u64,
                       /* gauges: */ past_keys_retained, pending_commits, commit_log_len,
                       known_peers /*untrusted candidates*/, member_peers /*proven members*/ : usize }

// RoutingState: the routing label L + retained ns_secret_L history, transferred to a
// joiner in the join response (sealed, signature-bound). Opaque; pass to new_joined.
pub struct RoutingState;

pub async fn request_join<T: MeshTransport>(transport:&T, inviter:PeerId, device:&MlsDevice, invite:&InviteToken)
    -> Result<(ServerGroup, RoutingState), SyncError>;  // join over the wire; authenticates the inviter (binds the sealed routing transfer) + rechecks group_id
pub async fn request_join_from_reply<T: MeshTransport>(transport:&T, first_contact:PeerId, inviter:PeerId,
    device:&MlsDevice, invite:&InviteToken, reply_joiner_nonce:[u8;16], reply_joiner_peer:&[u8],
    clock:&dyn Clock, expires_at_ms:u64)
    -> Result<(ServerGroup, RoutingState, PeerId), SyncError>;  // proves each contact before disclosing the bearer invite; ignores hostile contacts until an inviter-signed Welcome arrives
pub async fn request_join_from_switchboards<T: MeshTransport>(transport:&T, first_contact:PeerId,
    allowed_contacts:&[(PeerId,u64)], inviter:PeerId, device:&MlsDevice, invite:&InviteToken,
    signed_join_plan:&[u8], clock:&dyn Clock)
    -> Result<(ServerGroup, RoutingState, PeerId), SyncError>;

// Additive standing-assistance wire types. PeerDescriptor v1 remains byte-for-byte strict.
pub struct SwitchboardOffer { group_id, device_pubkey, peer_id, addresses, seq, expires_at_ms, signature }
pub struct SwitchboardRoute { offer:SwitchboardOffer }
pub struct InviteJoinPlan { invite:InviteToken, inviter_peer, switchboards, signature }
// The desktop labels the outer envelope `mewtual-invite-v3:`. New clients accept both that form
// and plain invite hex; old strict readers reject v3 rather than silently confusing helpers with
// the inviter. Each route retains the helper's own signed two-minute offer under the inviter's
// outer endorsement, so the inviter cannot replace its addresses or extend consent. A helper
// requires local opt-in plus a live exact inviter peer record, checks both signatures and the
// deadline, forwards only KIND_JOIN/KIND_WELCOME, and applies the resulting Add before returning
// the inviter-signed Welcome.
```

**Recovery internals (private to `catcoms-sync`, for orientation):** `commit_log`
(VecDeque, served to peers) · `pending_commits` (BTreeMap, out-of-order buffer,
ordered replay when the gap fills) · `past_keys` (BTreeMap `(DocType,doc_id,epoch)`
→ `Zeroizing<[u8;32]>`, captured by `snapshot_epoch_keys` *before* each advance) ·
`routing_secrets` (BTreeMap `L → Zeroizing<[u8;32]>`, `{L-2,L-1,L}`, the source of the
blinded topics + namespaces) · **two-pool peers:** `known_peers` (untrusted candidates)
vs `member_peers` (promoted via a verifying *signed* catch-up; preferred). Each transient
`member_peers` entry retains the roster `DeviceId` that signed its request-bound response, so an
MLS removal invalidates the departed signer's proof without discarding unaffected live peers ·
`catchup_queue`.

---

## 6b. Discovery & eclipse-resistance  *(catcoms-discovery; pure, no I/O, no ambient time/RNG)*

```rust
// The ONLY thing that decides what to dial; the net Actor never auto-dials. Ranks
// candidates into a bounded dial plan and consumes a Clock-paced, RNG-jittered budget.
pub struct DiscoveryPolicy;  new() / with_config(PolicyConfig);  remaining_budget()->u32;
  plan(candidates:Vec<Candidate>, roster_size:usize, &dyn Clock, &mut impl CryptoRngCore) -> Vec<PlannedDial>;
  // ranking: tag-verified member > multi-rendezvous corroboration > cache > raw junk (never dropped);
  // ≤1 trust root/rendezvous; round-robin interleave; roster clamp; seq-freshness (drop stale/replayed).
pub enum Source { Rendezvous(PeerKey), Pex(PeerKey), Cache }   pub type PeerKey = Vec<u8>;
pub enum FreshnessPrincipal { Device(PeerKey), Transport(PeerKey) }
pub struct Candidate { peer:PeerKey /*canonical transport merge/dial key*/, addresses:Vec<String>,
                       source:Source, freshness:FreshnessPrincipal, seq:u64, tag_verified:bool }
// `seq` is compared only within its verified signer domain: device-signed PeerDescriptor cache
// rows use Device(device id); transport-signed rendezvous PeerRecords use Transport(peer id).
pub struct PlannedDial { peer:PeerKey, addresses:Vec<String> }
pub struct PolicyConfig { dial_budget, window_ms, jitter_ms, roster_headroom, min_dial_slots, max_addresses, max_tracked_peers }

// Advisory eclipse warning; NEVER gates messaging or a Remove (it has no gate path).
pub struct EclipseDetector;  new(EclipseConfig);  level()->EclipseLevel;  observe(EclipseObservation, &dyn Clock)->EclipseLevel;
pub enum EclipseLevel { Ok, Caution }
pub struct EclipseObservation { roster_size /*R*/, reachable_devices /*D, incl self*/, trust_roots /*S*/ : usize }
pub struct EclipseConfig { roster_floor, min_reach:f64, min_sources, grace_ms, clear_ms }  // suspect = R>floor && (D-1)/(R-1)<min_reach && S<min_sources, hysteretic

// Cross-session cache of proven members (first-contact eclipse). SQLCipher backing deferred.
pub struct AddressCache;  new(CacheConfig);  insert(CachedPeer, &mut impl CryptoRngCore);  get(&PeerKey)->Option<&CachedPeer>;  candidates()->Vec<CachedPeer>;
  to_bytes(&[u8;32])->Vec<u8>;  from_bytes(&[u8], &[u8;32], CacheConfig)->Result<Self,CacheError>;  // BLAKE3 keyed tag → tamper-detected (constant-time) on load
pub struct CachedPeer { peer:PeerKey /*device-id storage key; not Candidate.peer*/, addresses:Vec<String>, seq:u64, record:Vec<u8> }
```

---

## 7. Wire formats & protocol kinds (the on-wire schema)

All multi-byte ints big-endian; all variable fields length-prefixed (`catcoms-wire`).

- **Routing-keyed derivations** (6e-3d): every gossip topic + rendezvous string is now
  `BLAKE3_keyed(ns_secret_L, …)` over a **canonical length-prefixed** preimage (so an
  invite-holding non-member can't compute them; they rotate on member **removal** via the
  label `L`). Channel topic: `keyed(ns_secret_L, "catcoms/topic/v2" ‖ group_id ‖ u64 slot ‖ u16 type ‖ u128 id)`.
  Control topic: `keyed(ns_secret_L, "catcoms/control/v3" ‖ group_id ‖ u64 slot)`.
  Rendezvous namespace: `"catcoms1-" ‖ hex(keyed(ns_secret_L, "…/rendezvous/ns/v1" ‖ group_id ‖ slot ‖ rz_peer)[..20])`.
  Pre-dial **membership tag**: `keyed(ns_secret_L, "catcoms/rz-tag/v1" ‖ group_id ‖ slot ‖ rz_peer ‖ peer_id ‖ u64 seq)[..16]`.
  Pre-join **`join_ns`**: `"catcoms1-" ‖ hex(keyed(derive_key("…/join-rz/hkdf/v1", invite_nonce), "catcoms/join-rz/v1" ‖ group_id ‖ rz_peer)[..20])`.
- **`SealedOp`** (gossip payload): `u16 doc_type ‖ u128 doc_id ‖ u64 epoch ‖ bytes nonce(24) ‖ bytes ciphertext`.
  Ciphertext = XChaCha20-Poly1305 of the encoded `SignedOp` under `channel_secret(doc,epoch)`.
  - **The edit is the acceptance point, not the broadcast.** Everything that can legitimately
    refuse a `post` happens before the edit is applied: an unopened document, a missing routing
    secret, a sealing or automerge failure. Past that the operation exists with a stable change
    hash in a document this process will serve and persist, so a refused *publication* is a
    delivery still owed rather than a failed post; it is queued in the same bounded `outbox` as a
    membership commit and retried on the next tick. Reporting it as an error told every caller
    above that nothing had happened while something had: the app layer skipped delivery tracking,
    the actor skipped its UI delta and the desktop skipped its save, while the message sat in the
    channel, so retyping it minted a second copy under a new id and both surfaced later. After
    the transport's own classification (see `MeshService`), the only publication failure that
    still reaches this layer is the command channel being closed, i.e. a send racing shutdown.
- **`CommitRecord`** (control payload): `bytes group_id ‖ u64 commit_epoch ‖ bytes committer_device(32) ‖ bytes mls_commit ‖ bytes base_auth(32) ‖ bytes committer_sig(64)`.
- **Request/response** (`ProtocolId("/catcoms/rr/1")`): first payload byte = **kind**:
  - `0` KIND_CATCHUP; **authed** body wrapping `u16 doc_type ‖ u128 doc_id`; response = op bundle,
    bare, with **no marker byte in front of it**. This is the shape every build understands, and
    it is frozen: it predates `KIND_CATCHUP_SINCE` and is the only catch-up a peer built before
    that kind existed can ask for or parse. It carries no way to say "there is more", so it is
    capped at the 16 MiB response ceiling rather than at the paging chunk: a peer that cannot be
    told to page has nothing that re-asks after a short answer, and its only route back would be
    the next gossiped op whose dependencies it lacks. Because of that ceiling it gets its own
    deadline, `FULL_CATCHUP_REQUEST_MS`, sized to the same ~1 Mb/s floor the chunk budget leaves;
    `CATCHUP_REQUEST_MS` applied to a legal maximum response would demand roughly 67 Mb/s. Two
    current peers never spend that deadline on each other, because this request is only reached
    when the paged one came back empty or with a marker we do not know.
  - `19` KIND_CATCHUP_SINCE; **authed** body wrapping `u16 doc_type ‖ u128 doc_id ‖ u32 count(≤64) ‖
    32-byte change hashes`; response = `[marker] ‖ op bundle`, carrying only the ops behind the server's
    frontier and not behind the hashes named. The hashes are the requester's automerge heads **plus
    their immediate ancestors** (`EncryptedDoc::sync_frontier`): a member that wrote while it could
    not reach anyone has a head nobody else has seen, and a peer that cannot see a hash can subtract
    nothing behind it, so naming the parents keeps the exchange incremental. Sound because holding a
    change means holding its dependencies, so anything causally behind a named hash is already held.
    A hash the server does not know selects nothing. Members-only on exactly the same terms as
    `KIND_CATCHUP`.
  - **The authenticated request binds the transport peer it is sent from**, for
    `KIND_CATCHUP_SINCE` and **only** for it. The requester's own peer id goes into the request
    transcript, and the server rebuilds it from where the bytes actually arrived, so a request
    relayed verbatim by a third endpoint fails to verify rather than being answered on that
    endpoint's behalf. Without it, an endpoint could forward a member's live request to a real
    member, collect the answer signed against it, and present that answer as evidence about
    itself.
    A transcript is what a signature is over, so adding a field to one changes what an older build
    computes: `KIND_PEX` and `KIND_COMMIT_CATCHUP` exist on the released build and keep signing
    exactly what it signs, or a mixed pair fails in both directions — and a commit catch-up that
    stops verifying is a member stuck at an old epoch, unable to open anything sealed under the
    new one. They instead stop being a way to *prove* anything about a transport peer: a proof from
    them is recorded as unbound (see `ProvenMemberPeer::bound`). An unbound proof may **prefer** a
    source and nothing else: it cannot be an obligation a document's search waits on, cannot let an
    unsigned empty legacy answer discharge one, and cannot displace a binding proved at both ends
    on either axis (the same device from another peer, or another device at the same peer). A
    refusal is decided before anything is touched, so a claim that will be refused cannot clear the
    state it was going to replace on the way to being refused; an accepted replacement carries the
    displaced identity's continuation claim with it rather than leaving one naming an endpoint the
    member no longer answers at. Reciprocal forwarding is excluded from binding for a different
    reason: it relays on somebody's behalf by design.
    **Not yet gated on `bound`:** `proven_member_path`, `has_proven_connected_member_peer` and the
    reconnect sweep still accept an unbound proof, as they did before this work. PEX is how most
    peers become proven at all, so requiring a bound proof there would leave a node with no member
    path until a document catch-up happened; tightening it is its own change with its own behaviour
    to re-establish, and is tracked with the periodic anti-entropy work.
  - **The `KIND_CATCHUP_SINCE` response is responder-signed**, like commit catch-up, PEX and blob
    fetch: `bytes responder_pubkey ‖ bytes signature(64) ‖ bytes answer`, where `answer` is
    `[marker] ‖ op bundle` and the signature covers a transcript under
    `catcoms/doc-catchup-resp/v1` binding the group id, the **requester's** key, the **responder's
    transport peer**, and the request's timestamp, nonce and epoch. The requester rebuilds the
    responder peer from the one it actually contacted, so an answer cannot be attributed to an
    endpoint other than the one that produced it. The bundle inside is capped at
    `MAX_SIGNED_CATCHUP_BUNDLE`, the response ceiling less the envelope, because the ceiling
    limits the frame that leaves the node rather than the bundle within it: sizing the bundle to
    the whole ceiling produced answers that were legal to build and impossible to send. The requester rejects an answer whose signer is not a current
    roster member, or whose signature does not verify against the request it actually sent. Sealed
    operations authenticate their own authors, but an **empty** bundle carries no such proof and
    the marker in front of it is durable control state: it can end a search, open a claim on a
    document, or push the requester between sources. Catch-up sources are deliberately drawn from
    untrusted candidates as well as proven members, so that state carries membership proof of its
    own. An empty response is the one reply that needs none, because it asserts nothing.
  - **`KIND_CATCHUP_SINCE` response markers**, which are a continuation signal and not a version:
    - `1` `CATCHUP_SINCE_UNDERSTOOD`: this bundle is everything the server was holding for you.
    - `2` `CATCHUP_SINCE_MORE`: the bundle was cut to the budget and unsent ops remain; ask again.
    - empty response (no marker at all): "I do not know this kind". A peer built before kind 19
      existed answers every unknown kind this way, which is how a requester detects it and re-asks
      with `KIND_CATCHUP`.
    - **any other first byte**: treated exactly like the empty response, i.e. as a peer that
      cannot page for us, and never as a malformed response. A malformed reading would propagate
      out of `request_catchup` and take the whole-history fallback with it, stranding the document
      against a peer that was answering perfectly well. This is what lets a later build add a
      marker without breaking the population installed today. It is a defensive downgrade, not a
      guarantee of convergence: what it buys is another route to try, and the fallback it takes is
      the compatibility grammar with the costs described under `KIND_CATCHUP`.
    - `3` `CATCHUP_SINCE_ABSENT`: understood, and this peer does not hold the document at all.
      Deliberately **not** `1` with an empty bundle, and deliberately not silence. Silence reads as
      an older build and takes the compatibility path; `1` reads as "the document is complete",
      which retires the gap on the strength of an answer that was never about it. Peer selection is
      group-wide, not per document, so the best source known can easily be an honest member who
      never opened that channel. The requester keeps the task, sets that source aside, and asks
      somebody else.
    - **Only the peer that claimed a continuation can retire it.** A `2` **that moved the
      requester's frontier** records that peer as the claimant for the document (for
      `CATCHUP_CONTINUATION_TTL_MS` of elapsed time, so a claimant that never returns cannot pin
      the task forever). While that claim stands, a `1` from any *other* peer does not close the
      task: it says what that peer has, not what the claimant was withholding. The drain also asks
      the claimant first. Without this, one peer's zero-progress completion erased another's proven
      continuation, and a modified peer could skip the non-progress bound entirely by answering `1`
      instead of `2`.
    - A `2` that moved nothing neither mints nor refreshes a claim, and exhausting the
      non-progress budget **revokes** the claim as well as cooling the peer off. Otherwise a
      claimant could keep a document open indefinitely by answering "ask me again" on a timer:
      the cooldown paused it, the ten-minute claim was renewed on its next round, and no other
      source's completion could ever end the search.
    - **`1` with an empty bundle is one source's answer, not the group's.** A member that has
      opened the channel but not yet caught it up holds a real, empty document, so it answers `1`
      rather than `3`, and that answer is true and useless. Each such answer marks that source as
      checked at the current local version; the document is treated as caught up only once every
      eligible source has said it at the same version, and any answer that moves the version
      starts the sweep again. A later member connection is a natural reason for another sweep.
      **A cooling source still owes the sweep an answer**: being unavailable for a moment is not
      the same as having answered, and treating it as the same lost a document to an ordinary
      schedule (a cancelled tick cools the peer it was mid-request with, and the next empty replica
      to answer would find nobody outstanding). The picker excludes both cooling and
      already-checked sources from an attempt; only the second is a discharged obligation. Only
      **proven** members are owed to, so an unproven candidate cannot pin a sweep open by staying
      connected and saying nothing; the exception is a node with **no** proven source at all
      (a fresh join or restore), where nothing that answers has proved it was entitled to, so each
      connected candidate is asked once before the search can end.
      A source's answers are forgotten when it **reconnects** and when it is **newly proven**.
      Otherwise a peer could leave, write something, return, and be excluded from the very sweep
      its reconnect queued; and a peer that was only a candidate when it answered would keep that
      answer after becoming a source the sweep must hear from. Forgetting only forgets: queueing
      the work is left to `sweep_docs_on_reconnect`, which declines to aim a whole-node sweep at a
      peer that has not proved it can serve one.
      **A restore also sweeps, once, on its first bound proof.** `member_peers` is session-local,
      so a node that restores with documents on disk reaches its first connection with nothing
      proven and correctly declines the sweep there; `restore` records the obligation
      (`first_proof_sweep_owed`, set only when a snapshot brought documents back) and
      `promote_member_peer_bound` discharges it. Without that, recovery depended on a further
      connection edge that a stable link never produces, and a room that stayed quiet remained as
      short as the restore left it for the whole session; restoring, opening one channel and
      finding the rest still short is the user-visible shape.
      Two conditions, both load-bearing. **Bound only**, because a bound proof is exactly a peer
      that has served *this node* a document catch-up, so its membership check ran and passed:
      PEX and commit catch-up promote unbound, a peer still mid-join answers both, and sweeping
      on those aims members-only recovery at the joiner and blocks the loop on a reply it cannot
      give; that is the connection-time deadlock reached one step later. **Once, and only after a
      restore**, because a session that founded or joined has nothing older than itself to
      recover, and a whole node's documents in front of its discovery and presence work costs
      those the ticks they converge in.
      **Known gap.** A source that quietly advances while staying connected and never changing
      standing is still not re-swept. Closing that needs a bounded periodic anti-entropy pass,
      spread across documents and peers.
      One member device holds at most one proof, and one transport peer at most one device, so a
      device cannot manufacture sweep obligations or evict honest proofs by answering from many
      identities.
    - **A `1` that carried operations discharges only its own sender's claim.** Carrying content
      proves that peer had something to give, not that it knows what a different peer is still
      withholding, so it restarts the version sweep but leaves another peer's claim standing.
    - **`KIND_CATCHUP` answers go through the same rule.** Its bundle is bare and unsigned, so an
      empty one cannot say who sent it: a zero-operation legacy answer marks the source checked
      only when that transport peer already carries an authenticated current-member binding, and
      otherwise just cools it and continues. Without that, declining to page and then answering
      with nothing was a way around the whole state machine, available to any peer on the wire.
    - Continuation is the **server's** answer, because only it knows what it withheld. A requester
      that inferred it from "did I apply anything" stopped on a chunk of ops it already held, and
      on a chunk that could carry nothing because the next op was larger than the budget.
  - **`KIND_CATCHUP_SINCE` serves are capped at `MAX_CATCHUP_CHUNK` (256 KiB)**, not at the 16 MiB
    a response may legally reach. The cap is what makes `CATCHUP_REQUEST_MS` a deadlock bound
    rather than a throughput requirement: without it, a valid connection that could not move a
    whole history inside the deadline would time out, retry, and time out again, so a large gap
    could never converge at all. A non-empty remainder always serves **at least one op**, up to the
    16 MiB ceiling, so an individually oversized op travels alone instead of being silently
    withheld: a bundle of zero ops is how the wire says "nothing for you", and a peer still holding
    something must never send one.
  - **Requester termination and non-progress.** The exchange ends on `1`. A `2` that applied
    nothing is legitimate (the frontier on the wire is capped, so an honest peer can replay a
    prefix already held), so it is believed and asked again; but an unbroken run of
    `MAX_NONPROGRESSING_CATCHUP_ROUNDS` (8) such rounds from one peer for one document marks that
    peer under a per-document cooldown, so the next drain prefers another source. That bounds the
    one loop an authenticated member can drive by itself, where every answer says "ask me again"
    and no answer carries anything usable. The run resets the moment that peer applies anything.
    Per document rather than globally on purpose: a peer that cannot serve one document may be the
    only holder of another, and the global failed set is cleared by any inbound traffic from that
    peer, so it could never be a durable statement about a document anyway.
  - **Request order, always**: `KIND_CATCHUP_SINCE` first, *including* when the frontier is empty
    (which subtracts nothing and so asks for everything, still in bounded chunks), falling back to
    `KIND_CATCHUP` only on an empty or unrecognised-marker response. Asking the paged way only
    when there was something to subtract sent the commonest case there is, a freshly opened or
    freshly joined document, to the largest response in the protocol under the shortest deadline.
  - **A detected gap stays owned until it is filled.** The queue keeps the task across every
    outcome that is not an authoritative "that is everything": a timeout, a refusal, a malformed
    answer and an exhausted non-progress allowance all mark the peer and leave the work queued for
    the next drain to hand to another source. The task in flight lives in `catchup_inflight` on
    the synchronizer rather than on the stack, because the actor cancels its tick whenever a
    command arrives, and a quiet document has no later inbound op to rediscover a lost gap with.
    A restored task is put back ahead of the queue cap, since it was admitted before the drain took
    it and the command that cancelled the tick may have taken its slot.
  - **A UI-facing catch-up never waits on the compatibility request.** `request_catchup` (what the
    open-channel and explicit catch-up commands call, awaited inline by the actor) stops after the
    paged attempt and queues the document instead; only the drain follows through to `KIND_CATCHUP`
    and its two-minute deadline. Otherwise a peer that answers "I cannot page" and then withholds
    could hold a server's actor, and everything it serves, for that whole window. `request_catchup`
    also queues the document when its own attempt fails, so a named request made outside the drain
    (an explicit catch-up, a channel discovered through a join contact) does not lose the gap to a
    timeout or a malformed answer.
  - `1` KIND_JOIN; body `bytes invite.encode() ‖ bytes key_package`; response =
    `[JOIN_READY] ‖ bytes welcome ‖ bytes signature(64) ‖ bytes sealed_routing` (the admitter signs
    `join_transcript = "catcoms/join-resp/v1" ‖ group_id ‖ nonce ‖ welcome ‖ sealed_routing`). Not member-authed.
  - `2` KIND_COMMIT_CATCHUP; **authed** body wrapping `u64 from_epoch`; response is **responder-signed**:
    `bytes responder_pubkey ‖ bytes sig(64) ‖ bytes bundle`, sig over `"catcoms/catchup-resp/v1" ‖ group_id ‖ requester_pubkey ‖ u64 req_ts ‖ nonce(16) ‖ u64 req_epoch ‖ bundle`.
    The drain judges the exchange by what it **established** (`CommitCatchupOutcome`), not by how
    far the epoch moved: `Verified` (a current member signed a decodable bundle), `Empty` (no
    bundle at all) or `Unanswered` (failed, timed out, oversized, undecodable, or not signed by a
    current member). Only an answer can retire a task. A non-committer's proactive probe on
    `PeerConnected` carries no proven gap and a restarted node has nothing buffered, so every
    other term of "finished" is already true there; treating a timeout as a completion meant an
    ordinary member that missed an epoch while offline discarded its own recovery on the way back.
    `Empty` still counts as an answer, because it is exactly what an up-to-date member sends: the
    wire does not distinguish "nothing from `from_epoch`" from "refused", and the variant is kept
    separate so that conflation is visible where it is relied on.
  - `4` KIND_PEX (6e-3d-7); **authed** body (empty); response responder-signed like commit catch-up but under
    `"catcoms/pex-resp/v1"`; bundle = `u32 count(≤64) ‖ len-prefixed PeerDescriptor`s, each self-signed under `"catcoms/peer-record/v1"`.
  - `14` KIND_RECIPROCAL_FORWARD; authed exact requester/target descriptor references + random
    attempt + expiry. A proven connected helper checks both live paths, rate-limits, signs the
    original frame and queues kind 15 delivery. The sender does not await the response.
  - `15` KIND_RECIPROCAL_DELIVERY; connected-only helper-attested original kind-14 frame. The
    target rechecks helper/requester membership, both exact current descriptors, signature,
    expiry/replay/rate bounds, then queues a later direct peer-bound dial intent.
  - `16` KIND_INDIRECT_PROBE; authed exact target descriptor + attempt + expiry. A connected helper
    queues kind 17 from current proven path state and never dials.
  - `17` KIND_INDIRECT_RESULT; separately authed exact target hash + echoed attempt + boolean. It
    is accepted only from the pending proven helper while the target descriptor remains current.
    Two distinct negatives are suspicion only; one positive may queue kind 14 through that helper.
  - `18` KIND_DELIVERY_RECEIPT; connected-only authed document type, document id, and 32-byte
    change hash. A receiver queues it only when that exact signed op is newly applied, and the
    sender accepts it only for one of its bounded recent targets. Duplicates are inert; unknown
    hashes cannot allocate state. It proves that member device received the op, never that a human
    displayed or read it.
  - **Authed body** (members-only gate): `bytes inner ‖ bytes requester_pubkey ‖ u64 timestamp_ms ‖ bytes nonce(16) ‖ u64 req_epoch ‖ bytes signature(64)`,
    signature over `"catcoms/catchup-auth/v1" ‖ group_id ‖ u16 kind ‖ inner ‖ requester_pubkey ‖ timestamp_ms ‖ nonce ‖ req_epoch`.
    Served only if `requester_pubkey` content-addresses a **current member**, the timestamp is fresh (`MAX_REQUEST_AGE_MS` 60s),
    and the signature verifies. The per-request **nonce + epoch** (6e-3d-6) bind the responder's signed reply to *this exact*
    request, so a captured response cannot be replayed (closes the same-ms `ts`-collision window).
  - Caps: requests 64 KiB; responses 16 MiB (catch-up) / **512 KiB (PEX)**; bundle element counts capped.
- **`InviteToken`** signed payload (v2): `"catcoms/invite/v2" ‖ group_id ‖ inviter_device_id ‖ inviter_public_key ‖ nonce ‖ u64 expires ‖ u32 n ‖ n×bootstrap_str ‖ u32 m ‖ m×rendezvous_str`, then `signature(64)`.

**Member recovery code.** The text prefix is `mewtual-reconnect-v1:` followed by the hex encoding
of a canonical group id, member device public key, transport peer, candidate list, nonce, issue and
expiry times, and the member signature. The whole text is at most 8 KiB, has at most four 512-byte
direct literal-IP TCP/QUIC candidates, lives exactly ten minutes, and tolerates at most 30 seconds
of future skew. Every route terminates in the signed transport peer; the signing device must still
be in the receiver's current MLS roster. This is not an invite or a membership operation.

## 8. `DocType` tags (stable; only append)
`Channel=1, Wiki=2, Status=3, Calendar=4, InviteLedger=5, MemberRoles=6, FileIndex=7, Routing=8,
Profile=9, Livery=10, Badges=11, Devices=12, ChannelIndex=13, Moderation=14,
StudioIndex=15, StudioObject=16, PostReplies=17, DocRegistry=18`.
Exporter context = `u16 tag ‖ u128 doc_id` (18 bytes, fixed-width → injective). `Routing` has no content
doc; it feeds the **metadata** exporter label to derive the per-removal `ns_secret_L`.

`Status` document compatibility is additive. Older posts remain maps in the root `messages` list.
New posts are message-schema maps stored directly at distinct root keys
`"status_post/" ‖ random_post_id`; readers and post mutators accept both layouts. The keyed layout
avoids concurrent first authors independently creating conflicting `messages` list objects and
silently hiding one branch after merge. Status readers enumerate **all** legacy `messages`
conflicts with Automerge `get_all`, so an already-conflicted old feed recovers both branches.
Addressable ids are exactly 32 lowercase hex characters and must resolve to exactly one object
across both layouts; ambiguous/malformed keyed rows are hidden and mutations fail closed. Feed
order is materialized deterministically by `(timestamp, post_id)`. The root `members_may_post`
scalar remains the posting-policy field. Mixed-version limitation: older clients know only the
legacy list and therefore do not display posts authored in the keyed layout; clients must upgrade
to participate in the new status feed. New clients intentionally do not dual-write, because doing
so would recreate both the container race and cross-layout id ambiguity.

---

## 9. Moderation, storage health and desktop continuity  *(catcoms-app / Tauri bridge)*

```rust
pub struct StorageHealth {
    listed_files:usize, referenced_chunks:usize, verified_chunks:usize,
    missing_chunks:usize, unreadable_chunks:usize, invalid_manifests:usize,
    verified_bytes:u64, has_peers:bool,
}
pub struct StorageRepair { attempted_chunks:usize, recovered_chunks:usize, health:StorageHealth }
impl Server {
    fn storage_health(&self) -> StorageHealth;
    async fn repair_storage(&mut self) -> Result<StorageRepair,AppError>;

    async fn open_moderation(&mut self) -> Result<(),AppError>;
    async fn request_moderation_catchup(&mut self, peer:PeerId) -> Result<usize,AppError>;
    fn moderation_state(&self) -> ModerationState;
    async fn warn_message(&mut self, channel:u128, message_id:&str, reason:&str) -> Result<String,AppError>;
    async fn create_kick_case(&mut self, target:&str, reason:&str, evidence_ids:&[String]) -> Result<String,AppError>;
    async fn cast_kick_vote(&mut self, case_id:&str, yes:bool) -> Result<(),AppError>;
    async fn resolve_kick_case(&mut self, case_id:&str, remove:bool) -> Result<(),AppError>;
    fn delivery_snapshot(&mut self, channel:u128) -> DeliverySnapshot;
}
pub struct DeliverySnapshot { revision:u64, states:Vec<DeliveryState> }
impl ServerStore {
    fn open(dir:impl AsRef<Path>, passphrase:&[u8], rng:&mut impl CryptoRngCore) -> Result<Self,AppError>; // owns lifetime installation lock
    fn verify_passphrase(&self, passphrase:&[u8]) -> Result<(),AppError>; // verify-only; no second mount
    fn save_ui_state(&self, json:&[u8], rng:&mut impl CryptoRngCore) -> Result<(),AppError>; // ≤1 MiB, vault-sealed + atomic
    fn load_ui_state(&self) -> Result<Vec<u8>,AppError>;
    fn load_epoch_recovery(&self, server:u64, document:&LogicalDocument) -> Result<EpochRecoveryState,AppError>;
    fn update_epoch_recovery(&mut self, server:u64, document:&LogicalDocument, action:EpochRecoveryAction,
        clock:&dyn Clock, rng:&mut impl CryptoRngCore) -> Result<EpochRecoveryUpdate,AppError>;
    fn backup_source_dir(&self) -> &Path;
    fn change_passphrase(&self, current:&[u8], new:&[u8], rng:&mut impl CryptoRngCore) -> Result<(),AppError>;
}
```

`catcoms_app::store::{EpochRecoveryAction, EpochRecoveryState, EpochRecoveryUpdate}` persist P1's
recovery slots, not settlement itself. Actions are `Stage(RecoverySnapshot)`,
`Acknowledge { oldest_snapshot, staged_snapshot }`, and `AdvanceTime`. State exposes read-only
`retained()`, `staged()` and `eviction_pending()`; the update returns that saved state and the
`RecoveryTransition`. Full local-server/group/type/key scope is inside the sealed record as well
as its domain-separated filename derivation. Exclusive mutable store access serializes reload,
transition and replacement. Even idempotent retries re-save before returning, including exact
acknowledgement retries after `CommittedButNotDurable`; stale acknowledgements cannot promote a
newer warning. Reading alone never expires recovery. The registry store installation below now
uses it before checkpoint selection/source pruning; actor/bridge, live inventory bootstrap and
removal cleanup remain unwired.

`catcoms_app::store::epoch_budget` adds `StorageScope`, `Footprint`, `StorageRecord`,
`Replacement`, `WritePurpose`, `EpochStorageBudget`, and `BudgetError`. A budget is constructed or
reconciled from a **complete trusted local inventory**, including temporary/orphan records. Its
1984 MiB content, 16 MiB protocol and 48 MiB settlement pools sum to 2 GiB. `reserve` verifies
permanent and peak old+replacement+scratch occupancy without early deletion credit, marks the
budget unready before returning an exclusive guard, and rejects another document while staged
bytes pin the reserve. `commit` follows successful durable I/O; `cancel_before_write` is valid only
before I/O. Dropped/forgotten guards or uncertain writes require reconciliation. There is a local
65,536-record metadata rail; it does not alter replicated registry admission. Empty crash-orphan
files consume a slot too. An over-rail inventory refuses reconciliation until bounded cleanup;
discarding entries from accounting to fit the rail is not safe.

`ServerStore::epoch_recovery_inventory_record(server, document)` returns **one** authenticated
physical inventory entry, not a complete inventory. `update_epoch_recovery_accounted` takes the
same arguments as the low-level save plus `&mut EpochStorageBudget`, verifies the old pool split
and owner, reserves the entire replacement, saves, then commits counters. Failed reads/writes
invalidate accounting; matching lengths alone do not prove a matching pool split. The existing
`update_epoch_recovery` remains explicitly unaccounted for bootstrap/tooling. A coordinator must
own the sole budget, exclude unaccounted writers, and provide complete inventory discovery before
production use; this API alone neither enforces a vault-wide policy nor settles multiple records.

`ServerStore::scan_epoch_recovery(&mut self)` returns an `EpochRecoveryScan` borrowing the store
exclusively through all steps. `step()` visits at most 64 directory entries and authenticates at
most one bounded recovery record; it returns count-only `RecoveryScanProgress`. EOF alone permits
`finish()` to return `EpochRecoveryInventory`. Errors, exhausted rails and caught parser panics
permanently poison a scan. The job caps all traversal at 131,072 entries (ignored legacy files
included), recovery files plus temporaries at 65,536, and total authenticated bytes at that record
count times the per-record sealed cap. It stores metadata only, not recovery bodies.

Discovery verifies the sealed full server/group/type/key scope and its canonical filename without
consulting the current server registry, so removed-server recovery is not omitted. Canonical
staging siblings are observed, not promoted/deleted; their bodies are never read. Attribution uses
verified destinations after EOF, independent of traversal order. `records_for_server(server,
group)` returns **recovery-only** storage inputs and refuses if any orphan lacks a verified
destination. Known temporary bytes are settlement scratch, even when empty; incompatible pinned
documents or over-cap usage still refuse budget construction until cleanup. The completed view is
not a write lease or a complete P1 inventory: the future coordinator must exclude intervening
writes, inventory all other managed types and resolve orphan cleanup before production wiring.

`ServerStore::cleanup_epoch_recovery_staging(&mut self)` returns an exclusive
`EpochRecoveryCleanup`. Its count-only `step()` result, `RecoveryCleanupProgress`, names visited
entries, removed files and their **observed ciphertext bytes**, not filesystem free space. Each
step visits at most 64 names, with the same 131,072-entry traversal rail as inventory, and removes
only exact canonical recovery-write staging siblings. It does not read bodies or touch published
recovery files, even corrupt ones; logical staged snapshots stay inside their final record.
Failure/panic poisons completion and can leave partial deletions. Every successful batch/EOF,
including an empty retry, runs the existing parent sync (Unix-only durability). A successful EOF
means that traversal ended, not that directory iteration during deletion found every orphan.
`into_inventory()` requires successful EOF and transfers the exclusive borrow to a fresh scan;
callers repeat cleanup if necessary and reconcile complete current inventories before refunding
any budget. This standalone operation has no startup, actor/bridge or network invocation yet.

`ServerStore::{load_epoch_owner_receipts, prepare_epoch_owner_receipt,
mark_epoch_owner_receipt_published}` add the owner-local publication journal. Preparation takes a
local server id, signed `Receipt`, current `ServerGroup`, externally observed tenure-start epoch,
RNG and complete `EpochStorageBudget`; completion takes server/document, exact receipt hash, RNG
and budget. Both reload and atomically replace under exclusive store access, returning
`EpochOwnerReceiptState` only after save success. Its `pending()` and `published()` references are
historical state, not current-authority capabilities. Exact completed retries preserve any newer
pending decision and re-save; a strictly newer verified tenure can replace an older pending decision,
but same-tenure conflicting choices reject. `OwnerReceiptJournal::published()` exposes the latest
high-water receipt; the journal wire encoding is unchanged.

The bounded sealed `.owner-receipts` record binds local server/group/type/key. Permanent bytes
charge protocol allowance; replacement copies borrow the same logical-document settlement reserve
as recovery records. `epoch_owner_receipt_inventory_record` observes one final file only; the combined
inventory below covers its namespace and orphan temporaries. The sole coordinator and network
publication remain deferred; the recovery-only scanner still excludes owner files. Before actual sending,
the publisher must re-prepare/re-save the exact choice and recheck current owner, tenure and session.
It must validate the close and deterministic seed before signing in the first place. File-sync and
atomic replacement use the existing store primitive, with parent-directory durability on Unix only.

`scan_epoch_storage()` returns `EpochStorageScan`; `cleanup_epoch_storage_staging()` returns
`EpochStorageCleanup`. These share the recovery engine and its unchanged aggregate traversal,
metadata and per-step rails, but include both `.recovery` and `.owner-receipts` files. `coverage()`
on cleanup, scan and completed `EpochStorageInventory` is fixed as
`EpochInventoryCoverage::RecoveryAndOwnerReceipts`; the old recovery-only APIs return
`RecoveryOnly`. The old recovery type names remain aliases of the common types. None means all P1
storage or a continuing write lease. `EpochStorageScanProgress` retains `recovery_records` and adds
`owner_receipt_records`; each step authenticates at most one body TOTAL. Each namespace uses its
own bounded reader, scope domain and schema. `EpochStorageInventoryEntry::kind` and
`EpochStorageOrphan::kind()` expose the physical family; orphan attribution keys on both that kind
and digest. Owner final bytes charge protocol allowance; owner temporaries charge settlement
scratch to their authenticated destination's shared logical-document owner. Unknown ownership in
either covered family blocks per-server composition. Owner journals are historical vault state:
inventory never requires current-owner authorization and never grants publication authority.

Combined cleanup removes only canonical unpublished staging siblings, never saved pending/published
journals or recovery slots. Failure can leave partial removals; empty retries still sync. Its
`into_inventory()` keeps both exclusive access and coverage unchanged. Recovery-only APIs continue
ignoring even malformed owner-only aliases (while charging traversal); combined APIs reject them.
Other managed families, sole-writer admission, startup/actor/network integration and retention of
saved records remain separate work. No cleanup operation is automatically invoked yet.

`ServerStore::load_epoch_intents(server, document)` returns read-only `EpochIntentState::pending()`
entries in derived-id order. `prepare_epoch_intent(server, document, operation, device, group, rng,
server_budget, intent_budget)` reloads, verifies the local device's current roster signing key and
group, checks the domain envelope bounds/scope, and durably adds one intent before returning.
The caller must validate type-specific semantics first. There is no public raw-save or retirement
API. Exact duplicate intents preserve newer entries and sync the authenticated unchanged final
file plus parent without rewriting; sync-only admission reserves no bytes. New entries use ordinary
content replacement accounting. Reads alone do not repair an uncertain save or authorize replay
as a different author. File and parent durability are the existing file-sync/Unix-parent-sync model.

The new explicit `scan_epoch_storage_with_intents` and
`cleanup_epoch_storage_staging_with_intents` APIs return the same incremental jobs with fixed
`EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents`. Earlier APIs still exclude intents,
even malformed intent-only aliases. `EpochRecordKind::Intents` identifies `.intents` finals and
staging siblings; `EpochStorageScanProgress::intent_records` counts authenticated ledgers.
Namespace-specific bounds and `(kind, digest)` orphan attribution apply unchanged. Intent final
AND temporary bytes charge content, never settlement. Saved ledgers are never cleanup targets.

`EpochIntentBudget::from_inventory(&completed_inventory)` requires coverage including these three families,
counts intent files across ALL servers in the vault, and includes unknown-owner temporaries.
`reconcile` blocks the old budget on failure; `bytes()` is observed physical occupancy. The
64 MiB cap (`MAX_VAULT_INTENT_BYTES`) includes sealing/framing and peak temporary copies, a
conservative interpretation of the design's payload cap; the 65,536-record rail counts empty
intent temporaries too. A private process-local mount/generation token is retained by inventories
and budgets, and replaced before intent write/sync/cleanup I/O: duplicate budgets, prior-session
budgets and stale completed inventories cannot authorize another intent update. Exact flush
retries need neither replacement headroom nor a new RNG nonce. Both accounting guards remain
unready after errors/panics, with no guessed refunds. The token does not track other file families:
their metadata/admission and the sole complete per-server budget remain coordinator work.
`IntentLedger::document()` exposes its full scope for the enclosing store decoder's equality check.

This is a persist-before-edit prerequisite, not live editing or automatic replay. The registry
adapter below now invokes it; retirement still requires checkpoint/recovery persistence and no
actor/network path invokes these adapters yet.

### Durable registry edits, sealing and checkpoint installation (P1, not yet live-wired)

`ServerStore` now exposes:

```rust
load_registry_epoch(server, &ServerGroup, bucket:u8, &MlsDevice)
  -> Result<Option<EpochRegistryState>, AppError>;
edit_registry_epoch(server, &ServerGroup, bucket, expected_doc_id:u128, &MlsDevice, DomainOp, rng,
                    &mut EpochStorageBudget, &mut EpochIntentBudget)
  -> Result<(SealedOp, EpochRegistryState), AppError>;
ingest_registry_epoch(server, &ServerGroup, bucket, &MlsDevice, &SealedOp, rng, &mut EpochStorageBudget)
  -> Result<(Admission, EpochRegistryState), AppError>;
seal_registry_epoch(server, &ServerGroup, bucket, &MlsDevice, Receipt, tenure_start, rng, &mut EpochStorageBudget)
  -> Result<(ReceiptIngest, EpochRegistryState), AppError>;
plan_registry_settlement(server, &ServerGroup, bucket, &MlsDevice, close_bytes, tenure_start)
  -> Result<RegistrySettlementPlan, AppError>;
stage_registry_recovery(server, &ServerGroup, bucket, &MlsDevice, close_bytes, tenure_start,
                        &dyn Clock, rng, &mut EpochStorageBudget)
  -> Result<Option<EpochRecoveryUpdate>, AppError>;
install_registry_checkpoint(server, &ServerGroup, bucket, &MlsDevice, receipt_bytes, close_bytes,
                            tenure_start, &dyn Clock, rng, &mut EpochStorageBudget,
                            &mut EpochIntentBudget)
  -> Result<(RegistryInstallOutcome, EpochRegistryState), AppError>;
replay_registry_intent(server, &ServerGroup, bucket, expected_doc_id:u128, &MlsDevice,
                       intent_id:[u8;32], rng, &mut EpochStorageBudget, &mut EpochIntentBudget)
  -> Result<(RegistryReplayOutcome, EpochRegistryState), AppError>;
begin_registry_replay(server, &ServerGroup, bucket, expected_doc_id:u128, &MlsDevice,
                      &mut EpochStorageBudget, &mut EpochIntentBudget)
  -> Result<RegistryReplayPass, AppError>;
step_registry_replay(&mut RegistryReplayPass, &ServerGroup, &MlsDevice, &dyn Clock, rng,
                     &mut EpochStorageBudget, &mut EpochIntentBudget)
  -> Result<RegistryReplayStep, AppError>;
```

`EpochRegistryState` exposes only `doc_id`, `epoch`, `phase`, `op_count`, `quarantined_len` and a detached
`projection`; no mutable document or arbitrary-save API escapes. Records are versioned,
scope-bound, vault-sealed `.registry-epoch` files. Every mutation reloads and validates the full
signed source under one exclusive store borrow, checks its exact accounting record, and saves
before returning the outcome. Missing ingress may create epoch zero only. A receipt requires
current owner/tenure and bucket verification before disk work; a normally absent source refuses
without invalidating accounting, whereas loss of an inventoried source requires reconciliation.
Sealing retains all source history; saved post-seal quarantine hashes are NOT accepted edits.

Settlement preparation is read-only and accepts a canonical close of at most 4 KiB before disk
work. It reloads the checked vault unit, requires Closing and the exact selected current-owner/
tenure receipt, reconstructs only its dependency-complete closure and verifies the deterministic
seed against the receipt's expected hash. Missing source/heads, malformed closes, another close
from a current member, a wrong seed, stale authority and Fault all refuse without writes.
The plan retains the full source projection (including overflow/tombstones), included operation
ids plus their full envelopes and excluded accepted author-bound domain operations in canonical id order. Quarantined
late bodies are not accepted source work. Excluded peer operations are recovery evidence, never
permission to journal or replay as that peer. The plan's Debug excludes all content.

`source_version` fingerprints the entire normalized local restart unit, not just its receipt:
peers with the same receipt can have different excluded edits. It is serialization-version
dependent and is not a wire id, currency proof or durability/installation permit. Settlement
must reload/revalidate authority and the source version under the document gate, preflight and
persist typed recovery, then install. A plan does not guarantee the eventual recovery encoding
fits its byte cap/reservation. This API does not replace a source, write recovery, retire intents,
or finish settlement; the source remains Closing and fully retained.

`stage_registry_recovery` takes no stale plan: it reloads and verifies the source's exact accounting
record, recomputes the plan, then validates every existing registry recovery slot before a nonempty
save through the
accounted recovery adapter. It holds the exclusive store borrow throughout, with no await or source
mutation. None means no excluded operations, overflow or tombstones need a new snapshot; it is not
an installation permit or a health check of old recovery files, which that path does not load.
Some returns the saved slots/warning only after durable replacement.
`EvictionPending` still holds future installation in Closing. Exact retries preserve snapshot ids
and the warning deadline even if late packets added quarantine hashes. Failed I/O poisons accounting
and grants no success; the source remains intact. A content-full first/second snapshot may still
refuse, without crediting future source deletion. Settlement-wide capacity reservation is later.

`install_registry_checkpoint` requires bounded canonical receipt/close bytes, the exact held
current-owner decision and independently observed tenure. Under one exclusive store borrow it
flushes the checked source, validates/account-checks old recovery even for an empty new plan,
saves needed typed recovery, then holds any eviction warning. `RecoveryPending` is not success
at installing: callers surface the existing saved warning and its acknowledgement/timer actions.
Next it compares receipt-covered intent ids AND full author/domain envelopes, saves monotonic
included-only retirement, reloads/rechecks the entire source version and atomically selects the
verified successor seed. The receipt book (including repair anti-replay state) is preserved.
`Installed` returns only after the final vault barrier. Until replacement the durable Closing
source proves why included intents are final; excluded/unaccepted intents are never retired.

On an exact opening-receipt retry, `AlreadyInstalled` flushes the actual successor unchanged,
even if newer edits or a newer closing receipt arrived. It does not repeat old retirement or
reinstall the seed. Fault and stale/other receipt scopes refuse. Failed writes/flushes poison the
affected inventories; reconcile actual files before retry. No new journal or record family is
introduced. Full-quota progress is still limited by per-record content reservations and the
64-MiB physical intent ceiling: a shrinking ledger also needs its full replacement copy.
Automatic replay, repair, discovery and actor/network orchestration remain separate work.

Delayed current-owner equivocation against the retained opening receipt faults the successor
even while a newer receipt seals it. Accepted content, seed, opening receipt and high-water are
retained. Restart requires one fault-evidence member to equal that seed's exact opening receipt;
unrelated older evidence refuses. ReceiptBook v2 encodes only the new case of a retained newer
high-water above the opening-epoch pair; ordinary books remain v1, new readers accept both, and
old readers reject v2 rather than dropping its fault. No network receipt encoding changes.

Registry recovery has a versioned typed payload inside the unchanged generic `RecoverySnapshot` v1.
It carries the full source pointers, overflow and pointer-key tombstones, selected receipt hash,
and excluded author-bound domain operations; `applied_ops` is the sorted source-operation id union.
The outer base-close is the close that opened the source (None at epoch zero). Generic collection
metadata arrays are empty: registry keys are not Studio random element ids. Snapshot identity omits
the ephemeral whole-source fingerprint, quarantine and quota-owner metadata. The exact aggregate
6-MiB cap is checked before payload allocation/persistence; decoding validates the full wrapper,
scope, canonical ordering, disjoint key sets, bounds and excluded-id membership in `applied_ops`.
`RecoverySnapshot` Debug now redacts content even when nested in a generic Option/Result. These are
vault-local records, not independently signed replay requests; pointer verification and Restore,
repair/rewind-specific typed records and actor/bridge integration remain unwired.

Local editing uses two ordered barriers under the exclusive store borrow: validate the canonical
registry operation and current local author, save/flush its intent, then reload, apply and save/flush
the registry epoch. The nonce/envelope and captured concrete `expected_doc_id` are supplied by
the caller and MUST stay unchanged on retry. The id is checked before journaling and again before
the final gated edit; a retired-epoch retry refuses instead of creating a second edit after its
old log/markers were pruned. Deliberate author-owned replay targets the newly read concrete id.
`validate_local_edit` checks scope, canonical body, actor/membership, Open/epoch ceiling and any
retained-id conflict before journaling. It is not a storage or prospective projection permit.
`edit_or_reseal` repeats those checks and either authors normally through the typed projection
preflight or reseals the exact retained signed change, never reauthoring it against newer heads.
Because ids omit the body, a retained id's full envelope is compared even if no local ledger exists.

No ciphertext escapes until both records cross their barriers. A later failure keeps the already
saved intent for retry/recovery; a marker never retires it. Closing/Fault refuses both new local edits
and retries without adding intents. Restored intents must be replayed only by their original author;
this API accepts new local operations, not a foreign author's replay authorization. It prepares
ciphertext under the current MLS epoch but does not send it. The cooperative sender below owns
server-instance, membership, MLS and Open checks; native session cancellation remains its caller's
responsibility before live integration.

`replay_registry_intent` performs one bounded replay step from the saved ledger, without accepting
a body/nonce from its caller. The original author must equal the actual device, still a current
member. Unknown ids, missing/stale concrete epochs, Closing/Fault, malformed slots and mismatched
inventories refuse without new intents or outbound ciphertext. Both server and vault-intent
inventories are checked, including freshness, before the decision; every retained/staged recovery
slot is type-checked/accounted even for an exact retry. Prepared uses the normal two-barrier edit
path, preserving the saved envelope and returning ciphertext only after intent/epoch durability.

`RegistryReplayOutcome` is `Prepared(SealedOp)` or `Held(RegistryReplayHold)`; Debug redacts the
ciphertext/content. New authoring is held with `DeletedPointer` on current or retained/staged
tombstones, or `SupersededPointer` when a current admitted/overflow hint exceeds the saved Put's
epoch. Deletion takes precedence. Held retains the intent and source bytes, is not an ack or
finality/durability claim for that intent, and must not be immediately auto-looped. Registry keys
are stable and intents lack origin epochs: a deliberate later re-put can conservatively need
manual recovery. Missing deletion evidence after two-slot eviction is best-effort, not proof a key
was never deleted. No saved envelope is silently rewritten to "catch up" its hint.

An exact authenticated CURRENT signed-log match (full author/envelope, never just a marker)
instead reseals the original change unchanged, even after deletion or a higher hint. It cannot
overwrite newer state and is required to retry a Tombstone after a failed post-rename flush.
Scope, membership, Open, all recovery validation and both durability barriers still apply.
This one-step API neither schedules/sends replay nor retires intents, repairs forks or implements
Restore; those remain separate actor/transport/recovery work.

The optional `RegistryReplayPass` is a cooperative driver around that single-intent API, not a
worker or transport outbox. Begin checks current local membership and both ledger inventories,
then retains only that author's sorted saved ids, at most 10,000 / 320,000 id payload bytes in a
boxed slice. It does NOT read/flush the source or prove the captured concrete id is current/Open.
Every actual step reloads all enforcement state through `replay_registry_intent`. New intents are
outside the snapshot; disappearance of a selected id pauses rather than implying finality.

`RegistryReplayStep` is `Complete`, `Wait { retry_at_ms }`, `Paused`, `AwaitingSubmission`,
`Held { intent_id, reason, state }` or `Prepared { intent_id, ticket, op, state }`. At most one
actual replay runs per step. A monotonic 100-ms PER-PASS deadline is charged before calling replay,
including failed attempts. Checked deadline arithmetic refuses exhaustion before work, and Paused
is installed before any error/unwind can escape. Waiting/paused/complete/awaiting steps do no disk
work. This is neither a global rate limit nor a bound on how many passes the future actor creates.

Prepared waits for an exact `RegistryReplayTicket`; its private fresh allocation identifies ONE
attempt, so stale/cross-pass/duplicate acknowledgements cannot advance. `pass.submitted(&ticket)`
advances after the caller acknowledges local transport submission, not delivery. It retires nothing
and cannot establish that sending was authorized. `pass.retry_submission(&ticket)` invalidates the
ticket without advancing; the next eligible attempt reseals the same saved operation using the
existing current-member and Open checks. `pass.retry_failed()` resumes a paused id without resetting
its deadline or repairing budgets. No timeout skips work. Losing a ticket requires dropping and
restarting the pass; durable intents/logs preserve exact retry semantics.

`pass.progress()` reports selected/visited/submitted/held counts, with visited = submitted + held.
Complete means the original ids were traversed, possibly with holds, NOT that the current ledger
is empty, all edits were sent or any edit is final. Held advances once but preserves the intent for
explicit recovery. Dropping/restarting may revisit previously submitted work; it never retires it.

Passes are non-cloneable and bound to numeric server, full group/device, bucket, concrete epoch and
a stable private physical-mount token, separate from rotating budget freshness. Wrong mount/group/
device calls refuse without consuming the pass. This token is NOT a native unlock/server-incarnation
lease: explicit UI lock can keep the vault mounted. Raw pass consumers must cancel lifecycle-stale
passes and recheck session/incarnation/membership/MLS epoch/Open immediately before sending. No
ciphertext is retained in the pass; pass/step/ticket Debug redacts ids, scope and content.

`Server::{begin_registry_replay, send_registry_replay_step}` adds a cooperative sender. Begin takes
the store, numeric server, bucket, concrete document id and both inventories, then returns a private
`ServerRegistryReplay` bound immediately to this exact `ChannelSync` instance. Restoring even the
same group/device mints a new process-local token and refuses an old cursor before preparation.
The cursor exposes only `progress()` and `retry_failed()`, not raw submission tickets or packets.
One async step takes exclusive Server/store/cursor/budget borrows and returns
`RegistryReplaySendStep::{Complete, Wait, Paused, Held, Attempt}`. Attempt carries the intent id,
`Result<PublishSubmission, SyncError>` and saved registry state; Debug omits ids and content.

Preparation uses the Server's actual group/device/Clock/RNG through synchronous trusted-local
`ChannelSync::with_registry_context`, not caller-provided snapshots. Exclusive borrows remain held
through awaited dispatch, excluding interleaving changes to known local membership and the store
gate. This is not proof of remote currency or native UI-unlock authority. Immediately after
Prepared, a private guard owns the exact ticket: only Submitted advances; Duplicate, error, future
drop and unwind invalidate the attempt without skipping its saved id or resetting its 100-ms
deadline. No result retires an intent. A later attempt reruns store checks and freshly seals the
same saved operation. Cancellation after driver admission is ambiguous, not rollback.

`ChannelSync::publish_local_registry_once(expected_doc_id, SealedOp)` rejects wrong type/id/MLS
epoch, oversized ciphertext, non-current local membership, invalid signatures and nonlocal full
authors. It opens and validates the canonical registry domain envelope under the actual group,
then uses the current blinded DocRegistry topic and awaits `publish_once` without an outbox or a
legacy document-map entry. `SyncError::Publication` preserves the transport's typed refusal.
This low-level trusted API does not itself establish Open/durability; the Server adapter obtains
the packet from the checked store while retaining exclusive access. `RegistrySyncInstance` uses
allocation identity, not equality of token contents, and is neither persisted nor sent.

There is no autonomous worker, driver deadline, aggregate pass scheduler, native lock cancellation,
managed catch-up or automatic wakeup yet. The existing gossip-size limit can refuse
an otherwise accepted P1 operation; this slice changes no transport limit or wire format.

### Opt-in registry gossip receive (P1, not actor-owned yet)

`Server::watch_registry_epoch(&ServerStore, server:u64, bucket:u8)` synchronously returns an opaque
`ServerRegistryWatch`. It reads the checked local current epoch, falling back to deterministic epoch
zero only for an absent record; it creates no file and proves no remote currency. The handle binds
numeric server, physical mount, full group through the exact sync instance, bucket, concrete id and
a fresh installation generation. There is one desired watch per bucket (at most 256). Rewatching
even the same id clears its old queued packets and invalidates its old handle. Dropping a handle
alone does not unsubscribe; use `unwatch_registry_epoch(&watch)` or replace it. Revocation needs
only the exact sync/watch generation, not the old mount, so it still works after a vault reopen;
only ingestion requires the physical mount to match.

The next `sync_once` reconciles desired subscriptions before reading network events. Optional
`flush_registry_subscriptions().await` performs reconciliation without waiting for an event.
Current and retained routing-window topics are included without opening a generic document.
Reconciliation tracks each successful subscription and one uncertain in-flight subscribe, undoing
the latter on retry before recomputing desired topics. Unsubscribes likewise remove their local
subscription claim and retain an uncertain topic before awaiting, so a same-topic rewatch after
cancellation cannot remain silently unsubscribed. The retry flag remains armed across errors
and cancellation, and revoked watches reject traffic immediately, even before unsubscribe finishes.
This also fixes lost retry/cleanup ownership for interrupted ordinary routing subscriptions.

`sync_once` intercepts registry-tagged frames before legacy ingestion. Encoded frames above
256 KiB + 78 bytes reject before SealedOp decode; scope, current MLS epoch, AEAD, full current
signed author, local membership, canonical domain/bucket and exact watched blinded topic must
validate before enqueueing. A global pre-auth token bucket allows 50/s with burst 200, across ALL
senders, before cryptographic work. Verified full-author + concrete-document rows allow 10/s with
burst 50. At most 4096 rows exist, and only fully refilled rows are reclaimed; watch changes cannot
reset debt. Refill uses injected monotonic time, saturating arithmetic and no wall-clock credit.
The global ceiling also implies the design's per-device server ceiling but is intentionally stricter:
one sender can exhaust it for everyone. It is a bounded-work rail, not a fairness guarantee.

The inbox retains at most 16 compact SealedOps (16 x (256 KiB + 20) ciphertext bytes plus fixed
metadata), never the transport Bytes backing allocation or decrypted bodies. Thus at most 16
documents have queued/active validation work; the store drain itself is synchronous and serial.
`receive_registry_step(&mut ServerStore, &watch, &mut EpochStorageBudget)` consumes at most one
packet for the exact handle. It rechecks watch/mount/instance, current receiver/author and MLS
before calling the existing durable `ingest_registry_epoch` with the captured server/bucket and
actual group/device/RNG. It returns `Option<RegistryReceived { admission, state }>` only after the
store's barrier. Accepted, Duplicate, Quarantined and RejectedQuarantineFull remain distinct. No
network delivery receipt, local-author intent, or legacy accepted-op statistic is produced.
Watch/result Debug redacts scope/content. Low-level `ChannelSync::{watch_registry,
registry_watch_is_current, unwatch_registry, drain_registry_inbound}` are trusted local adapters,
not independently authoritative store APIs. Only their app wrapper checks physical-mount binding.

Errors/unwinds consume only the volatile packet and grant no acknowledgement; queue overflow,
invalid/stale packets and unregistered traffic are dropped. Past/future MLS epochs do not enter
legacy catch-up or past-key paths. Retrying the original author's saved operation or future managed
catch-up must recover drops; neither is automatically scheduled yet. Watches/inboxes/rate debt are
process-local and reset on sync restore; no wire or persistence format changes. Live actor/native
store ownership, lifecycle cancellation, registry/receipt-head discovery and managed catch-up remain
unimplemented. A rotated persisted epoch needs a newly installed watch, never a retargeted old handle.

### Cooperative registry catch-up page serving

`Server::begin_registry_page_provider(&ServerStore, server:u64, bucket:u8)` returns an opaque,
non-cloneable `ServerRegistryPageProvider` with a random provider-local MAC key. It binds the
exact sync instance, physical mount, captured numeric server and bucket; it reads/creates no
document and subscribes no topic. Drop on runtime/mount replacement. This is not a native lock
lease, a network request registration or an aggregate provider-count limit.

`serve_registry_page(&ServerStore, &mut provider, RegistryPageRequest)` synchronously loads the
checked source and returns `RegistryPageOutcome::{Page, Restart, CheckpointRequired,
HistoricalAuthorizationRequired}` or an error. The request carries a full requester device id,
concrete doc id, at most 64 strictly sorted unique initial heads, optional claimed verified seed
hash and optional 81-byte opaque cursor. Runtime/mount and current local/requester membership,
field caps, cursor HMAC and expiry are checked before source I/O. The current concrete id and
seed match are checked after loading. Corruption errors, absence restarts; neither becomes a
successful empty document. Fault refuses. Closing history may be read without opening its gate.

The low-level `RegistryPageProvider` uses HMAC-SHA256 over length-framed provider/requester,
full group/logical/type/concrete scope, initial heads/seed and a version-1 cursor payload:
`version:u8, frozen_count:u32, next_position:u32, issued_monotonic_ms:u64, prefix_digest:32,
mac:32`. Integers are big-endian. The prefix digest includes the seed hash and length-framed
signed envelopes in accepted-log order. This provider-local order is already dependency-complete;
it need not equal another peer's order. MAC comparison uses the library's constant-time verify.
The ephemeral key is zeroized on drop and must be reminted across runtime restarts. The fixed
ten-minute expiry is not renewed by continuation; wall-clock changes grant no credit.

Repeat the ORIGINAL heads/seed on every continuation. Unknown heads restart. The frozen count
and digest allow append-only growth or byte-identical source reload, but refuse changed prefixes.
Returned pages subtract the initial ancestor closure and advance by cursor position, including
already emitted dependencies. A requester with more than 64 heads can begin at [] (plus its
verified seed if rotated) and deduplicate repeats. A page contains at most 32 freshly current-MLS
sealed operations and 512 KiB summed `4 + SealedOp::encode().len()`; cursor/response envelope
bytes are extra. Any nonterminal page emits at least one operation. No plaintext seed, vault
snapshot, quarantined body or old ciphertext is exported. `next:None` means the captured prefix
ended, not that later edits do not exist or that the provider is current. Debug redacts identities,
scope, cursor bytes and content.

Missing removed-author operations in the REMAINING page range produce
`HistoricalAuthorizationRequired`, never omission or impersonation. Previously emitted cursor
positions and initial ancestors are claimed held history, so author removal after delivery need
not block later descendants. These are not possession proofs: a conforming requester derives its
heads/seed from verified state and continues only after persisting a dependency-complete page.
Historical authority transfer, automatic durable receiver continuation and checkpoint/head/seed
discovery remain unwired.

The direct trusted-local page API imposes no aggregate call-rate limit. One call rebuilds at most
the bounded saved epoch, then walks its bounded change index/ancestor sets. It grants no delivery
acknowledgement, intent retirement, saved mutation or finality. Network callers use the adapter below.

### Authenticated registry page exchange (cooperative, kind 20)

`Server::request_registry_page(peer, RegistryPageQuery)` requests exactly one unadmitted page.
The endpoint must already have a transport-bound proof for a current full device identity;
candidate discovery is not permission to disclose registry identifiers. The requester signs
with its actual device. Kind 20 uses the existing authenticated-request envelope, including
actual requester transport peer binding. Current MLS epoch and exact current member public keys
are required at queue and source-read time; old request kinds retain their compatible transcripts.

The version-1 inner query is canonical big-endian:
`version:u8=1, bucket:u8, doc_id:u128, head_count:u8, heads:[bytes32], seed:bytes, cursor:bytes`.
Every `bytes` field has a u32 length prefix. Heads are strictly increasing, unique and at most
64; seed is empty or 32 bytes, cursor empty or 81 version-1 bytes. No trailing data is accepted.
The inner cap is 2444 bytes and authenticated body cap 2588 bytes (plus one kind byte).
Requester identity is derived solely from the verified outer signature, never a body field.

`run_once` queues requests only for an exact installed registry watch. There are at most eight
pending responders per sync instance and one per full requester across all buckets, with a
five-second monotonic admission lifetime. Replacement/unwatch drops that bucket's queue.
Global pre-authentication work is limited to 10 requests/sec, burst 20; each verified full
requester gets 1/sec, burst 2, with at most 4096 debt rows. Replacement and cancellation do not
reset debt; only fully refilled rows are reclaimed. Source callbacks additionally share a 2/sec,
burst-4 rail across all watches. Expiry is swept on enqueue/drain; stale records can remain
bounded in an idle queue but can never be served after expiry. Refusals/errors drop the responder.

`Server::serve_registry_request_step(store, provider, watch)` drains one request through the
checked saved source. The provider must match the watch's captured numeric server, bucket,
runtime and physical mount. Scope checks happen before queue consumption; current membership,
MLS epoch, freshness, watch generation and service budget are checked before source I/O.
`None` means no eligible work, including throttling. `Some(())` means a response was handed to
the responder, NOT delivery. Cancelled requesters may still cause bounded source work because
Responder does not expose cancellation. No source mutation, intent retirement or generic ack occurs.

Answer bytes are `version:u8=1, status:u8`. Status 1=Restart, 2=CheckpointRequired and
3=HistoricalAuthorizationRequired end there. Status 0 adds `op_count:u8`, length-framed
SealedOps and one length-framed optional cursor. At most 32 ops and 512 KiB of framed ops are
allowed; each op must match DocRegistry, concrete id and current MLS epoch. Empty nonterminal
pages reject. Signed framing is `[provider_pubkey:bytes32, signature:bytes64, answer:bytes]`.
The answer cap is 524376 bytes, signed response cap 524484 bytes. The latter is checked after
the transport has buffered its globally bounded frame but before payload copies/signature work.

The response signature uses `catcoms/registry-page-response/v1` in the existing length-framed
signed-response transcript (group, requester key, timestamp, nonce, request epoch), with a bound
bundle of length-framed actual provider PeerId, entire canonical query and entire answer. The
client requires the same full device that proved the selected endpoint. Tampered, relayed or
cross-query responses cannot become a page. An empty unsigned response means unsupported/refused,
not an empty document; there is no legacy catch-up fallback. Decrypted author/projection/DAG
admission is still the durable receiver's job; it must save all operations before using `next`.

One outbound request has a ten-second injected-clock deadline. Four fixed per-sync permits are
retained by `RequestCancellation` accounting until the transport actually releases its work;
dropping or timing out the caller publishes cancellation but does not recycle a live stream's
permit. No retry, durable receive cursor, actor/native scheduler or receipt/seed discovery is
provided by this exchange alone.

### Registry persistence and inventory constraints

Public receipt fields and ciphertext are capped before encoding/decryption. Changed state uses
an accounted atomic replacement; identical pre/post mutation snapshots instead sync the unchanged
authenticated file and parent. Both snapshots use the restored current owner: refreshing this
derived quota owner alone needs no copy at the content cap. Actual old bytes remain accounted and
flushed, and each restore derives the owner again. Failed writes, flushes or unwinds grant no
success and poison accounting until reconciliation. A loaded state alone grants no acknowledgement
after an uncertain rename. The existing durability model remains file sync plus Unix-only parent sync.

Peer-writable history, seed and metadata charge ordinary content. Only exact receipt-book growth,
the opening receipt and the optional gate receipt hash charge protocol allowance; an owner seal
therefore remains admissible at the content ceiling if protocol/settlement headroom remains.
`RegistryEpoch::validate_vault_snapshot` shares full restart validation but returns ONLY that
computed receipt byte count. It requires authenticated local bytes, not a current owner, and
grants neither an editable object nor network authority.

`scan_epoch_storage_with_registry` and `cleanup_epoch_storage_staging_with_registry` opt into
`EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsAndRegistry`. Older coverage stays unchanged.
`EpochRecordKind::Registry` and progress `registry_records` identify this fourth family; cleanup
keeps the same coverage through its rescan and also invalidates intent freshness tokens. Final
records are never cleanup targets. Unpublished registry temporaries conservatively charge content
regardless of their intended mutation; a receipt-copy orphan at the content ceiling may require
explicit cleanup before reconciliation. Unresolved ownership still blocks server composition.

These APIs require the caller's sole complete server budget; inventory is not a continuing write
lease. Checked installation/retirement and single-intent replay are implemented at the store layer.
The cooperative sender and opt-in receiver supply live gossip, not catch-up serving, receipt-head
discovery, autonomous replay/drain scheduling or actor/Studio wiring. Each mutation rebuilds a
bounded saved graph (local editing also checks the source before journaling); the receiver applies
the ingress rails above, while aggregate actor-owned scheduling remains to be integrated.

`get_delivery(server,channel)` and `delivery-changed` both carry the actor-issued `revision` beside
the complete bounded `states` array. The webview accepts only a strictly newer revision for its
current server/channel view, so a delayed query completion or event cannot replace fresher receipt
evidence. Revisions are process-local ordering tokens, not persisted delivery evidence.

Diagnostic events similarly carry their capture-time `capture_mode` and `capture_epoch`. Before a
Safe event enters the hub, literal `AddressValue` bytes are removed, arbitrary `SafeText` and legacy
`BridgedMessage` values are replaced by fixed typed placeholders, runtime field names become ordinal
slots, and targets are reduced to a closed component-root allowlist. Later viewer changes therefore
cannot recover those discarded strings. Every rendered row carries its capture mode and epoch;
mixed-history reports name both the current setting and all epochs actually present.
Native event envelopes use `__seq`, `__ord`, `__gen`, optional `__trace`, and optional
`__trace_proof`. Webview-origin trace hex is untrusted and is reduced to a session-local token before
ring admission. `__trace_proof` is an opaque, trace-bound MAC under the diagnostic session salt; it
allows an unchanged native trace to return through `record_ui_events` without becoming
`H(H(trace))`. It is neither diagnostic data nor authority, is never persisted/rendered, and a
missing/invalid proof causes normalization rather than trust. After Tauri has decoded an invoke's
JSON body, structured UI commands retain at most 256 events and 32 ordered fields per event;
omitted fields increment the canonical row's dropped-field count. This is a ring/command work bound,
not a pre-parse IPC byte limit.
The optional `catcoms-log` debug file is a separate raw tracing sink, not an export of this Safe
ring. It can retain arbitrary native tracing and frontend console/error prose—including names,
message fragments, paths, URLs, tokens, serialized objects, and stacks—whenever a call site emits
them. Its rate, line, queue, rotation, and session-size bounds limit work and retention; they do not
provide content minimization. Users must review that file before sharing it.
`set_capture_mode` preserves per-section levels, while the separate `reset_section_capture` command
restores recommended levels. Turning capture Off stops new admission but does not retroactively erase
bounded history. Local Copy/Save reports are honestly labelled and receive validator disclosure
findings. `open_public_diagnostics_issue` accepts no webview payload: native code renders the ring
through a canonical allowlist, validates it, builds the fixed tracker title/body/URL, and launches
that exact URL atomically. Targets, wall-clock time, addresses, runtime field names, user prose and
legacy tracing events are absent from its report by construction. Each included public row states
its capture mode and epoch, so a mixed-history clipboard fallback cannot imply one privacy setting
for bytes admitted under another. Only the URL excerpt is bounded;
when it is shortened, the exact full publication envelope is returned for clipboard review.

The version-1 UI-continuity JSON retains the required `drafts` and `readMarks` objects and may also
carry `statusCursors` plus bounded per-server `fileTrustPolicies`. Each file policy is local to this
installation (`on-demand`, `specific` with exact authenticated full device identities, or `everyone`),
vault-sealed with the other continuity state, and never enters group replication or the wire.
Missing/malformed policy data decodes to on-demand. It governs passive media fetch/decoding only;
an explicit Load/Play/Open/Download action is a separate user grant. Third-party HTTP(S) image
URLs always require that explicit grant because they have no authenticated file origin.

`lock_session(ui_state_json)` closes the native UI-session boundary unconditionally and resolves to
`{ continuity_error: string | null }`. A non-null error means locking completed but the final bounded
continuity snapshot did not persist; it is deliberately not an IPC rejection, because the webview
must distinguish confirmed locking from an ambiguous bridge failure. Ctrl+L retains its immutable
snapshot. `close_vault_window(ui_state_json, discard_continuity_error)` repeats the idempotent lock
under the same native commit mutex and owns the lock-before-destroy ordering. The newest exact
snapshot is registered in a native pending transaction before either lock waits on the session
mutexes, so a snapshot-less close after a webview remount consumes that transaction and its outcome
instead of overtaking it. The first continuity failure leaves a confirmed-locked warning visible;
repeating close acknowledges losing only that latest screen snapshot. An ambiguous bridge failure
destroys the main webview directly or restarts the process rather than presenting an unverified
visual lock. Unlock waits for any outstanding Ctrl+L request before authenticating a new generation.
If native locking retained a validated snapshot after a write failure, unlock retries those exact
bytes under the same commit boundary and keeps IPC locked while persistence is still unavailable.
A newer lock generation is never retired by an older unlock attempt. Malformed snapshots are not
retryable: the first unlock surfaces their loss and a deliberate repeat acknowledges only that
latest invalid screen state.

New file-index rows append `signer_key` and `signature` fields. The signature domain
`catcoms/file-entry-attestation/v1` length-prefixes the stable group id, name, claimed author,
normalized path, and encoded `FileManifest`/legacy `FileRef`. Readers recompute the signer device
fingerprint, require it to equal `author`, and verify the signature before exposing
`author_verified=true` plus the signer's full `DeviceId`. The eight-hex-character fingerprint is
display-only; `specific` authorization compares the full identity. Missing fields are the
backward-compatible legacy shape and remain downloadable, but a `specific` local trust policy must
not auto-load them. Readers and writers cap the index at 256 rows and bound each signed field before
clone/decode/verification, containing malicious replicated-index work.

`ServerNet` record version 3 adds a reconnect-policy tag after the version-2 switchboard flag:
`Disabled`, `AuthorizedPeer(peer_id)`, or `LegacyPending`, followed by at most two
`ReconnectRoute { peer_id, address }` rows. A row is valid only under `AuthorizedPeer` and must name
that exact peer. Version 4 appends an optional `pending_recovery_peer` plus the signed code's
absolute expiry. Versions 1 and 2 decode with an empty route list and `LegacyPending`; version 3
decodes with no pending recovery. New founders
and helper/reply/switchboard admissions persist `Disabled`, while a successful direct admission
persists only its named inviter as `AuthorizedPeer`. Each address is capped at 512 bytes and the
entire record remains vault-sealed and atomically replaced. Every `ServerStore` record uses the
same durability primitive: write and sync a sibling staging file, rename it over the destination,
then sync the parent directory on Unix. Abrupt termination before rename therefore retains the
complete authenticated predecessor; after rename readers see the complete replacement. Staging
siblings are destination-specific, unique per invocation and opened with create-new semantics, so
concurrent record types cannot alias and a pre-planted symlink is not followed. A parent-directory
sync failure is reported distinctly as `CommittedButNotDurable`: the replacement is already visible
and a retry is safe. Logical read-modify-write operations still require serialization to prevent a
stale last writer from replacing newer state. `ServerStore::open` additionally acquires a
non-blocking OS session lock before unsealing and holds it for the store's lifetime. A second
desktop process therefore receives `VaultBusy` instead of mounting duplicate MLS, invite-ledger
and transport actors from the same snapshot. Normal drop and abrupt process termination release
the OS lock. An explicitly UI-locked webview re-authenticates with `verify_passphrase`, under the
short vault transaction lock, rather than trying to mount a second `ServerStore` in its own native
process; a wrong passphrase still leaves the IPC boundary locked.

The rows are installation-local: they never enter the group snapshot, PEX, rendezvous, or webview,
and reconnect diagnostics retain route shape rather than the private coordinate. Reload installs
them into `ChannelSync`, which reparses the canonical terminal peer binding, permits literal-IP raw
TCP/QUIC (including private/loopback) but rejects DNS, relay, WebSocket, link-local, multicast,
unspecified and IPv4 0/8 or 240/4 hosts, requires exactly one current roster record to claim that
transport peer, skips live/self peers, and spends the same process-wide endpoint scheduler as other
untrusted recovery dials. Direct admission makes one bounded best-effort PEX request before the
first post-join snapshot so the inviter's signed descriptor normally accompanies the sealed socket.
On the discovery cadence, an authorized record may refresh only that inviter. `LegacyPending` may
promote once only when the group has exactly one other member and exactly one unique live member
claim; its captured route must additionally be private/loopback. A non-empty observation replaces
and installs the bounded hints; an empty observation does not erase them merely because the remote
app is closed.

Applying a member recovery code first verifies its group, signature, current roster membership,
exact unique device→transport record, deadline and route grammar **without dialing**. While the UI
session commit gate is held, the desktop atomically seals only that peer and deadline as pending;
the previous `ReconnectPolicy` and proven routes remain intact. Applying then repeats validation
and submits scheduler-charged dials. A coalesced worker installed for both new and restored servers
may promote the pending peer only before the deadline and only from this process's bounded recent
outbound Noise-authenticated route evidence. Establish+close therefore cannot erase proof before a
vault wait completes. Promotion atomically changes `ReconnectPolicy` and installs the exact route;
a pasted candidate alone never becomes durable. Recovery may retain a safe public direct literal,
while ordinary legacy migration remains private/loopback-only. The Tauri commands are
`mint_member_recovery(server) -> {code,expires_at_ms,candidate_count}` and
`apply_member_recovery(server,code) -> {fingerprint,submitted_routes}`. A successful apply result
means bounded dial attempts were submitted, not that the member is connected.

`storage_health` counts a chunk as verified only after its storage seal/content address and the
file-layer decryption both succeed. `repair_storage` explicitly fetches repairable missing or
unreadable referenced chunks over the authenticated blob path and verifies again; `has()` alone
must never short-circuit repair. Over-cap contradictory exact references remain unreadable without
a fetch because the same content address cannot reconcile different wrapped keys.

The Tauri `get_storage_health(server)` command adds a cached, deduplicated inventory projection:
`checked_at_ms`, unique/logical/local-estimated/pinned totals, category rows, the ten largest
files, and `local_files`, the deduplicated files whose complete encrypted chunk set is held by this
installation. The Storage pane can pass one of those rows through the existing authenticated
`save_group_file` path. That explicit “Unlock copy” action verifies/decrypts the managed chunks and
creates a separate, non-overwriting plaintext Downloads file; it does not alter or remove the
vault-encrypted copy. Its final staging-file rename and reveal are authorized by the exact unlock
generation that began the export; locking, then unlocking again, cannot revive the old operation.
Chunk health is keyed by the exact encoded `FileRef`, and the inventory joins
that verdict to the exact manifest in one actor snapshot; a reused ciphertext or plaintext CID
cannot borrow another row's successful verification. `StorageHealth.resolvable_manifest_versions`
contains exact digests admitted by the shared bounded manifest resolver; this is compatibility
metadata, not a possession verdict. Inventory selects a complete exact-verified local variant only
when all differing same-CID manifests are compatible, so a healthy repair can appear in `local_files`
alongside an unavailable original. Incompatible sets stay excluded. Authentication attempts are
capped at four distinct exact references per ciphertext
CID; a larger contradictory set fails that CID and every dependent manifest closed instead of
multiplying large-blob decryption work. It performs at most one ordinary scan per server per
unlocked UI session (the cache survives HMR but explicit lock clears it). Cache publication is
bound to both the exact UI generation and process-local server incarnation; a removed/reinstalled
server id cannot inherit a late old scan. The webview applies the same unlocked exact-view gate to
deferred results. Only `repair_storage(server)` replaces that cache after its mandatory post-repair
verification.

Media presented to the WebView uses an exact inert MIME allow-list and must have a matching common
image/audio/video container signature in authenticated chunk zero; SVG, mismatches and unrecognized
containers receive a bodyless scheme denial instead of relying on `application/octet-stream` or
`nosniff`, because media elements may still sniff an opaque response. The validated head and each
decrypted chunk cache are bound to the complete sorted set of current encrypted manifest digests
(single-manifest identities retain their prior digest). Up to four variants may coexist only if
total size, MIME and ordered plaintext chunk CID/size/MIME agree. Every request re-resolves this set,
and each fallback is independently opened against its own exact reference, so a claimed plaintext
CID cannot inherit a stale MIME decision. Downloads, plans and media share this resolution policy;
all local alternatives are tried before bounded sequential provider attempts. Upload dedup requires
complete verified local possession; otherwise publication retains fresh staged chunks and posts an
attested repair. Only a verified local-device row at the same name/path/CID can be replaced, preserving
its expiry. A new repair row has a fresh default expiry. No wire or persistence encoding changes.
Every head/chunk cache access and URI-responder publication is bound to the initiating unlocked UI
generation; explicit lock clears cached plaintext and a delayed actor read can publish only a
bodyless denial afterward. Plaintext exports report `contentValidation` as `matched`, `mismatch`,
`unrecognized`, or absent for a
non-media file after inspecting a fixed 64-byte prefix. This is bounded type evidence, not full
bitstream validation or a claim that the platform decoder/external application is safe.

Moderation uses one server-wide `DocType::Moderation` document (`doc_id=0`). Events and votes have
their own canonical, group-bound Ed25519 signatures in addition to the replicated-op envelope.
Warning evidence is a bounded immutable message snapshot. Readers separately expose signature
validity and authorization; signer→origin attribution and the current owner-signed role state are
checked before an event can affect the honest UI. Votes are one current origin identity per case;
departed identities remain attributable but are ineligible for the live tally. Votes are advisory.
Only `resolve_kick_case(remove=true)` by the owner reaches the existing
protocol-enforced MLS removal path.

The bridge's `create_backup` snapshots every actor, persists the registry/snapshots, then copies
the sealed store into a fresh non-overwriting directory under Downloads while holding the store
lock. It refuses symbolic links and special files. This is an encrypted export under the existing
vault secret, not a secret-reset mechanism; automated restore is deferred until it can run while
locked with staged verification and rollback.

The bridge's `change_vault_secret(current_secret,new_secret)` holds the store mutex and calls
`ServerStore::change_passphrase`. The storage layer authenticates the current wrapper and atomically
rewraps the unchanged root DEK under a fresh salt/nonce. A sibling OS file lock serializes both
first creation and rewrap across desktop processes; unique create-new staging, file sync, rename and
Unix directory sync then publish the wrapper without shared-temp aliasing. Lock contention fails
promptly as `VaultBusy` so a suspended process cannot hang another app's unlock command. This short
transaction lock is distinct from the lifetime installation lock described above. Existing derived
data keys do not rotate; older exported vaults remain bound to their old secret.

New and replacement vault secrets are non-empty and capped at 4096 bytes before Argon2 work. Vault
wrapper v1 feeds that bounded secret directly to Argon2. For compatibility only, an existing v1
wrapper may be opened with a 4097..65536-byte legacy secret; the first successful open atomically
rewrites the fixed-size wrapper as v2, which domain-separates and BLAKE3-normalizes that long secret
to a fixed KDF input. Secrets above 64 KiB are rejected. This migration is intentionally
forward-only: a v1-only older binary does not understand the v2 wrapper, so a user who triggers the
legacy-long migration must return to a v2-capable build rather than roll back. Both versions are
exactly 89 bytes; other lengths are rejected before allocation/decryption.

---

## 10. Channel deltas & unread state  *(catcoms-app / Tauri bridge / desktop)*

```rust
pub struct ChannelChange {          // WHAT moved, carried by every ChannelUpdated
    messages_appended: bool,        // a message id that was not there before: a real arrival
    messages_changed: bool,         // the log re-rendered without one: edit/delete/reaction/pin
    topic: bool,
    jukebox: bool,
}
pub enum AppEvent { ChannelUpdated { channel: u128, change: ChannelChange }, /* … */ }

pub struct ChannelHead {            // one per directory channel; no message text
    channel: u128, count: u64, latest_ts: u64,
    latest_incoming_ts: u64,        // newest message THIS device did not write (0 if none)
    latest_incoming_id: String,
}
impl Server { fn channel_heads(&self) -> Vec<ChannelHead>; }
impl ServerActor { async fn channel_heads(&self) -> Vec<ChannelHead>; }
```

One channel document holds the message log, the topic and the jukebox queue, so an untyped "it
changed" event is ambiguous exactly where the UI needs certainty. **Only `messages_appended` may
create unread state**; the other three refresh what is on screen and nothing else. The actor keeps
a per-channel signature of the three parts plus the set of message ids, and an arrival is "an id
never seen before" rather than a count that grew: a concurrent append+delete batch or a catch-up
merge can leave the count untouched. First sight of a channel reports nothing, because the UI
fetches messages when it opens one. The bridge forwards the flags on the `channel-updated` payload
(`messages_appended`, `messages_changed`, `topic`, `jukebox`).

### The jukebox queue: two kinds of entry

```rust
pub struct JukeEntry {
    pub id: String, pub cid: String, pub name: String,
    pub author: String, pub added_ms: u64,
    pub source: String, // "" = a shared file; "youtube" = a linked video
    pub link: String,   // the provider's video id; empty for a file entry
}
impl Server {
    // cid: 1..=128 lowercase hex, shape-checked only (the blob may still be in flight)
    async fn jukebox_add(&mut self, channel: u128, cid: &str, name: &str) -> Result<String, AppError>;
    // link: 1..=64 URL-safe base64 chars; reaches no network and checks no provider
    async fn jukebox_add_link(&mut self, channel: u128, source: &str, link: &str, name: &str)
        -> Result<String, AppError>;
}
```

An entry names **exactly one** way of getting the track. A file entry is content the group holds,
addressed by `cid` and served out of the vault; a linked entry names a video on a third party's
service and has no `cid`, because nothing here holds it and each listener fetches it themselves.
`source` and `link` are written only for a linked entry, so a file entry is byte-for-byte the
document older builds wrote and their absence reads as "file".

`read_jukebox` is the boundary, not the add call: a peer writes the channel document directly. It
skips any entry that is not exactly one well-formed kind, which means an unknown `source`, a
linked entry whose `link` is not the storable alphabet, and (deliberately) an entry carrying
**both** a `cid` and a `link`. Resolving that last case either way would be this device deciding
what a peer meant by two claims that disagree about who fetches what from where. Both kinds share
the one `MAX_JUKEBOX_ENTRIES` cap.

The link's **storage** check is an alphabet, not a format: URL-safe base64 within a length budget,
which is what stops a link leaving the URL path segment it is written into. The exact eleven-character
YouTube id shape is checked in the frontend (`youtube.ts`), next to the code that builds an address
from it, so a change at the provider's end cannot make stored entries unreadable. The call-transport
frame carries `link` alongside `cid` and re-validates it at the edge to the exact id shape, rejecting
a frame that names both.

The signature is recomputed only for a channel whose document **moved**. `Server::doc_version(doc_type,
doc_id)` is the number of signed ops applied to a document this session (O(1); every content change,
local or remote, live or caught up, is exactly one op, and duplicates never count), and the actor keeps
the last version it projected for every document it watches. A network event that touched nothing a
projection reads costs no document walk; before this, every gossip frame, presence blip and receipt
re-materialized every open channel plus the status feed, wiki, roles and (Ed25519-verified)
moderation records. Membership-derived projections (roles, moderation) also re-read on an epoch or
member-count change, and the wiki keeps its full compare while a review is pending, because a pending
edit auto-accepts at its deadline with no op written. Delivery snapshots are deliberately **not**
gated: every tick still dirties every open channel (the recompute is throttled to one second per
channel and short-circuits when this device has no recent own messages), because the one-second
delivery timer that this arms is also what cancels a sync tick blocked in an outbound request.
The actor's `select!` drops the in-flight `sync_once` future whenever the timer or a command
fires; without that wake, two members that simultaneously issue a request to each other (a doc
catch-up one way, PEX the other) each sit on the other's inbound request until the request
timeout, and `process_recovery_e2e` fails. `Server::messages` is served from a per-channel materialization
cached under the same version (`with_messages` borrows it without copying), so the actor's change
check, `get_messages`, the unread heads and the inbox share one walk per change.

**Paged history** (`docs/design-native-paging.md`). The webview no longer reads a channel whole:
`get_message_page(server, channel, anchor, before, after, unread?)` (`Server::message_page`) returns
one contiguous slice `{version, total, start, anchor_index, rows, unread}` around an anchor
(`{kind:"tail"}`, `{kind:"id", id}`, `{kind:"index", index}`, `{kind:"first_reply_to", id}`), with
`before`/`after` rows either side (each clamped to 2048 at the bridge). Ids are the durable anchor
(they survive concurrent inserts, edits and deletes around them); an id that names no current row
yields `anchor_index: null` and no rows, and the client re-anchors by index. Every row carries
`targets_me`, `reply_count` and `reply_to_preview` (`{id, author, text[..200]}` of the parent),
resolved natively against the whole channel. `get_pinned_messages(server, channel)` returns the
pinned rows by name. `get_messages` remains for the explicit whole-history readers (server-wide
search corpus, moderation timeline).

**Only the newest page response may be applied.** Every path that can replace the loaded slice
(open, refresh, reveal, jump, re-anchor) awaits a promise, and those settle in whatever order the
actor and the bridge produce them; checking only that the server and channel are unchanged let a
slow older response overwrite a newer one and put the reader back on stale rows. `PageAdmission`
(`message-paging.ts`) issues one token per request, `applyPage` refuses any other, and leaving the
conversation invalidates every outstanding request. Deliberately not a comparison of `version`: two
requests can carry the same version and still need an order.

**A channel's rows are read in timestamp order, from every conflicting `messages` list.** Two
things the CRDT does not do for us, both of which reached users:

- The list is created lazily by whoever posts first, so two members posting before either has seen
  the other's creation op each make one. Automerge keeps both and `get(ROOT, MESSAGES)` returns
  only one, leaving the other member's posts in the document and unreachable. This is the defect
  the status feed escaped by moving to distinct root keys; channels did not. Reads now take every
  conflicting list. Writes still use the one `get` returns, so writers converge on a single list
  as soon as they have seen each other. **Known limit:** editing, deleting and reacting still
  resolve through `get`, so a row stranded in a losing list is visible but not yet mutable.
- A send appends at the end of the list *as the sending replica sees it*, so a member that is
  behind appends after the last row it knows about. When the missing history arrives, the two are
  concurrent insertions and the merge orders them by a rule that knows nothing about when anything
  was said: a day of messages could sit above two earlier days. Rows are therefore sorted by `ts`
  with a **stable** sort, so ties keep the merge's own deterministic order — which for the case
  that produces ties, one person typing faster than the clock ticks, is the order they were sent
  in. Adding the id as a tiebreak would replace that with alphabetical order.

**A send is stamped `max(now, newest_seen + 1)`** (`Server::next_message_ts`), not with the wall
clock alone. Because the order above is the timestamp order, a device whose clock runs behind wrote
straight into the past: what it said appeared above conversation that had already happened, and a
reply sorted above its own parent. Reconnecting is where that became visible rather than subtle,
since catch-up hands a returning member a block of history whose newest row is later than their own
clock and everything they then say lands inside it. A Lamport-style step over the wall clock fixes
causality without needing the clocks to agree: when they do agree it is exactly `now`, and when they
do not, a message still sorts under everything its sender had already seen. It is **bounded** by
`CLOCK_SKEW_GRACE_MS` past this device's own clock — the same grace the unread ceiling applies —
so one member whose clock is far in the future cannot drag a group's whole timeline forward with no
way back; past that bound the message sorts under the out-of-range row instead of chasing it. This
is still not a causal ordering key: two members who both send while neither has seen the other order
by their clocks alone.

**A `channel-updated` delta names the rows that arrived** (`arrivals`, capped at 32 ids, in the
order they now read), because with rows in timestamp order an arrival is not always the last row:
a delayed message, or one from a device whose clock is behind, lands wherever its stamp says.
Anything describing an arrival reads those ids; taking the end of the list describes whatever is
newest, which after such an arrival is a message the reader has very likely already seen.

`get_messages_by_id(server, channel, ids)` (`Server::messages_by_id`) reads those rows, wherever
they sort, each with the same `targets_me` bit `get_message_tail` carries. Knowing an arrival's id
is no use if the query cannot reach it: a delayed row can sort arbitrarily far back, so neither a
bounded tail nor the page the reader has loaded is guaranteed to contain it, and searching those
and falling back to the last row announces a message already announced. Ids naming no row come
back absent, which is how a caller learns a row was deleted rather than substituting another for
it. Classification comes from the arriving row's own `targets_me`, not from
`addressed_after_cursor`: that scan answers over the rows past the read mark, and a message that
sorts before the mark is not among them, so a delayed mention read as an ordinary message.

**Still open, and the reason this does not close the whole "buried message" report:** the unread
summary decides by position, so a message that arrives with a timestamp below the read mark sorts
before it and is counted as read. Before rows were ordered, a late arrival usually appended after
the mark and was counted; ordering them made that case worse rather than better. Fixing it needs
observation state — what this device had actually seen when the mark was set — rather than a
better ordering, since a genuinely unseen message can legitimately belong earlier in the
conversation. The `arrivals` ids above are the signal that work should build on.

This is not a causal ordering key: two senders' clocks can still disagree, and a reply can still
sort above its parent if they disagree badly enough. What it guarantees is that every member
renders the same conversation in the same order, and that paging, day dividers, the unread
boundary, the notification tail and the delivery anchor all mean the same thing, because they all
read this one order.

With an `unread` probe (`{divider_id | null, divider_ts | null, now_ms}`: the client's frozen read
cursor, its timestamp, and its own clock) the page also carries `{ceiling_ts, first_index, count}`
over the whole channel, so a client holding one slice still places the divider and counts past it.
**`divider_id` is the cursor**, matching the durable badge in `unread.ts`: when it names a row,
everything after that row that somebody else wrote is unread, which needs no clock and so cannot be
fooled by two messages sharing a millisecond or by a sender whose clock ran backwards. The
timestamp rule (`readCeiling`/`effectiveTs`, five-minute grace, own rows never count) applies only
when there is no usable cursor: a read mark written before ids existed, or one whose message has
since been deleted.

`get_message_tail(server, channel, limit, afterId, afterTs)` (`Server::message_tail`) returns the
newest `limit` rows (oldest first, `limit` clamped to 1..=256 at the bridge) each with a
`targets_me` flag, plus `addressed_after_cursor`. The flag on a row is an `@[my name]` mention under
the composer's normalization, or a reply to one of my messages, with the parent resolved against the
**whole** channel. `addressed_after_cursor` answers the same question over **every** row after the
cursor (normally the channel's read mark), carried or not: one catch-up or one busy minute can
append more messages than a tail holds, and a mention followed by that many ordinary messages must
still ring. `afterId` is the cursor; `afterTs` is what is left of it once the message it named has
been deleted, and decides on its own in that case. Only a cursor with neither means nothing has been
read here, where the whole channel counts; without `afterTs` a deleted read mark read as exactly
that, so an old mention rang again on every arrival, forever.
It exists for the arrival ticker, which runs for every arrival in every channel not on screen and
used to fetch that channel's entire history to read its last row.

`get_channel_heads(server)` is how unread badges survive what the event stream cannot. Actor
notifications are deliberately dropped at the native boundary while the vault is locked, and a
restart begins with no event history at all, so anything that arrived in the meantime has no event
left to raise a badge. The desktop compares each head's `latest_incoming_ts` with its own durable
read marks at unlock, at resume and after each server's channel directory settles.

**Read marks are never advanced by a refresh.** The desktop's `chatIsObserved` predicate
(`unread.ts`, pure and unit-tested) requires the chat surface to be the active one, no takeover
overlay or call focus surface over it, the window focused, `document.visibilityState === "visible"`
and the log pinned to its newest row. Anything else marks the selected channel unread exactly as an
inactive one would. Notification noise uses the same predicate with the scroll condition relaxed,
so reading history stays unread but stays quiet.

A message timestamp is the **sender's** clock. `readCeiling(timestamps, now)` is the newest
timestamp present that is within `CLOCK_SKEW_GRACE_MS` (5 min) of this machine's clock, and
`effectiveTs` pulls anything above it down to it. Read cursors, the unread divider, mention
detection and the inbox's unseen test all measure the clamped value, so one broken or hostile clock
can neither park the cursor in the future (hiding every later message) nor stick as a permanently
unread row. Wall time remains display metadata only.

The single record of a server's outstanding activity is its `unread` channel-id list. The rail
badge, the orbit glow and the DM circle's dot are all derived from it, so no separate mutable
"activity" flag can disagree with the channel list.

---

## 11. Adaptive screen-share signalling  *(desktop WebRTC)*

Call signalling's existing authenticated peer state adds an optional `rx` field with exactly one
of `720`, `1080`, `1440`, or `2160`. Older senders ignore it; newer receivers default an absent or
invalid field to 1080p. In automatic mode the receiver computes the largest 16:9 physical-pixel
surface fitting the current Mewtual window, rounds it to the nearest bucket, and keeps the prior
bucket through a 72-pixel midpoint dead band. Exact window and monitor dimensions never leave the
device.

Screen capture settings are local and persisted as resolution (720p/1080p/1440p/2160p), frame rate
(15/24/30/60), quality priority, and a 0.5--50 Mbps full-resolution per-peer cap. For each connected
viewer the sender chooses `min(capture_height, rx)`, parks that edge, applies
`scaleResolutionDownBy`, `maxBitrate`, `maxFramerate`, and degradation preference to the independent
`RTCRtpSender`, and attaches the screen track only after success. Resize/settings bursts retain one
active plus one overwriteable pending mutation per sender. A rejected cap leaves that edge paused;
if it cannot be parked, screen sharing stops rather than sending uncapped. The panel shows per-peer
and aggregate estimates excluding audio/protocol overhead. The WebView performs the actual
compressed encoding and may vary below the cap. Codec preferences are H.265/HEVC, AV1, VP9, H.264,
then VP8 when exposed by the runtime; the UI labels only the codec observed in WebRTC outbound
stats as negotiated.

## 12. In-call jam layer; patches, drums, clock, takes  *(desktop WebRTC; contract `jam:v2`)*

Everything below rides the existing negotiated per-peer data channel (`"inst"`, id 7, ordered),
carrying JSON text frames. Notes stay EVENTS, never audio: each receiver synthesizes locally, and
the receiver is sovereign; it owns the final gain, a master limiter, per-peer mute buses, the
Deafen gate (hard: gates rendering, releases every ringing voice including local previews, and
destroys buffered room-effect state), an 8 s release-time
ceiling, and forced node teardown. Two prerequisites land before any of this ships: Deafen must
actually gate instrument rendering, and roster revocation must tear down the removed member's call
connections (`removePeer`), not merely refresh lists. All constants here are mirrored as named
exports in `apps/desktop/src/jam-contract.ts`; the validator, the tests, and this section cite
that one module so numbers cannot drift.

**Authenticated channel seam.** Opening an authenticated peer's inst channel mints one opaque
`JamSourceChannel` capability. That exact channel's callbacks close over it and every patch, note,
drum, and metronome delivery must present it; a callback may never recover authority from a
sender-controlled field or fingerprint string. Reopen replaces the capability before accepting
new events, while removal tombstones it before teardown. Work that starts or finishes under an old
capability cannot recreate source state, consume the replacement's sequence, change its patch
session, or render. Patch hashing is serialized in ordered-channel receipt order and rechecks the
capability after WebCrypto completes. Serialization is scoped to that exact channel capability,
so a stalled digest from a disconnected generation cannot block its replacement. A per-generation causal queue extends ordered channel
delivery across asynchronous patch and drum digests, so a dependent note can never overtake its
announce. Outbound events similarly carry one immutable, fully hashed publication object through
the wire frame, local renderer and recorder; each edge emits that exact prerequisite announcement
before its dependent events. Distinct global recipe changes are sender-paced to at least 2 s
apart (editor churn coalesces onto the latest draft), matching the receiver's patch bucket; a new
edge may recover exactly one byte-identical current recipe that this peer's call-epoch budget and
engine previously hash-verified. That reconnect exception spends the persistent all-frame budget
but neither a patch token nor another digest; any distinct frame still spends the persistent patch
bucket, so reconnect cannot mint descriptor/hash allowance.
Only one local publication digest runs while at most the latest draft waits, and that coalescer is
replaced on leaving so a stalled old call cannot head-of-line block the next call.
An unopened edge retains **no musical events**: when it opens, it sends only the current patch
announcement and begins with fresh live traffic. This intentionally records one transient-loss
gap instead of collapsing seconds of history into an instantaneous receiver-budget burst or
separating a held note from its release. A future sustain-across-connect feature requires a newly
sequenced bounded held-state snapshot, not historical edge replay. Local input waiting for its
first immutable publication is capped at 256 events and drops an overflowed transient as a unit.
The inbound async lane is likewise capped at 256 pending operations; overflow receive-mutes that
source rather than retaining an unbounded digest backlog.

**Frame admission (before parse).** Any delivered frame over 1024 bytes is rejected before parsing; after parse,
every type except `t:"p"` must still fit 200 bytes. Each peer has an all-frame token bucket
(refill 80/s, burst 160) charged BEFORE `JSON.parse` for every frame, valid or not; an empty
bucket drops the frame. A peer whose bucket stays exhausted for 10 cumulative seconds inside a
rolling minute is auto-muted receive-side for the rest of the call (their held notes release, the
UI says so, manual unmute allowed). The musical bucket is unchanged: 30 note-ons/s, burst 60,
note-offs never charged. Auto-mute does not freeze consent or media state: exact bounded `t:"s"`
frames retain a separate pre-parse control lane (refill 2/s, burst 4); no jam frame or unknown
extension can use that lane.
The main peer budget and auto-mute state belong to the authenticated fingerprint for the local
call epoch, not to one reconnectable channel: reconnecting cannot refill its bursts or clear an
auto-mute. Only the receiver's explicit unmute resets it; leaving clears the call epoch.

**Messages.** `t:"s"` is unchanged. Note v2: `{t:"n",on:1,n:0..127,w:<legacy wave>,p?:<id>,q}` and
`{t:"n",on:0,n,q}`. `q` is a per-sender uint32, monotonically increasing across all note and drum
events for one jam session (session = one `sn`, below); gaps mean lost events, which recorders
record and live rendering ignores. `w` MUST always be one of the four legacy waves so old builds
render something; `p` is honoured only after a validated announce matched it, else the receiver
falls back to `w`. Drum hit: `{t:"d",n:<pad 0..9>,q}`. A distinct type is required for mixed
versions: an old build ignores it, whereas `t:"n",d:1` would make that build hold inaudible MIDI
notes 0..9 forever because drums deliberately have no note-off. Patch announce:
`{t:"p",v:1,id:<64 hex>,sn:<16 hex>,d:{descriptor}}`,
sent on channel open and on patch change, rate-limited to 1 per 2 s (burst 3). `sn` is a random
per-call-join sender-session nonce; all-zero is reserved for the tagged legacy receive path and
rejects on v2 frames. A changed `sn` resets that sender's `q` domain. The receiver
validates the descriptor, canonicalizes, hashes, and requires the hash to equal `id`, else the
whole announce is discarded (legacy `w` continues). Per-peer validated-patch cache: 4 entries, LRU.
"Legacy timbres only" is a receive-side preference and signals nothing on the wire.
For the other mixed-version direction, a new receiver accepts only the exact pre-v2 note shapes
`{t:"n",on:1,n,w}` / `{t:"n",on:0,n}` and normalizes them onto a receive-only legacy session and
local sequence counter bound to that authenticated channel generation. Extra fields still reject;
legacy note-ons spend the same musical bucket and receive the same held/watchdog limits. This
compatibility path cannot announce patches, drums, metronome state, or takes.

**`jam-patch:v1` descriptor.** One fixed topology; data interpreted by a receiver-controlled
synthesizer. No arbitrary nodes or edges, no feedback routing, no AudioWorklet/WASM/scripts, no
samples, no external assets, no user automation curves; these are explicit non-goals, not
omissions. All values are integers; unknown fields REJECT (strictness keeps the hash meaningful).
Fields, in canonical order: `v` (literal 1); `o`: 1..3 oscillators, each
`{w:0..3 sin/tri/sqr/saw, t:-24..24 semitones, c:-50..50 cents, l:0..100}`; `e`:
`{a:0..5000, d:0..5000, s:0..100, r:0..8000}` (ms / percent); `f`:
`{m:0..2 LP/HP/BP, c:20..18000 Hz, q:0..100, e:-100..100}`; `l` (LFO):
`{r:1..1200 centi-Hz, d:0..100, t:0..2 off/cutoff/pitch}` (pitch depth caps at +/-25 cents); `x`
(sends): `{c:0..100, d:0..100, r:0..100}` into receiver-owned room-level chorus/delay/reverb
buses (fixed implementations; never one effect network per patch). Canonical serialization is
JSON with exactly these keys in exactly this order and no whitespace; the id is the full 64
lowercase hex characters of SHA-256 over the UTF-8 bytes. Truncating it to 64 bits would permit
generic collisions after roughly 2^32 work for no useful wire saving. Ids are immutable: editing mints a new id, and no id's
meaning is ever redefined. Validation is identical regardless of source: wire, local storage,
import, or take playback.

**Renderer contract `mewtual-synth:v1`.** Voice graph, fixed: oscillator stack, voice gain
(envelope), filter (envelope + LFO), per-peer instrument bus (mute/Deafen/level), dry plus sends
to the room effect buses, master limiter, destination. The contract promises stable parameter
meanings and topology, NOT identical samples; platform and implementation may colour the sound. A
materially different interpretation is `mewtual-synth:v2`, never a silent change to v1. Oscillator
levels are normalized as a blend under a receiver-owned 0.11 voice peak (all-zero is silent);
filter Q maps linearly to 0.1..18, filter-envelope amount to +/-6 octaves, cutoff LFO depth to up to
0..4 octaves (further reduced when needed to keep the whole envelope/LFO sweep within 20 Hz and
45% of sample rate), pitch LFO depth to 0..25 cents, and each effect send to 0..1 gain (raised from 0..0.5 on
2026-09-06: stacked with the effects' own wet returns, a maxed send arrived about 16 dB under the
dry signal, so the knobs moved and nothing was audibly different). The room master is
0.72 into a fixed compressor/limiter (-12 dB threshold, 6 dB knee, 12:1 ratio, 3 ms attack,
250 ms release). The receiver also clamps filter frequency to 45% of its sample rate, and floors
every release ramp at `JAM_VOICE_DECLICK_SECONDS` (8 ms), because a patch may legally ask for a
release of 0 and cutting a sustained waveform mid-cycle is a step discontinuity. That floor is
applied by one calculation (`effectiveReleaseSeconds`) shared by the note-off, the audio-clock
watchdog and the take transport's end-of-log horizon; a floor only some paths honour is not one.
These are **renderer** levels, not descriptor semantics: a patch id names a recipe, and
`mewtual-synth:v1` has never promised identical audio across builds, so a peer on an older build
hears the older amounts of the same recipe.
Deafen disconnects the entire old room graph rather than merely zeroing its master, so buffered
delay feedback cannot reappear after undeafening. Short call UI cues never create or resume a
suspended context, are cancelled by Deafen and leave, overlap at most four voices globally, and
share one receiver-owned limiter instead of connecting oscillators directly to the destination.

**Voice allocation.** A voice is one sounding note; every slot is sized for the worst permitted
patch (3 oscillators + filter + LFO), so there is no abstract cost model. Per-peer held cap stays
16; all reconnect lanes attributed to one performer inside a take share that cap. Playback uses
an engine-minted owner domain per validated take/participant, not the recorder-attested label, so
a take that claims a live fingerprint can neither consume that live peer's allowance nor choke or
preferentially steal its tails. The global ceiling is 64 voices, release and drum tails included. Allocation at the ceiling
steals the requesting sender's own oldest RELEASING tail first, then the oldest releasing tail
globally; if every voice is genuinely held, the new note is rejected; a held voice is never
stolen (fairness: stealing must not be a griefing mechanism). Stealing and every teardown path
disconnect all nodes and cancel all voice-owned scheduled automation, not merely `stop()`. A remote note held
over 30 s auto-releases (watchdog: a lost note-off may never drone). Repeated note-on for an already
held `(source,note)` is rejected, while a same-pitch note may be retriggered once the prior voice is
a releasing tail. On an inst-channel reopen the receiver replaces that channel's capability,
clears that peer's held state, and both sides re-announce state + patch via the existing `onopen`
push.

**`jam-kit:v1` drums.** Ten fixed pads: 0 kick, 1 snare, 2 rim, 3 clap, 4 closed hat, 5 open hat,
6 low tom, 7 high tom, 8 ride, 9 crash. Recipes are receiver-owned under the renderer contract;
max tail 3000 ms; tails occupy voices. Choke groups are SOURCE-SCOPED: pad 4 chokes pad 5 from
the same sender only. Noise is seeded deterministically per event from the canonical JSON array
SHA-256(`["catcoms-jam-drum:v1",callChannelId,senderFp,sn,q,pad]`); `callChannelId` is the
shared channel identifier, never the device-local bridge server number. Take playback retrieves
that stable call id and original performer from an opaque engine-validated archival capability,
while its synthetic channel remains only the sequencing, lifecycle, bus and mute authority; an
engine-minted take/participant owner aggregates allocator fairness, pending-work budgets and choke across
reconnect lanes without trusting the stored performer label as live authority. The seed selects the same
recipe, not identical audio. One digest per opaque channel generation commits in event order, with
32 active globally; stale-channel work cannot head-of-line block its replacement. Pending work is
independently bounded at 256 per engine-owned performer and 512 globally. Live, remote and take
integration applies causal backpressure instead of treating those bounds as ordinary event drops;
disposing an engine resolves all work that has not begun. No allocator slot/node is created until
the digest finishes and the channel, session, Deafen and audio-state checks pass again. Deafen and
per-source mute advance App admission generations on both gate edges and engine render generations
when work is revoked, so neither a frame retained by the outer causal queue nor an older digest can
exploit an on→off ABA transition to emerge after the receiver reopens audio. Local input captures the
same admission generation before asynchronous patch publication. Keys vs Pads is local presentation;
the wire carries only events.

**Metronome + clock.** While inactive, the first valid authenticated
`on:1` edge becomes the anchor; while active, other senders cannot supersede it. The anchor leaving
or sending `on:0` stops the metronome (dumb failover: anyone restarts under their own revision
domain). `{t:"m",v:1,sn,on:0|1,rev,bpm:40..240,bpb:1..8,org}` where `org` is beat-0 in the anchor's clock
domain; `rev` is uint32 monotonic only within that authenticated `(sender,sn)` session and tempo
revisions under 2 s apart are dropped. A newer `on:0` is a lifecycle edge and bypasses that
throttle, so a quick start/stop cannot strand remote clicks. This prevents a foreign
`rev=0xffffffff` from wedging every future
anchor. Simultaneous starts may choose differently until stopped; the metronome is coordination,
not Byzantine consensus. Offset estimation is
NTP-style over `{t:"c",q,tx}` / `{t:"c",r,tx,rx}` probes (1/s, burst 4); a correction beyond
+/-2000 ms from an already established estimate marks the clock unsynced and the click runs
local-only. Incoming probes have the same dedicated 1/s, burst-4 bucket before any reply, and
replies are accepted only once against one of at most four exact outstanding `(q,tx)` probes.
The lowest-RTT estimate is selected only from the latest eight samples so ordinary drift can age
out an old minimum. The first finite offset is not clamped because separate `performance.now()` clocks
have arbitrary origins. Clicks are scheduled against `AudioContext.currentTime` with a 150 ms
lookahead scheduler (webview timers throttle unfocused). A suspended context creates no click
nodes, cancels any already-scheduled lookahead nodes, and advances past missed beats before resume
instead of replaying them. This clock feeds ONLY the jam layer:
never authorization, expiry, freshness,
or persistence.

**`jam-take:v1` recordings *(phase 5; durable commitments remain phase 6)*.** Header
`{v:1, group, call, met:{bpm,bpb}, parts:[fingerprints], lanes:[{src:<parts index>,sn}],
patches:[full descriptors]}`. A note-on event carries `{ms,lane,n,on:1,w,p?:<patch index>,q}`;
note-off carries `{ms,lane,n,on:0,q}`; drum hit carries `{ms,lane,n:<pad>,d:1,q}`. The lane keeps
the authenticated source and its sequence/seed nonce together across reconnects, while the patch
index and legacy wave make the take independently playable. Events are stamped on the receiver's
local monotonic take timeline when the authenticated event is admitted. Caps: 10 minutes,
20 000 events, 512 KiB serialized, 16 participants, 64 lanes and 64 patches; group/call/performer
identity strings are each capped at 256 UTF-8 bytes. Playback validates
and hashes the bounded patch table once per take into an engine-owned archival table; it does not
multiply work by lanes or force archival recipes through the live peer's four-entry LRU. Across
rapid replacements, one preparation runs and only the latest waits; superseded work checks
cancellation between hashes, and leaving replaces the coalescer. At end-of-log, the scheduler first
moves any unmatched held note into its ordinary release envelope. Source teardown then waits for
the longest receiver-bounded patch/drum tail used by the take (up to 8 s), so a legal final release
is not truncated and a missing final note-off cannot sustain until hard teardown. Playback dispatches
at most 128 overdue events per macrotask pass and awaits each asynchronous drum before advancing,
so a valid dense/seeked take cannot monopolize the WebView, overflow the bounded digest lane, or
launch its whole event log concurrently. Each lane's `src` derives from the
authenticated channel at record time, never from a sender-supplied event field; `q` gaps are
surfaced, and the guarantee is
"musically aligned given the events received", not bit-identical. Recording state is visible to
the whole call and is an honest-client consent mechanism, not prevention. Takes are ephemeral
first. Local withdrawal updates the recorder gate synchronously before it is signalled. Every
decoded musical frame captures its receipt time, recorder identity and monotonic uninterrupted-
recording generation before entering the App causal queue; note, drum and later digest completion
can append only under that exact lease. An event received before consent, during withdrawal, or
before a recording→arming→recording cycle therefore cannot drift into the later interval. Losing a peer edge withdraws that
edge's consent before membership reconciliation; reconnect requires a fresh `rc`. Consent pauses
retain the take's original monotonic time origin, preventing resumed events from moving backward.
Saved takes go through the existing sealed blob + expiry + sharing machinery as an
application-specific format with its own type: the player re-runs this section's patch validator,
never hands bytes to a generic media decoder, never creates or resumes suspended audio from remote
transport, and byte-caps raw take text before
JSON parsing. Jukebox ingress also rejects a listing above 512 KiB before whole-file fetch, bounds
base64 before decode, and retains at most eight validated takes per call.
Every whole-file take load is serialized/coalesced to one running plus one latest request. Its
continuation is bound to the exact call lifecycle lease, server, channel and deck CID, and current
listing/size/trust admission is rerun after download before parsing, caching or starting playback.
Before `download_file`, the take path reserves one `begin_inline_download(cancellation)` token;
`cancel_inline_download(cancellation)` is observed inside the detached chunk wait, not merely
at the JavaScript continuation. Cancellation acknowledgement promptly retires the old JavaScript
coordinator slot. If libp2p already submitted a request, a shared native keepalive leaves that
registration charged until the exact request responds, fails, or times out; at most four such
lower requests exist process-wide, so four withholding debts deliberately pause new inline reads
instead of growing work without bound. Registrations use a small inert id grammar and exact
generation/RAII identity; caller ids include a cryptographic per-WebView nonce before resettable
call/sequence counters, so a reloaded view cannot collide with an active predecessor. Registration
holds the exact UI-session commit guard; explicit lock
signals every active slot and drops every unclaimed one. An unclaimed begin-only slot may be
displaced at the cap so a webview reload cannot starve it, including replacement by the same id.
Ordinary callers may omit `download_file.cancellation` for command compatibility, but native code
still assigns those reads a reserved, lock-cancellable slot under the same process-wide cap.
Tokenized `download-progress` carries that exact cancellation id; take UI accepts it only for the
current call lease, server and CID. Tokenless compatibility/media progress cannot impersonate a
take, so a queued old-group provider fingerprint cannot enter a replacement deck.
Sheet output floors
durations after one sixteenth but uses one sixteenth as the explicit visible minimum for shorter
taps. Native export accepts only the inert versioned renderer grammar and holds the exact UI
generation guard through plaintext write and reveal. Performer labels in v1 shared takes are
honestly presented as recorder-attested; the current `group` field is only a recorder-local scope
marker because the frontend does not yet receive the stable group id. It is not a durable binding;
durable attribution is phase 6, one signature per participant over the domain
`catcoms-jam-take-commitment:v1` and `{take id, stable group id, call id, device, aggregate hash of
all that participant's session lanes}`, which also exposes unrepaired gaps and prevents a
commitment being replayed into another group.
Phase 6 remains blocked until the native bridge exposes that stable group id.

**`jam-patch:v1` in the share (`.jampatch`, `application/x-mewtual-jampatch`).** A patch announce
tells the room how to render YOUR notes for the length of a call; it gives nobody a copy of the
sound and nobody a way to play through it themselves, so passing a patch to a friend meant reading
the knobs out loud. A patch can therefore be sealed into a server's share, through the same blob +
expiry + sharing machinery as a take. The file is exactly the canonical `jam-patch:v1` JSON the
wire announce carries and nothing else, so the ONE validator (`validateJamPatch`) admits both and a
downloaded patch can never be a shape the synth has not already agreed to render; the name is the
file's name, because a recipe has no identity of its own beyond its id. Ingress is bounded twice:
the listed size is refused above `JAM_PATCH_FILE_MAX_BYTES` (4 KiB) before the whole-file fetch, and
the transport string and its decoded length are refused again before `JSON.parse`. A loaded patch is
kept in the same twelve-slot local library as a saved one and becomes the loader's own sound.

### Bounded file fetch and kept-copy contracts

`MeshTransport::request_connected_cancellable` fails closed by default and requires a live connection
at actual driver admission. `ChannelSync::{prepare_blob_fetch, authenticate_blob_fetch,
complete_blob_fetch}` split one opaque authenticated attempt from actor-owned validation/storage.
`ServerActor::{fetch_file_chunk_cancellable, read_file_range}` use bounded detached network workers;
ranges above `CHUNK_BYTES` are rejected before I/O. See [limits and lifecycle](design-file-reliability.md).

The local-only `get_kept_files(server)`, `keep_file(server,cid,cancellation)` and
`forget_kept_file(server,cid)` commands expose explicit device ownership. Keep claims a native
`begin_inline_download` registration; lock/cancel signals the exact lease. Inventory returns bounded
CID/version/checked rows plus conservative allocated bytes and a fixed local limit, never saved
wrapped manifests or keys. Existing copies load unchecked and can be explicitly checked/repaired
from their saved manifest after unlisting. They do not enter `files()` or authorize media heads.
The additive `BlobStore` keep methods fail closed on unsupported stores. `KeptBlobStore` reserves,
verifies, flushes and separately owns copies; ordinary put/delete/staging target primary storage.
Remote confirmations and automated retention/eviction are not part of this interface.
