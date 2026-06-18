# CatComs — Interfaces & Hooks Schema

A reference for the **seams** (dependency-injection hooks) and the key public APIs.
Signatures are abbreviated; see the source for exact generics/lifetimes. This is the
contract a new contributor (or agent) builds against.

---

## 1. The seams (the load-bearing hooks)

Everything above these is generic over them, so the whole stack runs unchanged over
test impls (in-memory, deterministic) or production impls (OS / libp2p).

### `Clock` — injected time  *(catcoms-rt)*
```rust
pub trait Clock: Send + Sync + Debug { fn now_ms(&self) -> u64; }
pub struct SystemClock;                         // the ONLY OS-clock reader
pub struct ManualClock;  fn new(start_ms) -> Self;  advance_ms(delta)->u64;  set_ms(v);
```
Rule: no other code reads the OS clock. Pass `&dyn Clock` / `Box<dyn Clock + Send>`.

### RNG — injected randomness  *(catcoms-rt)*
```rust
pub use rand_core::{CryptoRng, CryptoRngCore, RngCore};
pub struct OsCryptoRng;   // the ONLY OS-RNG source; impl CryptoRngCore
```
Rule: take `&mut impl CryptoRngCore` (or generic `R: CryptoRngCore`). `Box<dyn
CryptoRngCore>` does **not** satisfy the bound (CryptoRng isn't forwarded through
`&mut dyn`) — be generic over the concrete RNG instead (see `ChannelSync<T, R>`).

### `MeshTransport` — pub/sub + request/response  *(catcoms-rt)*
```rust
pub trait MeshTransport: Send + Sync {
    fn local_peer(&self) -> PeerId;
    async fn subscribe(&self, topic: Topic) -> Result<(), TransportError>;
    async fn unsubscribe(&self, topic: Topic) -> Result<(), TransportError>;
    async fn publish(&self, topic: Topic, data: Bytes) -> Result<(), TransportError>;
    async fn request(&self, peer: PeerId, proto: ProtocolId, data: Bytes) -> Result<Bytes, TransportError>;
    async fn next_event(&self) -> Option<TransportEvent>;   // single-consumer
}
pub struct PeerId([u8;32]);   fn from_u64(n)->Self; fn as_bytes()->&[u8;32];
pub struct Topic(Bytes);      fn new(impl Into<Bytes>)->Self; fn as_bytes()->&[u8];
pub struct ProtocolId(pub &'static str);
pub enum TransportEvent {
    Gossip { topic: Topic, from: PeerId, data: Bytes },
    Request { from: PeerId, proto: ProtocolId, data: Bytes, responder: Responder },
    PeerConnected(PeerId), PeerDisconnected(PeerId),
}
pub struct Responder;  fn respond(self, Bytes);  fn channel() -> (Responder, ResponderRx);
pub struct ResponderRx; async fn recv(self) -> Option<Bytes>;
pub enum TransportError { Unreachable(PeerId), Timeout(PeerId), Closed, NoResponse }
```
Implementations:
- **`MemNetwork`** (tests): `let hub = Hub::new(); let net = hub.join(PeerId::from_u64(n));`
- **`MeshService`** (prod, catcoms-net): `spawn(swarm)` / `new_memory(listen, dial)`;
  `build_memory_swarm()` / `build_tcp_swarm()`. Maps `PeerId`↔libp2p PeerId, hex-encodes
  topics, queues+retries publishes until a subscriber appears.

### `SecureKeyStore` — at-rest DEK protection, tiered  *(catcoms-crypto)*
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
(OS-native impls — Secret Service / DPAPI / Android Keystore — are platform-phase work.)

### `HolderOracle` — fresh replica count  *(catcoms-storage)*
```rust
pub trait HolderOracle { fn reachable_holder_count(&self, cid: &Cid) -> usize; }
```
Injected into GC so it never evicts the last copy. (Network-backed impl is later.)

### `BlobStore` — content-addressed bytes  *(catcoms-storage)*
```rust
pub trait BlobStore {
    fn put(&mut self, bytes: &[u8]) -> Result<Cid, StorageError>;       // cid = Cid::of(bytes)
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StorageError>;  // integrity-checked
    fn has(&self, cid: &Cid) -> bool;
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
pub struct AccountKeypair; generate(rng); ...; user_id(); sign(...);
pub fn verify(&VerifyingKey, msg, &[u8;64]) -> bool;                 // strict
pub fn verify_with_public_bytes(pubkey: &[u8], msg, &[u8;64]) -> bool;

// Device-cert chains (account key signs founding device; devices cross-certify).
pub enum CertSigner { Account, Device(DeviceId) }
pub struct DeviceCert { user_id, device_id, device_pubkey:[u8;32], signer, created_at_ms, nonce:[u8;16], signature:[u8;64] }
  new_account_signed(&AccountKeypair, &VerifyingKey, created, nonce); new_device_signed(&DeviceKeypair, user_id, ...);
pub struct DeviceRevocation { ... }   new_account_signed/new_device_signed
pub struct RosterConfig { max_chain_depth, max_devices }
pub struct Roster;  build(&account_vk, &[DeviceCert], &[DeviceRevocation], &RosterConfig) -> Result<Roster, CertError>;
  contains(&DeviceId); verifying_key(&DeviceId); device_count(); device_ids();

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
  designated_committer() -> Option<DeviceId>;   // lowest leaf index
  is_designated_committer(&MlsDevice) -> bool;
  create_application_message(&mut, &MlsDevice, &[u8]) -> Result<Vec<u8>>;
  process_incoming(&mut, &MlsDevice, &[u8]) -> Result<Incoming>;   // applies commits / app msgs
  channel_secret(&MlsDevice, DocType, doc_id:u128) -> Result<[u8;32]>;   // per-doc content key (epoch exporter)
  metadata_secret(&MlsDevice, DocType, doc_id:u128) -> Result<[u8;32]>;  // network identifiers (separate label)
  epoch()->u64; group_id()->Vec<u8>; member_count(); member_device_ids(); contains_device(&DeviceId);
pub struct AddOutcome { pub welcome:Vec<u8>, pub commit:Vec<u8>, pub commit_epoch:u64 }
pub enum Incoming { Application(Vec<u8>), CommitApplied, Other }

pub struct InviteToken {           // pasteable, single-use, device-bound capability
    pub group_id:Vec<u8>, pub inviter_device_id:DeviceId, pub inviter_public_key:Vec<u8>,
    pub invite_nonce:[u8;16], pub expires_at_ms:u64, pub bootstrap:Vec<String>, pub signature:[u8;64] }
  encode()->Vec<u8>; decode(&[u8])->Result<Self>;
  verify(inviter_pubkey)->bool;  verify_self()->bool;  verify_inviter_signature(msg, &[u8;64])->bool;
pub struct MembershipCredential { device_id, group_id, invite_nonce }  encode()/decode();  // the MLS leaf credential binding
pub struct InviteLedger;  new(); revoke(nonce); is_consumed(&nonce); is_revoked(&nonce);
  check(&InviteToken, now_ms) -> Result<(),InviteError>;  consume(nonce) -> Result<(),InviteError>;
```

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
  new(transport:T, group:ServerGroup, device:MlsDevice, rng:R, clock:Box<dyn Clock + Send>) -> Self;
  set_config(SyncConfig);                                  // override recovery/key-window bounds
  async subscribe_control() -> Result<()>;                 // receive membership commits
  epoch() -> u64;   stats() -> SyncStats;                  // SyncStats = diagnostic counters + gauges
  mint_invite(nonce:[u8;16], expires_at_ms, bootstrap) -> Result<InviteToken>;
  async open_channel(DocType, doc_id) -> Result<()>;       // create doc + subscribe blinded topic
  async post(DocType, doc_id, FnOnce(&mut AutoCommit)->Result<(),AutomergeError>) -> Result<()>;  // edit + gossip
  async run_once() -> Result<bool>;                        // drain outbox + recovery; then handle ONE event; early-returns if a catch-up fired
  async request_catchup(peer:PeerId, DocType, doc_id) -> Result<usize>;        // document history catch-up
  async request_commit_catchup(peer:PeerId, from_epoch:u64) -> Result<usize>;  // missed-commit recovery (ordered replay)
  doc(DocType, doc_id) -> Option<&EncryptedDoc>;  local_peer() -> PeerId;

// Bounds (all hard caps; Default suits a desktop node). past:8, commit_log:256,
// pending:256, gap:1024, peers:64, catchup_queue:256, outbox:256.
pub struct SyncConfig { max_past_epochs:u64, max_commit_log:usize, max_pending_commits:usize,
                        max_commit_gap:u64, max_known_peers:usize, max_catchup_queue:usize, max_outbox:usize }
pub struct SyncStats { commits_applied, commits_buffered, commits_served, commit_catchups_requested,
                       ops_ingested, ops_recovered_past_epoch, ops_dropped_future_epoch, ops_dropped_old_epoch,
                       doc_catchups_requested, requests_rejected: u64,
                       /* gauges: */ past_keys_retained, pending_commits, commit_log_len, known_peers: usize }

pub async fn request_join<T: MeshTransport>(transport:&T, inviter:PeerId, device:&MlsDevice, invite:&InviteToken)
    -> Result<ServerGroup, SyncError>;   // join over the wire; authenticates the inviter + rechecks group_id
```

**Recovery internals (private to `catcoms-sync`, for orientation):** `commit_log`
(VecDeque, served to peers) · `pending_commits` (BTreeMap, out-of-order buffer,
ordered replay when the gap fills) · `past_keys` (BTreeMap `(DocType,doc_id,epoch)`
→ `Zeroizing<[u8;32]>`, captured by `snapshot_epoch_keys` *before* each advance,
evicted past `max_past_epochs`) · `known_peers` (VecDeque, MRU catch-up sources) ·
`catchup_queue` (deferred `Commits{from_epoch}` / `Doc{type,id}` work).

---

## 7. Wire formats & protocol kinds (the on-wire schema)

All multi-byte ints big-endian; all variable fields length-prefixed (`catcoms-wire`).

- **Channel op topic** (gossip): `BLAKE3("catcoms/topic/v1" ‖ group_id ‖ u16 doc_type ‖ u128 doc_id)`.
- **Control topic** (gossip, membership commits): `BLAKE3("catcoms/control/v1" ‖ group_id)` (stable per group).
- **`SealedOp`** (gossip payload): `u16 doc_type ‖ u128 doc_id ‖ u64 epoch ‖ bytes nonce(24) ‖ bytes ciphertext`.
  Ciphertext = XChaCha20-Poly1305 of the encoded `SignedOp` under `channel_secret(doc,epoch)`.
- **`CommitRecord`** (control payload): `bytes group_id ‖ u64 commit_epoch ‖ bytes committer_device(32) ‖ bytes mls_commit`.
- **Request/response** (`ProtocolId("/catcoms/rr/1")`): first payload byte = **kind**:
  - `0` KIND_CATCHUP — **authed** body (see below) wrapping `u16 doc_type ‖ u128 doc_id`; response =
    op bundle (`u32 count ‖ len-prefixed SealedOps`), a contiguous prefix within the response budget.
  - `1` KIND_JOIN — body `bytes invite.encode() ‖ bytes key_package`; response = `bytes welcome ‖ bytes signature(64)`
    (the admitter signs `join_transcript = "catcoms/join-resp/v1" ‖ group_id ‖ nonce ‖ welcome`). **Not** member-authed
    (the joiner isn't a member yet — it carries the invite/inviter auth instead).
  - `2` KIND_COMMIT_CATCHUP — **authed** body wrapping `u64 from_epoch`; response = `u32 count ‖ len-prefixed CommitRecords`
    with `commit_epoch >= from_epoch`, ascending, contiguous prefix within budget (empty if none).
    The requester replays them in order via `process_incoming` (missed-membership-commit recovery).
  - **Authed catch-up body** (members-only gate): `bytes inner ‖ bytes requester_pubkey ‖ u64 timestamp_ms ‖ bytes signature(64)`,
    where `inner` is the kind's own body and the signature is over
    `catchup_auth = "catcoms/catchup-auth/v1" ‖ group_id ‖ u16 kind ‖ inner ‖ requester_pubkey ‖ timestamp_ms`.
    The server serves only if `requester_pubkey` content-addresses a **current member** (`group.contains_device`),
    the timestamp is within `MAX_REQUEST_AGE_MS` (60s), and the signature verifies.
  - Requests capped at 64 KiB (`MAX_CONTROL_REQUEST`); responses capped at 16 MiB
    (`MAX_CATCHUP_RESPONSE` inbound, `MAX_CONTROL_RESPONSE` on the serving side);
    bundle element counts capped (`MAX_BUNDLE_ELEMENTS`).
- **`InviteToken`** signed payload: `"catcoms/invite/v1" ‖ group_id ‖ inviter_device_id ‖ inviter_public_key ‖ nonce ‖ u64 expires ‖ u32 n ‖ n×bootstrap_str`, then `signature(64)`.

## 8. `DocType` tags (stable; only append)
`Channel=1, Wiki=2, Status=3, Calendar=4, InviteLedger=5, MemberRoles=6, FileIndex=7`.
Exporter context = `u16 tag ‖ u128 doc_id` (18 bytes, fixed-width → injective).
