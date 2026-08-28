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
    async fn request(&self, peer: PeerId, proto: ProtocolId, data: Bytes) -> Result<Bytes, TransportError>;
    async fn notify(&self, peer: PeerId, proto: ProtocolId, data: Bytes) -> Result<(), TransportError>;
    async fn next_event(&self) -> Option<TransportEvent>;   // single-consumer
    async fn rendezvous_register(&self, namespace:&str, rz_node:&[u8]) -> Result<(),TransportError>;
    async fn rendezvous_discover(&self, namespace:&str, rz_node:&[u8]) -> Result<(),TransportError>;
    async fn dial_addr(&self, addr:&str) -> Result<(),TransportError>;
    async fn add_external_addr(&self, addr:&str) -> Result<(),TransportError>;
    async fn next_discovered(&self) -> Option<DiscoveredPeer>; // default never resolves
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
pub const MAX_CONNECTED_PEER_SNAPSHOT: usize = 320;
pub const MAX_CONNECTION_PATH_SNAPSHOT: usize = 64;
pub enum ConnectionFamily { Ipv4, Ipv6, Dns, Memory, Unknown }
pub enum ConnectionTransport { Tcp, QuicV1, WebSocket, CircuitRelay, Memory, Unknown }
pub enum ConnectionDirection { Dialer, Listener }
pub struct Responder;  fn respond(self, Bytes);  fn channel() -> (Responder, ResponderRx);
pub struct ResponderRx; async fn recv(self) -> Option<Bytes>;
pub enum TransportError { Unreachable(PeerId), Timeout(PeerId), Closed, NoResponse }
```
Implementations:
- **`MemNetwork`** (tests): `let hub = Hub::new(); let net = hub.join(PeerId::from_u64(n));`
- **`MeshService`** (prod, catcoms-net): `spawn(swarm)` / `new_memory(listen, dial)` /
  `new_tcp(...)`; `build_memory_swarm()` / `build_tcp_swarm()`. Maps `PeerId`↔libp2p
  PeerId, hex-encodes topics, queues+retries publishes until a subscriber appears.
  `MeshHandle::request_control_connected_only(peer, data)` is the deliberately narrow reciprocal-
  proof send: the actor succeeds only when its current peer map and `Swarm::is_connected` both say
  the transport is live. Unlike ordinary `request_control`, it never consults `recent_peers` and
  cannot implicitly redial after the shared scheduler denied a new socket attempt.
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
  reserve(&self, server:&[u8], endpoints:&[DialEndpoint], clock:&dyn Clock)
    -> Vec<String>;
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
refunded to the local policy because no dial command was submitted. Scheduler state is session-only,
uses `Clock::monotonic_ms`, and accounts submissions rather than actor-confirmed sockets; opaque
single-use permits/refunds and process-wide in-flight leases remain future hardening. Pre-join invite
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
pub struct SignedOp { doc_type:DocType, doc_id:u128, author_device:DeviceId, author_pubkey:Vec<u8>, delta:Vec<u8>, signature:[u8;64] }
  sign(&MlsDevice, DocType, doc_id, delta:Vec<u8>) -> Result<Self>;  verify()->bool;  encode()/decode();  hash()->[u8;32];
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
```

---

## 5. Storage & retention  *(catcoms-storage)*

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
  async post(DocType, doc_id, FnOnce(&mut AutoCommit)->Result<(),AutomergeError>) -> Result<()>;  // edit + gossip
  async run_once() -> Result<bool>;                        // drain outbox + recovery + sub-resync; then handle ONE event
  async request_catchup(peer:PeerId, DocType, doc_id) -> Result<usize>;        // document history catch-up
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
  cache_known_records() -> usize;  async dial_cached_peers() -> usize;
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
- **`CommitRecord`** (control payload): `bytes group_id ‖ u64 commit_epoch ‖ bytes committer_device(32) ‖ bytes mls_commit ‖ bytes base_auth(32) ‖ bytes committer_sig(64)`.
- **Request/response** (`ProtocolId("/catcoms/rr/1")`): first payload byte = **kind**:
  - `0` KIND_CATCHUP; **authed** body wrapping `u16 doc_type ‖ u128 doc_id`; response = op bundle.
  - `1` KIND_JOIN; body `bytes invite.encode() ‖ bytes key_package`; response =
    `[JOIN_READY] ‖ bytes welcome ‖ bytes signature(64) ‖ bytes sealed_routing` (the admitter signs
    `join_transcript = "catcoms/join-resp/v1" ‖ group_id ‖ nonce ‖ welcome ‖ sealed_routing`). Not member-authed.
  - `2` KIND_COMMIT_CATCHUP; **authed** body wrapping `u64 from_epoch`; response is **responder-signed**:
    `bytes responder_pubkey ‖ bytes sig(64) ‖ bytes bundle`, sig over `"catcoms/catchup-resp/v1" ‖ group_id ‖ requester_pubkey ‖ u64 req_ts ‖ nonce(16) ‖ u64 req_epoch ‖ bundle`.
  - `4` KIND_PEX (6e-3d-7); **authed** body (empty); response responder-signed like commit catch-up but under
    `"catcoms/pex-resp/v1"`; bundle = `u32 count(≤64) ‖ len-prefixed PeerDescriptor`s, each self-signed under `"catcoms/peer-record/v1"`.
  - **Authed body** (members-only gate): `bytes inner ‖ bytes requester_pubkey ‖ u64 timestamp_ms ‖ bytes nonce(16) ‖ u64 req_epoch ‖ bytes signature(64)`,
    signature over `"catcoms/catchup-auth/v1" ‖ group_id ‖ u16 kind ‖ inner ‖ requester_pubkey ‖ timestamp_ms ‖ nonce ‖ req_epoch`.
    Served only if `requester_pubkey` content-addresses a **current member**, the timestamp is fresh (`MAX_REQUEST_AGE_MS` 60s),
    and the signature verifies. The per-request **nonce + epoch** (6e-3d-6) bind the responder's signed reply to *this exact*
    request, so a captured response cannot be replayed (closes the same-ms `ts`-collision window).
  - Caps: requests 64 KiB; responses 16 MiB (catch-up) / **512 KiB (PEX)**; bundle element counts capped.
- **`InviteToken`** signed payload (v2): `"catcoms/invite/v2" ‖ group_id ‖ inviter_device_id ‖ inviter_public_key ‖ nonce ‖ u64 expires ‖ u32 n ‖ n×bootstrap_str ‖ u32 m ‖ m×rendezvous_str`, then `signature(64)`.

## 8. `DocType` tags (stable; only append)
`Channel=1, Wiki=2, Status=3, Calendar=4, InviteLedger=5, MemberRoles=6, FileIndex=7, Routing=8,
Profile=9, Livery=10, Badges=11, Devices=12, ChannelIndex=13, Moderation=14`.
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
}
impl ServerStore {
    fn save_ui_state(&self, json:&[u8], rng:&mut impl CryptoRngCore) -> Result<(),AppError>; // ≤1 MiB, vault-sealed + atomic
    fn load_ui_state(&self) -> Result<Vec<u8>,AppError>;
    fn backup_source_dir(&self) -> &Path;
    fn change_passphrase(&self, current:&[u8], new:&[u8], rng:&mut impl CryptoRngCore) -> Result<(),AppError>;
}
```

`storage_health` counts a chunk as verified only after its storage seal/content address and the
file-layer decryption both succeed. `repair_storage` explicitly fetches only missing or unreadable
referenced chunks over the authenticated blob path and verifies again; `has()` alone must never
short-circuit repair.

The Tauri `get_storage_health(server)` command adds a cached, deduplicated inventory projection:
`checked_at_ms`, unique/logical/local-estimated/pinned totals, category rows, and the ten largest
files. It performs at most one ordinary scan per server per process session. Only
`repair_storage(server)` replaces that cache after its mandatory post-repair verification.

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
rewraps the unchanged root DEK under a fresh salt/nonce. Existing derived data keys do not rotate;
older exported vaults remain bound to their old secret.

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
