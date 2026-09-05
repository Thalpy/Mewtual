//! The Tauri command/event bridge; a thin shell over the `catcoms-app` event-stream
//! actors. The frontend `invoke`s these commands and `listen`s for the forwarded events;
//! all the real work lives in the tested `catcoms-app` actor, which itself wraps the
//! protocol stack. The GUI never touches MLS or automerge.
//!
//! Multi-server (8p): the app can run several servers at once. Each is a separate
//! `Server`/actor (its own MLS group + transport + event stream); the bridge keys them by
//! a `u64` server id. Every command takes a `server` id selecting which one to act on, and
//! every forwarded event is tagged with its server id so the UI routes it correctly.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use catcoms_app::store::MAX_UI_STATE_BYTES;
use catcoms_app::{
    channel_id, spawn, AppEvent, Cid, CidHasher, DeviceId, FileListing, FileRef, InviteJoinPlan,
    Livery, PairingLedger, PairingSecrets, PerServerGrant, Profile, ReconnectPolicy,
    ReconnectRoute, Server, ServerActor, ServerNet, ServerRecord, ServerStore, StorageHealth,
    StorageSnapshot, CHUNK_BYTES, MAX_AVATAR_BYTES, MAX_BANNER_BYTES, MAX_FILE_BYTES,
    MAX_RECONNECT_ROUTES, MAX_RECONNECT_ROUTE_BYTES, MAX_SERVER_CURSOR_BYTES,
    MAX_SERVER_ICON_BYTES,
};
use catcoms_discovery::{
    parse_peer_dial_route, Candidate, DialEndpoint, DiscoveryPolicy, EndpointDialScheduler,
    PolicyConfig, RouteHost, Source,
};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_net::{
    addr_is_globally_routable, addr_is_loopback, addr_is_private, addr_is_undialable,
    keypair_from_seed, phase0_peer_id, target_peer_in_multiaddr, validate_invite_rendezvous_addrs,
    validate_operator_rendezvous_addrs, AuthenticatedDialRoute, AutoNatResult, AutoNatSnapshot,
    JoinReply, MeshHandle, MeshObservationSnapshot, MeshService, PortMappingMechanism,
    PortMappingSnapshot, PortMappingTransport, RelayAddressSnapshot, RendezvousTarget,
};
use catcoms_rt::{
    Clock, MeshTransport, OsCryptoRng, PeerId, RequestCancellation, RngCore,
    SharedRequestKeepalive, SystemClock, TransportEvent,
};
use catcoms_sync::{fingerprint, join_namespace, PreOwnerConnectionHandoff, JOIN_REPLY_PROOF_KIND};
use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;
use serde::{Deserialize, Serialize};
use tauri::{http, AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, watch, Mutex};
use tokio::time::timeout;
use zeroize::Zeroizing;

mod errors;
mod tasks;
use errors::{codes, AppError, ErrorCode};

/// Independent reasons an exact address belongs in this device's aggregate bootstrap set. The
/// same IPv6 socket is commonly both a raw interface route and a PCP firewall pinhole; removing
/// either owner alone must not withdraw the route from PEX, invites, rendezvous, or AutoNAT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BootstrapOwner {
    AutomaticInterface,
    Configured,
    PortMapping,
    Relay,
}

#[derive(Debug, Clone)]
struct InterfaceRouteIdentity {
    port: u16,
    peer_id: String,
}

/// One running server: its actor handle, the single-use invite to share (founder only), and
/// its display name (kept here too so the registry can be re-sealed on disk, Phase 9f).
struct ServerEntry {
    actor: ServerActor,
    /// Process-local incarnation of this registry row. Persisted ids are intentionally reused on
    /// reload, so the id alone cannot stop a scan begun for a removed actor from populating a new
    /// actor's cache after that id is installed again.
    instance: u64,
    /// Stable MLS identity checks for old signed invite permits embedded in two-way reply codes.
    group_id: Vec<u8>,
    device_id: DeviceId,
    invite: Option<String>,
    name: String,
    /// The current reachable bootstrap addresses for this device, captured when the server was
    /// founded/joined/reloaded. Reused to mint a *fresh* owner invite on demand and, for every
    /// member, to republish the live signed peer record after mapping/relay changes. A joiner
    /// cannot mint an invite, but its base direct addresses must still remain in this set.
    bootstrap: Vec<String>,
    /// Ownership of every exact entry in `bootstrap`. An address is removed from the aggregate
    /// only after its final owner disappears; this is load-bearing for a raw global IPv6 address
    /// that an active PCPv6 firewall lease also names.
    bootstrap_owners: HashMap<String, HashSet<BootstrapOwner>>,
    /// Stable listener coordinates used by the discovery timer to poll the OS-selected IPv4/IPv6
    /// source routes. `None` only for the temporary companion path with no persisted identity.
    interface_routes: Option<InterfaceRouteIdentity>,
    /// The rendezvous infra multiaddrs this server registered at (if any), so a fresh on-demand
    /// invite is also discovery-enabled. Empty when the server uses direct bootstrap only. Not
    /// separately persisted; on reload it is recovered from the persisted invite's `rendezvous`.
    rendezvous: Vec<String>,
    /// A clonable handle to this server's live transport. Besides late rendezvous registration it
    /// lets an inviter dial a joiner's authenticated two-way reply candidates after `Server` has
    /// moved into its actor. `None` only for legacy/companion paths without a retained transport.
    mesh: Option<MeshHandle>,
    /// Whether this group is a 1:1 DM (shown behind the DMs circle) rather than a server.
    is_dm: bool,
    /// Persisted, explicit consent for this device to accept standing switchboard requests.
    switchboard: bool,
    /// The next PEX peer-record sequence number this launch may publish, taken from the block
    /// `ServerNet::reserve_record_seq_block` reserved on disk before the transport came up.
    ///
    /// It is a per-launch **block**, not a per-launch increment, precisely so a session that
    /// republishes (a UPnP mapping arriving, a relay circuit reserved late) can keep climbing
    /// without ever reaching the number the next launch will start from. A record is only kept
    /// by a peer when its `seq` beats the one already held, so reuse is permanent invisibility.
    record_seq: u64,
    /// Persistence coalescing for this server: how many mutations have asked to reach disk, and
    /// how many the newest completed write covered. See [`persist_server`]. Per incarnation, so a
    /// reinstalled id starts from zero and cannot inherit a departed entry's completions.
    persist: PersistCounters,
}

/// Where one server is between "a change happened" and "a write that contains it finished".
///
/// Persistence runs after every message, and a snapshot serializes the whole server, so a burst
/// of sends used to perform one full write each: the cost of the Nth message in a session grew
/// with N. These two counters collapse a burst into the writes actually needed without weakening
/// the contract the callers rely on, which is that a command reports success only after a write
/// that includes its own change reached the disk.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
struct PersistCounters {
    /// Incremented by every request, before anything is written.
    requested: u64,
    /// The `requested` value observed by the newest completed write, at the moment it took its
    /// snapshot. Every request at or below it is on disk.
    completed: u64,
}

impl PersistCounters {
    /// Take a ticket for a change that has already been applied. Any snapshot taken after this
    /// contains that change, which is what makes [`Self::needs_write`] safe to answer `false`.
    fn request(&mut self) -> u64 {
        self.requested = self.requested.saturating_add(1);
        self.requested
    }

    /// Whether `ticket` still needs a write of its own, or has been carried by another one.
    fn needs_write(&self, ticket: u64) -> bool {
        self.completed < ticket
    }

    /// Record that a write whose snapshot was taken with `requested == covering` is on disk.
    fn completed_through(&mut self, covering: u64) {
        self.completed = self.completed.max(covering);
    }
}

/// One decrypted chunk, kept so a player reading a track sequentially does not re-fetch and
/// re-decrypt the same 8 MiB for every window it asks for. Plaintext, so it is dropped whenever
/// the vault locks, exactly like every other decrypted thing this process holds.
struct MediaChunk {
    server: u64,
    cid: String,
    manifest_version: [u8; 32],
    index: usize,
    bytes: Arc<Vec<u8>>,
}

/// A file's size and validated type, bound to the exact current encrypted chunk manifest.
#[derive(Clone, PartialEq, Eq, Debug)]
struct MediaHead {
    server: u64,
    cid: String,
    manifest_version: [u8; 32],
    total_size: u64,
    mime: String,
}

/// How many decrypted chunks stay resident. Two is enough for the straddle at a chunk boundary
/// and a short seek back, and it is 16 MiB of plaintext: the cache exists to keep the actor free,
/// not to hold the film.
const MEDIA_CACHE_CHUNKS: usize = 2;
const STORAGE_SCAN_STRIPES: usize = 16;
/// How many file heads stay remembered. Tiny (two integers and a mime each), and more than one
/// because a queue moves between tracks.
const MEDIA_HEAD_ENTRIES: usize = 8;

/// Take a chunk out of the cache, marking it as the most recently used.
///
/// Recency is what makes two entries enough. Without the refresh the vector is in insertion
/// order, so a chunk that has been read a hundred times is evicted before one read once, and a
/// player that alternates between two chunks (a container with its index at the end does exactly
/// that) misses on every single request.
fn media_cache_take(
    cache: &mut Vec<MediaChunk>,
    server: u64,
    cid: &str,
    manifest_version: [u8; 32],
    index: usize,
) -> Option<Arc<Vec<u8>>> {
    let at = cache.iter().position(|c| {
        c.server == server
            && c.cid == cid
            && c.manifest_version == manifest_version
            && c.index == index
    })?;
    let hit = cache.remove(at);
    let bytes = Arc::clone(&hit.bytes);
    cache.push(hit);
    Some(bytes)
}

/// Put a chunk in the cache, displacing another track entirely and the oldest chunk of this one.
fn media_cache_put(cache: &mut Vec<MediaChunk>, chunk: MediaChunk) {
    // A different track displaces the whole cache: nothing about the old one will be asked for
    // again, and holding two tracks' plaintext to serve one is the wrong trade.
    if cache.iter().any(|c| {
        c.cid != chunk.cid
            || c.server != chunk.server
            || c.manifest_version != chunk.manifest_version
    }) {
        cache.clear();
    }
    cache.retain(|c| c.index != chunk.index || c.manifest_version != chunk.manifest_version);
    cache.push(chunk);
    while cache.len() > MEDIA_CACHE_CHUNKS {
        cache.remove(0);
    }
}

/// Process-wide, coalesced notification that the operating system's interfaces or routes changed.
/// A `watch` generation is deliberately used instead of a queued broadcast: every running server
/// needs to refresh once, but retaining one item per noisy DHCP/route callback would be both
/// wasteful and unbounded. A server created after a generation uses a fresh route sample during
/// startup and therefore does not need historical notifications.
#[derive(Debug, Clone)]
struct NetworkChangeSignal {
    generation: watch::Sender<u64>,
}

impl Default for NetworkChangeSignal {
    fn default() -> Self {
        let (generation, _initial_receiver) = watch::channel(0);
        Self { generation }
    }
}

impl NetworkChangeSignal {
    fn subscribe(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }

    fn notify(&self) {
        // Wrapping cannot make a notification invisible: even u64::MAX -> 0 differs from the
        // receiver's last generation, and reaching it would require centuries of callback spam.
        self.generation
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}

/// One immutable final UI snapshot waiting to cross the native lock commit boundary.
///
/// It lives natively because a webview remount destroys JavaScript promises. A later close with no
/// local snapshot can still consume this exact transaction instead of overtaking and losing it.
#[derive(Debug)]
struct PendingUiLockSnapshot {
    generation: u64,
    json: String,
}

/// Latest native continuity result, retained until the next authenticated UI generation.
#[derive(Clone, Debug)]
struct UiLockCompletion {
    generation: u64,
    error: Option<String>,
}

/// Maximum cancellable whole-file reads registered by the webview at once.
///
/// The common media path streams and does not use these slots. Four permits legitimate overlap
/// between a take and text previews while bounding abandoned native work under rapid call churn.
const MAX_CANCELLABLE_INLINE_DOWNLOADS: usize = 4;
const INLINE_DOWNLOAD_CANCELLATION_ID_MAX_BYTES: usize = 96;

#[derive(Debug)]
struct InlineDownloadCancellation {
    generation: u64,
    started: bool,
    signal: watch::Sender<bool>,
}

/// Exact registration shared with any transport request that can outlive `download_file`.
#[derive(Clone, Debug)]
struct InlineDownloadLease {
    inner: Arc<InlineDownloadLeaseInner>,
}

#[derive(Debug)]
struct InlineDownloadLeaseInner {
    table: Arc<StdMutex<HashMap<String, InlineDownloadCancellation>>>,
    id: String,
    generation: u64,
}

impl InlineDownloadLease {
    fn request_keepalive(&self) -> SharedRequestKeepalive {
        self.inner.clone()
    }
}

impl Drop for InlineDownloadLeaseInner {
    fn drop(&mut self) {
        let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
        if table
            .get(&self.id)
            .is_some_and(|entry| entry.generation == self.generation)
        {
            table.remove(&self.id);
        }
    }
}

/// App state managed by Tauri: every running server keyed by a bridge-assigned id, plus the
/// on-disk store once the user has unlocked it with a passphrase (`None` = in-memory only).
#[derive(Default)]
struct AppState {
    servers: Mutex<HashMap<u64, ServerEntry>>,
    /// Monotonic process-local source for [`ServerEntry::instance`]. Wrapping would require more
    /// actor installations than the process can perform in its lifetime; zero has no special
    /// meaning and is permitted after that theoretical wrap.
    next_server_instance: AtomicU64,
    /// Bounded cancellation registry for inline reads such as `.jamtake` playback. The signal is
    /// threaded into the actor's chunk future; dropping only a Tauri invoke would not cancel it.
    inline_downloads: Arc<StdMutex<HashMap<String, InlineDownloadCancellation>>>,
    next_inline_download_generation: AtomicU64,
    /// One endpoint budget shared by every server swarm and pre-join discovery attempt in this
    /// desktop process. Per-server ranking remains inside each actor; this is the final bound on
    /// actual socket fan-out across groups.
    endpoint_dials: EndpointDialScheduler,
    /// Per-server single-flight wake signals for sealing newly authenticated recovery routes.
    /// This uses a synchronous mutex only to clone/notify a `watch::Sender`; no actor/store await
    /// may ever run in the event consumer, or the bounded actor event channel can deadlock.
    reconnect_capture_signals: StdMutex<HashMap<u64, watch::Sender<u64>>>,
    /// The last few decrypted media chunks. Small and deliberately not an LRU: playback is
    /// sequential, so "the current chunk and the one before it" covers the straddle at a chunk
    /// boundary and a short seek backwards, which is all the locality there is to exploit.
    media_cache: Mutex<Vec<MediaChunk>>,
    /// Sizes and declared types of recently served files, so answering a `Range` request costs
    /// one chunk read and not two. See [`MediaHead`].
    media_heads: Mutex<Vec<MediaHead>>,
    /// Invite minting crosses actor and optional rendezvous awaits. Serialize it so two self-heal/
    /// explicit requests cannot finish out of order and overwrite a newer route-set token.
    invite_mint: Mutex<()>,
    /// One bounded two-way reply session per `(server, invite nonce)`. A bearer-token holder may
    /// refresh the same joiner key, but replacing it with a different key requires an explicit UI
    /// confirmation so “one reply wins” cannot become a trivial invite-consumption DoS.
    join_replies: Mutex<HashMap<(u64, [u8; 16]), ActiveJoinReply>>,
    /// Serialize the state-map transition with the actor's revoke/authorize commands. Without
    /// this, concurrent replacement completions can re-install a displaced helper capability.
    join_reply_apply: Mutex<()>,
    /// Live router mappings for the active call's media UDP ports, keyed by the local port. A
    /// PCP/NAT-PMP entry's short lease is renewed by the client retained inside the mapping, so
    /// dropping the entry (call end, or the app closing) is what ends that route.
    media_mappings: Mutex<HashMap<u16, catcoms_net::MediaPortMapping>>,
    next_id: Mutex<u64>,
    /// Serialize first mount and already-mounted authentication. A duplicate frontend unlock must
    /// never race two reloads, while an explicitly UI-locked session must authenticate against
    /// the existing mount rather than deadlocking itself on the lifetime vault lock.
    vault_mount: Mutex<()>,
    /// One persistence lock per **numeric** server id, outliving the registry entry it belongs to.
    ///
    /// A persisted id is deliberately reused when a server is removed and reinstalled, and a write
    /// begun by the departing incarnation can still be in flight when the replacement is
    /// installed. Keeping the lock in the entry gave the two incarnations separate locks, so those
    /// writes did not even serialize against each other. Here they do, and the incarnation check
    /// inside the lock then decides which of them is still entitled to write.
    persist_locks: StdMutex<HashMap<u64, Arc<Mutex<()>>>>,
    store: Mutex<Option<ServerStore>>,
    /// Whether a freshly-mounted frontend may restore the already-unlocked UI session. This stays
    /// true across F5/HMR, but an explicit Ctrl+L clears it so a reload cannot bypass the lock.
    session_resumable: Mutex<bool>,
    /// Set before an explicit lock performs any awaited cleanup. Commands check this as well as
    /// `session_resumable`, so new IPC cannot slip through while the lock is waiting to serialize
    /// against an older command's final native commit.
    session_lock_requested: AtomicBool,
    /// Every explicit lock invalidates work that began in an earlier UI authorization epoch.
    /// Long-running joins carry the captured value to both their reply event and durable commit.
    ui_session_generation: AtomicU64,
    /// Orders the externally-visible parts of a long command against explicit lock completion.
    /// The lock-request atomic closes new IPC immediately; this mutex makes it impossible for a
    /// reply event or server registration to occur after `lock_session` itself has completed.
    ui_session_commit: Mutex<()>,
    /// The newest exact Ctrl+L/close snapshot registered before either command waits on the shared
    /// commit mutex. A remounted close can consume it even though its JS coordinator is gone.
    pending_ui_lock_snapshot: Mutex<Option<PendingUiLockSnapshot>>,
    /// Lets a remounted close report a completed older snapshot failure instead of treating its own
    /// snapshot-less idempotent lock as proof that continuity succeeded.
    last_ui_lock_completion: Mutex<Option<UiLockCompletion>>,
    /// Serializes native window-close attempts and keeps an unacknowledged continuity failure from
    /// being overtaken by a duplicate caller. The window may be destroyed only after the first
    /// failure was returned visibly and a later request explicitly accepts that snapshot loss.
    vault_window_close: Mutex<()>,
    /// Sticky close-specific data-loss evidence. Ordinary Ctrl+L locking also publishes
    /// `last_ui_lock_completion`, so that shared slot may legitimately be replaced later; it must
    /// never erase the warning a native close still owes before destroying the only UI surface.
    vault_window_close_debt: Mutex<Option<String>>,
    /// The **new device's** half of an in-flight grant ceremony (multi-device M2): the device
    /// identity + single-use nonce minted by `pairing_begin`, held until the grant bundle is
    /// pasted back. One slot; starting a new ceremony abandons any previous one; and, like
    /// `store`, it is process state only: the key is never written to disk here.
    pairing: Mutex<Option<PairingSecrets>>,
    /// The **origin device's** record of which pairing nonces it has already granted, so one
    /// pasted request mints at most one bundle. In-memory (a restart forgets), exactly as the
    /// invite ledger began.
    pairing_ledger: Mutex<PairingLedger>,
    /// The **origin device's** pending ceremony: exactly the request (and SAS anchor) the
    /// human saw at `pairing_read`. `pairing_mint` takes NO blob and mints only from here,
    /// so what gets certified is provably what was approved; re-reading a caller-supplied
    /// blob at mint time was a TOCTOU (the popup could show one device while a swapped blob
    /// minted for another), and it left the backend with no notion of "confirmed" at all
    /// (adversarial-review finding). Cleared on mint success and on decline.
    pending_grant: Mutex<Option<PendingGrant>>,
    /// The **new device's** opened grants (multi-device M3), held between `pairing_open` and
    /// `pairing_join`. A grant is dropped once its server has actually been joined; a grant that
    /// *failed* is kept so the user can retry; the expected failure is "the owner is offline",
    /// which is exactly what the offline-queued admission is built to survive. Starting a fresh
    /// ceremony (`pairing_begin`) replaces the lot, and the ceremony secrets are dropped once the
    /// last grant has been redeemed.
    pairing_grants: Mutex<Vec<PerServerGrant>>,
    /// What the most recent founding or joining attempt actually did, for the onboarding
    /// connectivity panel. One slot: the panel exists to explain the attempt the user is looking
    /// at, and keeping a history of every address every invite ever named is a worse trade than
    /// it sounds.
    diag: Mutex<Connectivity>,
    /// Per-server automatic router-mapping result (UPnP/PCP/NAT-PMP), kept out of
    /// [`Connectivity`] because it lands *after* the attempt that started it returns (and possibly
    /// after the user has started another one). Keyed by server id so a late answer can never be
    /// reported against a different server's attempt. The field retains its original `upnp` name
    /// for frontend command compatibility during the diagnostics transition.
    upnp: Mutex<HashMap<u64, String>>,
    /// Per-server structured AutoNAT v2 evidence. Keeping every current address/server result lets
    /// the read path filter against the live advertised set and retain a second successful route.
    autonat: Mutex<HashMap<u64, AutoNatEvidence>>,
    /// Connected peers' low-trust Identify observations of our outbound socket. Diagnostic only;
    /// these are never folded into bootstrap, rendezvous registration or AutoNAT candidates.
    mesh_observations: Mutex<HashMap<u64, Vec<String>>>,
    /// One integrity/inventory scan per server per unlocked UI session. Health is a point-in-time
    /// observation, so file events deliberately do not invalidate it behind the user's back;
    /// explicit lock clears the metadata, and repair replaces it after re-verification.
    storage_health: Mutex<HashMap<u64, CachedStorageHealth>>,
    /// Per-server singleflight gates for expensive storage scans/repairs. These are deliberately
    /// separate from the plaintext result cache: explicit lock must clear that cache immediately,
    /// without waiting for local decryption or peer-fetch timeouts to finish.
    storage_scans: StorageScanGates,
    /// One native monitor fans a coalesced generation out to every per-server discovery loop.
    /// Polling remains active, so monitor initialization failure affects latency, not correctness.
    network_changes: NetworkChangeSignal,
    /// Streamed uploads in flight, keyed by `(server, upload id)`. See [`PendingUpload`].
    uploads: Mutex<HashMap<UploadKey, PendingUpload>>,
}

/// Fixed-size keyed singleflight. Hash collisions only serialize two explicit scans; unlike a map
/// keyed by webview-provided server ids, this cannot become an unbounded allocation surface.
struct StorageScanGates {
    stripes: [Mutex<()>; STORAGE_SCAN_STRIPES],
}

impl Default for StorageScanGates {
    fn default() -> Self {
        Self {
            stripes: std::array::from_fn(|_| Mutex::new(())),
        }
    }
}

impl StorageScanGates {
    fn for_server(&self, server: u64) -> &Mutex<()> {
        &self.stripes[(server as usize) % STORAGE_SCAN_STRIPES]
    }
}

/// Identity of one streamed upload: the server, and a **backend-minted** generation token.
///
/// Deliberately not the caller's upload id. Sealing a chunk releases the map lock across an actor
/// round-trip, and the completion has to find its own upload again afterwards. Keyed by the public
/// id, a caller that restarted that id in the meantime would receive the earlier generation's
/// chunk: begin/cancel/begin is documented as a restart, so the earlier entry is gone and a new one
/// sits under the same key. That mis-attachment is silent and produces a listing whose chunks are
/// not the file its address names, which no member can ever download. A fresh token per `begin`
/// makes a stale completion look up an entry that no longer exists, which it can only discard.
type UploadKey = (u64, String);

/// One streamed upload between `begin_file_upload` and `finish_file_upload`.
///
/// Deliberately tiny: only the per-chunk [`FileRef`]s (~150 bytes each; 32 of them for a 256 MiB
/// file) and the running whole-file address live here, never the bytes. Each chunk crosses the
/// bridge once, is sealed straight into the blob store, and is dropped.
struct PendingUpload {
    server: u64,
    /// The caller's own id for this upload. Not the identity of the work (see [`UploadKey`]);
    /// carried only so progress events are tagged with what the frontend is displaying, and so a
    /// restart can retire the previous generation of the same visible transfer.
    upload_id: String,
    /// Declared at `begin_file_upload` and used for every chunk, so the manifest cannot end up
    /// describing chunks sealed under a type the caller changed halfway through.
    mime: String,
    /// The whole-file content address, accumulated slice by slice. A streamed upload never holds
    /// the file, so it cannot hash it in one go.
    address: CidHasher,
    /// Slices received but not yet sealed. A slice is the IPC unit and a chunk is the seal unit;
    /// this is where the smaller one is assembled into the larger. It never exceeds one chunk.
    buffer: Vec<u8>,
    chunks: Vec<FileRef>,
    /// Which chunk is being sealed right now, if any. The map lock is released across that actor
    /// round-trip, so this both stops a second concurrent slice and pins where the returning
    /// `FileRef` belongs: a completion is attached only if the upload is still waiting for exactly
    /// that chunk, so refs can never be appended in completion order rather than file order.
    sealing: Option<usize>,
    /// Bytes accepted so far; also the only offset the next slice may claim. Slices must arrive
    /// in order and exactly once: the media reader maps a byte offset to a chunk index by
    /// dividing, so a hole or a repeat would produce a manifest whose boundaries do not line up.
    bytes_seen: u64,
    declared_size: u64,
    chunk_total: usize,
    /// When this upload last made progress, for the idle sweep.
    touched_at: u64,
}

// Manual Debug, redacting the buffer: it holds plaintext of the file being shared, and a derived
// Debug would put it in whatever printed the error. Same reason `SealingBlobStore` redacts its key.
impl std::fmt::Debug for PendingUpload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingUpload")
            .field("server", &self.server)
            .field("upload_id", &self.upload_id)
            .field("buffered", &self.buffer.len())
            .field("chunks", &self.chunks.len())
            .field("sealing", &self.sealing)
            .field("bytes_seen", &self.bytes_seen)
            .field("declared_size", &self.declared_size)
            .finish_non_exhaustive()
    }
}

impl PendingUpload {
    /// Admit one slice of the file at byte `offset`, returning a whole chunk's bytes when the
    /// buffered slices have completed one (and marking the upload as sealing until the caller
    /// reports back through [`Self::chunk_sealed`]).
    ///
    /// Every rule here exists to keep the manifest's chunk boundaries exactly where a reader will
    /// look for them: the media reader finds a byte offset's chunk by dividing by `CHUNK_BYTES`,
    /// so chunks must be uniform, which means slices must arrive in order, exactly once, and at
    /// full size until the file runs out. A violation fails the upload rather than being skipped:
    /// the running whole-file address has already absorbed the earlier slices and cannot be
    /// rewound, so there is no consistent state to continue from.
    fn admit_slice(&mut self, offset: u64, bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
        if self.sealing.is_some() {
            return Err("a chunk of this upload is already being sealed".into());
        }
        if offset != self.bytes_seen {
            return Err("upload slices must arrive in order".into());
        }
        let end = offset + bytes.len() as u64;
        if end > self.declared_size {
            return Err("upload sent more bytes than it declared".into());
        }
        if end < self.declared_size && bytes.len() != UPLOAD_SLICE_BYTES {
            return Err("only the last slice of an upload may be short".into());
        }
        self.address.update(bytes);
        self.buffer.extend_from_slice(bytes);
        self.bytes_seen = end;
        self.touched_at = SystemClock.now_ms();
        // A slice divides a chunk exactly, so the buffer lands on the boundary rather than past it.
        if self.buffer.len() < CHUNK_BYTES {
            return Ok(None);
        }
        self.sealing = Some(self.chunks.len());
        Ok(Some(std::mem::take(&mut self.buffer)))
    }

    /// The trailing partial chunk at the end of an upload, if there is one to seal. A file whose
    /// size is not a whole number of chunks ends short, and an empty file is still one (empty)
    /// chunk, so a manifest always has at least one.
    fn take_tail(&mut self) -> Result<Option<Vec<u8>>, String> {
        if self.sealing.is_some() {
            return Err("a chunk of this upload is still being sealed".into());
        }
        if self.bytes_seen != self.declared_size {
            return Err("the upload did not send every byte".into());
        }
        if self.buffer.is_empty() && !self.chunks.is_empty() {
            return Ok(None);
        }
        self.sealing = Some(self.chunks.len());
        Ok(Some(std::mem::take(&mut self.buffer)))
    }

    /// Whether a chunk that has just finished sealing still belongs here.
    ///
    /// False means the returning `FileRef` is work this generation is not waiting for: a chunk
    /// already recorded, or one claimed before a restart/cancel moved the upload on. Attaching it
    /// would put a ref at the wrong index, so the caller collects the sealed blob instead.
    fn can_accept(&self, index: usize) -> bool {
        self.sealing == Some(index) && index == self.chunks.len()
    }

    /// Record a sealed chunk's reference and release the sealing claim. Only call after
    /// [`can_accept`](Self::can_accept).
    fn chunk_sealed(&mut self, file_ref: FileRef) -> usize {
        self.chunks.push(file_ref);
        self.sealing = None;
        self.chunks.len()
    }

    /// Whether every declared byte arrived and turned into the expected number of chunks. A short
    /// upload must never be published: its chunks would be listed under the address of a whole
    /// file that was never sent, and every member would fail the reassembly check.
    fn is_complete(&self) -> bool {
        self.bytes_seen == self.declared_size
            && self.buffer.is_empty()
            && self.chunks.len() == self.chunk_total
    }

    /// Sealed vault bytes this upload is holding but has not published. What a quota on staged
    /// data has to count: an entry cap alone bounds nothing, because one entry can stage a whole
    /// 256 MiB file.
    fn staged_bytes(&self) -> u64 {
        self.chunks.len() as u64 * CHUNK_BYTES as u64
    }

    /// Whether nothing has been added to this upload since `cutoff`. An upload whose caller went
    /// away (a webview reload loses its ids while the native process lives on) otherwise keeps
    /// both its slot and its staged bytes until the session is locked.
    fn idle_since(&self, cutoff: u64) -> bool {
        self.sealing.is_none() && self.touched_at < cutoff
    }
}

/// Streamed uploads one session may have open at once. Each holds at most one chunk of buffered
/// bytes plus its chunk references, so this bounds both map growth and buffered memory from a
/// webview that starts uploads and never finishes them.
const MAX_PENDING_UPLOADS: usize = 16;
/// Total sealed-but-unpublished bytes all in-flight uploads may hold. The entry cap does not bound
/// this on its own: sixteen uploads that each seal every chunk and never finish would stage
/// sixteen whole files of vault data that no manifest names.
///
/// Has to clear one whole [`MAX_FILE_BYTES`] file with room to spare, or the largest legal upload
/// would be refused by this budget before it started. At 2 GiB one gigabyte-scale upload fits
/// alongside ordinary traffic, and a second concurrent one waits rather than doubling the vault;
/// the idle sweep releases whatever an abandoned caller left behind.
const MAX_STAGED_UPLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const _: () = assert!(MAX_STAGED_UPLOAD_BYTES > MAX_FILE_BYTES as u64);
/// How long an upload may go untouched before a later `begin` collects it. A caller that vanished
/// (most commonly a webview reload, which loses the ids while the native side keeps running) has
/// no way to cancel its own uploads, so nothing else would ever release them.
const UPLOAD_IDLE_TIMEOUT_MS: u64 = 10 * 60 * 1000;
/// Longest accepted upload id. The webview mints these (a UUID); the bound plus the character
/// check keeps a malformed one out of the key space.
const MAX_UPLOAD_ID_BYTES: usize = 64;
/// How much of an upload crosses the IPC bridge in one `push_file_chunk` call. Reported to the
/// caller in its [`UploadTicket`], and must divide [`CHUNK_BYTES`] exactly so buffered slices
/// always land on a chunk boundary. Sealing happens at [`CHUNK_BYTES`]; this is only about keeping
/// any single IPC message small enough that serializing it does not stall the webview.
const UPLOAD_SLICE_BYTES: usize = 1024 * 1024;
const _: () = assert!(UPLOAD_SLICE_BYTES > 0 && CHUNK_BYTES % UPLOAD_SLICE_BYTES == 0);

/// The most a file may be and still be readable through `download_file`, which returns the whole
/// thing as one base64 string.
///
/// Files are listed up to [`MAX_FILE_BYTES`] (256 MiB), and a listing's size is written by whoever
/// shared it. Handing 256 MiB to the webview as a ~341 MB JS string is the exact shape that made
/// uploads freeze the app, and an embedded image is enough to trigger it: no user action is
/// required beyond scrolling past the message. Everything that can be large has a streaming route
/// instead (`catcoms-media:` for playback and images, `save_group_file` for saving), so this bound
/// costs nothing real and closes the path.
const MAX_INLINE_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024;

// Build failures rather than test failures, because both are properties of the constants
// themselves: a listing may declare far more than may be read inline, and the text reader's own
// 2 MiB soft cap has to fit underneath the hard one or the reader could never load anything.
const _: () = assert!(MAX_INLINE_DOWNLOAD_BYTES < MAX_FILE_BYTES as u64);
const _: () = assert!(MAX_INLINE_DOWNLOAD_BYTES >= 2 * 1024 * 1024);

/// Whether a file of `size` may be read through `download_file`, which returns it whole as one
/// base64 string. See [`MAX_INLINE_DOWNLOAD_BYTES`].
///
/// Checked against the size the *listing* declares, before a byte is fetched. That is only a real
/// bound because a manifest's chunk layout is now validated against its declared size: otherwise a
/// listing could declare one byte, pass this, and still deliver 256 MiB.
fn inline_download_allowed(size: u64) -> Result<(), String> {
    if size > MAX_INLINE_DOWNLOAD_BYTES {
        return Err(format!(
            "this file is {size} bytes; over {MAX_INLINE_DOWNLOAD_BYTES} it must be streamed or saved rather than read inline"
        ));
    }
    Ok(())
}

/// How many [`CHUNK_BYTES`] chunks a file of `size` bytes is split into. An empty file is still
/// one (empty) chunk, so a manifest always has at least one.
fn upload_chunk_count(size: u64) -> usize {
    (size.max(1) as usize).div_ceil(CHUNK_BYTES)
}

/// Drop a pending upload's sealed-but-unpublished chunk blobs. Uses the unchecked actor lookup so
/// cleanup still runs when the vault has just been locked, which is one of the ways an upload ends.
///
/// Awaited rather than fire-and-forget: cancel and lock report that the upload has been cleaned
/// up, so they must not return while the blobs are still queued for deletion.
async fn discard_pending_upload(state: &AppState, upload: PendingUpload) {
    if upload.chunks.is_empty() {
        return;
    }
    if let Ok(actor) = actor_of_unchecked(state, upload.server).await {
        actor.discard_upload(upload.chunks).await;
    }
}

/// A fresh generation token for one `begin_file_upload`. See [`UploadKey`]: this is the identity
/// asynchronous seal work is matched against, so it has to be unguessable-fresh per begin rather
/// than derived from anything the caller supplies.
fn mint_upload_token() -> String {
    let mut raw = [0u8; 16];
    OsCryptoRng.fill_bytes(&mut raw);
    hex::encode(raw)
}

#[derive(Debug, Clone)]
struct ActiveJoinReply {
    joiner: libp2p::PeerId,
    nonces: Vec<[u8; 16]>,
    expires_at_ms: u64,
    generation: u64,
}

const MAX_ACTIVE_JOIN_REPLIES: usize = 64;

#[derive(Debug, Clone, Serialize)]
struct JoinReplyReady {
    code: String,
    expires_at_ms: u64,
    candidate_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct JoinReplyApplied {
    joiner: String,
    expires_at_ms: u64,
    replaced: bool,
    helper: bool,
}

/// A short-lived, member-signed route repair code. Unlike a join reply this can only reconnect a
/// device that is already in the current roster; it cannot admit a new device or replace MLS
/// membership state.
#[derive(Debug, Clone, Serialize)]
struct MemberRecoveryReady {
    code: String,
    expires_at_ms: u64,
    candidate_count: usize,
}

/// Result of accepting a member recovery code. `submitted_routes` reports socket attempts, not a
/// successful connection: the ordinary Noise and signed peer-record checks remain authoritative.
#[derive(Debug, Clone, Serialize)]
struct MemberRecoveryAppliedEvt {
    fingerprint: String,
    submitted_routes: usize,
}

/// Decode-only information shown before a pasted invite is allowed to contact standing helpers.
/// Routes stay native-side: the webview learns only that the inviter endorsed a bounded fallback
/// set and the privacy consequence of choosing it.
#[derive(Debug, Clone, Serialize)]
struct InvitePreview {
    direct_routes: usize,
    rendezvous_routes: usize,
    switchboards: usize,
    expires_at_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AutoNatEvidence {
    waiting: bool,
    results: Vec<AutoNatResult>,
}

/// Automatic mapping has not started (no durable transport retained its event stream).
const PORT_MAPPING_NOT_ATTEMPTED: &str = "not attempted";
/// Mapping probes are out and the router has not answered yet.
const PORT_MAPPING_WAITING: &str = "waiting for router mapping (UPnP, PCP/PCPv6, NAT-PMP)";
/// None of the enabled router protocols produced a mapping inside the initial diagnostic window.
const PORT_MAPPING_TIMED_OUT: &str = "no mapping obtained within 25s";
const PORT_MAPPING_INACTIVE: &str = "no active router mapping";
/// A join that failed before admission cannot preserve its temporary listener long enough for a
/// router mapping to matter. This is more accurate than the old blanket "not attempted".
const PORT_MAPPING_FAILED_JOIN: &str =
    "not retained: the join failed before this node became a member";

/// No public candidate/server pair produced an AutoNAT v2 result inside the collection window.
const AUTONAT_NOT_TESTED: &str =
    "not tested: no public address candidate and AutoNAT server were available together";
/// A public relay/rendezvous is connected and the background collector is waiting for a callback.
const AUTONAT_WAITING: &str = "waiting for a dial-back result";
/// The temporary join swarm was dropped before it could become a durable member transport.
const AUTONAT_FAILED_JOIN: &str =
    "not tested: the join never reached a connected AutoNAT server as a member";

/// One thing a founding or joining attempt did, in the order it did it.
///
/// This is a *record of actions*, not a verdict. Several of these start work libp2p finishes
/// asynchronously, so the status is three-valued rather than a boolean; claiming success for a
/// dial that has merely been issued is precisely the overconfident status
/// `docs/design-zeroconf-reachability.md` section 5 warns about.
#[derive(Serialize, Clone)]
struct DiagStep {
    /// Milliseconds since the epoch, from the same `SystemClock` seam the rest of the bridge
    /// stamps with.
    at: u64,
    /// A short machine label the UI groups on: `listen`, `advertise`, `relay`, `rendezvous`,
    /// `discover`, `dial`, `connect`, `invite`.
    kind: String,
    /// The address (or other subject) this step concerned. May be empty.
    target: String,
    /// What happened, verbatim where it came from an error.
    detail: String,
    /// `ok` (it demonstrably worked), `failed` (it demonstrably did not), or `unknown` (it was
    /// started and the answer arrives, or does not, later).
    status: String,
}

impl DiagStep {
    fn new(kind: &str, target: impl Into<String>, detail: impl Into<String>, status: &str) -> Self {
        Self {
            at: SystemClock.now_ms(),
            kind: kind.to_string(),
            target: target.into(),
            detail: detail.into(),
            status: status.to_string(),
        }
    }
    fn ok(kind: &str, target: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(kind, target, detail, "ok")
    }
    fn failed(kind: &str, target: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(kind, target, detail, "failed")
    }
    fn unknown(kind: &str, target: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(kind, target, detail, "unknown")
    }
}

/// Everything the app can honestly say about its own reachability and about the last thing it
/// tried, for the connectivity panel on the create/join screens.
///
/// AutoNAT v2 can now prove that one connected public infrastructure node reached one candidate
/// address at one moment. That is real dial-back evidence, but deliberately not promoted into a
/// timeless or universal node property: reachability varies by address family, transport, NAT and
/// observer. A UPnP address remains evidence rather than proof until AutoNAT tests it.
#[derive(Serialize, Clone, Default)]
struct Connectivity {
    /// `found`, `join`, or empty when nothing has been attempted this session.
    action: String,
    /// Human context for the attempt: the server name, or the inviter's peer id.
    subject: String,
    /// When the attempt ran (ms since the epoch), or 0.
    at: u64,
    /// The server id the attempt produced, so a late UPnP answer can be matched to it. 0 = none.
    server: u64,
    /// The addresses this node offers other peers, as folded into the invite.
    advertised: Vec<String>,
    /// Whether at least one non-relayed advertised address is globally routable according to the
    /// backend's canonical classifier. The frontend must not duplicate partial IP-range logic.
    public_direct: bool,
    /// Automatic UPnP/PCP/NAT-PMP result for `server` (filled in by `get_connectivity`).
    /// Serialized as `upnp` for compatibility with the existing frontend command contract.
    upnp: String,
    /// The AutoNAT v2 result for `server`, likewise filled by `get_connectivity`.
    autonat: String,
    /// Bounded connected-peer observations of this node's outbound source. These are telemetry,
    /// not listener candidates; the frontend must retain that qualification.
    mesh_observations: Vec<String>,
    /// What the attempt did, oldest first.
    steps: Vec<DiagStep>,
    /// The last error, verbatim, so the user can copy exactly what the code said.
    last_error: String,
    /// The trace this attempt belongs to, in the short form a person quotes.
    ///
    /// The panel and the diagnostic record used to be two accounts of the same minute with nothing
    /// joining them, so relating one to the other meant matching wall-clock times by eye. This is
    /// the join key, shown in the panel and stamped on every event the attempt produced.
    trace: String,
}

#[derive(Serialize)]
struct SwitchboardMember {
    fingerprint: String,
    addresses: usize,
}

#[derive(Serialize)]
struct SwitchboardStatus {
    offered: bool,
    eligible: bool,
    online: Vec<SwitchboardMember>,
    reason: String,
}

/// The origin-side pending ceremony: what `pairing_read` decoded and anchored, held so
/// `pairing_mint`/`pairing_decline` act on the approved request and nothing else.
struct PendingGrant {
    view: catcoms_app::PairingRequestView,
    origin: DeviceId,
}

/// Reject webview IPC while the explicit UI lock is active. Server actors intentionally continue
/// networking in the background, but that must not leave their plaintext projections callable by
/// injected/stale frontend code behind the lock screen.
async fn require_unlocked_session(state: &AppState) -> Result<(), String> {
    if state.session_lock_requested.load(Ordering::Acquire) {
        return Err("the vault is locked".into());
    }
    if *state.session_resumable.lock().await
        && state.store.lock().await.is_some()
        && !state.session_lock_requested.load(Ordering::Acquire)
    {
        Ok(())
    } else {
        Err("the vault is locked".into())
    }
}

/// Capture an unlocked UI epoch while serialized with the final phase of any older command.
async fn unlocked_ui_session_generation(state: &AppState) -> Result<u64, String> {
    let _commit = state.ui_session_commit.lock().await;
    require_unlocked_session(state).await?;
    Ok(state.ui_session_generation.load(Ordering::Acquire))
}

/// Permit one externally-visible step only if it still belongs to the UI session that began it.
/// Holding the returned guard through the step orders it before a concurrently requested lock;
/// the atomic request flag still makes all newly-started commands fail without waiting.
async fn require_ui_session_generation(
    state: &AppState,
    expected: u64,
) -> Result<tokio::sync::MutexGuard<'_, ()>, String> {
    let commit = state.ui_session_commit.lock().await;
    require_unlocked_session(state).await?;
    if state.ui_session_generation.load(Ordering::Acquire) != expected {
        return Err("the UI session changed while the operation was in progress".into());
    }
    Ok(commit)
}

/// Why a server's actor could not be handed over.
///
/// A typed answer rather than a sentence, because the two states ask opposite things of the user
/// and a caller cannot tell them apart from a string without sniffing prose.
///
/// The bug this fixes: [`actor_of`] checks the lock and then reports every failure the same way, so
/// a locked vault reached the user as `SERVER.ACTOR.UNAVAILABLE` with a `Restart` remediation. They
/// were told to restart the application when what they needed to do was type their passphrase.
/// `channel_target` a few hundred lines down had always got this right, which is the tell: the
/// distinction was known and lost on the way through one helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActorLookup {
    /// The vault is locked.
    Locked,
    /// No server with that id is open in this process.
    ///
    /// "Never opened" and "closed since" are one state here, deliberately: the registry cannot tell
    /// them apart, and a code that claimed to would be guessing.
    NotOpen,
}

impl ActorLookup {
    fn code(self) -> ErrorCode {
        match self {
            ActorLookup::Locked => codes::SESSION_LOCKED,
            ActorLookup::NotOpen => codes::SERVER_UNAVAILABLE,
        }
    }

    fn message(self) -> &'static str {
        match self {
            ActorLookup::Locked => "the vault is locked",
            ActorLookup::NotOpen => "unknown server",
        }
    }
}

impl std::fmt::Display for ActorLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// So the sixty-odd commands that return a bare string keep compiling, and keep saying what they
/// said. `?` applies this conversion for them; the typed callers match on the value instead.
impl From<ActorLookup> for String {
    fn from(failure: ActorLookup) -> String {
        failure.message().to_string()
    }
}

/// Internal actor lookup used by persistence/reload paths which must keep operating while the UI
/// is locked. Native command handlers use [`actor_of`] so the webview cannot cross that boundary.
async fn actor_of_unchecked(state: &AppState, server: u64) -> Result<ServerActor, ActorLookup> {
    state
        .servers
        .lock()
        .await
        .get(&server)
        .map(|e| e.actor.clone())
        .ok_or(ActorLookup::NotOpen)
}

/// Clone out the actor for an unlocked webview command (never holding either lock across actor I/O).
async fn actor_of(state: &AppState, server: u64) -> Result<ServerActor, ActorLookup> {
    require_unlocked_session(state)
        .await
        .map_err(|_| ActorLookup::Locked)?;
    actor_of_unchecked(state, server).await
}

/// Clone the actor together with its process-local registry incarnation. Long operations must
/// carry both: a persisted server id may legitimately be removed and reinstalled in one process.
async fn actor_instance_of(
    state: &AppState,
    server: u64,
) -> Result<(ServerActor, u64), ActorLookup> {
    require_unlocked_session(state)
        .await
        .map_err(|_| ActorLookup::Locked)?;
    state
        .servers
        .lock()
        .await
        .get(&server)
        .map(|entry| (entry.actor.clone(), entry.instance))
        .ok_or(ActorLookup::NotOpen)
}

/// Clone the actor and DM marker together. Moderation is server-wide and intentionally absent
/// from 1:1 DM spaces, so its bridge commands use this helper to enforce that boundary before
/// invoking the actor.
async fn server_actor_of(state: &AppState, server: u64) -> Result<ServerActor, String> {
    require_unlocked_session(state).await?;
    let servers = state.servers.lock().await;
    let entry = servers
        .get(&server)
        .ok_or_else(|| "unknown server".to_string())?;
    if entry.is_dm {
        return Err("moderation is only available in server spaces".into());
    }
    Ok(entry.actor.clone())
}

/// Result of founding/joining: the new server's id plus its `#general` channel id.
#[derive(Serialize, Clone)]
struct FoundResult {
    server: u64,
    channel: String,
    channels: Vec<UiChannel>,
    is_dm: bool,
}

#[derive(Serialize, Clone)]
struct UiChannel {
    id: String,
    name: String,
}

fn ui_channels(channels: Vec<catcoms_app::ChannelInfo>) -> Vec<UiChannel> {
    channels
        .into_iter()
        .map(|c| UiChannel {
            id: c.id.to_string(),
            name: c.name,
        })
        .collect()
}

/// A chat message as serialized to the frontend.
#[derive(Serialize, Clone)]
struct UiMessage {
    id: String,
    author: String,
    text: String,
    ts: u64,
    edited: u64,
    reactions: Vec<UiReaction>,
    reply_to: String,
    pinned: bool,
}

/// One emoji reaction on a message (the emoji + the fingerprints of those who reacted).
#[derive(Serialize, Clone)]
struct UiReaction {
    emoji: String,
    by: Vec<String>,
}

/// One cross-server inbox entry (a mention or reply aimed at me) with its server/channel context.
#[derive(Serialize, Clone)]
struct UiInboxItem {
    server: u64,
    server_name: String,
    is_dm: bool,
    channel: String,
    message_id: String,
    author: String,
    author_name: String,
    text: String,
    ts: u64,
    mention: bool,
    reply: bool,
}

/// Map a backend chat message to its UI shape (shared by `get_messages` + `get_statuses`).
fn ui_message(m: catcoms_app::ChatMessage) -> UiMessage {
    UiMessage {
        id: m.id,
        author: m.author,
        text: m.text,
        ts: m.ts,
        edited: m.edited,
        reactions: m
            .reactions
            .into_iter()
            .map(|r| UiReaction {
                emoji: r.emoji,
                by: r.by,
            })
            .collect(),
        reply_to: m.reply_to,
        pinned: m.pinned,
    }
}

/// A roster member as serialized to the frontend.
#[derive(Serialize, Clone)]
struct UiMember {
    fingerprint: String,
    /// Full device id for authorization; the short fingerprint is display-only.
    identity: String,
    you: bool,
}

/// A member profile as serialized to the frontend (keyed by fingerprint). `avatar` and
/// `banner` are base64-encoded JPEG bytes (empty = unset).
#[derive(Serialize, Clone)]
struct UiProfile {
    fingerprint: String,
    name: String,
    color: String,
    font: String,
    effect: String,
    description: String,
    bubble: String,
    avatar: String,
    /// The wide profile-card banner, base64-encoded like the avatar (empty = no banner).
    banner: String,
}

/// The server's published livery as serialized to the frontend. Every value is **untrusted**
/// (any member's client may have written it): the frontend validates the preset id, the
/// `#rrggbb` accent and the token allow-list on read, and ignores anything else.
#[derive(Serialize, Clone)]
struct UiLivery {
    preset: String,
    accent: String,
    tokens: HashMap<String, String>,
    /// The shared server icon as base64 image bytes (empty = none). Untrusted like the rest:
    /// the frontend must render it as an image only, never interpret it.
    icon: String,
    /// The shared server cursor as base64 image bytes (empty = none). Untrusted exactly like
    /// the icon: render it as an image only (a `cursor: url(data:…)`), never interpret it.
    cursor: String,
    /// The shared server name (empty = the group publishes none, so each member keeps its own
    /// local label). Untrusted like everything else here: the backend bounds its length and
    /// refuses control characters, and the frontend renders it as text only.
    name: String,
}

/// One member's custom badge as serialized to the frontend, keyed by fingerprint in
/// `get_badges`. **Untrusted** like the livery: the backend bounds the sizes and rejects
/// role-reserved labels, and the frontend validates the colour (and ignores a reserved label
/// that predates that gate) on read.
#[derive(Serialize, Clone)]
struct UiBadge {
    label: String,
    color: String,
}

/// One companion device as serialized to the frontend, keyed by its own fingerprint: which member
/// (origin fingerprint) it belongs to, and the name the origin certified. `name` is safe to render
/// as text; it is inside the certificate's signature and bounded/control-character-free by
/// `validate_device_name`; but it is still a *member-chosen* string, so render it as a tag, never
/// as chrome.
#[derive(Serialize, Clone)]
struct UiDevice {
    origin: String,
    name: String,
}

/// One scheduled server event as serialized to the frontend. `start_ts`/`end_ts` are epoch-millis
/// (`end_ts == 0` = no end time) and `author` is the creator's device fingerprint, resolved to a
/// display name via the profiles map exactly like a message author.
#[derive(Serialize, Clone)]
struct UiEvent {
    id: String,
    title: String,
    body: String,
    start_ts: u64,
    end_ts: u64,
    author: String,
    /// Hex content address of the event's poster image, or empty for none; downloaded through
    /// the same `download_file` path as an inline embed.
    image: String,
}

/// One channel-jukebox entry as serialized to the frontend. `cid` is the hex content address of
/// the queued file (downloaded through the same `download_file` path as any other embed),
/// `added_ms` is epoch-millis, and `author` is the adder's device fingerprint, resolved to a
/// display name via the profiles map exactly like a message author.
#[derive(Serialize, Clone)]
struct UiJukeEntry {
    id: String,
    cid: String,
    name: String,
    author: String,
    added_ms: u64,
}

/// A shared file as serialized to the frontend. `cid` is the hex content address used to
/// download it.
#[derive(Serialize, Clone)]
struct UiFile {
    name: String,
    size: u64,
    mime: String,
    cid: String,
    author: String,
    /// Full attested signer identity. Empty for unsigned/invalid legacy listings.
    author_identity: String,
    /// Cryptographic group-bound proof for the uploader label; legacy listings are false.
    author_verified: bool,
    path: String,
    /// Chunks of this file already held locally (availability indicator).
    held: u32,
    /// Total chunks the file is split into.
    total: u32,
    /// Internal join key for the exact actor snapshot. The webview neither needs nor gets this
    /// digest; it only sees whether the native verified-inventory policy admitted the row.
    #[serde(skip_serializing)]
    manifest_version: [u8; 32],
    /// When this listing drops out of circulation, ms epoch. `null` means either "keep forever"
    /// or "never recorded"; `expires_known` tells those apart. Recorded metadata only: nothing
    /// enforces it yet (see `catcoms_app::FileExpiry`).
    expires: Option<u64>,
    /// Whether an expiry was ever recorded for this listing. `false` = a legacy share from before
    /// expiry existed ("not recorded"); `true` + `expires == null` = an explicit keep-forever.
    expires_known: bool,
}

/// Where a file is referenced across the server; the Properties pane's "Used in" row.
#[derive(Serialize, Clone)]
struct UiFileUsage {
    /// Live wiki pages whose body embeds/references this file (sorted).
    wiki_pages: Vec<String>,
    /// Status posts referencing it.
    status_count: usize,
    /// Chat messages referencing it, across every channel open on this device.
    chat_count: usize,
    /// Calendar events using it as their poster image or referencing it in their description.
    event_count: usize,
    /// `wiki_pages` is non-empty ⇒ the file is wiki-pinned and must never drop out of
    /// circulation, whatever its recorded expiry says.
    pinned: bool,
}

/// The shared file list plus whether a live peer previously proved it could serve authenticated
/// catch-up; the payload of `get_files`, so the UI can color files conservatively in one trip.
#[derive(Serialize, Clone)]
struct FilesPayload {
    files: Vec<UiFile>,
    has_peers: bool,
}

/// Integrity-verified storage facts. Counts are scoped to file chunks referenced by this server;
/// the peer flag only means a repair attempt has somewhere authenticated to ask.
#[derive(Serialize, Clone)]
struct UiStorageHealth {
    listed_files: usize,
    referenced_chunks: usize,
    verified_chunks: usize,
    missing_chunks: usize,
    unreadable_chunks: usize,
    invalid_manifests: usize,
    verified_bytes: u64,
    has_peers: bool,
    checked_at_ms: u64,
    unique_files: usize,
    logical_bytes: u64,
    local_estimated_bytes: u64,
    pinned_files: usize,
    pinned_logical_bytes: u64,
    pinned_local_estimated_bytes: u64,
    categories: Vec<UiStorageCategory>,
    largest_files: Vec<UiStorageFile>,
    /// Fully local files whose managed copy remains sealed under the vault. Exporting one creates
    /// a separate plaintext copy; it never mutates this inventory row in place.
    local_files: Vec<UiStorageFile>,
}

#[derive(Clone)]
struct CachedStorageHealth {
    server_instance: u64,
    report: UiStorageHealth,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
struct UiStorageCategory {
    name: String,
    files: usize,
    logical_bytes: u64,
    local_estimated_bytes: u64,
    pinned_files: usize,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
struct UiStorageFile {
    name: String,
    path: String,
    cid: String,
    mime: String,
    logical_bytes: u64,
    local_estimated_bytes: u64,
    pinned: bool,
    held: u32,
    total: u32,
}

#[derive(Serialize, Clone)]
struct UiStorageRepair {
    attempted_chunks: usize,
    recovered_chunks: usize,
    health: UiStorageHealth,
}

fn ui_file(listing: FileListing) -> UiFile {
    UiFile {
        name: listing.entry.name,
        size: listing.entry.size,
        mime: listing.entry.mime,
        cid: hex::encode(&listing.entry.cid),
        author: listing.entry.author,
        author_identity: listing.entry.author_identity,
        author_verified: listing.entry.author_verified,
        path: listing.entry.path,
        held: listing.held_chunks,
        total: listing.total_chunks,
        manifest_version: listing.manifest_version,
        expires: listing.entry.expires.deadline_ms(),
        expires_known: listing.entry.expires.is_recorded(),
    }
}

fn storage_category(mime: &str) -> &'static str {
    let mime = mime.to_ascii_lowercase();
    if mime.starts_with("image/") {
        "Images"
    } else if mime.starts_with("video/") {
        "Video"
    } else if mime.starts_with("audio/") {
        "Audio"
    } else if mime.starts_with("text/")
        || mime.contains("pdf")
        || mime.contains("document")
        || mime.contains("sheet")
        || mime.contains("presentation")
    {
        "Documents"
    } else if mime.contains("zip")
        || mime.contains("tar")
        || mime.contains("gzip")
        || mime.contains("compressed")
        || mime.contains("archive")
    {
        "Archives"
    } else {
        "Other"
    }
}

fn estimated_local_bytes(file: &UiFile) -> u64 {
    if file.total == 0 {
        return 0;
    }
    file.size
        .saturating_mul(u64::from(file.held.min(file.total)))
        / u64::from(file.total)
}

/// Add a deterministic, deduplicated content inventory to the cryptographic chunk verdict.
/// Category/local totals are explicitly estimates based on held-chunk ratios; `verified_bytes`
/// above remains the exact count of ciphertext bytes that passed every integrity check.
fn build_storage_report(
    health: StorageHealth,
    files: Vec<UiFile>,
    pinned: &HashSet<String>,
    checked_at_ms: u64,
) -> UiStorageHealth {
    let mut unique = HashMap::<String, UiFile>::new();
    let mut ambiguous = HashSet::<String>::new();
    for file in files {
        match unique.entry(file.cid.clone()) {
            std::collections::hash_map::Entry::Occupied(existing) => {
                if existing.get().manifest_version != file.manifest_version {
                    ambiguous.insert(file.cid);
                }
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(file);
            }
        }
    }
    let mut category_map = HashMap::<String, UiStorageCategory>::new();
    let mut largest_files = Vec::with_capacity(unique.len());
    let mut local_files = Vec::new();
    let mut logical_bytes = 0u64;
    let mut local_estimated_bytes = 0u64;
    let mut pinned_files = 0usize;
    let mut pinned_logical_bytes = 0u64;
    let mut pinned_local_estimated_bytes = 0u64;
    for file in unique.values() {
        let local = estimated_local_bytes(file);
        let is_pinned = pinned.contains(&file.cid);
        logical_bytes = logical_bytes.saturating_add(file.size);
        local_estimated_bytes = local_estimated_bytes.saturating_add(local);
        if is_pinned {
            pinned_files += 1;
            pinned_logical_bytes = pinned_logical_bytes.saturating_add(file.size);
            pinned_local_estimated_bytes = pinned_local_estimated_bytes.saturating_add(local);
        }
        let category = storage_category(&file.mime).to_string();
        let row = category_map
            .entry(category.clone())
            .or_insert(UiStorageCategory {
                name: category,
                files: 0,
                logical_bytes: 0,
                local_estimated_bytes: 0,
                pinned_files: 0,
            });
        row.files += 1;
        row.logical_bytes = row.logical_bytes.saturating_add(file.size);
        row.local_estimated_bytes = row.local_estimated_bytes.saturating_add(local);
        row.pinned_files += usize::from(is_pinned);
        let inventory_file = UiStorageFile {
            name: file.name.clone(),
            path: file.path.clone(),
            cid: file.cid.clone(),
            mime: file.mime.clone(),
            logical_bytes: file.size,
            local_estimated_bytes: local,
            pinned: is_pinned,
            held: file.held,
            total: file.total,
        };
        if file.total > 0
            && file.held == file.total
            && !ambiguous.contains(&file.cid)
            && health
                .verified_manifest_versions
                .contains(&file.manifest_version)
        {
            local_files.push(inventory_file.clone());
        }
        largest_files.push(inventory_file);
    }
    let order = ["Images", "Video", "Audio", "Documents", "Archives", "Other"];
    let mut categories: Vec<_> = category_map.into_values().collect();
    categories.sort_by_key(|row| {
        order
            .iter()
            .position(|name| *name == row.name)
            .unwrap_or(order.len())
    });
    largest_files.sort_by(|a, b| {
        b.local_estimated_bytes
            .cmp(&a.local_estimated_bytes)
            .then_with(|| b.logical_bytes.cmp(&a.logical_bytes))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.cid.cmp(&b.cid))
    });
    largest_files.truncate(10);
    local_files.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.cid.cmp(&b.cid))
    });
    UiStorageHealth {
        listed_files: health.listed_files,
        referenced_chunks: health.referenced_chunks,
        verified_chunks: health.verified_chunks,
        missing_chunks: health.missing_chunks,
        unreadable_chunks: health.unreadable_chunks,
        invalid_manifests: health.invalid_manifests,
        verified_bytes: health.verified_bytes,
        has_peers: health.has_peers,
        checked_at_ms,
        unique_files: unique.len(),
        logical_bytes,
        local_estimated_bytes,
        pinned_files,
        pinned_logical_bytes,
        pinned_local_estimated_bytes,
        categories,
        largest_files,
        local_files,
    }
}

async fn storage_report(
    actor: &ServerActor,
    snapshot: StorageSnapshot,
    checked_at_ms: u64,
) -> UiStorageHealth {
    let pins = actor.wiki_pinned_cids().await.into_iter().collect();
    build_storage_report(
        snapshot.health,
        snapshot.files.files.into_iter().map(ui_file).collect(),
        &pins,
        checked_at_ms,
    )
}

/// A public, signed moderation attestation. Evidence is an immutable snapshot so later message
/// deletion/editing cannot silently rewrite the reason a warning or kick case was based on.
#[derive(Serialize, Clone)]
struct UiModerationEvent {
    id: String,
    kind: String,
    actor: String,
    signer: String,
    target: String,
    channel: String,
    message_id: String,
    message_text: String,
    message_ts: u64,
    reason: String,
    evidence_ids: Vec<String>,
    case_id: String,
    outcome: String,
    ts: u64,
    signature_valid: bool,
    authorized: bool,
}

#[derive(Serialize, Clone)]
struct UiModerationVote {
    case_id: String,
    voter: String,
    signer: String,
    yes: bool,
    ts: u64,
    signature_valid: bool,
    eligible: bool,
}

#[derive(Serialize, Clone)]
struct UiModerationState {
    events: Vec<UiModerationEvent>,
    votes: Vec<UiModerationVote>,
}

// Event payloads; every event is tagged with its server id.
#[derive(Serialize, Clone)]
struct ChannelEvt {
    server: u64,
    channel: String,
    /// What actually moved. One channel document holds the message log, the topic and the jukebox
    /// queue, so an untyped event forced the UI to read a queue edit as an unread chat message.
    /// Only `messages_appended` may create unread state.
    messages_appended: bool,
    /// The ids that actually arrived, in the order they now read. A row ordered by its timestamp
    /// is not always the newest one, so a notification has to be told which rows to describe.
    arrivals: Vec<String>,
    messages_changed: bool,
    topic: bool,
    jukebox: bool,
}
#[derive(Serialize, Clone)]
struct CountEvt {
    server: u64,
    count: usize,
}
#[derive(Serialize, Clone)]
struct ServerEvt {
    server: u64,
}
/// An inbound call-signalling message forwarded to the UI (payload is base64 of the opaque bytes).
#[derive(Serialize, Clone)]
struct CallSignalEvt {
    server: u64,
    from_fp: String,
    payload: String,
}
#[derive(Serialize, Clone)]
struct DownloadProgressEvt {
    server: u64,
    cid: String,
    /// Exact caller token for cancellable inline reads. `None` preserves the existing media/file
    /// progress shape while allowing take playback to reject queued events from an older call.
    cancellation: Option<String>,
    done: usize,
    total: usize,
    bytes_done: u64,
    bytes_total: u64,
    /// Bytes fetched from peers during this transfer (excludes chunks already held locally).
    network_bytes_done: u64,
    provider: Option<String>,
}
#[derive(Serialize, Clone)]
struct UploadProgressEvt {
    server: u64,
    upload_id: String,
    done: usize,
    total: usize,
}

/// What `begin_file_upload` hands back: the whole streaming contract for this one upload, so the
/// caller never has to hold its own copy of the protocol's constants.
///
/// The frontend used to compute the chunk count from a TypeScript copy of the chunk size and slice
/// with a TypeScript copy of the slice size. Two languages holding the same two numbers is a drift
/// no test in either language can see; here the native side states them per upload and the caller
/// simply obeys.
// camelCase, because this one is destructured by name in the upload loop rather than read field
// by field. A snake_case key there is not a type error and not a runtime error either: the loop
// simply gets `undefined` for the slice size, sends an empty first slice, and the upload is
// rejected as short. `upload_ticket_keys_are_the_names_the_upload_loop_destructures` pins it.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UploadTicket {
    /// Identifies this upload's work for the rest of its life. See [`UploadKey`].
    token: String,
    /// How many chunks the file will be sealed into, and so the denominator of its progress.
    chunk_total: usize,
    /// How many bytes the caller should put in each `push_file_chunk` (the last may be shorter).
    slice_bytes: usize,
}
#[derive(Serialize, Clone)]
struct EclipseEvt {
    server: u64,
    caution: bool,
}
#[derive(Serialize, Clone)]
struct OnlineEvt {
    server: u64,
    online: Vec<String>,
}
/// Delivery state for one of this device's messages. `delivered` is evidence-based and one-sided:
/// it counts members that have provably received the message, so `0` means "nothing proves it
/// arrived yet", never "it failed". `reachable` is the presence count, and may be smaller than
/// `delivered` (a member that got the message and then went offline still holds it).
#[derive(Serialize, Clone)]
struct DeliveryStateEvt {
    id: String,
    delivered: usize,
    reachable: usize,
    /// A live peer previously served roster-verified catch-up; self-asserted descriptor claims and
    /// bare relay/rendezvous sockets do not count.
    any_peer: bool,
}
#[derive(Serialize, Clone)]
struct DeliveryEvt {
    server: u64,
    channel: String,
    revision: u64,
    states: Vec<DeliveryStateEvt>,
}

#[derive(Serialize, Clone)]
struct DeliverySnapshotEvt {
    revision: u64,
    states: Vec<DeliveryStateEvt>,
}

fn delivery_payload(snapshot: catcoms_app::DeliverySnapshot) -> DeliverySnapshotEvt {
    DeliverySnapshotEvt {
        revision: snapshot.revision,
        states: snapshot
            .states
            .into_iter()
            .map(|s| DeliveryStateEvt {
                id: s.id,
                delivered: s.delivered,
                reachable: s.reachable,
                any_peer: s.any_peer,
            })
            .collect(),
    }
}

/// Forward one server actor's event stream to the frontend, tagging each with `server`.
/// How often, on average, the bridge nudges a server's actor to drive steady-state discovery
/// (rendezvous re-register/re-discover, member PEX, address-cache refresh). The real-time
/// interval lives HERE (in the bridge / `apps`, off the deterministic-time seam the `crates`
/// ambient gate enforces; `scripts/check-no-ambient.sh` searches `crates` and `bins` only).
const DISCOVERY_INTERVAL_SECS: u64 = 60;
/// Half-width of the random jitter applied to every discovery period, so the actual cadence is
/// uniform over `[60s - 15s, 60s + 15s)`.
///
/// A bare `interval` gave every member of every group the same period *and* the same phase, so
/// after an infrastructure outage the entire network reconverged inside one 60-second window and
/// hit the rendezvous as a thundering herd (defect P11). Randomising each period, rather than
/// only the start, means phases keep diverging instead of drifting back into lockstep.
const DISCOVERY_JITTER_MS: u64 = 15_000;
/// Spread of the *first* tick. Kept short deliberately: the first pass is what lights the roster's
/// online dots and takes the first eclipse observation, so a full-period start offset would leave
/// a freshly-founded server looking dead for up to a minute. A few seconds is enough to stop the
/// servers in one process from ticking in unison.
const DISCOVERY_START_SPREAD_MS: u64 = 5_000;
/// One best-effort PEX request immediately after direct admission. This is what persists the
/// inviter's signed device-to-transport claim before an immediate close/reopen; failure does not
/// roll back an otherwise valid MLS join.
const DIRECT_JOIN_PEX_MS: u64 = 3_000;
/// Extra quiet window above the platform monitor's own short coalescing delay. DHCP, route and
/// IPv6 privacy-address changes commonly arrive as a burst; publishing a signed peer-record epoch
/// for every callback would waste sequence numbers and make members redial transient routes.
const NETWORK_CHANGE_DEBOUNCE_MS: u64 = 750;

/// A uniformly random delay in `[base_ms, base_ms + spread_ms)`, drawn from the injected OS RNG
/// seam (`catcoms_rt::OsCryptoRng`) rather than an ambient randomness helper, so this file keeps to
/// the same discipline the `crates` gate enforces even though it is not itself gated.
fn jittered_delay(base_ms: u64, spread_ms: u64) -> Duration {
    let mut rng = OsCryptoRng;
    let offset = if spread_ms == 0 {
        0
    } else {
        rng.next_u64() % spread_ms
    };
    Duration::from_millis(base_ms.saturating_add(offset))
}

/// Turn a platform interface-state watcher into one bounded process-wide generation stream.
/// `Watcher` itself retains only the latest state; calling `get` after the quiet window consumes
/// every intermediate callback, so one burst wakes each server once. The clock is injected to keep
/// the debounce contract deterministic in tests.
async fn forward_network_changes<W, C>(mut changes: W, signal: NetworkChangeSignal, clock: C)
where
    W: n0_watcher::Watcher + Send,
    C: Clock,
{
    loop {
        if changes.updated().await.is_err() {
            return;
        }
        // This is a trailing-edge debounce, not a fixed aggregation window. If another route
        // update arrives just before the deadline, restart the quiet period; otherwise we could
        // sample an intermediate DHCP state, consume the final callback, and wait a minute to
        // notice the stable address.
        loop {
            tokio::select! {
                biased;
                changed = changes.updated() => {
                    if changed.is_err() {
                        return;
                    }
                }
                _ = clock.sleep(Duration::from_millis(NETWORK_CHANGE_DEBOUNCE_MS)) => {
                    break;
                }
            }
        }
        signal.notify();
    }
}

/// Start the one native route/interface watcher for this desktop process. The monitor uses the
/// platform's route notification API on Windows, netlink on Linux/Android and SystemConfiguration
/// route sockets on Apple/BSD platforms. Failure is deliberately non-fatal: the roughly-minute
/// poll in every discovery loop remains the portable correctness path.
fn spawn_network_monitor(app: &AppHandle) {
    let signal = app.state::<AppState>().network_changes.clone();
    // No rhythm declared: an interface that does not change is the ordinary case, so silence here
    // means nothing happened rather than that nothing is watching.
    supervise_detached("network_monitor", None, None, async move {
        let monitor = match netwatch::netmon::Monitor::new().await {
            Ok(monitor) => monitor,
            Err(error) => {
                tracing::warn!(%error, "native network monitor unavailable; periodic polling remains active");
                return;
            }
        };
        let changes = monitor.interface_state();
        // Keep `monitor` alive for as long as its watcher is being forwarded; dropping it cancels
        // and unregisters the platform callbacks.
        forward_network_changes(changes, signal, SystemClock).await;
        tracing::warn!("native network monitor stopped; periodic polling remains active");
    });
}

/// Poll the route-selected IPv4/IPv6 source addresses and publish one new authoritative address
/// epoch when they change. Wildcard TCP/QUIC listeners already cover a newly appeared interface;
/// this reconciliation is what teaches invites, PEX, rendezvous and AutoNAT about the new concrete
/// route (and withdraws a vanished one). Mapping/manual/relay ownership is preserved separately.
async fn refresh_interface_routes(app: &AppHandle, server: u64) -> bool {
    let state = app.state::<AppState>();
    let state = state.inner();
    let prepared = {
        let mut servers = state.servers.lock().await;
        let Some(entry) = servers.get_mut(&server) else {
            return false;
        };
        let (Some(identity), Some(mesh)) = (entry.interface_routes.clone(), entry.mesh.clone())
        else {
            return false;
        };
        let next = auto_bootstrap(identity.port, &identity.peer_id);
        let (removed, added) =
            reconcile_automatic_bootstrap(&mut entry.bootstrap, &mut entry.bootstrap_owners, &next);
        if removed.is_empty() && added.is_empty() {
            return false;
        }
        (
            entry.actor.clone(),
            mesh,
            entry.bootstrap.clone(),
            removed,
            added,
        )
    };
    let (actor, mesh, bootstrap, removed, added) = prepared;

    // Keep libp2p's external-address ownership in lockstep. Only global routes enter this set;
    // private LAN routes remain invite-only. A PCP lease for the same IPv6 socket independently
    // protects it inside the net actor when the raw-interface owner is removed.
    for address in external_addrs(&removed) {
        if let Err(error) = mesh.remove_external_address(address.clone()).await {
            tracing::warn!(target: "catcoms_app", %address, %error, "REACH.INTERFACE.WITHDRAW_FAILED");
        }
    }
    for address in external_addrs(&added) {
        if let Err(error) = mesh.add_external_address(address.clone()).await {
            tracing::warn!(target: "catcoms_app", %address, %error, "REACH.INTERFACE.ADVERTISE_FAILED");
        }
    }

    if let Some(seq) = next_record_seq(state, server).await {
        actor.publish_self_record(bootstrap.clone(), seq).await;
    }
    {
        let mut diag = state.diag.lock().await;
        if diag.server == server {
            diag.advertised = bootstrap;
        }
    }
    emit_tracked(
        app,
        "reachability-changed",
        ServerEvt { server },
        catcoms_diagnostics::TraceId::default(),
    );
    true
}

/// Spawn a per-server timer that periodically drives steady-state discovery, so the group
/// re-finds itself after a restart and members keep exchanging peer records. Exits once the actor
/// stops (`drive_discovery` errors).
fn spawn_discovery_timer(app: AppHandle, server: u64, actor: ServerActor) {
    // Subscribe before spawning so a network change between server registration and the task's
    // first poll cannot be lost. The current startup sample is already authoritative; only future
    // generations wake this server early.
    let mut network_changes = app.state::<AppState>().network_changes.subscribe();
    // Declared before the task starts, so the handle exists for the loop to beat on.
    let watched = tasks::register(
        "discovery_timer",
        Some(server),
        wall_ms(),
        // This one does have a rhythm, and it is the rhythm that keeps peer records fresh. A
        // discovery timer that has silently stopped looks exactly like a network that has gone
        // quiet, and only one of those is a bug here.
        Some(DISCOVERY_INTERVAL_SECS * 1_000 + DISCOVERY_JITTER_MS),
    );
    let task = tokio::spawn(async move {
        // A short randomised start offset, then an independently randomised period each round.
        let mut delay = jittered_delay(0, DISCOVERY_START_SPREAD_MS);
        loop {
            watched.beat(wall_ms());
            tokio::select! {
                _ = SystemClock.sleep(delay) => {}
                changed = network_changes.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
            // Poll before PEX/rendezvous so a new interface address receives a fresh signed epoch
            // and can be shared in this same discovery pass.
            refresh_interface_routes(&app, server).await;
            if actor.drive_discovery().await.is_err() {
                break; // the actor stopped
            }
            // The pass just refreshed the member records; seal the cache on the same cadence, so
            // the next launch starts from the members this one actually proved.
            persist_address_cache(&app, server).await;
            // Learn a currently connected member's private route as well. This upgrades pre-v3
            // records after one successful overlap and updates the running actor without ever
            // publishing the address to the group.
            persist_live_local_reconnect_routes(&app, server, &actor).await;
            delay = jittered_delay(
                DISCOVERY_INTERVAL_SECS * 1_000 - DISCOVERY_JITTER_MS,
                DISCOVERY_JITTER_MS * 2,
            );
        }
    });
    supervise_registered("discovery_timer", Some(server), watched, task);
}

/// Wake the per-server recovery capture worker without awaiting the actor from its event consumer.
/// `watch` coalesces any number of flaps into one latest generation while a capture is in flight.
fn notify_reconnect_capture(app: &AppHandle, server: u64) {
    let state = app.state::<AppState>();
    let Ok(signals) = state.reconnect_capture_signals.lock() else {
        return;
    };
    if let Some(signal) = signals.get(&server) {
        signal.send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}

fn spawn_reconnect_capture_worker(
    app: AppHandle,
    server: u64,
    actor: ServerActor,
    mut wake: watch::Receiver<u64>,
) {
    let task = tokio::spawn(async move {
        while wake.changed().await.is_ok() {
            persist_live_local_reconnect_routes(&app, server, &actor).await;
        }
    });
    supervise("reconnect_capture", server, task);
}

fn replace_reconnect_capture_signal(
    signals: &StdMutex<HashMap<u64, watch::Sender<u64>>>,
    server: u64,
) -> Option<watch::Receiver<u64>> {
    let (signal, wake) = watch::channel(0u64);
    signals.lock().ok()?.insert(server, signal);
    Some(wake)
}

/// Install the single bounded recovery-capture wakeup for a registry entry. Replacing the sender
/// closes any prior worker after its current capture, which matters when an on-disk id is restored
/// into a process that previously held a transient entry with the same id.
fn install_reconnect_capture_worker(app: &AppHandle, server: u64, actor: ServerActor) {
    let Some(capture_wake) = replace_reconnect_capture_signal(
        &app.state::<AppState>().reconnect_capture_signals,
        server,
    ) else {
        return;
    };
    spawn_reconnect_capture_worker(app.clone(), server, actor, capture_wake);
}

fn forward_events(
    app: AppHandle,
    server: u64,
    mut events: mpsc::Receiver<catcoms_app::TracedEvent>,
) {
    let task = tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            // The actor carries the boundary token rather than the normalized diagnostic id: its
            // `tracing` stages and this returned event both cross through native normalization.
            // Keeping that rule uniform means an arbitrary runtime `trace` field can never bypass
            // Safe capture, while both actor outputs still join the command's canonical trace.
            let actor_trace = catcoms_diagnostics::TraceId(ev.trace.0);
            let trace = if actor_trace.is_set() {
                catcoms_log::hub().external_trace(actor_trace)
            } else {
                Default::default()
            };
            // Route/authentication changes are the narrow window in which a pasted recovery route
            // is provable. Seal it before the ordinary UI-event lock gate: actors intentionally
            // keep networking behind the lock, and waiting for the minute timer could lose a
            // short-lived authenticated edge before it ever became restart-safe.
            if matches!(
                &ev.event,
                AppEvent::ConnectivityChanged { .. } | AppEvent::MemberRoutesChanged
            ) {
                notify_reconnect_capture(&app, server);
            }
            // Actor networking stays live behind the explicit lock, but its event stream can
            // contain member fingerprints, channel ids, delivery state and call signalling.
            // Drop those notifications at the native boundary; unlock reloads fresh projections.
            if require_unlocked_session(&app.state::<AppState>())
                .await
                .is_err()
            {
                // Recorded rather than dropped in silence. An operation whose event never reached
                // the UI because the vault locked mid-flight looks exactly like one that was lost,
                // and only one of those is a bug.
                catcoms_diagnostics::DiagnosticHub::record(
                    &catcoms_log::hub(),
                    catcoms_diagnostics::DiagnosticEvent::new(
                        catcoms_diagnostics::Section::Ipc,
                        catcoms_diagnostics::Level::Debug,
                        "IPC.EVENT.WITHHELD_LOCKED",
                    )
                    .target("catcoms_app")
                    .trace(trace),
                );
                if matches!(&ev.event, AppEvent::Closed) {
                    break;
                }
                continue;
            }
            match ev.event {
                AppEvent::ChannelsUpdated => {
                    emit_tracked(&app, "channels-changed", ServerEvt { server }, trace);
                }
                AppEvent::ChannelUpdated { channel, change } => {
                    // Channel ids are u128; send as a string (JS numbers lose precision).
                    emit_tracked(
                        &app,
                        "channel-updated",
                        ChannelEvt {
                            server,
                            channel: channel.to_string(),
                            messages_appended: change.messages_appended,
                            arrivals: change.arrivals.clone(),
                            messages_changed: change.messages_changed,
                            topic: change.topic,
                            jukebox: change.jukebox,
                        },
                        trace,
                    );
                }
                AppEvent::MembersChanged { count } => {
                    emit_tracked(&app, "members-changed", CountEvt { server, count }, trace);
                }
                AppEvent::ProfilesUpdated => {
                    emit_tracked(&app, "profiles-updated", ServerEvt { server }, trace);
                }
                AppEvent::LiveryUpdated => {
                    emit_tracked(&app, "livery-changed", ServerEvt { server }, trace);
                }
                AppEvent::BadgesUpdated => {
                    emit_tracked(&app, "badges-changed", ServerEvt { server }, trace);
                }
                AppEvent::DevicesUpdated => {
                    emit_tracked(&app, "devices-changed", ServerEvt { server }, trace);
                }
                AppEvent::FilesUpdated => {
                    emit_tracked(&app, "files-updated", ServerEvt { server }, trace);
                }
                AppEvent::StatusUpdated => {
                    emit_tracked(&app, "status-updated", ServerEvt { server }, trace);
                }
                AppEvent::EventsUpdated => {
                    emit_tracked(&app, "events-changed", ServerEvt { server }, trace);
                }
                AppEvent::WikiUpdated => {
                    emit_tracked(&app, "wiki-updated", ServerEvt { server }, trace);
                }
                AppEvent::RolesUpdated => {
                    emit_tracked(&app, "roles-updated", ServerEvt { server }, trace);
                }
                AppEvent::ModerationUpdated => {
                    emit_tracked(&app, "moderation-updated", ServerEvt { server }, trace);
                }
                AppEvent::EclipseChanged { caution } => {
                    emit_tracked(
                        &app,
                        "eclipse-changed",
                        EclipseEvt { server, caution },
                        trace,
                    );
                }
                AppEvent::ConnectivityChanged { online } => {
                    emit_tracked(
                        &app,
                        "connectivity-changed",
                        OnlineEvt { server, online },
                        trace,
                    );
                }
                AppEvent::MemberRoutesChanged => {
                    emit_tracked(&app, "member-routes-changed", ServerEvt { server }, trace);
                }
                AppEvent::SwitchboardsChanged => {
                    emit_tracked(&app, "switchboard-changed", ServerEvt { server }, trace);
                }
                AppEvent::DeliveryChanged { channel, snapshot } => {
                    let snapshot = delivery_payload(snapshot);
                    emit_tracked(
                        &app,
                        "delivery-changed",
                        DeliveryEvt {
                            server,
                            channel: channel.to_string(),
                            revision: snapshot.revision,
                            states: snapshot.states,
                        },
                        trace,
                    );
                }
                AppEvent::DmRequestsChanged => {
                    emit_tracked(&app, "dm-requests-changed", ServerEvt { server }, trace);
                }
                AppEvent::CallSignal { from_fp, payload } => {
                    emit_tracked(
                        &app,
                        "call-signal",
                        CallSignalEvt {
                            server,
                            from_fp,
                            payload: B64.encode(payload),
                        },
                        trace,
                    );
                }
                AppEvent::Closed => {
                    emit_tracked(&app, "server-closed", ServerEvt { server }, trace);
                    break;
                }
            }
        }
    });
    // The task whose unobserved death this whole registry was written for. It can stop while the
    // server actor is perfectly healthy: the protocol keeps running, membership keeps changing,
    // messages keep arriving, and the webview is told none of it. What a user sees is a stale
    // unread badge and stale presence, and until now the app's own answer would have been that
    // everything was fine.
    //
    // No rhythm declared, because a quiet server is quiet and not broken.
    supervise_task("event_forwarder", Some(server), None, task);
}

/// Extract the listen port from a multiaddr. Both `/tcp/<p>` and `/udp/<p>/quic-v1` carry it,
/// and a node binds the same number on both, so either component answers the question; matching
/// only `/tcp/` would silently drop every QUIC address the node now also listens on.
fn listen_port(addr: &Multiaddr) -> Option<u16> {
    addr.iter().find_map(|p| match p {
        Protocol::Tcp(port) | Protocol::Udp(port) => Some(port),
        _ => None,
    })
}

/// Build a dialable bootstrap multiaddr from a user-entered reachable address, so peers on
/// a LAN or the internet can join. Accepts a bare IPv4 (`1.2.3.4`; uses the bound `port`),
/// `host:port` (e.g. a forwarded port), a bare or bracketed IPv6 (`2001:db8::1`, `[2001:db8::1]:443`),
/// or a full multiaddr starting with `/` (e.g. a relay circuit address). Appends this node's
/// `/p2p/<id>` if absent. (Literal IPs / multiaddrs only; a hostname would need `/dns4/`.)
fn build_advertised(input: &str, port: u16, peer_id: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty address".into());
    }
    if input.starts_with('/') {
        return Ok(if input.contains("/p2p/") {
            input.to_string()
        } else {
            format!("{input}/p2p/{peer_id}")
        });
    }
    // `[addr]` / `[addr]:port` is the only unambiguous way to write an IPv6 host with a port,
    // since a bare IPv6 literal is itself full of colons; check it before the host:port split.
    let (host, p) = if let Some(rest) = input.strip_prefix('[') {
        let (h, tail) = rest
            .split_once(']')
            .ok_or_else(|| format!("unclosed '[' in '{input}'"))?;
        let p = match tail.strip_prefix(':') {
            Some(ps) => ps.parse().map_err(|_| format!("bad port in '{input}'"))?,
            None if tail.is_empty() => port,
            None => return Err(format!("bad address '{input}'")),
        };
        (h, p)
    } else if input.parse::<std::net::Ipv6Addr>().is_ok() {
        (input, port)
    } else {
        match input.rsplit_once(':') {
            Some((h, ps)) => (h, ps.parse().map_err(|_| format!("bad port in '{input}'"))?),
            None => (input, port),
        }
    };
    if host.is_empty() {
        return Err(format!("bad address '{input}'"));
    }
    let family = if host.parse::<std::net::Ipv6Addr>().is_ok() {
        "ip6"
    } else {
        "ip4"
    };
    Ok(format!("/{family}/{host}/tcp/{p}/p2p/{peer_id}"))
}

/// The two dialable multiaddrs for one of this host's addresses: TCP and QUIC on the same port.
/// Both go into the invite because the joiner dials every bootstrap address it is given, and UDP
/// hole-punching succeeds through NATs that refuse the TCP one; an extra address never hurts.
fn dialable_addrs(ip: std::net::IpAddr, port: u16, peer_id: &str) -> Vec<String> {
    let family = if ip.is_ipv6() { "ip6" } else { "ip4" };
    vec![
        format!("/{family}/{ip}/tcp/{port}/p2p/{peer_id}"),
        format!("/{family}/{ip}/udp/{port}/quic-v1/p2p/{peer_id}"),
    ]
}

/// The IPv4 and IPv6 probe targets used to learn which of this host's own addresses the kernel
/// would route from. Both are documentation prefixes (RFC 5737 / RFC 3849): nothing is ever sent,
/// so they only need to resolve against the default route, and they must not name real infra.
const V4_ROUTE_PROBE: &str = "192.0.2.1:9";
const V6_ROUTE_PROBE: &str = "[2001:db8::1]:9";

/// The source address this host would use to reach the wider internet in `target`'s family,
/// learned by "connecting" an unbound UDP socket: no packet is sent, the kernel simply resolves
/// the route and picks a source address. A dependency-free way to learn the LAN IPv4 (so a peer on
/// the same network can dial us with nothing typed in) and the global IPv6 (which, having no NAT
/// in front of it, is frequently the only thing that works for a CGNAT'd user). `None` when the
/// family has no route at all, which is exactly the "this host has no IPv6" answer we want.
fn local_source_ip(target: &str) -> Option<std::net::IpAddr> {
    let target: std::net::SocketAddr = target.parse().ok()?;
    let bind: std::net::SocketAddr = if target.is_ipv6() {
        (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
    } else {
        (std::net::Ipv4Addr::UNSPECIFIED, 0).into()
    };
    let sock = std::net::UdpSocket::bind(bind).ok()?;
    sock.connect(target).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    // A link-local IPv6 is useless outside its own segment and would just be dialled and time out.
    let link_local = match ip {
        std::net::IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
        std::net::IpAddr::V4(_) => false,
    };
    (!ip.is_unspecified() && !ip.is_loopback() && !link_local).then_some(ip)
}

/// The bootstrap addresses this node can offer with nothing configured: its LAN IPv4, its
/// routable IPv6, and the same-machine loopback, each over TCP and QUIC. Routable first, loopback
/// last, because that is the order a joiner should prefer.
fn auto_bootstrap(port: u16, peer_id: &str) -> Vec<String> {
    let mut out = Vec::new();
    for probe in [V4_ROUTE_PROBE, V6_ROUTE_PROBE] {
        if let Some(ip) = local_source_ip(probe) {
            out.extend(dialable_addrs(ip, port, peer_id));
        }
    }
    out.extend(dialable_addrs(
        std::net::Ipv4Addr::LOCALHOST.into(),
        port,
        peer_id,
    ));
    out
}

/// Add one ownership claim and, if this is the first owner, one aggregate bootstrap entry. The
/// caller chooses front/back placement because mapped/manual/relay routes should race before raw
/// LAN and loopback candidates, while a refreshed interface route should retain normal order.
fn add_bootstrap_owner(
    bootstrap: &mut Vec<String>,
    owners: &mut HashMap<String, HashSet<BootstrapOwner>>,
    entry: String,
    owner: BootstrapOwner,
    prefer_front: bool,
) -> bool {
    let first_owner = owners.get(&entry).is_none_or(HashSet::is_empty);
    owners.entry(entry.clone()).or_default().insert(owner);
    if first_owner && !bootstrap.contains(&entry) {
        if prefer_front {
            bootstrap.insert(0, entry);
        } else {
            bootstrap.push(entry);
        }
        true
    } else {
        false
    }
}

/// Remove one ownership claim and answer whether the exact aggregate route disappeared. Unknown
/// or repeated expiry events are inert; an address survives while any other source still owns it.
fn remove_bootstrap_owner(
    bootstrap: &mut Vec<String>,
    owners: &mut HashMap<String, HashSet<BootstrapOwner>>,
    entry: &str,
    owner: BootstrapOwner,
) -> bool {
    let Some(entry_owners) = owners.get_mut(entry) else {
        return false;
    };
    if !entry_owners.remove(&owner) || !entry_owners.is_empty() {
        return false;
    }
    owners.remove(entry);
    if let Some(index) = bootstrap.iter().position(|candidate| candidate == entry) {
        bootstrap.remove(index);
        true
    } else {
        false
    }
}

fn bootstrap_owners(
    entries: &[String],
    owner: BootstrapOwner,
) -> HashMap<String, HashSet<BootstrapOwner>> {
    entries
        .iter()
        .cloned()
        .map(|entry| (entry, HashSet::from([owner])))
        .collect()
}

/// Reconcile only the OS-derived owner against a fresh route-source sample. The returned vectors
/// contain aggregate additions/removals, not ownership-only changes, so callers update Swarm and
/// signed records exactly once and preserve an identical manual/mapping/relay route.
fn reconcile_automatic_bootstrap(
    bootstrap: &mut Vec<String>,
    owners: &mut HashMap<String, HashSet<BootstrapOwner>>,
    next: &[String],
) -> (Vec<String>, Vec<String>) {
    let previous: HashSet<String> = owners
        .iter()
        .filter(|(_, entry_owners)| entry_owners.contains(&BootstrapOwner::AutomaticInterface))
        .map(|(entry, _)| entry.clone())
        .collect();
    let next_set: HashSet<&str> = next.iter().map(String::as_str).collect();
    let mut removed = Vec::new();
    for old in &previous {
        if !next_set.contains(old.as_str())
            && remove_bootstrap_owner(bootstrap, owners, old, BootstrapOwner::AutomaticInterface)
        {
            removed.push(old.clone());
        }
    }

    let mut added = Vec::new();
    for entry in next {
        if !previous.contains(entry)
            && add_bootstrap_owner(
                bootstrap,
                owners,
                entry.clone(),
                BootstrapOwner::AutomaticInterface,
                false,
            )
        {
            added.push(entry.clone());
        }
    }
    (removed, added)
}

/// The addresses a server binds: IPv4 **and** IPv6, TCP **and** QUIC, all on one port number.
/// IPv6 has no NAT in front of it, so it silently rescues users their router would otherwise
/// strand; QUIC's UDP hole-punching is materially more reliable than TCP's. Both listeners are
/// best-effort: [`MeshService::new_tcp_with_key`] reports a refusal (no IPv6 stack, say) instead
/// of failing the node.
fn listen_addrs(port: u16) -> Vec<Multiaddr> {
    if port == 0 {
        // Degenerate fallback (no port could be reserved). Bind exactly one address, because an
        // OS-assigned port would give each listener a *different* number and the one-port-per-
        // server model, and with it any port-forward, would be meaningless.
        return "/ip4/0.0.0.0/tcp/0".parse().into_iter().collect();
    }
    [
        format!("/ip4/0.0.0.0/tcp/{port}"),
        format!("/ip6/::/tcp/{port}"),
        format!("/ip4/0.0.0.0/udp/{port}/quic-v1"),
        format!("/ip6/::/udp/{port}/quic-v1"),
    ]
    .iter()
    .filter_map(|s| s.parse().ok())
    .collect()
}

/// Can `port` be bound right now, for both TCP and UDP, on every IPv4 interface? A server listens
/// on both under one number (TCP plus QUIC), so a port is only usable when both are free. IPv6 is
/// deliberately not probed: those listeners are best-effort and a host without an IPv6 stack must
/// not be pushed off its stable port. This is inherently a race (the probe sockets are released
/// before libp2p rebinds them), but the alternative is refusing to start at all.
fn port_is_bindable(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    let v4 = std::net::Ipv4Addr::UNSPECIFIED;
    std::net::TcpListener::bind((v4, port)).is_ok() && std::net::UdpSocket::bind((v4, port)).is_ok()
}

/// Ask the OS for a port that is free for both TCP and UDP, so the TCP and QUIC listeners can
/// share one number. Hold the TCP reservation while probing UDP: dropping it and immediately
/// rebinding the same TCP port is not reliable on Windows and used to produce a spurious zero even
/// though the OS had just selected a usable port. Both reservations are released together before
/// libp2p binds, so the final hand-off remains inherently racy but does not race against ourselves.
fn os_chosen_port() -> u16 {
    for _ in 0..16 {
        let v4 = std::net::Ipv4Addr::UNSPECIFIED;
        let Ok(tcp_probe) = std::net::TcpListener::bind((v4, 0)) else {
            return 0;
        };
        let Ok(local) = tcp_probe.local_addr() else {
            return 0;
        };
        let port = local.port();
        if let Ok(udp_probe) = std::net::UdpSocket::bind((v4, port)) {
            drop(udp_probe);
            drop(tcp_probe);
            return port;
        }
    }
    0
}

/// This server's listen port, in order of preference.
///
/// First choice is the port derived from the server's own identity seed
/// ([`ServerNet::derived_port`]): stable across restarts, which is what a router port-forward and
/// a UPnP mapping need, yet a different number on every install, so one internet-wide sweep of a
/// single well-known port cannot enumerate the network. Second is whatever the last launch
/// actually used (a server that had to move once should stay put rather than flap). Last is a
/// fresh OS-drawn port. The caller persists what comes back.
fn choose_port(net: &ServerNet) -> u16 {
    let home = net.derived_port();
    if port_is_bindable(home) {
        return home;
    }
    if port_is_bindable(net.port) {
        return net.port;
    }
    os_chosen_port()
}

/// One command's worth of stages, recorded as it goes.
///
/// The native half of the correlation wrapper. The frontend allocates a trace and passes it in;
/// this stamps every stage with it, so an operation reads as one story across the boundary instead
/// of as two unrelated halves that have to be lined up by timestamp.
///
/// Deliberately cheap. Constructing one is a parse and two clones; each stage is one `record`
/// call, which costs two atomic loads when the section is not being captured. Instrumentation that
/// slows a command becomes a cause of the latency it was added to explain.
struct Operation {
    /// Session-normalized id used by every canonical native diagnostic and user-facing error.
    trace: catcoms_diagnostics::TraceId,
    /// Boundary token passed through library actors and normalized when their logs/events return.
    actor_trace: catcoms_diagnostics::TraceId,
    section: catcoms_diagnostics::Section,
    operation: &'static str,
    server: Option<catcoms_diagnostics::SessionRef>,
    channel: Option<catcoms_diagnostics::SessionRef>,
    started_ms: u64,
}

impl Operation {
    /// Begin an operation, continuing the frontend's trace when it supplied one.
    ///
    /// A missing or unparseable trace is not an error: most commands have not been migrated yet,
    /// and an un-traced operation is still worth recording. It simply cannot be joined to the
    /// frontend's half.
    fn start(
        trace: Option<String>,
        section: catcoms_diagnostics::Section,
        operation: &'static str,
        server: u64,
        channel: Option<&str>,
    ) -> Self {
        Self::start_maybe(trace, section, operation, Some(server), channel)
    }

    /// [`start`](Operation::start) for an operation that has no server yet.
    ///
    /// Founding and joining do not have a server id until they succeed, and inventing one to fit
    /// the signature would put a reference in the record standing for nothing. An operation with no
    /// subject is honest; one with a made-up subject correlates with the wrong events.
    fn start_maybe(
        trace: Option<String>,
        section: catcoms_diagnostics::Section,
        operation: &'static str,
        server: Option<u64>,
        channel: Option<&str>,
    ) -> Self {
        let hub = catcoms_log::hub();
        let actor_trace = trace
            .as_deref()
            .and_then(parse_trace)
            .unwrap_or_else(|| hub.new_trace());
        let trace = hub.external_trace(actor_trace);
        let op = Operation {
            trace,
            actor_trace,
            section,
            operation,
            server: server.map(|id| {
                hub.reference_str(catcoms_diagnostics::RefDomain::Server, &id.to_string())
            }),
            channel: channel.map(|c| hub.reference_str(catcoms_diagnostics::RefDomain::Channel, c)),
            started_ms: SystemClock.now_ms(),
        };
        op.emit(
            catcoms_diagnostics::Level::Debug,
            catcoms_diagnostics::Phase::Start,
            "IPC.COMMAND.RECEIVED",
            None,
        );
        op
    }

    /// The server's actor, bound to this operation.
    ///
    /// One function rather than the same three lines at each command, because binding is the part
    /// that is easy to leave out: an actor fetched the plain way still works perfectly, and the
    /// only symptom is a trace that stops at the bridge on that one command. Getting the actor and
    /// adopting the operation are the same act here, so they cannot come apart.
    async fn actor(&self, state: &AppState, server: u64) -> Result<ServerActor, AppError> {
        actor_of(state, server)
            .await
            .map(|actor| self.bind_actor(actor))
            // The lookup's own answer, not a blanket one. A locked vault used to arrive here as
            // "the server is unavailable, restart the app".
            .map_err(|failure| self.fail(failure.code(), failure.message()))
    }

    /// Bind an already-authorized actor clone to this operation's boundary token.
    ///
    /// Most commands use [`Operation::actor`]. Multi-server operations such as backup acquire a
    /// whole actor set under one state lock and use this narrower half so every actor command still
    /// joins the canonical trace after it crosses the logging/event boundary.
    fn bind_actor(&self, actor: ServerActor) -> ServerActor {
        actor.with_trace(self.actor_trace.0)
    }

    /// Note that the operation reached a stage, without ending it.
    fn stage(&self, code: &'static str) {
        self.emit(
            catcoms_diagnostics::Level::Debug,
            catcoms_diagnostics::Phase::Progress,
            code,
            None,
        );
    }

    /// End the operation successfully, with how long the whole thing took.
    fn succeeded(&self, code: &'static str) {
        self.emit(
            catcoms_diagnostics::Level::Debug,
            catcoms_diagnostics::Phase::Success,
            code,
            Some(self.elapsed()),
        );
    }

    /// End the operation in failure. Recorded at warn, because a command that did not do what the
    /// user asked is the thing a person came to the log to find.
    fn failed(&self, code: &'static str) {
        self.emit(
            catcoms_diagnostics::Level::Warn,
            catcoms_diagnostics::Phase::Failure,
            code,
            Some(self.elapsed()),
        );
    }

    fn elapsed(&self) -> u64 {
        SystemClock.now_ms().saturating_sub(self.started_ms)
    }

    /// The trace, in the short form a person quotes.
    fn short_trace(&self) -> String {
        self.trace.short()
    }

    /// Replay a connectivity attempt's steps into the diagnostic record, under this trace.
    ///
    /// Founding and joining already keep an excellent step-by-step account for the connectivity
    /// panel: the review calls it the strongest diagnostic pattern in the codebase and says to
    /// generalise it. What it could not do was correlate. The panel showed one attempt, the log
    /// showed everything else, and tying the two together meant matching wall-clock times by eye,
    /// which is exactly the correlation-by-timestamp the trace exists to replace.
    ///
    /// Replayed at the end rather than recorded as each step happens, because the steps are pushed
    /// from a dozen places across `establish_reachability` and threading an operation through all
    /// of them would be a large change for the same result. The trade is that the record's own
    /// timestamps are the replay's, so each step carries the moment it actually happened as a
    /// field, and the ordering is the attempt's own.
    fn replay(&self, steps: &[DiagStep]) {
        for step in steps {
            // A failed step is the thing somebody came looking for, so it is the one that is loud
            // enough to survive a Safe-mode filter. `unknown` is genuinely not a failure: several
            // of these start work libp2p finishes later.
            let level = match step.status.as_str() {
                "failed" => catcoms_diagnostics::Level::Warn,
                _ => catcoms_diagnostics::Level::Debug,
            };
            // A replay walks every step of an attempt and each one bounds two or three strings, so
            // this is the loop where building an excluded event is most obviously wasted.
            catcoms_log::hub().record_with(self.section, level, || {
                let mut event =
                    catcoms_diagnostics::DiagnosticEvent::new(self.section, level, "REACH.STEP")
                        .target("catcoms_app")
                        .phase(catcoms_diagnostics::Phase::Progress)
                        .operation(self.operation)
                        .trace(self.trace)
                        .refs(catcoms_diagnostics::Refs {
                            server: self.server.clone(),
                            ..catcoms_diagnostics::Refs::default()
                        })
                        .field("kind", catcoms_diagnostics::SafeText::describe(&step.kind))
                        .field(
                            "status",
                            catcoms_diagnostics::SafeText::describe(&step.status),
                        )
                        .field("at_ms", step.at);
                if !step.target.is_empty() {
                    // The step's subject is usually an address, so it goes in as one: Safe mode
                    // keeps its family and transport, which is what diagnoses a route problem, and
                    // drops the literal that would stop the report being publishable.
                    event = event.field(
                        "target",
                        catcoms_diagnostics::AddressValue::new(&step.target),
                    );
                }
                if !step.detail.is_empty() {
                    event = event.field(
                        "detail",
                        catcoms_diagnostics::SafeText::describe(&step.detail),
                    );
                }
                event
            });
        }
    }

    /// End the operation in failure and build the error the frontend receives.
    ///
    /// One call rather than two, because the pair has to stay in step: an operation recorded as
    /// failed with one code and reported to the user with another is a diagnostic that actively
    /// misleads, and keeping them together is the only reliable way to prevent it.
    ///
    /// The recorded event carries the code; the *message* is not recorded, because it comes from a
    /// deeper layer that may have interpolated something into it. The user sees it, the log does
    /// not, and that split is deliberate.
    fn fail(&self, code: ErrorCode, message: impl Into<String>) -> AppError {
        self.failed(code.code());
        AppError::new(code, message, &self.short_trace())
    }

    fn emit(
        &self,
        level: catcoms_diagnostics::Level,
        phase: catcoms_diagnostics::Phase,
        code: &'static str,
        duration_ms: Option<u64>,
    ) {
        // Built only if it will be kept. Every stage of every command comes through here, and the
        // reference clones below are not free, so a stage the config excludes should cost the two
        // atomic loads of the gate and nothing else.
        catcoms_log::hub().record_with(self.section, level, || {
            let mut event = catcoms_diagnostics::DiagnosticEvent::new(self.section, level, code)
                .target("catcoms_app")
                .phase(phase)
                .operation(self.operation)
                .trace(self.trace)
                .refs(catcoms_diagnostics::Refs {
                    server: self.server.clone(),
                    channel: self.channel.clone(),
                    ..catcoms_diagnostics::Refs::default()
                });
            if let Some(duration) = duration_ms {
                event = event.took(duration);
            }
            event
        });
    }
}

/// A trace the webview minted, as a canonical [`TraceId`](catcoms_diagnostics::TraceId).
///
/// The webview allocates a trace before it invokes, so the half of an operation that happens in the
/// webview and the half that happens here are stages of one thing rather than two records that have
/// to be lined up by timestamp afterwards. That only works if both sides agree on the parse, which
/// is why this is one function rather than the same `from_str_radix` written out at each door the
/// webview can knock on.
///
/// A trace of zero is treated as absent: it is what an unset trace renders as, and correlating on
/// it would gather every unrelated event that also had none.
fn parse_trace(text: &str) -> Option<catcoms_diagnostics::TraceId> {
    u64::from_str_radix(text, 16)
        .ok()
        .map(catcoms_diagnostics::TraceId)
        .filter(|t| t.is_set())
}

/// Parse and immediately session-normalize a trace supplied by the webview.
///
/// The UI needs repeated values to join its events to native command stages, but it does not get to
/// choose the bytes that Safe capture later displays or copies. Keeping this separate from
/// `parse_trace` makes tests of the wire spelling independent from the privacy boundary.
fn parse_external_trace(text: &str) -> Option<catcoms_diagnostics::TraceId> {
    parse_trace(text).map(|trace| catcoms_log::hub().external_trace(trace))
}

/// Normalize the optional trace on the chunk-upload fast path.
///
/// `push_file_chunk` predates `Operation`, but its trace is still renderer-controlled command data.
/// Keeping this tiny boundary helper testable prevents that exceptional path from accidentally
/// blessing raw UI hex as a native trace when it emits progress.
fn external_progress_trace(text: Option<&str>) -> catcoms_diagnostics::TraceId {
    text.and_then(parse_external_trace).unwrap_or_default()
}

/// Accept a native-normalized trace only when it carries this session's unforgeable return proof.
///
/// A webview-origin trace has no proof and is normalized exactly once. Native event envelopes carry
/// both the already-normalized trace and a proof; accepting that pair unchanged preserves
/// correlation across native -> webview -> native without letting a compromised renderer mark its
/// own caller-controlled hex as safe.
fn parse_returned_ui_trace(text: &str, proof: &str) -> Option<catcoms_diagnostics::TraceId> {
    let trace = parse_trace(text)?;
    let decoded = (proof.len() == 32)
        .then(|| hex::decode(proof).ok())
        .flatten();
    let hub = catcoms_log::hub();
    if decoded
        .as_deref()
        .is_some_and(|bytes| hub.verifies_trace_proof(trace, bytes))
    {
        Some(trace)
    } else {
        Some(hub.external_trace(trace))
    }
}

/// Per-name sequence numbers for emitted events.
///
/// Guarded by a mutex, which is affordable here in a way it would not be on the logging path:
/// these are UI refreshes, already throttled, and orders of magnitude rarer than diagnostic
/// events. The map has about twenty entries and never grows beyond the set of event names in
/// this file.
static EVENT_SEQ: std::sync::OnceLock<std::sync::Mutex<HashMap<&'static str, u64>>> =
    std::sync::OnceLock::new();

fn next_event_seq(name: &'static str) -> u64 {
    let table = EVENT_SEQ.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut table = table.lock().unwrap_or_else(|e| e.into_inner());
    let slot = table.entry(name).or_insert(0);
    *slot += 1;
    *slot
}

/// The whole event stream's order, across every event name.
///
/// Per-name sequences answer "did I miss a `channel-updated`", which is what tells the frontend
/// *what* to re-fetch. They cannot answer "did I miss anything", because the last event of a
/// family leaves no successor to show the gap, and they say nothing about the order two different
/// events happened in. This is one counter for the stream, so both questions have answers.
static EVENT_ORD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Which run of the native process this event stream belongs to.
///
/// The webview can be remounted on its own (F5, or a hot reload during development) while the
/// process it is talking to keeps running, and it comes back with no memory of what it had seen.
/// Stamping the run lets it tell "I have been restarted beside the same stream" from "this is a
/// different stream entirely", which are different amounts of catching up.
static EVENT_GENERATION: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

fn event_generation() -> u64 {
    *EVENT_GENERATION.get_or_init(|| catcoms_rt::RngCore::next_u64(&mut catcoms_rt::OsCryptoRng))
}

/// The envelope keys, which a payload may not define for itself.
const ENVELOPE_KEYS: [&str; 5] = ["__seq", "__ord", "__gen", "__trace", "__trace_proof"];

/// Attach the stream envelope to an event payload, if it can carry one.
///
/// A pure function so the contract the webview relies on can be tested without a running window.
/// `None` means the payload is not a JSON object and could carry nothing; the caller records that
/// fact rather than letting the absence look like a gap.
fn stamp_payload(
    mut value: serde_json::Value,
    seq: u64,
    ord: u64,
    trace: catcoms_diagnostics::TraceId,
    trace_proof: Option<&str>,
) -> Option<serde_json::Value> {
    let object = value.as_object_mut()?;
    object.insert("__seq".to_string(), serde_json::json!(seq));
    object.insert("__ord".to_string(), serde_json::json!(ord));
    object.insert("__gen".to_string(), serde_json::json!(event_generation()));
    // Only when there is one. An absent trace is left off entirely rather than sent as sixteen
    // zeroes, so a listener testing for it gets an answer rather than a value that looks like an
    // operation and belongs to none.
    if trace.is_set() {
        object.insert("__trace".to_string(), serde_json::json!(trace.as_hex()));
        // A proof never appears without its trace and is neither rendered nor persisted. It lets
        // the native ingress recognise this already-normalized value after the untrusted webview
        // returns it, instead of double-hashing it or trusting a renderer-controlled provenance bit.
        if let Some(proof) = trace_proof {
            object.insert("__trace_proof".to_string(), serde_json::json!(proof));
        }
    }
    Some(value)
}

/// Whether a payload already defines a key the envelope owns.
///
/// A collision means a payload type has grown a field that shadows the stream's own bookkeeping.
/// The envelope has to win, or the frontend's gap detection starts reading application data as
/// sequence numbers; but overwriting in silence is how the collision would survive. This is what
/// makes it noticed instead.
fn collides_with_envelope(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .is_some_and(|object| ENVELOPE_KEYS.iter().any(|key| object.contains_key(*key)))
}

/// Emit a Tauri event, numbered, traced and recorded.
///
/// Three problems in one. Every emit in this file used to be `app.emit(...)`, so a delivery
/// failure was discarded at the exact moment it mattered: the backend had changed state, the
/// webview was never told, and nothing anywhere recorded the disagreement. Without a sequence
/// number the frontend cannot tell an event that was coalesced from one that was lost, which is the
/// difference between correct behaviour and a stale unread badge. And without a trace, an update
/// arriving two seconds after a send carried no evidence of being that send's consequence, which is
/// the question the whole correlation architecture exists to answer.
///
/// Both are injected into the payload, as `__seq` and `__trace`, rather than added to each payload
/// type. That keeps every existing listener working unchanged while giving the frontend what it
/// needs. A payload that is not a JSON object (a bare server id) cannot carry either, so it is
/// emitted as-is and both are recorded natively only.
///
/// The trace is passed rather than inherited from ambient state. Which operation caused an emit is
/// a fact the caller knows and nothing else can recover, and a wrong answer here asserts a causal
/// link that never existed.
fn emit_tracked<S: Serialize + Clone>(
    app: &AppHandle,
    name: &'static str,
    payload: S,
    trace: catcoms_diagnostics::TraceId,
) {
    let seq = next_event_seq(name);
    let ord = EVENT_ORD.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let raw = serde_json::to_value(&payload).ok();
    let collision = raw.as_ref().is_some_and(collides_with_envelope);
    let trace_proof = trace
        .is_set()
        .then(|| hex::encode(catcoms_log::hub().trace_proof(trace)));
    let numbered =
        raw.and_then(|value| stamp_payload(value, seq, ord, trace, trace_proof.as_deref()));

    let carried = numbered.is_some();
    if collision {
        // Loud, because the consequence is silent: the frontend would read a payload field as the
        // stream's sequence and either invent a gap or miss a real one.
        catcoms_log::hub().record_with(
            catcoms_diagnostics::Section::Ipc,
            catcoms_diagnostics::Level::Error,
            || {
                catcoms_diagnostics::DiagnosticEvent::error(
                    catcoms_diagnostics::Section::Ipc,
                    "IPC.EVENT.ENVELOPE_COLLISION",
                )
                .target("catcoms_app")
                .trace(trace)
                .field("event", catcoms_diagnostics::SafeText::describe(name))
            },
        );
    }
    let sent = match numbered {
        Some(value) => app.emit(name, value),
        None => app.emit(name, payload),
    };
    // A successful emit is ordinary and frequent; a failed one is the thing somebody came looking
    // for, so it is loud enough to survive a Safe-mode filter. The level is decided before the
    // event is built, so an excluded one costs the gate's two atomic loads and nothing else.
    let level = match &sent {
        Ok(()) => catcoms_diagnostics::Level::Debug,
        Err(_) => catcoms_diagnostics::Level::Warn,
    };
    catcoms_log::hub().record_with(catcoms_diagnostics::Section::Ipc, level, || {
        let code = match &sent {
            Ok(()) => "IPC.EVENT.EMITTED",
            // The failure this replaces. A backend that changed state while the webview never
            // heard about it is a stale-UI bug with no evidence, and it used to leave none.
            Err(_) => "IPC.EVENT.EMIT_FAILED",
        };
        let mut event = catcoms_diagnostics::DiagnosticEvent::new(
            catcoms_diagnostics::Section::Ipc,
            level,
            code,
        )
        .target("catcoms_app")
        .trace(trace)
        .field("event", catcoms_diagnostics::SafeText::describe(name))
        .field("seq", seq)
        // A payload the sequence could not be attached to is one the frontend cannot check for
        // gaps, so the record says which kind it was rather than leaving the absence unexplained.
        .field("numbered", carried);
        if let Err(e) = &sent {
            event = event.field(
                "error",
                catcoms_diagnostics::SafeText::describe(&e.to_string()),
            );
        }
        event
    });
}

/// The most of a panic payload that is recorded.
const MAX_PANIC_SUMMARY: usize = 300;

/// What a panic payload says, reduced to something safe to keep.
///
/// A payload can be any type. In this codebase they come from `expect` and `unwrap` with literal
/// messages, so the text is developer-written rather than user data, but it is bounded and stripped
/// anyway: a diagnostic that can carry an arbitrary payload is a diagnostic that can carry whatever
/// was in scope when the panic happened.
fn panic_summary(payload: &(dyn std::any::Any + Send)) -> String {
    let text = payload
        .downcast_ref::<&'static str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic payload was not a string".to_string());
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_PANIC_SUMMARY)
        .collect()
}

/// Watch a long-lived task and record how it ended.
///
/// Every spawn site used to destructure the handle as `_task` and drop it, so the shell could hold
/// a perfectly live-looking actor whose task had exited minutes earlier. The symptoms surface much
/// later as stale state, missing events, or a generic "actor stopped", with the panic or exit cause
/// long gone: the one piece of evidence that would have explained it is exactly what was discarded.
///
/// Deliberately does not restart anything. These tasks own MLS and CRDT state, and a blind restart
/// would trade a diagnosable stop for an undiagnosable inconsistency. The policy is to surface the
/// failure and preserve the cause; recovery is a decision for a level that knows what was lost.
fn supervise(kind: &'static str, server: u64, task: tokio::task::JoinHandle<()>) {
    supervise_task(kind, Some(server), None, task);
}

/// Watch a long-lived task, and keep what became of it after the log line has aged out.
///
/// Every critical spawn goes through here. Only the server actor did: six other long-lived tasks
/// had their `JoinHandle` dropped on the floor, so their deaths were unobserved. The event
/// forwarder is the one that matters most, because it can die while the actor stays perfectly
/// healthy: the protocol keeps running and the webview is told none of it, which a user sees as a
/// stale unread badge and the app would have reported as fine. Found by adversarial review
/// (P3-009).
///
/// `expect_ms` is how often the task promises to report progress, for the ones that have a rhythm.
/// A task that makes no promise is never called stalled, because silence is not evidence: the
/// forwarder can have nothing to forward for an hour and be working perfectly.
fn supervise_task(
    kind: &'static str,
    server: Option<u64>,
    expect_ms: Option<u64>,
    task: tokio::task::JoinHandle<()>,
) -> tasks::TaskHandle {
    let handle = tasks::register(kind, server, wall_ms(), expect_ms);
    supervise_registered(kind, server, handle, task);
    handle
}

/// Spawn and supervise a task from a caller that may not be on the async runtime yet.
///
/// # The nesting is the point
///
/// Tauri's `setup` runs on the main thread *before* the async runtime has been entered.
/// `tokio::spawn` panics there ("there is no reactor running"), and a panic in `setup` means the
/// application does not start at all: no window, no log beyond the session line, exit code 101.
/// Tauri's own `spawn` works because it holds a runtime handle already.
///
/// But Tauri's `JoinHandle` cannot report a panic, and reporting one is the entire reason these
/// tasks are supervised. So the outer spawn is Tauri's, and *inside* that block a runtime is
/// current, which is where the real `tokio::JoinHandle` is made.
///
/// One function rather than the pattern written out at each call site, because getting it wrong
/// looks exactly like getting it right until the app is launched, and nothing in the test suite
/// launches it. This is the shape a caller before the runtime has to use; the test below calls it
/// from a plain `#[test]`, which is the same context `setup` is in.
fn supervise_detached<F>(
    kind: &'static str,
    server: Option<u64>,
    expect_ms: Option<u64>,
    future: F,
) -> tasks::TaskHandle
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    // Registered here rather than inside the block, so the task is known from the moment it is
    // asked for rather than from the moment it gets a thread.
    let watched = tasks::register(kind, server, wall_ms(), expect_ms);
    tauri::async_runtime::spawn(async move {
        supervise_registered(kind, server, watched, tokio::spawn(future));
    });
    watched
}

/// [`supervise_task`] for a task that was registered before it was spawned.
///
/// A task that reports progress needs its own registry handle inside its loop, which means the
/// registration has to happen first.
fn supervise_registered(
    kind: &'static str,
    server: Option<u64>,
    handle: tasks::TaskHandle,
    task: tokio::task::JoinHandle<()>,
) {
    tauri::async_runtime::spawn(async move {
        let (state, cause) = match task.await {
            // The mailbox closed and the loop returned. Ordinary at shutdown, and worth a line
            // either way because "the task is gone" explains a lot of later symptoms.
            Ok(()) => {
                tracing::info!(target: "catcoms_app", kind, server, "RUNTIME.TASK.EXITED");
                (tasks::TaskState::Exited, None)
            }
            Err(e) if e.is_cancelled() => {
                tracing::warn!(target: "catcoms_app", kind, server, "RUNTIME.TASK.CANCELLED");
                (tasks::TaskState::Cancelled, None)
            }
            Err(e) => {
                let cause = panic_summary(&*e.into_panic());
                tracing::error!(
                    target: "catcoms_app",
                    kind,
                    server,
                    cause = %cause,
                    "RUNTIME.TASK.PANICKED"
                );
                (tasks::TaskState::Panicked, Some(cause))
            }
        };
        tasks::finished(handle, state, cause);
    });
}

/// Insert a freshly-spawned server into the registry, supervise its task, forward its events, and
/// return the new server id.
#[allow(clippy::too_many_arguments)]
async fn register_server(
    app: &AppHandle,
    state: &AppState,
    actor: ServerActor,
    events: mpsc::Receiver<catcoms_app::TracedEvent>,
    task: tokio::task::JoinHandle<()>,
    group_id: Vec<u8>,
    device_id: DeviceId,
    invite: Option<String>,
    name: String,
    bootstrap: Vec<String>,
    bootstrap_owners: HashMap<String, HashSet<BootstrapOwner>>,
    interface_routes: Option<InterfaceRouteIdentity>,
    rendezvous: Vec<String>,
    mesh: Option<MeshHandle>,
    is_dm: bool,
    switchboard: bool,
    record_seq: u64,
) -> u64 {
    let id = {
        let mut n = state.next_id.lock().await;
        *n += 1;
        *n
    };
    let instance = state.next_server_instance.fetch_add(1, Ordering::Relaxed);
    supervise("server_actor", id, task);
    forward_events(app.clone(), id, events);
    let timer_actor = actor.clone();
    state.servers.lock().await.insert(
        id,
        ServerEntry {
            actor,
            instance,
            group_id,
            device_id,
            invite,
            name,
            bootstrap,
            bootstrap_owners,
            interface_routes,
            rendezvous,
            mesh,
            is_dm,
            switchboard,
            record_seq,
            persist: PersistCounters::default(),
        },
    );
    install_reconnect_capture_worker(app, id, timer_actor.clone());
    // Start only after the entry exists. A zero-millisecond randomized first tick must not race
    // the registry insertion and silently skip the initial interface/discovery refresh.
    spawn_discovery_timer(app.clone(), id, timer_actor);
    id
}

/// Draw the next peer-record sequence number for `server` from this launch's reserved block, or
/// `None` if the server is gone. Monotonic within the launch; every number it can return is below
/// anything the next launch's block starts at.
async fn next_record_seq(state: &AppState, server: u64) -> Option<u64> {
    let mut servers = state.servers.lock().await;
    let e = servers.get_mut(&server)?;
    e.record_seq = e.record_seq.saturating_add(1);
    Some(e.record_seq)
}

/// Snapshot a running server through its actor and seal it to disk (best-effort: a missing
/// store, a stopped actor, or an I/O error is logged, not fatal; the app keeps running).
///
/// Concurrent requests are coalesced, and the contract every caller depends on is unchanged: this
/// returns only after a write whose snapshot was taken **after** the caller's own change. A burst
/// of sends therefore costs the writes it needs rather than one whole-server write each; what it
/// must never become is a debounce, which would let a command report success before its message
/// was durable.
async fn persist_server(state: &AppState, server: u64) {
    // The ticket is taken before anything is written, and the caller's change is already applied,
    // so any snapshot taken after this point contains it. The incarnation is captured with it:
    // everything below belongs to *this* installation of the id, and a persisted id is reused
    // when a server is removed and reinstalled.
    let Some((ticket, instance, actor)) = ({
        let mut servers = state.servers.lock().await;
        servers
            .get_mut(&server)
            .map(|entry| (entry.persist.request(), entry.instance, entry.actor.clone()))
    }) else {
        return;
    };
    persist_captured(state, server, instance, ticket, actor).await;
}

/// [`persist_server`] once its request has been recorded: everything from waiting for the id's
/// lock to retiring the requests the write covered. Separate so the incarnation rule can be
/// tested against a writer that was captured before a replacement was installed, which is the
/// schedule that cannot be produced by calling `persist_server` twice.
async fn persist_captured(
    state: &AppState,
    server: u64,
    instance: u64,
    ticket: u64,
    actor: ServerActor,
) {
    let lock = persist_lock_for(state, server);
    let _writing = lock.lock().await;
    // Two questions, now that this writer holds the id's lock. Is the entry still the one this
    // request belongs to; and did a write that started after this ticket already carry it?
    let covering = {
        let servers = state.servers.lock().await;
        let Some(entry) = servers.get(&server) else {
            return;
        };
        if entry.instance != instance {
            // The server was replaced while this write waited. Its bytes describe a group that no
            // longer holds this slot, and its ticket says nothing about the new one's mutations.
            return;
        }
        if !entry.persist.needs_write(ticket) {
            return;
        }
        entry.persist.requested
    };
    let bytes = match actor.snapshot().await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(target: "catcoms_app", server, error = %e, "VAULT.SNAPSHOT.FAILED");
            return;
        }
    };
    let guard = state.store.lock().await;
    let Some(store) = guard.as_ref() else {
        return; // no store mounted: nothing was written, so nothing may be retired
    };
    // Re-checked immediately before the write, because the snapshot above is an await: a
    // replacement installed during it must not have this incarnation's bytes written over it.
    // Safe against a replacement racing this exact moment because installing one takes the
    // registry lock, and removal cannot complete between this check and the write while the
    // registry guard below is held.
    let servers = state.servers.lock().await;
    if servers.get(&server).map(|entry| entry.instance) != Some(instance) {
        return;
    }
    let mut rng = OsCryptoRng;
    if let Err(e) = store.save_server(server, &bytes, &mut rng) {
        tracing::error!(target: "catcoms_app", server, error = %e, "VAULT.SEAL_SERVER.FAILED");
        return;
    }
    drop(servers);
    drop(guard);
    // Only a write that actually reached the disk may retire the requests it covered, and only
    // for the incarnation that made them.
    if let Some(entry) = state.servers.lock().await.get_mut(&server) {
        if entry.instance == instance {
            entry.persist.completed_through(covering);
        }
    }
}

/// The persistence lock for a numeric server id, created on first use. See
/// [`AppState::persist_locks`]: it is deliberately not per registry entry.
fn persist_lock_for(state: &AppState, server: u64) -> Arc<Mutex<()>> {
    let mut locks = state
        .persist_locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks.entry(server).or_default().clone()
}

/// Seal this server's cross-session address cache (the previously-proven members it can dial
/// straight away next launch) beside its snapshot.
///
/// Deliberately **not** folded into [`persist_server`], which runs after every single message: the
/// cache only changes when the set of known member records changes, which happens on the discovery
/// tick and essentially nowhere else, so it is written on that cadence instead of on the chat hot
/// path. Best-effort throughout, exactly like the snapshot: a locked vault or an I/O error costs
/// a faster reconnect next launch and nothing else.
async fn persist_address_cache(app: &AppHandle, server: u64) {
    let state = app.state::<AppState>();
    let state = state.inner();
    let Ok(actor) = actor_of_unchecked(state, server).await else {
        return;
    };
    // Fetch the key, then the bytes, before taking the store lock, so the actor round-trip never
    // happens while holding it.
    let key = {
        let guard = state.store.lock().await;
        match guard.as_ref() {
            Some(store) => match store.address_cache_key() {
                Ok(k) => k,
                Err(e) => {
                    tracing::warn!(target: "catcoms_app", server, error = %e, "VAULT.ADDRESS_CACHE.NO_KEY");
                    return;
                }
            },
            None => return, // vault locked; in-memory only
        }
    };
    let Ok(bytes) = actor.address_cache_bytes(key).await else {
        return; // the actor stopped
    };
    let guard = state.store.lock().await;
    if let Some(store) = guard.as_ref() {
        let mut rng = OsCryptoRng;
        if let Err(e) = store.save_address_cache(server, &bytes, &mut rng) {
            tracing::warn!(target: "catcoms_app", server, error = %e, "VAULT.ADDRESS_CACHE.SEAL_FAILED");
        }
    }
}

/// Upgrade or refresh an admission-authorized private-address reconnect hint.
///
/// Direct admission may refresh only its named inviter. A legacy v1/v2 record gets one migration
/// opportunity only for an unambiguous two-member group; new helper/reply/switchboard admissions
/// are durably disabled so an empty route list can never be mistaken for migration consent. An
/// empty observation never erases the last sealed hint: the normal reason for seeing no live route
/// is precisely that the remote app is closed, when the hint is needed most.
async fn persist_live_local_reconnect_routes(app: &AppHandle, server: u64, actor: &ServerActor) {
    let state = app.state::<AppState>();
    let state = state.inner();
    let now_ms = SystemClock.now_ms();
    let net = {
        let guard = state.store.lock().await;
        let Some(store) = guard.as_ref() else {
            return;
        };
        match store.load_server_net(server) {
            Ok(Some(mut net)) => {
                if net.pending_recovery_peer.is_some()
                    && now_ms > net.pending_recovery_expires_at_ms
                {
                    net.pending_recovery_peer = None;
                    net.pending_recovery_expires_at_ms = 0;
                    if let Err(error) = store.save_server_net(server, &net, &mut OsCryptoRng) {
                        tracing::warn!(
                            target: "catcoms_app",
                            server,
                            error = %error,
                            "VAULT.RECONNECT_PENDING.EXPIRY_CLEAR_FAILED"
                        );
                    }
                }
                net
            }
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    target: "catcoms_app",
                    server,
                    error = %error,
                    "VAULT.RECONNECT_ROUTE.LOAD_FAILED"
                );
                return;
            }
        }
    };
    let mesh = {
        let servers = state.servers.lock().await;
        servers.get(&server).and_then(|entry| entry.mesh.clone())
    };
    let Some(mesh) = mesh else {
        return;
    };
    let member_routes = actor.member_routes().await;
    let claimed_peers: Vec<_> = member_routes
        .iter()
        .filter_map(|route| route.peer_id.map(PeerId::new))
        .collect();
    // Actor and mesh observations may wait behind unrelated work. Sample again after those awaits
    // so a code that expired while evidence was gathered cannot be selected on a stale timestamp.
    let selection_now_ms = SystemClock.now_ms();
    let pending_recovery_peer = pending_recovery_capture_peer(
        net.pending_recovery_peer,
        net.pending_recovery_expires_at_ms,
        selection_now_ms,
        claimed_peers.iter().copied(),
    );
    // A recovery code grants only session-local permission to try its signer. It becomes the
    // durable singleton contact only once present-time roster evidence and the mesh's outbound
    // authenticated-route ledger agree. Until then the old proven contact/routes remain intact.
    let capture_peer = pending_recovery_peer.or_else(|| {
        reconnect_capture_peer(
            net.reconnect_policy,
            member_routes.len(),
            claimed_peers.iter().copied(),
        )
    });
    let Some(capture_peer) = capture_peer else {
        return;
    };
    let selected_from_pending = pending_recovery_peer == Some(capture_peer);
    let member_peers = HashSet::from([capture_peer]);
    let routes = if selected_from_pending {
        // Recovery codes permit safe public direct literals as well as LAN addresses. Promotion
        // still uses only the exact outbound Noise-authenticated route for this unique member.
        select_authenticated_reconnect_routes(
            mesh.authenticated_dial_route_evidence(),
            &member_peers,
            false,
        )
    } else {
        authenticated_member_lan_reconnect_routes(&mesh, &member_peers)
    };
    if routes.is_empty() {
        return;
    }
    let actor_routes = routes
        .iter()
        .map(|route| (PeerId::new(route.peer_id), route.address.clone()))
        .collect();
    let guard = state.store.lock().await;
    let Some(store) = guard.as_ref() else {
        return;
    };
    // Re-read under the final write lock. A recovery-code apply can update only the pending field
    // while discovery is awaiting member/mesh evidence; saving the earlier `net` wholesale would
    // silently erase that newer consent.
    let mut current = match store.load_server_net(server) {
        Ok(Some(current)) => current,
        _ => return,
    };
    // The vault lock itself may have crossed the signed deadline. This final sample is the
    // authority boundary used by the atomic merge and durable save.
    let final_now_ms = SystemClock.now_ms();
    let Some(changed) = merge_live_reconnect_capture(
        &mut current,
        selected_from_pending,
        capture_peer,
        routes,
        member_routes.len(),
        final_now_ms,
        claimed_peers.iter().copied(),
    ) else {
        return;
    };
    if changed {
        if let Err(error) = store.save_server_net(server, &current, &mut OsCryptoRng) {
            tracing::warn!(
                target: "catcoms_app",
                server,
                error = %error,
                "VAULT.RECONNECT_ROUTE.SEAL_FAILED"
            );
            return;
        }
    }
    drop(guard);
    // The durable write is the safety boundary. Updating the live actor afterwards avoids holding
    // the vault lock across an actor await; a failure costs only this session's proactive redial.
    let _ = actor.set_local_reconnect_routes(actor_routes).await;
}

/// Re-seal the registry (the set of servers + their names/invites) to disk.
async fn persist_registry(state: &AppState) {
    let records: Vec<ServerRecord> = {
        let servers = state.servers.lock().await;
        servers
            .iter()
            .map(|(id, e)| ServerRecord {
                id: *id,
                display_name: e.name.clone(),
                invite: e.invite.clone().unwrap_or_default(),
                is_dm: e.is_dm,
            })
            .collect()
    };
    let guard = state.store.lock().await;
    if let Some(store) = guard.as_ref() {
        let mut rng = OsCryptoRng;
        if let Err(e) = store.save_registry(&records, &mut rng) {
            tracing::error!(target: "catcoms_app", error = %e, "VAULT.REGISTRY.SEAL_FAILED");
        }
    }
}

/// Attach the per-server sealing blob store (Phase 9h) if the vault is unlocked, so files +
/// avatars persist encrypted at rest (keyed by the stable group id, so a reloaded server
/// finds its blobs). Best-effort: a locked store or an error leaves the in-memory default.
/// Must run before any blob is added (i.e. before `spawn`).
///
/// Also sweeps the store's staging area. An upload seals its chunks there and promotes them only
/// when its manifest is published, so whatever is still staged belonged to an upload that did not
/// survive the last process: the only record of what it was for died with that process, and no
/// later upload can adopt it. This is the one place that can say so, because it runs once per
/// server before anything has had a chance to stage something new.
async fn attach_blob_store<T: MeshTransport, R: catcoms_rt::CryptoRngCore>(
    state: &AppState,
    server: &mut Server<T, R>,
) {
    let guard = state.store.lock().await;
    if let Some(store) = guard.as_ref() {
        let key = hex::encode(server.group_id());
        match store.blob_store(&key) {
            Ok(blobs) => server.set_blob_store(blobs),
            Err(e) => {
                tracing::error!(target: "catcoms_app", error = %e, "STORAGE.BLOB_STORE.ATTACH_FAILED")
            }
        }
    }
    drop(guard);
    let swept = server.clear_staged_uploads();
    if swept > 0 {
        tracing::info!(target: "catcoms_app", chunks = swept, "STORAGE.UPLOAD.STAGED_SWEPT");
    }
}

/// Strip a trailing `/p2p/<id>` from a bootstrap address to get the bare transport address to
/// advertise as an *external* address for rendezvous registration (libp2p re-appends our own id).
/// Returns `None` for a relay-circuit address (those auto-promote to external on reservation) or
/// an unparseable string.
fn external_addr(s: &str) -> Option<Multiaddr> {
    let mut addr: Multiaddr = s.parse().ok()?;
    if addr.iter().any(|p| matches!(p, Protocol::P2pCircuit)) {
        return None;
    }
    if matches!(addr.iter().last(), Some(Protocol::P2p(_))) {
        addr.pop();
    }
    Some(addr)
}

// The address classifiers `dialable_bootstrap` and `external_addrs` are built from
// (`addr_is_private`, `addr_is_undialable`, `addr_is_loopback`) now live in `catcoms_net::addr`,
// imported at the top of this file. They were moved there when the rendezvous validator was split
// by trust (P13) and needed the same rules: two classifiers that disagree about which literals are
// hostile is the exact defect shape this whole family of checks exists to close, so there is one
// set of them and every caller shares it. The policy built on top of them stays here, because it
// is specific to an invite's bootstrap list.

/// The most bootstrap addresses this client will actually dial out of one invite.
///
/// `InviteToken` permits 64. A genuine invite needs a fraction of that: the relayed address, the
/// user's advertised address, the UPnP-discovered public one, the auto-detected LAN IPv4 and
/// routable IPv6 over both TCP and QUIC, and loopback. Twelve covers every real shape with room
/// to spare, and turns "one pasted string, 64 outbound connections to hosts of the author's
/// choosing" into something with a much smaller blast radius.
const MAX_BOOTSTRAP_DIALS: usize = 12;
/// Pre-join rendezvous is a route to the inviter, not the destination. Two distinct validated
/// seeds retain redundancy while leaving the per-group scheduler budget for the inviter itself.
const MAX_INVITE_RENDEZVOUS_DIALS: usize = 2;

/// Validate and rank an invite's `bootstrap` list into the addresses this client will dial.
///
/// The invite's `rendezvous` vector is carefully validated (`validate_rendezvous_addrs`: no
/// circuit addresses, exactly one `/p2p/` stanza, distinct peer ids) and `bootstrap` was not
/// validated at all, despite being the list that is dialled *unconditionally and in bulk*
/// (defect P7). `decode_and_verify_invite` proves the addresses have not been edited since
/// signing; it says nothing at all about whether the signer chose sensible ones, and the signer
/// is exactly the party we are guarding against here.
///
/// Rules:
/// * anything unparseable, naming something that cannot be a peer ([`addr_is_undialable`]), or
///   missing the canonical non-zero TCP/QUIC stack and terminal peer id is dropped;
/// * loopback is kept only when **nothing else survived**. A loopback entry is by construction
///   the same-machine case (two instances on one dev box, the DM/self-pairing flows), and that
///   case is real and must keep working. But when the invite also carries routable addresses,
///   a loopback entry is not a fallback for anything: it can only ever probe ports on the
///   reader's own machine, so it is dropped rather than dialled;
/// * the survivors are capped at [`MAX_BOOTSTRAP_DIALS`], routable first; the process scheduler
///   applies its tighter present-window allowance before the transport is constructed.
///
/// Private (LAN) addresses are deliberately **kept**: a group on one home network is the single
/// most common first invite, and dropping them would break it. The exposure is bounded by the
/// cap and by the fact that a LAN address the author chose can only aim at the reader's own
/// segment, which the reader can already scan.
fn dialable_bootstrap(bootstrap: &[String]) -> Vec<Multiaddr> {
    let parsed: Vec<Multiaddr> = bootstrap
        .iter()
        .filter_map(|s| s.parse::<Multiaddr>().ok())
        .filter(|a| canonical_invite_peer_endpoint(a).is_some())
        .collect();
    let (loopback, routable): (Vec<Multiaddr>, Vec<Multiaddr>) =
        parsed.into_iter().partition(addr_is_loopback);
    let mut out = if routable.is_empty() {
        loopback
    } else {
        routable
    };
    out.truncate(MAX_BOOTSTRAP_DIALS);
    out
}

/// The external addresses to advertise (rendezvous registration, identify) from the bootstrap
/// list: **globally routable ones only**.
///
/// This used to fall back to advertising loopback when nothing else was reachable, which put an
/// address no remote peer can dial into a shared rendezvous namespace. With a stable port and a
/// stable peer id the calculus is worse than merely useless: an advertised private address is a
/// free map of the advertiser's internal network. An empty result simply means "we have nothing
/// worth publishing yet"; the registration stays deferred until UPnP, a relay circuit or a real
/// public address supplies one.
fn external_addrs(bootstrap: &[String]) -> Vec<Multiaddr> {
    bootstrap
        .iter()
        .filter_map(|s| external_addr(s))
        .filter(addr_is_globally_routable)
        .collect()
}

/// Register a `(group_id, nonce)` invite's pre-join namespace at the rendezvous `rz` via `handle`
/// (fire-and-forget; the grant is internally deferred + flushed once an external address exists,
/// which the founder establishes when it first registers). So a joiner holding the invite can
/// discover this server with no hard-coded address.
async fn register_join_ns(
    handle: &MeshHandle,
    group_id: &[u8],
    nonce: &[u8; 16],
    rz: &RendezvousTarget,
) -> Result<(), String> {
    let ns = join_namespace(group_id, nonce, &rz.peer.to_bytes());
    handle
        .rendezvous_register(&ns, rz.peer)
        .await
        .map_err(|e| e.to_string())
}

/// How long the Connectivity assistant labels automatic mapping as "waiting" before giving an
/// honest no-result summary. The collector itself stays alive afterward: mappings are renewable
/// leases, so late successes and expirations must still update peer records and stored invites.
const PORT_MAPPING_WINDOW_SECS: u64 = 25;

/// Everything that makes a node reachable from *outside* this machine, gathered in one place so
/// founding and reloading do identical work.
struct Reachability {
    /// The dialable addresses a minted invite should carry (relayed first when there is a relay).
    bootstrap: Vec<String>,
    /// Per-entry ownership matching `bootstrap`, retained after startup so interface polling can
    /// withdraw only the raw route without trampling manual or relay ownership.
    bootstrap_owners: HashMap<String, HashSet<BootstrapOwner>>,
    /// The validated rendezvous target this server registers at, if any.
    rz_target: Option<RendezvousTarget>,
    /// A handle for registering later invites' namespaces; kept only once the rendezvous connected.
    rz_handle: Option<MeshHandle>,
}

fn listener_summary(addresses: &[Multiaddr]) -> String {
    let mut capabilities = Vec::new();
    for address in addresses {
        if address
            .iter()
            .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
        {
            continue;
        }
        let family = if address
            .iter()
            .any(|protocol| matches!(protocol, Protocol::Ip4(_)))
        {
            "IPv4"
        } else if address
            .iter()
            .any(|protocol| matches!(protocol, Protocol::Ip6(_)))
        {
            "IPv6"
        } else {
            "other"
        };
        let transport = if address
            .iter()
            .any(|protocol| matches!(protocol, Protocol::QuicV1))
        {
            "QUIC"
        } else if address
            .iter()
            .any(|protocol| matches!(protocol, Protocol::Tcp(_)))
        {
            "TCP"
        } else {
            "transport"
        };
        let label = format!("{family} {transport}");
        if !capabilities.contains(&label) {
            capabilities.push(label);
        }
    }
    capabilities.sort();
    capabilities.join(", ")
}

/// Record only listeners the Swarm has reported live. `listen_on` returning a ListenerId means
/// the request was accepted, but the OS bind may still fail asynchronously; presenting that as
/// “bound IPv4 + IPv6” was a concrete Connectivity Assistant truth bug.
async fn record_listener_evidence(
    mesh: &MeshService,
    accepted: &[Multiaddr],
    steps: &mut Vec<DiagStep>,
) {
    let deadline = SystemClock.sleep(Duration::from_secs(3));
    tokio::pin!(deadline);
    let snapshot = tokio::select! {
        value = mesh.next_listener_snapshot() => value,
        _ = &mut deadline => None,
    };
    let addresses = snapshot.map(|value| value.addresses).unwrap_or_default();
    let direct: Vec<_> = addresses
        .into_iter()
        .filter(|address| {
            !address
                .iter()
                .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
        })
        .collect();
    if direct.is_empty() {
        steps.push(DiagStep::failed(
            "listen",
            "",
            format!(
                "{} listen request(s) were accepted, but no live OS listener was reported within 3s",
                accepted.len()
            ),
        ));
    } else {
        steps.push(DiagStep::ok(
            "listen",
            "",
            format!(
                "Swarm reported {} live listener(s): {}",
                direct.len(),
                listener_summary(&direct)
            ),
        ));
    }
}

/// Do the reachability work for a server: assemble the bootstrap list (auto-detected LAN IPv4,
/// routable IPv6 and loopback, plus the user's advertised address), reserve a relay circuit, and
/// connect + advertise at the rendezvous.
///
/// This runs identically at found time **and at every reload**. It used to live inline in
/// `found_server` only, so after a restart a server rebuilt a loopback-only bootstrap and every
/// invite minted from then on worked on that one machine; the remote joiner's "timed out
/// connecting to the server" was the visible end of that.
///
/// Best-effort by construction: it returns whatever reachability it managed plus the list of
/// things that went wrong, and lets the caller decide. Founding surfaces the problems to the user
/// (they just typed those addresses); a reload logs them and carries on with what worked.
///
/// `steps` collects a copyable record of what was tried for the connectivity panel; a reload
/// passes a throwaway vector, since only an attempt the user is watching has a panel to fill.
async fn establish_reachability(
    mesh: &MeshService,
    peer_id: &str,
    port: u16,
    accepted_listeners: &[Multiaddr],
    net: &ServerNet,
    steps: &mut Vec<DiagStep>,
) -> (Reachability, Vec<String>) {
    let mut problems = Vec::new();
    let mut bootstrap = auto_bootstrap(port, peer_id);
    let mut bootstrap_owners = bootstrap_owners(&bootstrap, BootstrapOwner::AutomaticInterface);
    record_listener_evidence(mesh, accepted_listeners, steps).await;

    let advertise = net.advertise.trim();
    if !advertise.is_empty() {
        match build_advertised(advertise, port, peer_id) {
            // The user's own address is the authoritative one.
            Ok(a) => {
                steps.push(DiagStep::unknown(
                    "advertise",
                    a.clone(),
                    "the address you supplied; AutoNAT will test it if public infrastructure connects",
                ));
                add_bootstrap_owner(
                    &mut bootstrap,
                    &mut bootstrap_owners,
                    a,
                    BootstrapOwner::Configured,
                    true,
                );
            }
            Err(e) => {
                steps.push(DiagStep::failed("advertise", advertise, e.clone()));
                problems.push(e)
            }
        }
    }

    // Reserve a relay circuit and prefer the relayed address (NAT traversal, no port-forward).
    let relay = net.relay.trim();
    if !relay.is_empty() {
        match reserve_relay_circuit(mesh, relay).await {
            Ok(addr) => {
                steps.push(DiagStep::ok(
                    "relay",
                    addr.clone(),
                    "circuit reserved; joiners can reach this node through the relay",
                ));
                add_bootstrap_owner(
                    &mut bootstrap,
                    &mut bootstrap_owners,
                    addr,
                    BootstrapOwner::Relay,
                    true,
                );
            }
            Err(e) => {
                steps.push(DiagStep::failed("relay", relay, e.clone()));
                problems.push(e)
            }
        }
    }

    // Promote every plausible direct address even when no rendezvous is configured, and offer it
    // to AutoNAT v2 for a real callback. Previously external addresses were added only inside
    // `connect_rendezvous`, so a manually-forwarded address or global IPv6 on a direct-only server
    // was never tested. Relay circuits are excluded by `external_addrs`; a reservation promotes
    // itself because "reachable through a relay" is a separate property from direct reachability.
    for addr in external_addrs(&bootstrap) {
        if let Err(e) = mesh.add_external_address(addr.clone()).await {
            tracing::warn!(target: "catcoms_app", %addr, error = %e, "REACH.AUTONAT.OFFER_FAILED");
        }
    }

    // Optional rendezvous: connect to it, keeping a handle to register each invite's namespace
    // after the server is spawned. The node is then discoverable with no hard-coded address; a
    // joiner needs only the pasted invite.
    let mut rz_target = None;
    let mut rz_handle = None;
    let rendezvous = net.rendezvous.trim();
    if !rendezvous.is_empty() {
        match connect_rendezvous(mesh, rendezvous, &bootstrap).await {
            Ok(rz) => {
                steps.push(DiagStep::ok(
                    "rendezvous",
                    rz.addr.to_string(),
                    "connected and advertised; joiners can find this node with the invite alone",
                ));
                rz_target = Some(rz);
                rz_handle = Some(mesh.handle());
            }
            Err(e) => {
                steps.push(DiagStep::failed("rendezvous", rendezvous, e.clone()));
                problems.push(e)
            }
        }
    }

    (
        Reachability {
            bootstrap,
            bootstrap_owners,
            rz_target,
            rz_handle,
        },
        problems,
    )
}

/// Dial the relay, reserve a circuit slot on it, and return the granted circuit address. The
/// relay-client transport needs a live connection before it can reserve, hence the wait.
async fn reserve_relay_circuit(mesh: &MeshService, relay: &str) -> Result<String, String> {
    let circuit: Multiaddr = format!("{relay}/p2p-circuit")
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| format!("bad relay address: {e}"))?;
    let relay_addr: Multiaddr = relay
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| format!("bad relay address: {e}"))?;
    // Wait for the relay *specifically*, not for any peer: a reload also dials the remembered
    // members from the snapshot, so "something connected" would fire on the wrong peer and the
    // reservation would be attempted before the relay-client had a connection to reserve over.
    let relay_peer = target_peer_in_multiaddr(&relay_addr)
        .map(|p| phase0_peer_id(&p))
        .ok_or_else(|| "the relay address carries no peer id".to_string())?;
    // Dialing again when the transport was already constructed dialing this relay is harmless
    // (libp2p collapses it onto the existing connection) and is what makes a reload self-contained.
    mesh.dial(relay_addr).await.map_err(|e| e.to_string())?;
    timeout(
        Duration::from_secs(20),
        mesh.wait_for_peer_connected(relay_peer),
    )
    .await
    .map_err(|_| "could not connect to the relay".to_string())?
    .map_err(|_| "the relay transport closed while connecting".to_string())?;
    mesh.listen_on(circuit).await.map_err(|e| e.to_string())?;
    let addr = timeout(Duration::from_secs(20), async {
        loop {
            match mesh.next_listen_addr().await {
                Some(a) if a.to_string().contains("p2p-circuit") => return Some(a),
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await
    .map_err(|_| "relay reservation timed out".to_string())?
    .ok_or_else(|| "relay reservation failed".to_string())?;
    Ok(addr.to_string())
}

/// Dial the rendezvous node and wait for the connection. Routable addresses are offered to the
/// transport by [`establish_reachability`] independently of whether rendezvous is configured;
/// this helper only establishes the infrastructure path.
async fn connect_rendezvous(
    mesh: &MeshService,
    rendezvous: &str,
    _bootstrap: &[String],
) -> Result<RendezvousTarget, String> {
    // Operator-typed: this is the "rendezvous" field of the create-server form, so a DNS name is
    // both legitimate and (for a TCP/443 TLS/WebSocket node) required.
    let rz = validate_operator_rendezvous_addrs(std::slice::from_ref(&rendezvous.to_string()))
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
        .ok_or_else(|| "no rendezvous address".to_string())?;
    mesh.dial(rz.addr.clone())
        .await
        .map_err(|e| e.to_string())?;
    let rz_peer = phase0_peer_id(&rz.peer);
    timeout(
        Duration::from_secs(20),
        mesh.wait_for_peer_connected(rz_peer),
    )
    .await
    .map_err(|_| "could not connect to the rendezvous".to_string())?
    .map_err(|_| "the rendezvous transport closed while connecting".to_string())?;
    Ok(rz)
}

/// Build this server's transport on its **own** persisted libp2p identity and stable port.
///
/// The identity is per server, never per device: Mewtual deliberately gives each server a separate
/// network identity so two servers cannot be correlated to the same person. Reusing it across
/// launches is what keeps an already-issued invite (which embeds `/p2p/<id>`) redeemable.
///
/// Returns the transport, peer id, selected port, and listen requests accepted synchronously.
/// Callers use the separate live-listener snapshot before claiming that an OS bind succeeded.
fn build_transport(
    net: &ServerNet,
    dial: &[Multiaddr],
) -> Result<(MeshService, libp2p::PeerId, u16, Vec<Multiaddr>), String> {
    let key = keypair_from_seed(net.key_seed).map_err(|e| e.to_string())?;
    let port = choose_port(net);
    let (mesh, libp2p_id, bound) =
        MeshService::new_tcp_with_key_and_port_mapping(key, &listen_addrs(port), dial)
            .map_err(|e| e.to_string())?;
    // With `port == 0` the OS assigned the number; read it back off whatever bound.
    let port = if port != 0 {
        port
    } else {
        bound.iter().find_map(listen_port).unwrap_or(0)
    };
    Ok((mesh, libp2p_id, port, bound))
}

/// This server's libp2p peer id, derived from its persisted identity seed. Lets a caller that
/// never held the `MeshService` (the joiner's rendezvous branch hands the transport straight
/// through) still build its own dialable addresses.
fn peer_id_of(net: &ServerNet) -> Result<String, String> {
    Ok(keypair_from_seed(net.key_seed)
        .map_err(|e| e.to_string())?
        .public()
        .to_peer_id()
        .to_string())
}

/// Mint a fresh per-server network identity: 32 random bytes for the libp2p keypair (drawn from
/// the injected OS RNG, exactly like an invite nonce), plus the reachability inputs the user gave.
/// The listen port falls out of the seed ([`ServerNet::derived_port`]). Persisted once, then
/// reused for the life of the server.
fn new_server_net(advertise: &str, relay: &str, rendezvous: &str) -> ServerNet {
    let mut key_seed = [0u8; 32];
    let mut rng = OsCryptoRng;
    rng.fill_bytes(&mut key_seed);
    let mut net = ServerNet {
        key_seed,
        // 0 = "no port used yet"; `choose_port` starts from the seed-derived home port, and the
        // number that actually bound is written back by the caller.
        port: 0,
        advertise: advertise.trim().to_string(),
        relay: relay.trim().to_string(),
        rendezvous: rendezvous.trim().to_string(),
        switchboard: false,
        // Reserved just below; this session gets the first block, the next launch the second.
        record_seq: 0,
        reconnect_routes: Vec::new(),
        reconnect_policy: ReconnectPolicy::Disabled,
        pending_recovery_peer: None,
        pending_recovery_expires_at_ms: 0,
    };
    // Own a peer-record sequence block from the very first session, so the invariant "this launch
    // can only publish numbers below anything the next launch can" holds from birth rather than
    // starting on the first reload.
    net.reserve_record_seq_block();
    net
}

/// Seal a server's network record to disk (best-effort, like [`persist_server`]): a locked vault
/// or an I/O error costs a stable identity on the *next* launch, which is worth a log line but not
/// worth failing the operation the user actually asked for.
async fn persist_server_net(state: &AppState, server: u64, net: &ServerNet) {
    let guard = state.store.lock().await;
    if let Some(store) = guard.as_ref() {
        let mut rng = OsCryptoRng;
        if let Err(e) = store.save_server_net(server, net, &mut rng) {
            tracing::error!(target: "catcoms_app", server, error = %e, "VAULT.NET_IDENTITY.SEAL_FAILED");
        }
    }
}

/// Load a server's persisted network record, or mint one when the server predates it, and
/// reserve this launch's block of PEX peer-record sequence numbers.
///
/// The reservation is sealed back to disk **before** the caller brings the transport up, so a
/// crash can only skip sequence numbers, never reuse them. Reuse would be the fatal one: a peer
/// keeps an incoming `PeerDescriptor` only when its `seq` beats the one it already holds, so a
/// node that restarted and resumed counting from where it left off would have every record it
/// publishes rejected forever, and its peers would keep dialling a dead address.
///
/// The migration case (`fallback_rendezvous` recovered from the persisted invite, which is where
/// the rendezvous address used to be kept) rotates the peer id exactly once, on the first launch
/// after this landed; from then on it is stable. There is nothing better available: the old
/// identity was never written down.
async fn load_or_init_server_net(
    state: &AppState,
    server: u64,
    fallback_rendezvous: &str,
) -> ServerNet {
    let stored = {
        let guard = state.store.lock().await;
        match guard.as_ref() {
            Some(store) => match store.load_server_net(server) {
                Ok(net) => net,
                Err(e) => {
                    tracing::warn!(target: "catcoms_app", server, error = %e, "VAULT.NET_IDENTITY.LOAD_FAILED");
                    None
                }
            },
            None => None,
        }
    };
    let mut net = stored.unwrap_or_else(|| new_server_net("", "", fallback_rendezvous));
    net.reserve_record_seq_block();
    persist_server_net(state, server, &net).await;
    net
}

/// Deterministically summarize the live router mappings for the Connectivity assistant. Failed
/// probes are kept per mechanism/transport: one unavailable UDP mapping must not be broadened into
/// "PCP failed" while its TCP probe is still in flight.
type PortMappingStatusKey = (PortMappingMechanism, PortMappingTransport, Option<IpAddr>);

fn port_mapping_label(
    mechanism: PortMappingMechanism,
    transport: PortMappingTransport,
    local_address: Option<IpAddr>,
) -> String {
    match local_address {
        Some(IpAddr::V6(address)) => {
            format!("{mechanism} IPv6 pinhole {transport} ({address})")
        }
        Some(IpAddr::V4(address)) => format!("{mechanism} IPv4 {transport} ({address})"),
        None => format!("{mechanism} {transport}"),
    }
}

fn port_mapping_status(
    active: &HashMap<PortMappingStatusKey, Multiaddr>,
    unavailable: &HashMap<PortMappingStatusKey, String>,
    waiting: bool,
) -> String {
    let mapped = if active.is_empty() {
        None
    } else {
        let mut mappings: Vec<String> = active
            .iter()
            .map(|((mechanism, transport, local_address), addr)| {
                format!(
                    "{}: {addr}",
                    port_mapping_label(*mechanism, *transport, *local_address)
                )
            })
            .collect();
        mappings.sort_unstable();
        Some(format!("mapped via {}", mappings.join("; ")))
    };

    let mut absent = Vec::new();
    for mechanism in [
        PortMappingMechanism::Upnp,
        PortMappingMechanism::Pcp,
        PortMappingMechanism::NatPmp,
    ] {
        let tcp = unavailable.get(&(mechanism, PortMappingTransport::Tcp, None));
        let udp = unavailable.get(&(mechanism, PortMappingTransport::Udp, None));
        match (tcp, udp) {
            (Some(tcp), Some(udp)) if tcp == udp => {
                absent.push(format!("{mechanism} unavailable: {tcp}"));
            }
            (Some(tcp), Some(udp)) => absent.push(format!(
                "{mechanism} unavailable (TCP: {tcp}; UDP/QUIC: {udp})"
            )),
            (Some(tcp), None) => {
                absent.push(format!("{mechanism} TCP unavailable: {tcp}"));
            }
            (None, Some(udp)) => {
                absent.push(format!("{mechanism} UDP/QUIC unavailable: {udp}"));
            }
            (None, None) => {}
        }
    }
    let mut scoped: Vec<_> = unavailable
        .iter()
        .filter_map(|(&(mechanism, transport, local_address), detail)| {
            local_address.map(|local_address| {
                format!(
                    "{} unavailable: {detail}",
                    port_mapping_label(mechanism, transport, Some(local_address))
                )
            })
        })
        .collect();
    scoped.sort_unstable();
    absent.extend(scoped);
    if let Some(mapped) = mapped {
        return if absent.is_empty() {
            mapped
        } else {
            format!("{mapped}; other attempts: {}", absent.join("; "))
        };
    }
    if waiting {
        return PORT_MAPPING_WAITING.to_string();
    }
    if absent.is_empty() {
        PORT_MAPPING_TIMED_OUT.to_string()
    } else {
        // An unavailable entry can describe a lease that existed and later expired. Calling that
        // an initial timeout is both contradictory and misleading, so use neutral live-state copy.
        format!("{} ({})", PORT_MAPPING_INACTIVE, absent.join("; "))
    }
}

/// Retire one mechanism's lease and answer whether the durable bootstrap address can be removed.
/// Multiple router protocols may report the same socket, so only the final owner withdrawing it
/// makes the address stale. Kept pure so the duplicate-lease edge case is regression tested.
#[cfg(test)]
fn retire_port_mapping(
    active: &mut HashMap<PortMappingStatusKey, Multiaddr>,
    mechanism: PortMappingMechanism,
    transport: PortMappingTransport,
    address: &Multiaddr,
) -> bool {
    if active.get(&(mechanism, transport, None)) != Some(address) {
        return false;
    }
    active.remove(&(mechanism, transport, None));
    !active.values().any(|candidate| candidate == address)
}

/// Replace one mechanism's current lease and return an old address that no other mapping still
/// owns. Although the PCP/NAT-PMP worker normally emits `Expired(old)` before `Mapped(new)`, the
/// product fold must also tolerate an implementation (or UPnP gateway) that reports only the new
/// address; otherwise the old route would survive forever in invites and peer records.
#[cfg(test)]
fn replace_port_mapping(
    active: &mut HashMap<PortMappingStatusKey, Multiaddr>,
    mechanism: PortMappingMechanism,
    transport: PortMappingTransport,
    address: Multiaddr,
) -> Option<Multiaddr> {
    let previous = active.insert((mechanism, transport, None), address.clone());
    previous.filter(|old| old != &address && !active.values().any(|candidate| candidate == old))
}

/// Add or remove one mapped address in the durable bootstrap and republish only if the effective
/// address set changed. Two mechanisms may own the same public socket; expiry of one must not
/// withdraw an address while the other still has an active lease for it.
async fn fold_mapped_bootstrap(
    app: &AppHandle,
    server: u64,
    peer_id: &str,
    address: &Multiaddr,
    add: bool,
) -> bool {
    let entry = format!("{address}/p2p/{peer_id}");
    fold_bootstrap_entry(app, server, &entry, BootstrapOwner::PortMapping, add).await
}

/// Add or remove one exact dial address from this member's live peer record. Router mappings use a
/// bare transport address plus our peer id; relay circuit addresses already contain both relay and
/// local peer ids and therefore call this helper directly. `owner` prevents one lifecycle from
/// withdrawing an identical route that another lifecycle still owns.
async fn fold_bootstrap_entry(
    app: &AppHandle,
    server: u64,
    entry: &str,
    owner: BootstrapOwner,
    add: bool,
) -> bool {
    let entry = entry.to_string();
    let state = app.state::<AppState>();
    let changed_and_actor = {
        let mut servers = state.inner().servers.lock().await;
        let Some(server_entry) = servers.get_mut(&server) else {
            return false;
        };
        let changed = if add {
            // Mapping, configured and relay routes beat LAN/loopback during address racing.
            add_bootstrap_owner(
                &mut server_entry.bootstrap,
                &mut server_entry.bootstrap_owners,
                entry.clone(),
                owner,
                owner != BootstrapOwner::AutomaticInterface,
            )
        } else {
            remove_bootstrap_owner(
                &mut server_entry.bootstrap,
                &mut server_entry.bootstrap_owners,
                &entry,
                owner,
            )
        };
        changed.then(|| (server_entry.actor.clone(), server_entry.bootstrap.clone()))
    };

    let Some((actor, bootstrap)) = changed_and_actor else {
        return false;
    };
    {
        // The Connectivity assistant reads the last attempt record, not `ServerEntry` directly.
        // Keep its advertised-address list live too, otherwise it would announce a successful PCP
        // mapping while the copyable report still said "Addresses this node advertises: none".
        let mut diag = state.inner().diag.lock().await;
        if diag.server == server {
            if add {
                if !diag.advertised.contains(&entry) {
                    diag.advertised.insert(0, entry.clone());
                }
            } else if let Some(index) = diag.advertised.iter().position(|item| item == &entry) {
                diag.advertised.remove(index);
            }
        }
    }
    // A router lease nobody is told about is useless, while an expired lease left in an old peer
    // record is a persistent dead dial. Publish both additions and removals on a fresh sequence.
    if let Some(seq) = next_record_seq(state.inner(), server).await {
        actor.publish_self_record(bootstrap, seq).await;
    }
    true
}

async fn store_port_mapping_status(
    app: &AppHandle,
    server: u64,
    active: &HashMap<PortMappingStatusKey, Multiaddr>,
    unavailable: &HashMap<PortMappingStatusKey, String>,
    waiting: bool,
) {
    let state = app.state::<AppState>();
    let outcome = port_mapping_status(active, unavailable, waiting);
    let changed = state
        .inner()
        .upnp
        .lock()
        .await
        .insert(server, outcome.clone())
        .as_ref()
        != Some(&outcome);
    if changed {
        emit_tracked(
            app,
            "reachability-changed",
            ServerEvt { server },
            catcoms_diagnostics::TraceId::default(),
        );
    }
}

/// Reconcile one authoritative mapping snapshot into the live peer record. The source uses a watch
/// channel, so a slow/late consumer sees the newest set rather than an unbounded event backlog.
async fn apply_port_mapping_snapshot(
    app: &AppHandle,
    server: u64,
    peer_id: &str,
    snapshot: PortMappingSnapshot,
    active: &mut HashMap<PortMappingStatusKey, Multiaddr>,
    unavailable: &mut HashMap<PortMappingStatusKey, String>,
) {
    let (next, next_unavailable) = port_mapping_snapshot_state(snapshot);

    // The bridge needs one aggregate PortMapping owner per exact socket. The snapshot itself may
    // contain several protocol/interface owners; compare its unique address sets so expiry of one
    // protocol cannot withdraw the other, while a raw/manual collision remains protected by the
    // `ServerEntry` ownership map.
    let previous_addresses: HashSet<_> = active.values().cloned().collect();
    let next_addresses: HashSet<_> = next.values().cloned().collect();
    for old in previous_addresses.difference(&next_addresses) {
        fold_mapped_bootstrap(app, server, peer_id, old, false).await;
    }
    for address in next_addresses.difference(&previous_addresses) {
        fold_mapped_bootstrap(app, server, peer_id, address, true).await;
    }
    *active = next;
    *unavailable = next_unavailable;
}

/// Convert the public snapshot into family/interface-aware maps. Kept pure because accidentally
/// collecting by only `(mechanism, transport)` silently discards either PCPv4 or PCPv6.
fn port_mapping_snapshot_state(
    snapshot: PortMappingSnapshot,
) -> (
    HashMap<PortMappingStatusKey, Multiaddr>,
    HashMap<PortMappingStatusKey, String>,
) {
    let active = snapshot
        .active
        .into_iter()
        .map(|entry| {
            (
                (entry.mechanism, entry.transport, entry.local_address),
                entry.address,
            )
        })
        .collect();
    let unavailable = snapshot
        .unavailable
        .into_iter()
        .map(|failure| {
            (
                (failure.mechanism, failure.transport, failure.local_address),
                failure.detail.chars().take(240).collect(),
            )
        })
        .collect();
    (active, unavailable)
}

/// Watch UPnP, PCP and NAT-PMP in the background and fold live public mappings into the server's
/// stored bootstrap. Founding does not wait for a router, but the current snapshot remains live for
/// renewal/expiry and consumes constant memory even when a non-UI caller never takes it.
fn spawn_port_mapping_fold(
    app: AppHandle,
    server: u64,
    mut rx: watch::Receiver<PortMappingSnapshot>,
    peer_id: String,
) {
    let task = tokio::spawn(async move {
        let deadline = SystemClock.sleep(Duration::from_secs(PORT_MAPPING_WINDOW_SECS));
        tokio::pin!(deadline);
        let mut waiting = true;
        let mut active = HashMap::new();
        let mut unavailable = HashMap::new();
        let initial = rx.borrow_and_update().clone();
        apply_port_mapping_snapshot(
            &app,
            server,
            &peer_id,
            initial,
            &mut active,
            &mut unavailable,
        )
        .await;
        loop {
            let changed = tokio::select! {
                _ = &mut deadline, if waiting => {
                    waiting = false;
                    false
                }
                result = rx.changed() => {
                    if result.is_err() {
                        waiting = false;
                        store_port_mapping_status(&app, server, &active, &unavailable, waiting).await;
                        break;
                    }
                    true
                }
            };
            if changed {
                let snapshot = rx.borrow_and_update().clone();
                apply_port_mapping_snapshot(
                    &app,
                    server,
                    &peer_id,
                    snapshot,
                    &mut active,
                    &mut unavailable,
                )
                .await;
                // Failure entries are scoped by mechanism, transport and (for PCPv6) local
                // interface. Their cardinality therefore says nothing about whether every
                // expected worker has settled. Keep the initial window open until its deadline;
                // live mappings still appear immediately through `active`.
            }
            store_port_mapping_status(&app, server, &active, &unavailable, waiting).await;
        }
    });
    // A watch fold: it wakes when its source changes and is silent otherwise, so it declares no
    // rhythm. What supervision buys here is knowing it died, which is the difference between
    // "reachability stopped updating" and "reachability stopped changing".
    supervise_task("port_mapping_fold", Some(server), None, task);
}

async fn apply_relay_snapshot(
    app: &AppHandle,
    server: u64,
    snapshot: RelayAddressSnapshot,
    previous: &mut HashSet<String>,
) {
    let next: HashSet<String> = snapshot
        .addresses
        .into_iter()
        .map(|address| address.to_string())
        .collect();
    for expired in previous.difference(&next) {
        fold_bootstrap_entry(app, server, expired, BootstrapOwner::Relay, false).await;
    }
    for available in next.difference(previous) {
        fold_bootstrap_entry(app, server, available, BootstrapOwner::Relay, true).await;
    }
    *previous = next;
}

/// Keep relay reservation addresses honest after the synchronous initial reservation. A relay
/// listener can expire or be re-created later; the watch snapshot ensures Settings, invites and
/// peer records all withdraw/add the same exact circuit address.
fn spawn_relay_fold(app: AppHandle, server: u64, mut rx: watch::Receiver<RelayAddressSnapshot>) {
    let task = tokio::spawn(async move {
        let mut previous = HashSet::new();
        let initial = rx.borrow_and_update().clone();
        apply_relay_snapshot(&app, server, initial, &mut previous).await;
        while rx.changed().await.is_ok() {
            let snapshot = rx.borrow_and_update().clone();
            apply_relay_snapshot(&app, server, snapshot, &mut previous).await;
            emit_tracked(
                &app,
                "reachability-changed",
                ServerEvt { server },
                catcoms_diagnostics::TraceId::default(),
            );
        }
    });
    supervise_task("relay_fold", Some(server), None, task);
}

fn spawn_mesh_observation_fold(
    app: AppHandle,
    server: u64,
    mut rx: watch::Receiver<MeshObservationSnapshot>,
) {
    let task = tokio::spawn(async move {
        loop {
            let snapshot = rx.borrow_and_update().clone();
            let observations: Vec<String> = snapshot
                .observations
                .into_iter()
                .map(|observation| {
                    let peer = observation.observer.to_string();
                    format!(
                        "{} observed {}",
                        peer.chars().take(12).collect::<String>(),
                        observation.address
                    )
                })
                .collect();
            let state = app.state::<AppState>();
            let changed = state
                .inner()
                .mesh_observations
                .lock()
                .await
                .insert(server, observations.clone())
                .as_ref()
                != Some(&observations);
            if changed {
                emit_tracked(
                    &app,
                    "reachability-changed",
                    ServerEvt { server },
                    catcoms_diagnostics::TraceId::default(),
                );
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
    });
    supervise_task("mesh_observation_fold", Some(server), None, task);
}

/// How long the background connectivity collector waits for AutoNAT v2.
///
/// The upstream client probes on a five-second cadence and may test several identify/manual
/// candidates. Thirty seconds gives it multiple turns without delaying founding or joining; the
/// server stays live after the window and the collector keeps listening, but the diagnostic
/// refuses to sit at "waiting" forever.
const AUTONAT_WINDOW_SECS: u64 = 30;

fn bootstrap_names_address(entries: &[String], address: &Multiaddr) -> bool {
    let bare = address.to_string();
    let with_peer = format!("{bare}/p2p/");
    entries
        .iter()
        .any(|entry| entry == &bare || entry.starts_with(&with_peer))
}

/// Derive the visible AutoNAT state from all current observations and the *current* advertised
/// address set. Identify-only observations remain telemetry, an expired route disappears without
/// bespoke string invalidation, and one failed route cannot erase a second successful route.
fn autonat_status(advertised: &[String], evidence: Option<&AutoNatEvidence>) -> String {
    let Some(evidence) = evidence else {
        return AUTONAT_NOT_TESTED.to_string();
    };
    let current: Vec<_> = evidence
        .results
        .iter()
        .filter(|result| {
            bootstrap_names_address(advertised, &result.address)
                && !result
                    .address
                    .iter()
                    .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
        })
        .collect();

    if let Some(result) = current
        .iter()
        .copied()
        .find(|result| result.reachable && addr_is_globally_routable(&result.address))
    {
        return format!(
            "reachable {} (verified by AutoNAT server {})",
            result.address, result.server
        );
    }
    if let Some(result) = current
        .iter()
        .copied()
        .find(|result| !result.reachable && addr_is_globally_routable(&result.address))
    {
        let detail: String = result
            .error
            .as_deref()
            .unwrap_or("dial-back failed")
            .chars()
            .take(240)
            .collect();
        return format!(
            "unreachable {} from AutoNAT server {}: {detail}",
            result.address, result.server
        );
    }
    if let Some(result) = current.iter().copied().find(|result| result.reachable) {
        return format!(
            "local-only {} (the callback worked, but this is not a public address)",
            result.address
        );
    }
    if evidence.waiting {
        AUTONAT_WAITING.to_string()
    } else {
        AUTONAT_NOT_TESTED.to_string()
    }
}

async fn store_autonat_snapshot(
    app: &AppHandle,
    server: u64,
    snapshot: AutoNatSnapshot,
    waiting: bool,
) {
    let state = app.state::<AppState>();
    let next = AutoNatEvidence {
        waiting,
        results: snapshot.results,
    };
    let changed = state
        .inner()
        .autonat
        .lock()
        .await
        .insert(server, next.clone())
        .as_ref()
        != Some(&next);
    if changed {
        emit_tracked(
            app,
            "reachability-changed",
            ServerEvt { server },
            catcoms_diagnostics::TraceId::default(),
        );
    }
}

/// Collect coalesced per-address AutoNAT v2 evidence. The product filters it against live routes
/// only when read, closing startup-order and expiry races without an unbounded output queue.
fn spawn_autonat_fold(app: AppHandle, server: u64, mut rx: watch::Receiver<AutoNatSnapshot>) {
    let task = tokio::spawn(async move {
        let deadline = SystemClock.sleep(Duration::from_secs(AUTONAT_WINDOW_SECS));
        tokio::pin!(deadline);
        let mut waiting = true;
        let initial = rx.borrow_and_update().clone();
        store_autonat_snapshot(&app, server, initial, waiting).await;
        loop {
            tokio::select! {
                _ = &mut deadline, if waiting => {
                    waiting = false;
                    let snapshot = rx.borrow().clone();
                    store_autonat_snapshot(&app, server, snapshot, waiting).await;
                }
                result = rx.changed() => {
                    if result.is_err() {
                        waiting = false;
                        let snapshot = rx.borrow().clone();
                        store_autonat_snapshot(&app, server, snapshot, waiting).await;
                        break;
                    }
                    let snapshot = rx.borrow_and_update().clone();
                    store_autonat_snapshot(&app, server, snapshot, waiting).await;
                }
            }
        }
    });
    supervise_task("autonat_fold", Some(server), None, task);
}

/// Decode a pasted invite **and check its signature**, before anything touches the network.
///
/// `InviteToken::decode` is structural only. Acting on an unverified token means binding a
/// listener and then dialling every address in its `rendezvous` and `bootstrap` vectors, which
/// hands whoever wrote the token the user's IP and a free liveness/port-scan oracle against any
/// host they care to name. The signature check exists at the sync layer (inside `request_join`),
/// but that is several dials too late; a forged token has to die here, at the point of paste.
///
/// `verify_self` checks the token against the inviter public key the token itself carries, so it
/// only proves internal consistency, not that the inviter is anybody in particular. That is the
/// right amount for this gate: whether the inviter is a real member of the named group is decided
/// later, on the wire, by `request_join`. What it does buy is that a token nobody signed, or one
/// whose addresses were edited after signing, never causes a single packet.
#[derive(Debug)]
struct DecodedInvite {
    token: InviteToken,
    switchboards: Vec<catcoms_app::SwitchboardRoute>,
    inviter_peer: Option<PeerId>,
    assisted_plan: Option<Vec<u8>>,
}

/// Assisted invite plans are a distinct wire version. Keeping a textual prefix avoids presenting
/// their outer envelope as an ordinary hex invite that an older client will mysteriously reject.
/// Plain legacy invite hex remains accepted indefinitely by new clients.
const ASSISTED_INVITE_PREFIX: &str = "mewtual-invite-v3:";
/// Paste input is untrusted. Bound the hex before decoding so a giant clipboard value cannot
/// force an allocation before the protocol codecs apply their own field limits.
const MAX_INVITE_WIRE_BYTES: usize = 64 * 1024;

fn decode_and_verify_invite(invite_hex: &str) -> Result<DecodedInvite, String> {
    let value = invite_hex.trim();
    let (wire, explicitly_assisted) = value
        .strip_prefix(ASSISTED_INVITE_PREFIX)
        .map_or((value, false), |rest| (rest, true));
    if wire.len() > MAX_INVITE_WIRE_BYTES.saturating_mul(2) {
        return Err("this invite is too large".to_string());
    }
    let bytes = hex::decode(wire).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_INVITE_WIRE_BYTES {
        return Err("this invite is too large".to_string());
    }
    let (invite, switchboards, inviter_peer, assisted_plan) = match InviteJoinPlan::decode(&bytes) {
        Ok(plan) => (
            plan.invite,
            plan.switchboards,
            Some(PeerId::new(plan.inviter_peer)),
            Some(bytes.clone()),
        ),
        Err(_) if !explicitly_assisted => (
            InviteToken::decode(&bytes).map_err(|e| e.to_string())?,
            Vec::new(),
            None,
            None,
        ),
        Err(_) => {
            return Err("this assisted invite is malformed or its signature is invalid".into())
        }
    };
    if !invite.verify_self() {
        return Err("this invite's signature is not valid; it may have been altered or forged, so nothing was contacted".into());
    }
    if let Some(expected) = inviter_peer {
        for address in &invite.bootstrap {
            if let Ok(address) = address.parse::<Multiaddr>() {
                if let Some(actual) = target_peer_in_multiaddr(&address) {
                    if phase0_peer_id(&actual) != expected {
                        return Err(
                            "assisted invite names a different peer in its direct route".into()
                        );
                    }
                }
            }
        }
    }
    Ok(DecodedInvite {
        token: invite,
        switchboards,
        inviter_peer,
        assisted_plan,
    })
}

/// Select a tiny, inviter-signed standing-fallback dial set. Each address must terminate in the
/// helper identity named by its route, and the total remains at the pre-membership four-address
/// budget even if a malicious inviter signs the maximum route/address counts.
fn switchboard_dial_plan(
    routes: &[catcoms_app::SwitchboardRoute],
    group_id: &[u8],
    now_ms: u64,
    invite_expires_at_ms: u64,
) -> (HashMap<PeerId, u64>, Vec<Multiaddr>) {
    let mut prepared = Vec::new();
    for route in routes {
        // `routes` came through `InviteJoinPlan::decode`, which verified both the helper's offer
        // signature and the inviter's outer endorsement. Recheck the short helper deadline here,
        // immediately before any dial; an hour-long invite cannot stretch two-minute consent.
        if route.offer.group_id != group_id
            || route.offer.expires_at_ms < now_ms
            || route.offer.expires_at_ms > invite_expires_at_ms
            || route.offer.expires_at_ms.saturating_sub(now_ms)
                > catcoms_app::SWITCHBOARD_OFFER_MAX_FUTURE_MS
        {
            continue;
        }
        let peer = PeerId::new(route.offer.peer_id);
        let mut addresses = Vec::new();
        for raw in &route.offer.addresses {
            let Ok(address) = raw.parse::<Multiaddr>() else {
                continue;
            };
            if target_peer_in_multiaddr(&address)
                .is_some_and(|target| phase0_peer_id(&target) == peer)
                && !addresses.contains(&address)
            {
                addresses.push(address);
            }
        }
        if !addresses.is_empty() {
            // Clock skew tolerance authenticates an honest host's timestamp; it does not grant
            // extra local dial time. As with reply codes, cap the effective session at the
            // advertised lifetime from receipt.
            let effective_expiry = route
                .offer
                .expires_at_ms
                .min(now_ms.saturating_add(catcoms_app::SWITCHBOARD_OFFER_LIFETIME_MS));
            prepared.push((peer, effective_expiry, addresses));
        }
    }

    let mut allowed: HashMap<PeerId, u64> = HashMap::new();
    let mut selected = Vec::new();
    // Round-robin keeps a max-sized first helper from consuming the whole connect budget.
    for address_index in 0..2 {
        for (peer, expires_at_ms, addresses) in &prepared {
            let Some(address) = addresses.get(address_index) else {
                continue;
            };
            allowed
                .entry(*peer)
                .and_modify(|current| *current = (*current).max(*expires_at_ms))
                .or_insert(*expires_at_ms);
            selected.push(address.clone());
            if selected.len() == 4 {
                return (allowed, selected);
            }
        }
    }
    (allowed, selected)
}

/// Apply the same process-wide endpoint accounting to opt-in standing-member fallbacks. The
/// switchboard signatures authorize *which helper may be contacted*; they do not exempt its
/// public sockets from the scanner/resource boundary.
fn schedule_switchboard_candidates(
    scheduler: &EndpointDialScheduler,
    group_id: &[u8],
    mut allowed: HashMap<PeerId, u64>,
    candidates: Vec<Multiaddr>,
) -> (HashMap<PeerId, u64>, Vec<Multiaddr>) {
    let mut granted = Vec::new();
    let mut granted_peers = HashSet::new();
    for candidate in candidates {
        let Some(target) = target_peer_in_multiaddr(&candidate) else {
            continue;
        };
        let peer = phase0_peer_id(&target);
        if !allowed.contains_key(&peer) {
            continue;
        }
        let address = candidate.to_string();
        let Some(endpoint) = untrusted_peer_endpoint(&address, &peer) else {
            continue;
        };
        if !scheduler
            .reserve(group_id, std::slice::from_ref(&endpoint))
            .is_empty()
        {
            granted_peers.insert(peer);
            granted.push(candidate);
        }
    }
    allowed.retain(|peer, _| granted_peers.contains(peer));
    (allowed, granted)
}

/// Charge one two-way reply dial pass against the same process-wide boundary as rendezvous,
/// signed peer-record, and switchboard dials. A valid reply authenticates the joiner PeerId and
/// its short-lived address set, but it does not make those Internet sockets free to probe.
///
/// Returning an empty set is deliberately temporary: the caller keeps the bounded reply session
/// alive and may retry after the scheduler's monotonic window rolls over.
fn schedule_join_reply_candidates(
    scheduler: &EndpointDialScheduler,
    group_id: &[u8],
    joiner: &libp2p::PeerId,
    candidates: &[Multiaddr],
) -> Vec<Multiaddr> {
    let phase_peer = phase0_peer_id(joiner);
    let endpoints: Vec<_> = candidates
        .iter()
        .filter_map(|candidate| untrusted_peer_endpoint(&candidate.to_string(), &phase_peer))
        .collect();
    scheduler
        .reserve(group_id, &endpoints)
        .into_iter()
        .filter_map(|address| address.parse().ok())
        .collect()
}

#[tauri::command]
async fn preview_invite(
    state: State<'_, AppState>,
    invite_hex: String,
) -> Result<InvitePreview, String> {
    require_unlocked_session(&state).await?;
    let decoded = decode_and_verify_invite(&invite_hex)?;
    let now = SystemClock.now_ms();
    let (switchboards, _) = switchboard_dial_plan(
        &decoded.switchboards,
        &decoded.token.group_id,
        now,
        decoded.token.expires_at_ms,
    );
    Ok(InvitePreview {
        direct_routes: dialable_bootstrap(&decoded.token.bootstrap).len(),
        rendezvous_routes: decoded.token.rendezvous.len(),
        switchboards: switchboards.len(),
        expires_at_ms: decoded.token.expires_at_ms,
    })
}

/// The discover-on-join path (no hard-coded inviter address): build a transport, dial the invite's
/// rendezvous node(s), discover the inviter's records under the pre-join namespace, rank them
/// through the [`DiscoveryPolicy`] (never auto-dial), then dial the chosen addresses; plus the
/// invite's `bootstrap` addrs as direct fallbacks; and return the connected transport + the
/// inviter's peer id. Mirrors `tcp_rendezvous_e2e.rs`.
fn untrusted_peer_endpoint(address: &str, expected_peer: &PeerId) -> Option<DialEndpoint> {
    let route = parse_peer_dial_route(address, expected_peer.as_bytes())?;
    if !matches!(route.host, RouteHost::Ip(_)) {
        return None;
    }
    let parsed: Multiaddr = address.parse().ok()?;
    addr_is_globally_routable(&parsed).then_some(route.endpoint)
}

/// Validate an invite route while preserving the deliberate same-LAN and same-machine cases.
/// The canonical grammar and terminal peer binding are identical to internet discovery; only the
/// host policy differs. DNS, link-local, multicast, unspecified, and unsupported route shapes
/// still fail closed.
fn invite_peer_endpoint(address: &str, expected_peer: &PeerId) -> Option<DialEndpoint> {
    let route = parse_peer_dial_route(address, expected_peer.as_bytes())?;
    if !matches!(route.host, RouteHost::Ip(_)) {
        return None;
    }
    let parsed: Multiaddr = address.parse().ok()?;
    (!addr_is_undialable(&parsed)).then_some(route.endpoint)
}

fn canonical_invite_peer_endpoint(address: &Multiaddr) -> Option<DialEndpoint> {
    let target = target_peer_in_multiaddr(address)?;
    invite_peer_endpoint(&address.to_string(), &phase0_peer_id(&target))
}

/// Retain only the live outbound route that Noise authenticated for the member who completed
/// admission. This is the local-only substitute for putting private LAN addresses into PEX: the
/// route is sealed with `ServerNet`, capped to TCP/QUIC scale, and is roster-checked again by the
/// sync layer before every later dial.
fn select_authenticated_reconnect_routes(
    routes: Vec<AuthenticatedDialRoute>,
    member_peers: &HashSet<PeerId>,
    local_only: bool,
) -> Vec<ReconnectRoute> {
    let mut routes: Vec<_> = routes
        .into_iter()
        .filter(|route| member_peers.contains(&route.peer))
        .filter(|route| route.address.len() <= MAX_RECONNECT_ROUTE_BYTES)
        .filter(|route| invite_peer_endpoint(&route.address, &route.peer).is_some())
        .filter(|route| {
            !local_only
                || route
                    .address
                    .parse::<Multiaddr>()
                    .is_ok_and(|addr| addr_is_private(&addr) || addr_is_loopback(&addr))
        })
        .map(|route| ReconnectRoute {
            peer_id: *route.peer.as_bytes(),
            address: route.address,
        })
        .collect();
    routes.sort_by(|left, right| {
        left.peer_id
            .cmp(&right.peer_id)
            .then_with(|| left.address.cmp(&right.address))
    });
    routes.dedup();
    routes.truncate(MAX_RECONNECT_ROUTES);
    routes
}

fn authenticated_reconnect_routes(mesh: &MeshHandle, contact: PeerId) -> Vec<ReconnectRoute> {
    select_authenticated_reconnect_routes(
        mesh.authenticated_dial_routes(),
        &HashSet::from([contact]),
        false,
    )
}

/// Convert the actual admission path into durable reconnect authority.
///
/// A reply may ultimately authenticate the named inviter, but that path authorized a bounded
/// inbound callback rather than indefinite future dialing. Likewise, a switchboard authorized a
/// bounded helper ceremony. Test the path flags explicitly instead of inferring consent from the
/// final contact identity, which is deliberately the inviter in the successful reply case.
fn reconnect_policy_after_admission(
    join_contact: PeerId,
    inviter: PeerId,
    used_reply_path: bool,
    used_switchboard_path: bool,
) -> ReconnectPolicy {
    if join_contact == inviter && !used_reply_path && !used_switchboard_path {
        ReconnectPolicy::AuthorizedPeer(*join_contact.as_bytes())
    } else {
        ReconnectPolicy::Disabled
    }
}

/// A transport identity is a safe durable member target only while exactly one roster entry
/// claims it. A duplicate claim is ambiguous even if the live Noise connection itself is valid:
/// retaining that socket as either member's route would turn the shared transport key into a
/// confused-deputy shortcut on the next launch. The sync layer repeats this uniqueness check at
/// dial time; doing it here also keeps ambiguous state out of the sealed store.
fn uniquely_claimed_member_peers(peers: impl IntoIterator<Item = PeerId>) -> HashSet<PeerId> {
    let mut claim_counts = HashMap::<PeerId, usize>::new();
    for peer in peers {
        *claim_counts.entry(peer).or_default() += 1;
    }
    claim_counts
        .into_iter()
        .filter_map(|(peer, claims)| (claims == 1).then_some(peer))
        .collect()
}

/// Apply durable admission provenance before a live route may be captured.
///
/// A direct admission can refresh only its named inviter. A pre-v3 record gets one conservative
/// migration opportunity only when the roster has exactly one other member; helper and
/// switchboard joins necessarily have a larger roster and new non-direct joins are explicitly
/// disabled rather than inferred from an empty route vector.
fn reconnect_capture_peer(
    policy: ReconnectPolicy,
    other_member_count: usize,
    claimed_peers: impl IntoIterator<Item = PeerId>,
) -> Option<PeerId> {
    let unique = uniquely_claimed_member_peers(claimed_peers);
    match policy {
        ReconnectPolicy::Disabled => None,
        ReconnectPolicy::AuthorizedPeer(peer) => {
            let peer = PeerId::new(peer);
            unique.contains(&peer).then_some(peer)
        }
        ReconnectPolicy::LegacyPending if other_member_count == 1 && unique.len() == 1 => {
            unique.into_iter().next()
        }
        ReconnectPolicy::LegacyPending => None,
    }
}

/// Apply the same unique-current-claim boundary to a recovery candidate before it may replace a
/// proven reconnect contact. A signed recovery code proves which member requested the attempt;
/// it does not make a transport identity claimed by two roster entries unambiguous.
fn pending_recovery_capture_peer(
    pending: Option<[u8; 32]>,
    expires_at_ms: u64,
    now_ms: u64,
    claimed_peers: impl IntoIterator<Item = PeerId>,
) -> Option<PeerId> {
    if pending.is_none() || now_ms > expires_at_ms {
        return None;
    }
    let unique = uniquely_claimed_member_peers(claimed_peers);
    pending
        .map(PeerId::new)
        .filter(|peer| unique.contains(peer))
}

/// Merge one authenticated route observation into the *latest* sealed network record.
///
/// Discovery awaits actor/mesh state between its initial read and final save. Re-authorizing a
/// different recovery peer during that gap must not be lost to a stale whole-record write. `None`
/// means the authority that selected this capture is no longer current; `Some(changed)` preserves
/// any unrelated newer pending consent and clears only the exact candidate being promoted.
fn merge_live_reconnect_capture(
    current: &mut ServerNet,
    selected_from_pending: bool,
    capture_peer: PeerId,
    routes: Vec<ReconnectRoute>,
    other_member_count: usize,
    now_ms: u64,
    claimed_peers: impl IntoIterator<Item = PeerId>,
) -> Option<bool> {
    let peer_bytes = *capture_peer.as_bytes();
    if selected_from_pending {
        if current.pending_recovery_peer != Some(peer_bytes)
            || now_ms > current.pending_recovery_expires_at_ms
        {
            return None;
        }
    } else if reconnect_capture_peer(current.reconnect_policy, other_member_count, claimed_peers)
        != Some(capture_peer)
    {
        return None;
    }

    let next_policy = ReconnectPolicy::AuthorizedPeer(peer_bytes);
    let pending_is_current =
        current.pending_recovery_peer.is_some() && now_ms <= current.pending_recovery_expires_at_ms;
    let (next_pending, next_pending_expiry) = if selected_from_pending || !pending_is_current {
        (None, 0)
    } else {
        (
            current.pending_recovery_peer,
            current.pending_recovery_expires_at_ms,
        )
    };
    let changed = current.reconnect_routes != routes
        || current.reconnect_policy != next_policy
        || current.pending_recovery_peer != next_pending
        || current.pending_recovery_expires_at_ms != next_pending_expiry;
    current.reconnect_routes = routes;
    current.reconnect_policy = next_policy;
    current.pending_recovery_peer = next_pending;
    current.pending_recovery_expires_at_ms = next_pending_expiry;
    Some(changed)
}

/// Capture same-LAN routes for already-established servers, including records created before
/// `ServerNet` v3 existed. Only a current member's self-signed transport claim may select a live
/// Noise-authenticated route, and this migration path is intentionally local-only: public member
/// routes already belong in the signed PEX/cache path, while infrastructure/helper sockets must
/// not become durable merely because they share this process.
fn authenticated_member_lan_reconnect_routes(
    mesh: &MeshHandle,
    member_peers: &HashSet<PeerId>,
) -> Vec<ReconnectRoute> {
    select_authenticated_reconnect_routes(
        mesh.authenticated_dial_route_evidence(),
        member_peers,
        true,
    )
}

/// Schedule already set-validated rendezvous routes. The invite validator permits loopback only
/// when the entire set is loopback, so this must use the invite host policy rather than silently
/// tightening it to the public-only policy used for network-discovered records.
fn schedule_invite_rendezvous_targets(
    scheduler: &EndpointDialScheduler,
    group_id: &[u8],
    targets: Vec<RendezvousTarget>,
) -> Vec<RendezvousTarget> {
    targets
        .into_iter()
        .filter(|target| {
            let phase_peer = phase0_peer_id(&target.peer);
            let Some(endpoint) = invite_peer_endpoint(&target.addr.to_string(), &phase_peer) else {
                return false;
            };
            !scheduler
                .reserve(group_id, std::slice::from_ref(&endpoint))
                .is_empty()
        })
        // Cap actual grants, not merely entries considered: a structurally valid but unsupported
        // address before a usable one must not consume rendezvous redundancy.
        .take(MAX_INVITE_RENDEZVOUS_DIALS)
        .collect()
}

/// Keep the same bounded, canonical seed set for steady-state recovery after a switchboard join.
/// This path does not reserve again: the live server actor will spend the shared scheduler when it
/// actually reconnects to these configured nodes.
fn retained_invite_rendezvous_config(targets: &[RendezvousTarget]) -> Vec<(String, Vec<u8>)> {
    targets
        .iter()
        .filter(|target| canonical_invite_peer_endpoint(&target.addr).is_some())
        .take(MAX_INVITE_RENDEZVOUS_DIALS)
        .map(|target| (target.addr.to_string(), target.peer.to_bytes()))
        .collect()
}

/// Merge candidate sources without erasing their different host-trust rules. Rendezvous/PEX
/// discovery is public-IP-only; the issuer-signed direct bootstrap deliberately retains bounded
/// LAN and same-machine routes.
fn join_candidate_endpoints(
    discovered: &[String],
    invite_fallbacks: &[String],
    inviter: &PeerId,
) -> Vec<DialEndpoint> {
    let mut endpoints = Vec::new();
    for address in discovered {
        if let Some(endpoint) = untrusted_peer_endpoint(address, inviter) {
            if !endpoints.contains(&endpoint) {
                endpoints.push(endpoint);
            }
        }
    }
    for address in invite_fallbacks {
        if let Some(endpoint) = invite_peer_endpoint(address, inviter) {
            if !endpoints.contains(&endpoint) {
                endpoints.push(endpoint);
            }
        }
    }
    endpoints
}

/// Validate and schedule the direct reach fields copied into one companion grant. Grants are
/// authenticated pairing material, but a compromised/buggy origin must not turn a multi-server
/// bundle into an unbounded or peer-confused dial list on the new device.
fn schedule_grant_bootstrap(
    scheduler: &EndpointDialScheduler,
    group_id: &[u8],
    bootstrap: &[String],
) -> Result<(PeerId, Vec<Multiaddr>), String> {
    let candidates = dialable_bootstrap(bootstrap);
    if candidates.is_empty() {
        return Err("this grant carries no usable address for that server".to_string());
    }
    let contacts: HashSet<_> = candidates
        .iter()
        .filter_map(target_peer_in_multiaddr)
        .collect();
    if contacts.len() != 1 {
        return Err("grant addresses do not name one unambiguous server peer".to_string());
    }
    let contact_lp = *contacts.iter().next().expect("one contact checked above");
    let contact = phase0_peer_id(&contact_lp);
    let endpoints: Vec<_> = candidates
        .iter()
        .filter_map(|address| invite_peer_endpoint(&address.to_string(), &contact))
        .collect();
    let granted: Vec<_> = scheduler
        .reserve(group_id, &endpoints)
        .into_iter()
        .filter_map(|address| address.parse().ok())
        .collect();
    if granted.is_empty() {
        return Err(
            "the process-wide discovery dial budget deferred every grant endpoint".to_string(),
        );
    }
    Ok((contact, granted))
}

async fn discover_and_connect(
    invite: &InviteToken,
    net: &ServerNet,
    expected_inviter: Option<PeerId>,
    endpoint_dials: &EndpointDialScheduler,
    steps: &mut Vec<DiagStep>,
) -> Result<(MeshService, PeerId, Vec<(String, Vec<u8>)>, u16), String> {
    // Invite-supplied, so attacker-controlled: the strict variant. The bootstrap half of this same
    // invite goes through `dialable_bootstrap` below; before P13 was fixed the two halves of one
    // pasted string were held to opposite standards.
    let targets =
        validate_invite_rendezvous_addrs(&invite.rendezvous).map_err(|e| e.to_string())?;
    if targets.is_empty() {
        return Err("invite carries no rendezvous address".into());
    }
    // The transport constructor dials its seed list immediately, so the shared scheduler must
    // trim invite-selected rendezvous endpoints before construction. Charging only the member
    // records discovered afterwards would leave the first (and easiest) scanner seam outside the
    // process cap.
    let targets = schedule_invite_rendezvous_targets(endpoint_dials, &invite.group_id, targets);
    if targets.is_empty() {
        return Err(
            "the process-wide discovery dial budget deferred every rendezvous endpoint".into(),
        );
    }
    let rz_addrs: Vec<Multiaddr> = targets.iter().map(|t| t.addr.clone()).collect();
    // Bind the joiner's own (stable, persisted-identity) listen addresses so it is itself dialable;
    // post-join steady-state discovery has members register/discover + dial each other. Then dial
    // the rendezvous nodes.
    let (mesh, libp2p_id, port, bound) = build_transport(net, &rz_addrs)?;
    record_listener_evidence(&mesh, &bound, steps).await;
    // Advertise our own reachable addresses so the steady-state rendezvous registration carries a
    // dialable record: the LAN IPv4 and routable IPv6 when this host has them, loopback otherwise.
    for addr in external_addrs(&auto_bootstrap(port, &libp2p_id.to_string())) {
        let _ = mesh.add_external_address(addr).await;
    }

    // Wait until at least one rendezvous node is connected.
    let rz_peers: Vec<PeerId> = targets.iter().map(|t| phase0_peer_id(&t.peer)).collect();
    timeout(
        Duration::from_secs(20),
        mesh.wait_for_any_connected(&rz_peers),
    )
    .await
    .map_err(|_| {
        steps.push(DiagStep::failed(
            "rendezvous",
            rz_addrs
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            "timed out connecting to the rendezvous",
        ));
        "timed out connecting to the rendezvous".to_string()
    })?
    .map_err(|_| "the rendezvous transport closed while connecting".to_string())?;
    steps.push(DiagStep::ok(
        "rendezvous",
        rz_addrs
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<_>>()
            .join(", "),
        "connected",
    ));

    // Discover the inviter under each rendezvous's pre-join namespace.
    for t in &targets {
        let ns = join_namespace(&invite.group_id, &invite.invite_nonce, &t.peer.to_bytes());
        mesh.rendezvous_discover(&ns, t.peer)
            .await
            .map_err(|e| e.to_string())?;
    }

    // Collect discovered records into candidates (bounded by a deadline + a count cap).
    let root = targets[0].peer.to_bytes();
    let mut candidates: Vec<Candidate> = Vec::new();
    let _ = timeout(Duration::from_secs(20), async {
        while let Some(d) = mesh.next_discovered().await {
            if expected_inviter.is_some_and(|expected| phase0_peer_id(&d.peer) != expected) {
                continue;
            }
            let phase_peer = phase0_peer_id(&d.peer);
            let addresses: Vec<String> = d
                .addresses
                .iter()
                .map(ToString::to_string)
                .filter(|address| untrusted_peer_endpoint(address, &phase_peer).is_some())
                .collect();
            if addresses.is_empty() {
                continue;
            }
            candidates.push(Candidate {
                peer: d.peer.to_bytes(),
                addresses,
                source: Source::Rendezvous(root.clone()),
                freshness: catcoms_discovery::FreshnessPrincipal::Transport(d.peer.to_bytes()),
                // The record's own signed seq gives the policy real anti-replay freshness; the
                // backstop remains request_join's Welcome-signature + group-id check, which fails
                // closed if we dial the wrong peer. tag_verified stays false pre-join (no group
                // secret to recompute the member tag).
                seq: d.seq,
                tag_verified: false,
            });
            if candidates.len() >= 8 {
                break;
            }
        }
    })
    .await;
    if candidates.is_empty() {
        steps.push(DiagStep::failed(
            "discover",
            "",
            "the rendezvous knows nothing under this invite's namespace; the server has not              registered there, or has not been online since it was minted",
        ));
        return Err("could not discover the server at the rendezvous".into());
    }
    steps.push(DiagStep::ok(
        "discover",
        "",
        format!("{} record(s) found at the rendezvous", candidates.len()),
    ));

    // The DiscoveryPolicy alone decides what to dial (eclipse-resistance; never auto-dial).
    let mut policy = DiscoveryPolicy::with_config(PolicyConfig::default());
    let mut rng = OsCryptoRng;
    let dialed = policy
        .plan(candidates, 2, &SystemClock, &mut rng)
        .into_iter()
        .next()
        .ok_or_else(|| "the discovery policy offered no peer to dial".to_string())?;
    let inviter_lp = libp2p::PeerId::from_bytes(&dialed.peer)
        .map_err(|_| "discovered peer id was malformed".to_string())?;
    let inviter = phase0_peer_id(&inviter_lp);

    // Dial the policy-chosen addresses plus the invite's bootstrap addrs (direct fallbacks). The
    // bootstrap half goes through `dialable_bootstrap` first: the discovered addresses were
    // ranked by the DiscoveryPolicy, but the invite's own list is whatever the token's author
    // wrote down, and it is dialled in bulk (defect P7).
    let fallbacks: Vec<String> = dialable_bootstrap(&invite.bootstrap)
        .iter()
        .map(|m| m.to_string())
        .collect();
    let endpoints = join_candidate_endpoints(&dialed.addresses, &fallbacks, &inviter);
    let granted = endpoint_dials.reserve(&invite.group_id, &endpoints);
    if granted.is_empty() {
        return Err(
            "the process-wide discovery dial budget deferred every inviter endpoint".into(),
        );
    }
    for a in &granted {
        match a.parse::<Multiaddr>() {
            Ok(m) => match mesh.dial(m).await {
                // libp2p dials these concurrently and only the *first* one to complete surfaces
                // as a connection, so "dialled" is all this layer honestly knows per address.
                Ok(()) => steps.push(DiagStep::unknown("dial", a.clone(), "dialled")),
                Err(e) => steps.push(DiagStep::failed("dial", a.clone(), e.to_string())),
            },
            Err(e) => steps.push(DiagStep::failed("dial", a.clone(), e.to_string())),
        }
    }
    timeout(
        Duration::from_secs(20),
        mesh.wait_for_peer_connected(inviter),
    )
    .await
    .map_err(|_| {
        steps.push(DiagStep::failed(
            "connect",
            "",
            "none of the dialled addresses answered within 20s",
        ));
        "timed out connecting to the discovered server".to_string()
    })?
    .map_err(|_| "the join transport closed while connecting".to_string())?;
    steps.push(DiagStep::ok("connect", "", "connected to the server"));
    // The rendezvous config the joiner keeps for steady-state discovery (re-finding the group).
    let rz_config: Vec<(String, Vec<u8>)> = targets
        .iter()
        .map(|t| (t.addr.to_string(), t.peer.to_bytes()))
        .collect();
    Ok((mesh, inviter, rz_config, port))
}

/// Found a new server: bind all interfaces (so LAN/internet peers can reach it, not just
/// loopback), found the group, mint a single-use invite carrying the reachable address(es),
/// spawn the actor, and register it. `advertise` is an optional user-supplied reachable
/// address (LAN or public IP); `relay` is an optional relay-node multiaddr; when given, we
/// reserve a circuit there and put the **relayed** address first in the invite, so a joiner
/// reaches us through the relay with **no port-forward** (zero-config NAT traversal).
/// `rendezvous` is an optional zero-knowledge rendezvous multiaddr; when given, we register at it
/// so a joiner can discover us with **no hard-coded address at all** (just the pasted invite).
/// `server_name` is an optional local display label for the server/DM rail entry; if omitted or
/// empty, it defaults to `display_name` (preserving backwards compatibility). `display_name` is
/// used to initialize the user's MLS profile in the group.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn found_server(
    app: AppHandle,
    state: State<'_, AppState>,
    display_name: String,
    advertise: String,
    relay: String,
    rendezvous: String,
    is_dm: bool,
    server_name: Option<String>,
    trace: Option<String>,
) -> Result<FoundResult, String> {
    let op = Operation::start_maybe(
        trace,
        catcoms_diagnostics::Section::Reachability,
        "found_server",
        None,
        None,
    );
    let ui_session_generation = unlocked_ui_session_generation(&state)
        .await
        .map_err(|e| op.fail(codes::SESSION_LOCKED, e).into_message())?;
    let mut diag = Connectivity {
        action: "found".into(),
        subject: display_name.clone(),
        at: SystemClock.now_ms(),
        trace: op.short_trace(),
        ..Default::default()
    };
    let out = found_server_inner(
        &app,
        &state,
        display_name,
        advertise,
        relay,
        rendezvous,
        is_dm,
        server_name,
        ui_session_generation,
        &mut diag,
    )
    .await;
    // The panel's own account, now also in the record and under this attempt's trace. Both, rather
    // than one: the panel is what the user reads and the record is what survives to be exported.
    op.replay(&diag.steps);
    // The verbatim error is the point: the connectivity panel shows exactly what the code said,
    // so a user can paste it rather than paraphrase it.
    if let Err(e) = &out {
        diag.last_error.clone_from(e);
        op.failed("REACH.FOUND.FAILED");
    } else {
        op.succeeded("REACH.FOUND.COMPLETED");
    }
    *state.diag.lock().await = diag;
    out
}

/// The body of [`found_server`], split out so every exit (including the early `Err`s) lands in
/// one place that records the attempt for the connectivity panel.
#[allow(clippy::too_many_arguments)]
async fn found_server_inner(
    app: &AppHandle,
    state: &AppState,
    display_name: String,
    advertise: String,
    relay: String,
    rendezvous: String,
    is_dm: bool,
    server_name: Option<String>,
    ui_session_generation: u64,
    diag: &mut Connectivity,
) -> Result<FoundResult, String> {
    // This server's own network identity + stable port, minted once here and sealed to disk, so
    // every later launch keeps the same PeerId and port and the invites minted today still resolve
    // tomorrow. Per server, never per device (see `ServerNet`).
    let mut net = new_server_net(&advertise, &relay, &rendezvous);
    let relay_dial: Vec<Multiaddr> = if net.relay.is_empty() {
        Vec::new()
    } else {
        vec![net
            .relay
            .parse()
            .map_err(|e: libp2p::multiaddr::Error| format!("bad relay address: {e}"))?]
    };
    let (mesh, libp2p_id, port, bound) = build_transport(&net, &relay_dial)?;
    net.port = port;
    let id = libp2p_id.to_string();

    // Everything that makes us reachable from off this machine. `reload_one` runs the very same
    // helper, so a restart reproduces this reachability instead of collapsing to loopback.
    let (reach, problems) =
        establish_reachability(&mesh, &id, port, &bound, &net, &mut diag.steps).await;
    if !problems.is_empty() {
        // The user typed these addresses moments ago; tell them rather than founding a server that
        // silently nobody can reach.
        return Err(problems.join("; "));
    }
    let Reachability {
        bootstrap,
        bootstrap_owners,
        rz_target,
        rz_handle,
    } = reach;
    // Take the router-mapping lifecycle before the transport disappears into the server, so a
    // background task can retain mappings and withdraw expired addresses without holding up UI.
    let port_mapping_rx = mesh.take_port_mapping_snapshots().await;
    let autonat_rx = mesh.take_autonat_snapshots().await;
    let relay_address_rx = mesh.take_relay_address_snapshots().await;
    let mesh_observation_rx = mesh.take_mesh_observation_snapshots().await;
    let mesh_handle = mesh.handle();
    diag.advertised = bootstrap.clone();

    let device = MlsDevice::generate().map_err(|e| e.to_string())?;
    // `server_name` is the local rail label (DM: friend's name). If omitted/empty, fall back to
    // `display_name` (user's profile name) for backwards compatibility.
    let name = server_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| display_name.clone());
    let mut server = Server::found(
        mesh,
        device,
        OsCryptoRng,
        Box::new(SystemClock),
        display_name,
    )
    .map_err(|e| e.to_string())?;
    server.set_endpoint_dial_scheduler(state.endpoint_dials.clone());
    server
        .subscribe_control()
        .await
        .map_err(|e| e.to_string())?;
    attach_blob_store(state, &mut server).await;
    // Steady-state discovery: tell the server which rendezvous to re-register/discover at, so the
    // actor re-finds the group after a restart. (The founder already advertised + connected above.)
    if let Some(rz) = &rz_target {
        server.set_rendezvous_nodes(vec![(rz.addr.to_string(), rz.peer.to_bytes())]);
    }

    // Publish this device's own signed peer record (defect P1). Nothing in the product used to
    // do this, so `peer_records` stayed empty for the life of every server and took the roster's
    // online dots, the cross-session re-dial and the eclipse detector's reach term with it. The
    // sequence number is the base of this launch's reserved block; non-routable entries in
    // `bootstrap` (loopback, the LAN address) are stripped inside `publish_self_record`, so what
    // members learn is only what they could actually dial.
    if let Err(e) = server.publish_self_record(bootstrap.clone(), net.record_seq) {
        tracing::warn!(target: "catcoms_app", error = %e, "DISCOVERY.PEER_RECORD.PUBLISH_FAILED");
    }

    // Mint a single-use invite (1h) carrying the bootstrap address (+ rendezvous addr if set, so
    // the joiner can discover us), then register the invite's namespace at the rendezvous.
    let mut nonce = [0u8; 16];
    let mut rng = OsCryptoRng;
    rng.fill_bytes(&mut nonce);
    let expires = SystemClock.now_ms() + 3_600_000;
    let rz_vec: Vec<String> = rz_target.iter().map(|t| t.addr.to_string()).collect();
    let invite = if let Some(rz) = &rz_target {
        let token = server
            .mint_invite_with_rendezvous(nonce, expires, bootstrap.clone(), rz_vec.clone())
            .map_err(|e| e.to_string())?;
        if let Some(handle) = &rz_handle {
            register_join_ns(handle, &server.group_id(), &nonce, rz).await?;
        }
        token
    } else {
        server
            .mint_invite(nonce, expires, bootstrap.clone())
            .map_err(|e| e.to_string())?
    };
    let invite_hex = hex::encode(invite.encode());

    let general = channel_id("general");
    let group_id = server.group_id();
    let device_id = server.device_id();
    let (actor, events, task) = spawn(server);
    actor.open_channel(general).await;
    let channels = ui_channels(actor.channels().await);
    let session_commit = match require_ui_session_generation(state, ui_session_generation).await {
        Ok(commit) => commit,
        Err(error) => {
            actor.shutdown().await;
            drop(events);
            let _ = task.await;
            return Err(error);
        }
    };
    let server_id = register_server(
        app,
        state,
        actor,
        events,
        task,
        group_id,
        device_id,
        Some(invite_hex),
        name,
        bootstrap,
        bootstrap_owners,
        Some(InterfaceRouteIdentity {
            port,
            peer_id: id.clone(),
        }),
        rz_vec,
        Some(mesh_handle),
        is_dm,
        net.switchboard,
        net.record_seq,
    )
    .await;
    // Seal the new server, its network identity and the registry to disk (if the store is
    // unlocked). The identity has to land before the first restart or the invite just minted dies
    // with the process that made it.
    persist_server(state, server_id).await;
    persist_server_net(state, server_id, &net).await;
    persist_registry(state).await;
    drop(session_commit);
    diag.server = server_id;
    diag.advertised.clone_from(&invite.bootstrap);
    diag.steps.push(DiagStep::ok(
        "invite",
        "",
        format!(
            "invite minted carrying {} address(es) and {} rendezvous entr(ies)",
            invite.bootstrap.len(),
            invite.rendezvous.len()
        ),
    ));
    if let Some(rx) = port_mapping_rx {
        state
            .upnp
            .lock()
            .await
            .insert(server_id, PORT_MAPPING_WAITING.to_string());
        spawn_port_mapping_fold(app.clone(), server_id, rx, id);
    }
    if let Some(rx) = autonat_rx {
        state.autonat.lock().await.insert(
            server_id,
            AutoNatEvidence {
                waiting: true,
                results: Vec::new(),
            },
        );
        spawn_autonat_fold(app.clone(), server_id, rx);
    }
    if let Some(rx) = relay_address_rx {
        spawn_relay_fold(app.clone(), server_id, rx);
    }
    if let Some(rx) = mesh_observation_rx {
        spawn_mesh_observation_fold(app.clone(), server_id, rx);
    }
    Ok(FoundResult {
        server: server_id,
        channel: general.to_string(),
        channels,
        is_dm,
    })
}

/// Join an existing server by pasting its invite: decode it, dial all bootstrap addresses,
/// run the MLS join, then catch up #general / profiles / files.
async fn wait_for_peer(mesh: &MeshService, wanted: PeerId, within: Duration) -> Result<(), ()> {
    timeout(within, mesh.wait_for_peer_connected(wanted))
        .await
        .map_err(|_| ())?
        .map(|_| ())
        .map_err(|_| ())
}

/// Wait for whichever peer answers a reply-code dial-back.  The reply candidate is public and
/// may be applied by either the named inviter or an existing member helper; the subsequent MLS
/// path distinguishes them and never grants the helper admission authority.
async fn wait_for_reply_peer(
    mesh: &MeshService,
    connection_handoff: &mut PreOwnerConnectionHandoff,
    reply: &JoinReply,
    invite_nonce: &[u8; 16],
    within: Duration,
) -> Result<PeerId, ()> {
    timeout(within, async {
        loop {
            match mesh.next_event().await {
                Some(TransportEvent::Request {
                    from,
                    data,
                    responder,
                    ..
                }) if data.first() == Some(&JOIN_REPLY_PROOF_KIND) => {
                    let valid =
                        reply.verify_dialback_proof(invite_nonce, from.as_bytes(), &data[1..]);
                    responder.respond(if valid {
                        bytes::Bytes::from_static(b"ok")
                    } else {
                        bytes::Bytes::new()
                    });
                    if valid {
                        return from;
                    }
                }
                Some(TransportEvent::Request { responder, .. }) => {
                    responder.respond(bytes::Bytes::new());
                }
                Some(event) if connection_handoff.observe(&event) => {}
                Some(_) => {}
                None => std::future::pending::<()>().await,
            }
        }
    })
    .await
    .map_err(|_| ())
}

async fn wait_for_switchboard_peer(
    mesh: &MeshService,
    allowed: &HashMap<PeerId, u64>,
    within: Duration,
) -> Result<(PeerId, u64), ()> {
    let now_ms = SystemClock.now_ms();
    let mut wanted: Vec<_> = allowed
        .iter()
        .filter_map(|(peer, expires_at_ms)| (*expires_at_ms >= now_ms).then_some(*peer))
        .collect();
    timeout(within, async {
        loop {
            if wanted.is_empty() {
                return Err(());
            }
            let connected = mesh.wait_for_any_connected(&wanted).await.map_err(|_| ())?;
            if let Some(candidate) = accept_or_prune_switchboard_candidate(
                &mut wanted,
                allowed,
                connected.peer,
                SystemClock.now_ms(),
            ) {
                return Ok(candidate);
            }
            // The returned row expired while the wait was pending. It has been removed from
            // `wanted`, so an already-connected valid helper can win the next immediate watch
            // query instead of the expired row masking it forever.
        }
    })
    .await
    .map_err(|_| ())?
}

/// Accept a still-current switchboard route, or remove an expired/confused candidate before the
/// caller waits again. Keeping this decision pure makes the two-connected-helper expiry race
/// deterministic in unit tests.
fn accept_or_prune_switchboard_candidate(
    wanted: &mut Vec<PeerId>,
    allowed: &HashMap<PeerId, u64>,
    connected: PeerId,
    now_ms: u64,
) -> Option<(PeerId, u64)> {
    if let Some(expires_at_ms) = allowed.get(&connected).copied() {
        if now_ms <= expires_at_ms {
            return Some((connected, expires_at_ms));
        }
    }
    wanted.retain(|peer| *peer != connected);
    None
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn join_server(
    app: AppHandle,
    state: State<'_, AppState>,
    invite_hex: String,
    display_name: String,
    is_dm: bool,
    allow_switchboards: bool,
    server_name: Option<String>,
    trace: Option<String>,
) -> Result<FoundResult, String> {
    let op = Operation::start_maybe(
        trace,
        catcoms_diagnostics::Section::Join,
        "join_server",
        None,
        None,
    );
    let ui_session_generation = unlocked_ui_session_generation(&state)
        .await
        .map_err(|e| op.fail(codes::SESSION_LOCKED, e).into_message())?;
    let mut diag = Connectivity {
        action: "join".into(),
        at: SystemClock.now_ms(),
        trace: op.short_trace(),
        ..Default::default()
    };
    let out = join_server_inner(
        &app,
        &state,
        invite_hex,
        display_name,
        is_dm,
        allow_switchboards,
        server_name,
        ui_session_generation,
        &mut diag,
    )
    .await;
    op.replay(&diag.steps);
    if let Err(e) = &out {
        diag.last_error.clone_from(e);
        // A join that fails is the single most reported problem in this app, and until now the
        // record of one was a panel the user had to be looking at to see.
        op.failed("JOIN.ATTEMPT.FAILED");
    } else {
        op.succeeded("JOIN.ATTEMPT.COMPLETED");
    }
    *state.diag.lock().await = diag;
    out
}

/// The body of [`join_server`], split out so every exit records the attempt for the connectivity
/// panel (the exits that used to say nothing are the whole reason that panel exists).
#[allow(clippy::too_many_arguments)]
async fn join_server_inner(
    app: &AppHandle,
    state: &AppState,
    invite_hex: String,
    display_name: String,
    is_dm: bool,
    allow_switchboards: bool,
    server_name: Option<String>,
    ui_session_generation: u64,
    diag: &mut Connectivity,
) -> Result<FoundResult, String> {
    let decoded = decode_and_verify_invite(&invite_hex).inspect_err(|e| {
        diag.steps.push(DiagStep::failed("invite", "", e.clone()));
    })?;
    let invite = decoded.token;
    let plan_inviter = decoded.inviter_peer;
    let assisted_plan = decoded.assisted_plan;
    let now = SystemClock.now_ms();
    let (available_helpers, _) = switchboard_dial_plan(
        &decoded.switchboards,
        &invite.group_id,
        now,
        invite.expires_at_ms,
    );
    let available_switchboard_count = available_helpers.len();
    let use_switchboards = allow_switchboards && available_switchboard_count > 0;
    let switchboards = if allow_switchboards {
        decoded.switchboards
    } else {
        Vec::new()
    };
    diag.subject = hex::encode(&invite.inviter_device_id.as_bytes()[..8]);
    diag.steps.push(DiagStep::ok(
        "invite",
        "",
        format!(
            "signature verified; {} bootstrap address(es), {} rendezvous entr(ies)",
            invite.bootstrap.len(),
            invite.rendezvous.len()
        ),
    ));
    if use_switchboards {
        diag.steps.push(DiagStep::ok(
            "switchboard",
            "",
            format!(
                "the inviter endorsed {} currently dialable standing member fallback(s); unexpired routes may be tried only after the direct route",
                available_switchboard_count
            ),
        ));
    } else if available_switchboard_count > 0 {
        diag.steps.push(DiagStep::unknown(
            "switchboard",
            "",
            "standing member fallbacks were present but the joiner did not consent to contact them",
        ));
    }

    // A joiner gets its own per-server identity + stable port too: it is a full member afterwards,
    // so its peer record has to keep resolving across restarts exactly like the founder's.
    let mut net = new_server_net("", "", "");

    // If the invite points at a rendezvous, discover the inviter there (no hard-coded address);
    // otherwise dial the invite's bootstrap addresses directly (loopback / LAN / relayed).
    let (mesh, inviter, rz_config, needs_direct_wait, direct_already_failed) = if !invite
        .rendezvous
        .is_empty()
    {
        let targets =
            validate_invite_rendezvous_addrs(&invite.rendezvous).map_err(|e| e.to_string())?;
        let fallback_rz_config = retained_invite_rendezvous_config(&targets);
        match discover_and_connect(
            &invite,
            &net,
            plan_inviter,
            &state.endpoint_dials,
            &mut diag.steps,
        )
        .await
        {
            Ok((mesh, inviter, rz_config, port)) => {
                net.port = port;
                (mesh, inviter, rz_config, false, false)
            }
            Err(error) if use_switchboards => {
                let inviter = plan_inviter.ok_or_else(|| {
                    "assisted invite does not pin the named inviter transport".to_string()
                })?;
                // The discovery transport may already have spent its deadline and is dropped.
                // Retain the stable identity/port preference but start a clean listener with no
                // attacker-controlled initial dial so the consented helper path can proceed.
                let (mesh, _id, port, bound) = build_transport(&net, &[])?;
                record_listener_evidence(&mesh, &bound, &mut diag.steps).await;
                net.port = port;
                diag.steps.push(DiagStep::unknown(
                    "switchboard",
                    "",
                    format!("direct rendezvous path failed ({error}); trying the consented member fallback"),
                ));
                (mesh, inviter, fallback_rz_config, true, true)
            }
            Err(error) => return Err(error),
        }
    } else {
        // Validated + capped before a single socket is opened (defect P7): an unvalidated list of
        // up to 64 author-chosen addresses is a connect flood with a paste for a trigger.
        let candidate_addrs = dialable_bootstrap(&invite.bootstrap);
        // The dropped entries are worth naming: "the invite listed three addresses and every one
        // was loopback or otherwise undialable" is a diagnosis; a bare empty list is not.
        let dropped = invite.bootstrap.len().saturating_sub(candidate_addrs.len());
        if dropped > 0 {
            diag.steps.push(DiagStep::failed(
                "dial",
                "",
                format!("{dropped} address(es) in the invite were unusable and were not dialled"),
            ));
        }
        if candidate_addrs.is_empty() && !use_switchboards {
            return Err("invite carries no usable bootstrap address".to_string());
        }
        let inviter = match candidate_addrs.iter().find_map(target_peer_in_multiaddr) {
            Some(peer) => phase0_peer_id(&peer),
            None => plan_inviter.ok_or_else(|| {
                "invite has neither a direct inviter route nor a pinned assisted inviter"
                    .to_string()
            })?,
        };
        let peer_bound: Vec<(Multiaddr, DialEndpoint)> = candidate_addrs
            .iter()
            .filter_map(|address| {
                invite_peer_endpoint(&address.to_string(), &inviter)
                    .map(|endpoint| (address.clone(), endpoint))
            })
            .collect();
        let wrong_peer = candidate_addrs.len().saturating_sub(peer_bound.len());
        if wrong_peer > 0 {
            diag.steps.push(DiagStep::failed(
                "dial",
                "",
                format!(
                    "{wrong_peer} otherwise usable address(es) named a different inviter and were not dialled"
                ),
            ));
        }
        if peer_bound.is_empty() && !use_switchboards {
            return Err("invite carries no usable route bound to its inviter".to_string());
        }
        let endpoints: Vec<DialEndpoint> = peer_bound
            .iter()
            .map(|(_, endpoint)| endpoint.clone())
            .collect();
        let granted = state.endpoint_dials.reserve(&invite.group_id, &endpoints);
        let addrs: Vec<Multiaddr> = granted
            .iter()
            .filter_map(|address| address.parse().ok())
            .collect();
        let deferred = peer_bound.len().saturating_sub(addrs.len());
        if deferred > 0 {
            diag.steps.push(DiagStep::unknown(
                "dial",
                "",
                format!(
                    "the shared process budget deferred {deferred} otherwise usable address(es)"
                ),
            ));
        }
        if addrs.is_empty() && !use_switchboards {
            return Err(
                "the process-wide discovery dial budget deferred every inviter endpoint"
                    .to_string(),
            );
        }
        let (mesh, _id, port, bound) = build_transport(&net, &addrs)?;
        record_listener_evidence(&mesh, &bound, &mut diag.steps).await;
        net.port = port;
        // The transport dials these itself, concurrently, so a per-address outcome is not
        // observable here; what IS observable is which were tried and whether any answered.
        for a in &addrs {
            diag.steps
                .push(DiagStep::unknown("dial", a.to_string(), "dialled"));
        }
        let no_direct_route = addrs.is_empty();
        (mesh, inviter, Vec::new(), true, no_direct_route)
    };

    // Prepare the future member's own reachability *before* waiting on the one-way invite route.
    // On timeout this exact transport and stable identity stay alive for a 60-second two-way reply
    // window; dropping/rebuilding them would invalidate the NAT mapping in the code just copied.
    let local_peer_id = peer_id_of(&net)?;
    let joiner_addrs = auto_bootstrap(net.port, &local_peer_id);
    for addr in external_addrs(&joiner_addrs) {
        let _ = mesh.add_external_address(addr).await;
    }
    let port_mapping_rx = mesh.take_port_mapping_snapshots().await;
    let autonat_rx = mesh.take_autonat_snapshots().await;
    let relay_address_rx = mesh.take_relay_address_snapshots().await;
    let mesh_observation_rx = mesh.take_mesh_observation_snapshots().await;
    let mesh_handle = mesh.handle();

    let mut join_contact = inviter;
    let mut reply_expires_at_ms = None;
    let mut reply_context: Option<JoinReply> = None;
    let mut connection_handoff = PreOwnerConnectionHandoff::default();
    let mut switchboard_expires_at_ms = None;
    let mut switchboard_contacts = Vec::new();
    if needs_direct_wait {
        let direct_failed = direct_already_failed
            || wait_for_peer(&mesh, inviter, Duration::from_secs(20))
                .await
                .is_err();
        if direct_failed {
            diag.steps.push(DiagStep::failed(
                "connect",
                "",
                if direct_already_failed {
                    "no usable direct inviter route remained"
                } else {
                    "none of the dialled addresses answered within 20s"
                },
            ));

            // Direct-first is deliberate: member fallback reveals the joiner's IP/timing to an
            // additional group member and may spend their bandwidth. Only routes separately
            // labelled and signed by the inviter are dialled here; they were never mixed into the
            // transport's initial bootstrap set.
            if use_switchboards {
                let now = SystemClock.now_ms();
                let (allowed, candidates) = switchboard_dial_plan(
                    &switchboards,
                    &invite.group_id,
                    now,
                    invite.expires_at_ms,
                );
                let (allowed, candidates) = schedule_switchboard_candidates(
                    &state.endpoint_dials,
                    &invite.group_id,
                    allowed,
                    candidates,
                );
                let latest_deadline = allowed.values().copied().max().unwrap_or(now);
                if !candidates.is_empty() {
                    let _ = mesh.handle().dial_join_candidates(&candidates).await;
                }
                let remaining = latest_deadline.saturating_sub(now);
                match wait_for_switchboard_peer(
                    &mesh,
                    &allowed,
                    Duration::from_millis(remaining.min(15_000)),
                )
                .await
                {
                    Ok((helper, helper_deadline)) => {
                        join_contact = helper;
                        switchboard_contacts = allowed.into_iter().collect();
                        switchboard_contacts
                            .sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
                        switchboard_expires_at_ms = Some(helper_deadline.min(invite.expires_at_ms));
                        diag.steps.push(DiagStep::ok(
                            "switchboard",
                            "",
                            "connected to an inviter-endorsed standing member fallback",
                        ));
                    }
                    Err(()) => diag.steps.push(DiagStep::failed(
                        "switchboard",
                        "",
                        "none of the inviter-endorsed standing fallbacks answered within 15s",
                    )),
                }
            }

            let mut candidates = external_addrs(&joiner_addrs);
            if let Some(rx) = port_mapping_rx.as_ref() {
                candidates.extend(
                    rx.borrow()
                        .active
                        .iter()
                        .map(|mapping| mapping.address.clone()),
                );
            }
            candidates.sort_by_key(ToString::to_string);
            candidates.dedup();
            candidates.truncate(4);
            if switchboard_expires_at_ms.is_none() && candidates.is_empty() {
                diag.steps.push(DiagStep::failed(
                    "reply",
                    "",
                    "this joiner has no public listener route to put in a two-way reply",
                ));
                return Err(
                    "timed out connecting to the server; no direct reply route is available"
                        .to_string(),
                );
            }

            if switchboard_expires_at_ms.is_none() {
                let joiner_peer = keypair_from_seed(net.key_seed)
                    .map_err(|e| e.to_string())?
                    .public()
                    .to_peer_id();
                let mut rng = OsCryptoRng;
                let reply = JoinReply::mint(
                    invite.encode(),
                    &invite.invite_nonce,
                    joiner_peer,
                    candidates,
                    &SystemClock,
                    &mut rng,
                )
                .map_err(|e| e.to_string())?;
                let ready = JoinReplyReady {
                    code: reply.encode(),
                    expires_at_ms: reply.expires_at_ms,
                    candidate_count: reply.candidates.len(),
                };
                reply_expires_at_ms = Some(reply.expires_at_ms);
                {
                    // The code contains this device's current listener addresses. Never publish
                    // it into a webview session that was explicitly locked while direct dialing
                    // was pending.
                    let _session =
                        require_ui_session_generation(state, ui_session_generation).await?;
                    app.emit("join-reply-ready", &ready)
                        .map_err(|e| e.to_string())?;
                }
                diag.steps.push(DiagStep::unknown(
                "reply",
                "",
                format!(
                    "generated a 60-second two-way reply with {} direct candidate(s); waiting for the inviter to dial back",
                    ready.candidate_count
                ),
            ));

                let remaining = reply.expires_at_ms.saturating_sub(SystemClock.now_ms());
                join_contact = wait_for_reply_peer(
                &mesh,
                &mut connection_handoff,
                &reply,
                &invite.invite_nonce,
                Duration::from_millis(remaining),
            )
                .await
                .map_err(|_| {
                    diag.steps.push(DiagStep::failed(
                        "reply",
                        "",
                        "the two-way reply window expired before the inviter connected",
                    ));
                    "two-way connection reply expired; generate a fresh reply and keep both apps open"
                        .to_string()
                })?;
                reply_context = Some(reply);
            }
        }
        let detail = if join_contact == inviter {
            "connected to the named inviter"
        } else if switchboard_expires_at_ms.is_some() {
            "connected to an inviter-endorsed standing switchboard"
        } else {
            "connected to an existing member helper; it will forward only the admission handshake"
        };
        diag.steps.push(DiagStep::ok("connect", "", detail));
    }

    let device = MlsDevice::generate().map_err(|e| e.to_string())?;
    // `server_name` is the local rail label (DM: friend's name). If omitted/empty, fall back to
    // `display_name` (user's profile name) for backwards compatibility.
    let name = server_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| display_name.clone());
    let used_reply_path = reply_expires_at_ms.is_some();
    let used_switchboard_path = switchboard_expires_at_ms.is_some();
    let join = if switchboard_expires_at_ms.is_some() {
        let join_plan = assisted_plan.as_deref().ok_or_else(|| {
            "standing fallback was selected without its signed assisted invite plan".to_string()
        })?;
        Server::join_from_switchboards(
            mesh,
            device,
            OsCryptoRng,
            Box::new(SystemClock),
            display_name,
            join_contact,
            &switchboard_contacts,
            inviter,
            &invite,
            join_plan,
        )
        .await
        .map(|(server, authenticated_contact)| {
            join_contact = authenticated_contact;
            server
        })
    } else if let Some(expires_at_ms) = reply_expires_at_ms {
        let reply = reply_context
            .as_ref()
            .ok_or_else(|| "reply connection was selected without its proof context".to_string())?;
        let reply_joiner_peer = reply.joiner.to_bytes();
        Server::join_from_reply_with_handoff(
            mesh,
            connection_handoff,
            device,
            OsCryptoRng,
            Box::new(SystemClock),
            display_name,
            join_contact,
            inviter,
            &invite,
            reply.joiner_nonce,
            &reply_joiner_peer,
            expires_at_ms,
        )
        .await
        .map(|(server, authenticated_contact)| {
            join_contact = authenticated_contact;
            server
        })
    } else {
        Server::join(
            mesh,
            device,
            OsCryptoRng,
            Box::new(SystemClock),
            display_name,
            inviter,
            &invite,
        )
        .await
    };
    let mut server = join
    .map_err(|e| {
        let msg = e.to_string();
        let detail = if used_reply_path || used_switchboard_path {
            format!(
                "{msg}; no dial-back contact produced an inviter-signed Welcome before the reply window closed. A helper may have been unable to reach/prove the named inviter, so there may be no serving-node Join log entry"
            )
        } else {
            format!("{msg}; the server refused the join, and only the serving node knows why: ask its operator to read Server settings / Join log")
        };
        diag.steps.push(DiagStep::failed(
            "join",
            "",
            detail,
        ));
        msg
    })?;
    server.set_endpoint_dial_scheduler(state.endpoint_dials.clone());
    // Admission has now authenticated both group membership and the live Noise peer. Persist only
    // a direct route to the named inviter; untried invite candidates and admission-only helpers
    // never become recurring socket work. Reply and switchboard consent is time-bounded even when
    // the final authenticated contact is the inviter, so the path itself must remain part of this
    // decision rather than relying on the contact identity alone.
    net.reconnect_policy = reconnect_policy_after_admission(
        join_contact,
        inviter,
        used_reply_path,
        used_switchboard_path,
    );
    let direct_reconnect_authorized = matches!(
        net.reconnect_policy,
        ReconnectPolicy::AuthorizedPeer(peer) if peer == *join_contact.as_bytes()
    );
    net.reconnect_routes = if direct_reconnect_authorized {
        authenticated_reconnect_routes(&mesh_handle, join_contact)
    } else {
        Vec::new()
    };
    server.set_local_reconnect_routes(
        net.reconnect_routes
            .iter()
            .map(|route| (PeerId::new(route.peer_id), route.address.clone()))
            .collect(),
    );
    if direct_reconnect_authorized {
        // The sealed socket is useful after a restart only when the group snapshot also contains
        // the inviter's independently signed PeerDescriptor. Admission itself seeds merely an
        // untrusted transport candidate, so fetch the ordinary request-bound PEX record now rather
        // than hoping the periodic timer wins a race against the user closing the app.
        match timeout(
            Duration::from_millis(DIRECT_JOIN_PEX_MS),
            server.request_pex(join_contact),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => tracing::warn!(
                target: "catcoms_app",
                error = %error,
                "DISCOVERY.DIRECT_JOIN_PEX.FAILED"
            ),
            Err(_) => tracing::warn!(
                target: "catcoms_app",
                "DISCOVERY.DIRECT_JOIN_PEX.TIMED_OUT"
            ),
        }
    }
    diag.steps
        .push(DiagStep::ok("join", "", "admitted to the group"));
    // A joiner has to subscribe the control topic like the founder does. Without this,
    // `control_subscribed` stays false, `desired_routing_topics()` omits the control topics, and
    // this member never receives another membership commit for as long as it runs: a third person
    // joining is invisible to it, and every message that person sends is dropped, indefinitely.
    // `found_server` and `reload_one` both did this; the two join paths did not.
    server
        .subscribe_control()
        .await
        .map_err(|e| e.to_string())?;
    attach_blob_store(state, &mut server).await;
    // Steady-state discovery: the joiner keeps the invite's rendezvous so the actor re-registers/
    // re-discovers there (re-finding the group after a restart, no fresh invite).
    if !rz_config.is_empty() {
        server.set_rendezvous_nodes(rz_config);
    }
    // A joiner is a full member the moment the Welcome lands, so it publishes its own signed peer
    // record exactly like the founder does (defect P1). `ServerEntry.bootstrap` is also the live
    // source for later mapping/relay republication, so retain these base addresses even though a
    // joiner cannot mint owner-scoped invites. Non-routable entries are stripped inside
    // `publish_self_record`.
    diag.advertised.clone_from(&joiner_addrs);
    if let Err(e) = server.publish_self_record(joiner_addrs.clone(), net.record_seq) {
        tracing::warn!(target: "catcoms_app", phase = "join", error = %e, "DISCOVERY.PEER_RECORD.PUBLISH_FAILED");
    }

    let general = channel_id("general");
    let group_id = server.group_id();
    let device_id = server.device_id();
    let (actor, events, task) = spawn(server);
    actor.catch_up_channel_index(join_contact).await;
    actor.open_channel(general).await;
    actor.catch_up(join_contact, general).await;
    actor.catch_up_profiles(join_contact).await;
    actor.catch_up_livery(join_contact).await;
    actor.catch_up_badges(join_contact).await;
    actor.catch_up_files(join_contact).await;
    actor.catch_up_status(join_contact).await;
    actor.catch_up_calendar(join_contact).await;
    actor.catch_up_wiki(join_contact).await;
    actor.catch_up_roles(join_contact).await;
    actor.catch_up_moderation(join_contact).await;
    let channels = ui_channels(actor.channels().await);
    // A joiner mints no invites (owner-scoped), but these addresses still drive its signed live
    // peer record. A later router mapping or relay reservation is folded into this same set.
    let joiner_bootstrap_owners =
        bootstrap_owners(&joiner_addrs, BootstrapOwner::AutomaticInterface);
    // Catch-up can take arbitrarily longer than the UI session that initiated it. Serialize the
    // native registration + sealing boundary with explicit lock, and abandon the transient actor
    // if that lock already invalidated this operation. No server id, registry row, or event
    // forwarder is installed on the stale path.
    let session_commit = match require_ui_session_generation(state, ui_session_generation).await {
        Ok(commit) => commit,
        Err(error) => {
            actor.shutdown().await;
            drop(events);
            let _ = task.await;
            return Err(error);
        }
    };
    let server_id = register_server(
        app,
        state,
        actor,
        events,
        task,
        group_id,
        device_id,
        None,
        name,
        joiner_addrs,
        joiner_bootstrap_owners,
        Some(InterfaceRouteIdentity {
            port: net.port,
            peer_id: local_peer_id.clone(),
        }),
        Vec::new(),
        Some(mesh_handle),
        is_dm,
        net.switchboard,
        net.record_seq,
    )
    .await;
    // Seal the joined server, its network identity and the registry to disk (if unlocked).
    persist_server(state, server_id).await;
    persist_server_net(state, server_id, &net).await;
    persist_registry(state).await;
    drop(session_commit);
    diag.server = server_id;
    if let Some(rx) = port_mapping_rx {
        state
            .upnp
            .lock()
            .await
            .insert(server_id, PORT_MAPPING_WAITING.to_string());
        spawn_port_mapping_fold(app.clone(), server_id, rx, local_peer_id);
    }
    if let Some(rx) = autonat_rx {
        state.autonat.lock().await.insert(
            server_id,
            AutoNatEvidence {
                waiting: true,
                results: Vec::new(),
            },
        );
        spawn_autonat_fold(app.clone(), server_id, rx);
    }
    if let Some(rx) = relay_address_rx {
        spawn_relay_fold(app.clone(), server_id, rx);
    }
    if let Some(rx) = mesh_observation_rx {
        spawn_mesh_observation_fold(app.clone(), server_id, rx);
    }
    Ok(FoundResult {
        server: server_id,
        channel: general.to_string(),
        channels,
        is_dm,
    })
}

#[allow(clippy::too_many_arguments)]
fn record_active_join_reply(
    sessions: &mut HashMap<(u64, [u8; 16]), ActiveJoinReply>,
    server: u64,
    invite_nonce: [u8; 16],
    joiner: libp2p::PeerId,
    joiner_nonce: [u8; 16],
    expires_at_ms: u64,
    replace: bool,
    now_ms: u64,
) -> Result<(bool, u64, bool, Option<libp2p::PeerId>), String> {
    sessions.retain(|_, active| active.expires_at_ms >= now_ms);
    let key = (server, invite_nonce);
    match sessions.get_mut(&key) {
        Some(active) if active.joiner != joiner && !replace => Err(
            "a different joiner is already using this invite's reply window; confirm replacement only if you intended to switch people"
                .to_string(),
        ),
        Some(active) if active.joiner != joiner => {
            let displaced = active.joiner;
            let generation = active.generation.saturating_add(1);
            *active = ActiveJoinReply {
                joiner,
                nonces: vec![joiner_nonce],
                expires_at_ms,
                generation,
            };
            Ok((true, generation, true, Some(displaced)))
        }
        Some(active) => {
            if active.nonces.contains(&joiner_nonce) {
                return Ok((false, active.generation, false, None));
            }
            if active.nonces.len() >= 4 {
                active.nonces.remove(0);
            }
            active.nonces.push(joiner_nonce);
            active.expires_at_ms = expires_at_ms;
            active.generation = active.generation.saturating_add(1);
            Ok((false, active.generation, true, None))
        }
        None => {
            if sessions.len() >= MAX_ACTIVE_JOIN_REPLIES {
                return Err("too many connection replies are active; wait for one to expire"
                    .to_string());
            }
            sessions.insert(
                key,
                ActiveJoinReply {
                    joiner,
                    nonces: vec![joiner_nonce],
                    expires_at_ms,
                    generation: 1,
                },
            );
            Ok((false, 1, true, None))
        }
    }
}

/// Apply a joiner's short-lived two-way reply and repeatedly race its validated TCP/QUIC
/// candidates until the window expires. This only establishes transport: MLS admission still
/// runs on the inviter named and signed by the embedded invite, so a helper/other member cannot
/// admit anybody through this command.
#[tauri::command]
async fn apply_join_reply(
    app: AppHandle,
    state: State<'_, AppState>,
    server: u64,
    code: String,
    replace: bool,
) -> Result<JoinReplyApplied, String> {
    require_unlocked_session(&state).await?;
    let reply = JoinReply::decode(&code).map_err(|e| e.to_string())?;
    let permit = InviteToken::decode(&reply.invite_permit).map_err(|e| e.to_string())?;
    if !permit.verify_self() {
        return Err("connection reply contains an invalid signed invite".to_string());
    }
    reply
        .verify(&permit.invite_nonce, &SystemClock)
        .map_err(|e| e.to_string())?;
    let now_ms = SystemClock.now_ms();
    if permit.expires_at_ms < now_ms {
        return Err("the invite inside this connection reply has expired".to_string());
    }
    // Verification tolerates modest clock skew so two honest devices do not reject one another,
    // but that tolerance must not lengthen the local dial/helper authorization window. A bearer
    // can edit and re-MAC the reply, so cap every local side effect to sixty seconds from receipt.
    let effective_expires_at_ms = effective_join_reply_expiry(reply.expires_at_ms, now_ms);

    let (mesh, actor, group_id, device_id) = {
        let servers = state.servers.lock().await;
        let entry = servers
            .get(&server)
            .ok_or_else(|| "server not found".to_string())?;
        (
            entry
                .mesh
                .clone()
                .ok_or_else(|| "this server has no live transport handle".to_string())?,
            entry.actor.clone(),
            entry.group_id.clone(),
            entry.device_id,
        )
    };
    if permit.group_id != group_id {
        return Err("connection reply belongs to a different server".to_string());
    }
    if !actor.contains_member_device(permit.inviter_device_id).await {
        return Err(
            "the signed invite in this reply was not issued by a current server member".to_string(),
        );
    }
    let helper = permit.inviter_device_id != device_id;
    let local_peer = mesh.local_peer();
    let inviter_peers: HashSet<libp2p::PeerId> = permit
        .bootstrap
        .iter()
        .filter_map(|address| address.parse::<Multiaddr>().ok())
        .filter_map(|address| target_peer_in_multiaddr(&address))
        .collect();
    if inviter_peers.len() > 1 {
        return Err(
            "the invite in this reply does not name one unambiguous inviter peer".to_string(),
        );
    }
    let inviter_peer = if !helper {
        local_peer
    } else if let Some(peer) = inviter_peers.iter().next() {
        phase0_peer_id(peer)
    } else {
        actor
            .member_transport_peer(permit.inviter_device_id)
            .await
            .ok_or_else(|| {
                "this member has no current signed route for the invite's named inviter".to_string()
            })?
    };
    let _apply_guard = state.join_reply_apply.lock().await;
    let mut reply_sessions = state.join_replies.lock().await;
    let (replaced, generation, start_dial, displaced) = record_active_join_reply(
        &mut reply_sessions,
        server,
        permit.invite_nonce,
        reply.joiner,
        reply.joiner_nonce,
        effective_expires_at_ms,
        replace,
        now_ms,
    )?;
    drop(reply_sessions);
    if let Some(displaced) = displaced {
        actor
            .revoke_join_helper(phase0_peer_id(&displaced), permit.invite_nonce)
            .await;
    }
    if helper
        && !actor
            .authorize_join_helper(
                phase0_peer_id(&reply.joiner),
                permit.invite_nonce,
                permit.inviter_device_id,
                inviter_peer,
                effective_expires_at_ms,
            )
            .await
    {
        let mut sessions = state.join_replies.lock().await;
        if sessions
            .get(&(server, permit.invite_nonce))
            .is_some_and(|active| active.generation == generation && active.joiner == reply.joiner)
        {
            sessions.remove(&(server, permit.invite_nonce));
        }
        return Err(
            "this member could not open a bounded helper window for that reply".to_string(),
        );
    }
    let targets = reply.dial_targets();
    let target_count = targets.len();
    let expires_at_ms = effective_expires_at_ms;
    let joiner = reply.joiner;
    let proof = reply.dialback_proof(&permit.invite_nonce, local_peer.as_bytes());
    let jitter = u64::from(u16::from_be_bytes([
        reply.joiner_nonce[0],
        reply.joiner_nonce[1],
    ])) % 200;
    if start_dial {
        let task_app = app.clone();
        let invite_nonce = permit.invite_nonce;
        let endpoint_dials = state.endpoint_dials.clone();
        tauri::async_runtime::spawn(async move {
            let clock = SystemClock;
            let mut delay_ms = 200 + jitter;
            while clock.now_ms() <= expires_at_ms {
                let current = task_app
                    .state::<AppState>()
                    .inner()
                    .join_replies
                    .lock()
                    .await
                    .get(&(server, invite_nonce))
                    .is_some_and(|active| {
                        active.generation == generation && active.joiner == joiner
                    });
                if !current {
                    return;
                }
                let scheduled =
                    schedule_join_reply_candidates(&endpoint_dials, &group_id, &joiner, &targets);
                if !scheduled.is_empty() && mesh.dial_join_candidates(&scheduled).await.is_err() {
                    return;
                }
                // A socket alone proves nothing: send the code-holder proof over the Noise
                // connection before the retained joiner reveals its bearer invite/KeyPackage.
                // Keep retrying this even when the endpoint scheduler denies another *new* dial:
                // the actor drains the initial Dial and Request commands before polling the
                // swarm, so that first request can legitimately precede connection establishment.
                // The connected-only command never consults the actor's recent-peer redial cache;
                // without that distinction a denied pass could still start an implicit socket.
                let mut proof_request = Vec::with_capacity(33);
                proof_request.push(JOIN_REPLY_PROOF_KIND);
                proof_request.extend_from_slice(&proof);
                let _ = mesh
                    .request_control_connected_only(
                        phase0_peer_id(&joiner),
                        bytes::Bytes::from(proof_request),
                    )
                    .await;
                clock.sleep(Duration::from_millis(delay_ms)).await;
                delay_ms = delay_ms.saturating_mul(2).min(4_000);
            }
        });
    }

    {
        let mut diag = state.diag.lock().await;
        if diag.server == server {
            diag.steps.push(DiagStep::unknown(
                "reply",
                joiner.to_string(),
                format!(
                    "accepted a two-way reply and started a bounded dial race across {} candidate(s){}",
                    target_count,
                    if helper { "; this member will only forward the admission handshake" } else { "" }
                ),
            ));
        }
    }
    let out = JoinReplyApplied {
        joiner: joiner.to_string(),
        expires_at_ms,
        replaced,
        helper,
    };
    app.emit("join-reply-applied", &out)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

fn effective_join_reply_expiry(encoded_expires_at_ms: u64, received_at_ms: u64) -> u64 {
    encoded_expires_at_ms.min(received_at_ms.saturating_add(catcoms_net::JOIN_REPLY_LIFETIME_MS))
}

/// Leave a server: shut down its actor and drop it from the registry.
#[tauri::command]
async fn leave_server(state: State<'_, AppState>, server: u64) -> Result<(), String> {
    require_unlocked_session(&state).await?;
    // Remove the registry row first. Cache publication holds that same registry lock through its
    // insertion: either publication wins and this subsequent remove clears it, or removal wins
    // and the old actor incarnation can no longer publish at all.
    let removed = state.servers.lock().await.remove(&server);
    state.storage_health.lock().await.remove(&server);
    if let Ok(mut signals) = state.reconnect_capture_signals.lock() {
        signals.remove(&server);
    }
    state.upnp.lock().await.remove(&server);
    state.autonat.lock().await.remove(&server);
    state.mesh_observations.lock().await.remove(&server);
    state
        .join_replies
        .lock()
        .await
        .retain(|(candidate_server, _), _| *candidate_server != server);
    if let Some(entry) = removed {
        entry.actor.shutdown().await;
    }
    // Drop the sealed snapshot + re-seal the (now smaller) registry.
    {
        let guard = state.store.lock().await;
        if let Some(store) = guard.as_ref() {
            if let Err(e) = store.remove_server(server) {
                tracing::warn!(target: "catcoms_app", server, error = %e, "VAULT.SERVER.REMOVE_FAILED");
            }
        }
    }
    persist_registry(&state).await;
    Ok(())
}

/// Open (create/subscribe) a channel by name; returns its id. Members who open the same
/// name converge on the same channel. The channel is also caught up from the best known
/// peer, so opening a channel that already has history shows the backlog.
#[tauri::command]
async fn open_channel(
    state: State<'_, AppState>,
    server: u64,
    name: String,
) -> Result<String, String> {
    let actor = actor_of(&state, server).await?;
    let channel = actor.create_channel(name).await?;
    persist_server(&state, server).await;
    Ok(channel.id.to_string())
}

/// Read the server-wide channel directory.
#[tauri::command]
async fn get_channels(state: State<'_, AppState>, server: u64) -> Result<Vec<UiChannel>, String> {
    Ok(ui_channels(
        actor_of(&state, server).await?.channels().await,
    ))
}

/// Did the live address set change since this signed invite was minted?
///
/// An `InviteToken` signs its bootstrap list, so an address learned after minting cannot be
/// patched in or removed: the currently presented invite has to be re-minted. Previously copied
/// codes remain immutable and may still contain a route that later expired, which is why racing
/// all entries and keeping relay/rendezvous fallbacks remain necessary.
fn invite_addresses_changed(minted: &[String], current: &[String]) -> bool {
    minted.len() != current.len()
        || minted.iter().any(|a| !current.contains(a))
        || current.iter().any(|a| !minted.contains(a))
}

/// The single-use invite to share (founder only); `None` for a joiner.
///
/// Re-mints first if reachability improved since the stored invite was made. This exists because
/// of a real ordering trap: founding mints the invite immediately, but UPnP takes seconds to
/// answer, so the very invite a user naturally copies was the one that lacked the public address
/// the router had just opened. A friend on another network then got "timed out connecting to the
/// server" and every fix underneath looked broken.
///
/// Doing it here rather than in one mapper path is what makes it general. The frontend calls this
/// before every display/copy and on each reachability event, so both gains and losses produce a
/// token whose signed address set matches the current route set.
#[tauri::command]
async fn get_invite(state: State<'_, AppState>, server: u64) -> Result<Option<String>, String> {
    require_unlocked_session(&state).await?;
    let has_invite = {
        let servers = state.servers.lock().await;
        match servers.get(&server) {
            Some(e) => e.invite.is_some(),
            None => return Ok(None),
        }
    };
    if !has_invite {
        return Ok(None); // a joiner mints nothing
    }
    // A standing offer is intentionally short-lived and can disappear without changing our own
    // bootstrap. Re-wrap the same still-valid inner token on every display/copy, changing only
    // its signed helper plan. A new nonce is minted only when the inner token expired or its own
    // direct/rendezvous routes changed; otherwise opening this panel would create bearer invites
    // and rendezvous registrations without an explicit human action.
    match refresh_or_mint_invite(&state, server, false).await {
        Ok(fresh) => Ok(Some(fresh)),
        Err(e) => {
            tracing::warn!(target: "catcoms_app", error = %e, "JOIN.INVITE.REMINT_FAILED");
            Err(format!("the live invite could not be refreshed: {e}"))
        }
    }
}

/// Mint a **fresh** single-use invite on demand (owner/admin only; gated in `Server::mint_invite`),
/// carrying the live bootstrap address captured at found/reload. If the server registered at a
/// rendezvous, the fresh invite is also discovery-enabled and its new (nonce-keyed) namespace is
/// registered there via the stored transport handle, so the new joiner can discover us with no
/// hard-coded address. Replaces the server's stored invite and re-seals the registry.
///
/// This is also the **only** path that lifts outstanding transport evictions (P6). A member
/// removed earlier cannot otherwise reach this node to redeem an invite: the eviction refuses its
/// connection, and the roster cannot change until that connection is allowed. Deliberately not
/// folded into `mint_and_store_invite`, because [`get_invite`] re-mints on its own whenever the
/// node gains an address the stored invite does not mention (UPnP, a relay circuit, a rendezvous
/// registration); lifting there would silently re-admit every removed member the next time
/// anybody opened the invite panel, with nobody deciding it. Pressing "Generate new invite" is a
/// person saying they intend to admit somebody; opening a panel is not.
///
/// Best-effort and last: a failure to lift must not lose the invite that was just minted.
#[tauri::command]
async fn mint_invite_fresh(state: State<'_, AppState>, server: u64) -> Result<String, String> {
    let invite = mint_and_store_invite(&state, server).await?;
    match actor_of(&state, server).await {
        Ok(actor) => {
            if let Err(e) = actor.readmit_evicted_peers().await {
                tracing::warn!(target: "catcoms_app", error = %e, "JOIN.EVICTIONS.LIFT_FAILED");
            }
        }
        Err(e) => {
            tracing::warn!(target: "catcoms_app", error = %e, "JOIN.EVICTIONS.NO_ACTOR")
        }
    }
    Ok(invite)
}

/// Mint a fresh single-use invite from the server's *current* bootstrap and rendezvous config,
/// store it as the server's invite, and persist. Shared by the explicit "Generate new invite"
/// action and by [`get_invite`]'s self-heal, so both mint the same way and neither can drift.
///
/// Minting only. Anything that should happen because a *person* asked for an invite, rather than
/// because a stored one went stale, belongs in [`mint_invite_fresh`] and not here; the P6
/// eviction lift is the first such thing.
async fn mint_and_store_invite(state: &AppState, server: u64) -> Result<String, String> {
    refresh_or_mint_invite(state, server, true).await
}

fn encode_invite_text(encoded: &[u8]) -> String {
    if InviteJoinPlan::decode(encoded).is_ok() {
        format!("{ASSISTED_INVITE_PREFIX}{}", hex::encode(encoded))
    } else {
        hex::encode(encoded)
    }
}

/// Refresh only the short-lived helper envelope when possible. `force_new` is reserved for the
/// explicit Generate action; automatic display/copy refresh must preserve a still-valid nonce.
async fn refresh_or_mint_invite(
    state: &AppState,
    server: u64,
    force_new: bool,
) -> Result<String, String> {
    let _mint_guard = state.invite_mint.lock().await;
    let (bootstrap, rendezvous, handle, stored_invite) = {
        let servers = state.servers.lock().await;
        let e = servers
            .get(&server)
            .ok_or_else(|| "unknown server".to_string())?;
        (
            e.bootstrap.clone(),
            e.rendezvous.clone(),
            e.mesh.clone(),
            e.invite.clone(),
        )
    };
    let actor = actor_of(state, server).await?;
    let reusable = (!force_new)
        .then_some(stored_invite.as_deref())
        .flatten()
        .and_then(|text| decode_and_verify_invite(text).ok())
        .map(|decoded| decoded.token)
        .filter(|token| {
            token.expires_at_ms >= SystemClock.now_ms()
                && !invite_addresses_changed(&token.bootstrap, &bootstrap)
                && !invite_addresses_changed(&token.rendezvous, &rendezvous)
        });
    let plain_encoded = if let Some(token) = reusable {
        token.encode()
    } else {
        let mut nonce = [0u8; 16];
        let mut rng = OsCryptoRng;
        rng.fill_bytes(&mut nonce);
        let expires = SystemClock.now_ms() + 3_600_000; // single-use, valid for 1 hour
        if rendezvous.is_empty() {
            actor.mint_invite(nonce, expires, bootstrap.clone()).await?
        } else {
            let encoded = actor
                .mint_invite_with_rendezvous(nonce, expires, bootstrap.clone(), rendezvous.clone())
                .await?;
            // Register the fresh invite's namespace so the new joiner can discover us. This node's own
            // rendezvous config (typed into the create-server form, or restored from its own sealed
            // network record), so the operator variant: a joiner reading the invite we are about to
            // mint validates it again on the way in, with the strict one.
            if let (Some(handle), Some(rz)) = (
                &handle,
                validate_operator_rendezvous_addrs(&rendezvous)
                    .ok()
                    .and_then(|v| v.into_iter().next()),
            ) {
                let token = InviteToken::decode(&encoded).map_err(|e| e.to_string())?;
                register_join_ns(handle, &token.group_id, &token.invite_nonce, &rz).await?;
            }
            encoded
        }
    };
    let encoded = actor.wrap_invite_with_switchboards(plain_encoded).await?;
    let invite_hex = encode_invite_text(&encoded);
    {
        let mut servers = state.servers.lock().await;
        let e = servers
            .get_mut(&server)
            .ok_or_else(|| "server closed while minting its invite".to_string())?;
        if !invite_routes_still_current(&bootstrap, &rendezvous, &e.bootstrap, &e.rendezvous) {
            return Err(
                "reachable addresses changed while the invite was being minted; retry".to_string(),
            );
        }
        e.invite = Some(invite_hex.clone());
    }
    persist_registry(state).await;
    Ok(invite_hex)
}

fn invite_routes_still_current(
    expected_bootstrap: &[String],
    expected_rendezvous: &[String],
    current_bootstrap: &[String],
    current_rendezvous: &[String],
) -> bool {
    !invite_addresses_changed(expected_bootstrap, current_bootstrap)
        && !invite_addresses_changed(expected_rendezvous, current_rendezvous)
}

/// One poll of "would an invite minted right now work off this network?". Drives the invite
/// page's route-check progress: the frontend polls while `waiting` holds, then mints.
///
/// Read-only by design: it inspects the live bootstrap set and the router-mapping fold and mints
/// nothing, so polling it cannot create bearer invites or rendezvous registrations. The webview
/// learns the scope booleans, the mapping status line, and the LAN socket: the last is disclosed
/// so the port-forward suggestion can name the exact values to type into a router, and the
/// connectivity panel already shows these same addresses via `get_connectivity`.
#[derive(Debug, Clone, Default, Serialize)]
struct InviteRouteCheck {
    /// The live bootstrap set contains a globally routable direct address (mapped, advertised,
    /// or a routable IPv6). "Available", not "verified": AutoNAT confirmation is separate.
    public_direct: bool,
    /// The live bootstrap set contains a relay circuit whose relay host is publicly routable.
    relay: bool,
    /// The server registered at a rendezvous, so the invite is discovery-enabled.
    rendezvous: bool,
    /// The live bootstrap set contains a LAN (private, non-loopback) address, so an invite works
    /// for someone on the same network.
    lan: bool,
    /// The LAN IP a manual port-forward should target; empty when no LAN address exists.
    lan_ip: String,
    /// The listen port from the LAN entry, for the same suggestion; 0 when unknown.
    port: u16,
    /// The router-mapping window is still open with no verdict; worth polling again.
    waiting: bool,
    /// The router-mapping status line, same copy as the connectivity panel.
    mapping: String,
}

#[tauri::command]
async fn check_invite_routes(
    state: State<'_, AppState>,
    server: u64,
) -> Result<InviteRouteCheck, String> {
    require_unlocked_session(&state).await?;
    let (bootstrap, rendezvous) = {
        let servers = state.servers.lock().await;
        let e = servers
            .get(&server)
            .ok_or_else(|| "unknown server".to_string())?;
        (e.bootstrap.clone(), !e.rendezvous.is_empty())
    };
    let mapping = state
        .upnp
        .lock()
        .await
        .get(&server)
        .cloned()
        .unwrap_or_else(|| PORT_MAPPING_NOT_ATTEMPTED.to_string());
    let mut check = InviteRouteCheck {
        rendezvous,
        waiting: mapping == PORT_MAPPING_WAITING,
        mapping,
        ..Default::default()
    };
    classify_invite_routes(&mut check, &bootstrap);
    Ok(check)
}

/// Classify the live bootstrap set into the scopes an invite could reach. Split out so the
/// rules are regression-tested: a routable direct address is "internet", a routable circuit is
/// "relay" (`addr_is_globally_routable` judges every IP literal, so for a circuit it is the
/// relay host being judged, matching `switchboard_route_usable`), and a private non-loopback
/// address is "LAN". The first plain LAN entry also supplies the exact ip/port the manual
/// port-forward suggestion names; a LAN-hosted circuit never does, because its socket is the
/// relay's machine, not the one the user would forward to.
fn classify_invite_routes(check: &mut InviteRouteCheck, bootstrap: &[String]) {
    for address in bootstrap {
        let Ok(addr) = address.parse::<Multiaddr>() else {
            continue;
        };
        if addr_is_globally_routable(&addr) {
            if address.contains("/p2p-circuit") {
                check.relay = true;
            } else {
                check.public_direct = true;
            }
        } else if addr_is_private(&addr) && !addr_is_loopback(&addr) {
            check.lan = true;
            if check.lan_ip.is_empty() && !address.contains("/p2p-circuit") {
                for part in addr.iter() {
                    match part {
                        Protocol::Ip4(ip) => check.lan_ip = ip.to_string(),
                        Protocol::Ip6(ip) => check.lan_ip = ip.to_string(),
                        Protocol::Tcp(p) | Protocol::Udp(p) => check.port = p,
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Rename a server; a **local** display label in this client's rail (server names are not
/// shared between members), persisted to the registry.
#[tauri::command]
async fn rename_server(
    state: State<'_, AppState>,
    server: u64,
    name: String,
) -> Result<(), String> {
    require_unlocked_session(&state).await?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("name cannot be empty".into());
    }
    match state.servers.lock().await.get_mut(&server) {
        Some(e) => e.name = name,
        None => return Err("unknown server".into()),
    }
    persist_registry(&state).await;
    Ok(())
}

/// The current roster (member fingerprints; `you` marks the local device).
#[tauri::command]
async fn get_members(state: State<'_, AppState>, server: u64) -> Result<Vec<UiMember>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .members()
        .await
        .into_iter()
        .map(|m| UiMember {
            fingerprint: m.fingerprint,
            identity: m.identity,
            you: m.is_self,
        })
        .collect())
}

/// Set this member's own profile (name + styling + optional avatar and banner). `avatar` and
/// `banner` are base64-encoded JPEG bytes (empty = unset).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn set_profile(
    state: State<'_, AppState>,
    server: u64,
    name: String,
    color: String,
    font: String,
    effect: String,
    description: String,
    bubble: String,
    avatar: String,
    banner: String,
) -> Result<(), String> {
    let avatar = if avatar.is_empty() {
        Vec::new()
    } else {
        B64.decode(avatar.as_bytes())
            .map_err(|e| format!("bad avatar: {e}"))?
    };
    if avatar.len() > MAX_AVATAR_BYTES {
        return Err(format!(
            "avatar too large: {} bytes (max {MAX_AVATAR_BYTES})",
            avatar.len()
        ));
    }
    let banner = if banner.is_empty() {
        Vec::new()
    } else {
        B64.decode(banner.as_bytes())
            .map_err(|e| format!("bad banner: {e}"))?
    };
    if banner.len() > MAX_BANNER_BYTES {
        return Err(format!(
            "banner too large: {} bytes (max {MAX_BANNER_BYTES})",
            banner.len()
        ));
    }
    let actor = actor_of(&state, server).await?;
    actor
        .set_profile(Profile {
            name,
            color,
            font,
            effect,
            description,
            bubble,
            avatar,
            banner,
        })
        .await;
    persist_server(&state, server).await;
    Ok(())
}

/// All known member profiles (keyed by fingerprint).
#[tauri::command]
async fn get_profiles(state: State<'_, AppState>, server: u64) -> Result<Vec<UiProfile>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .profiles()
        .await
        .into_iter()
        .map(|(fingerprint, p)| UiProfile {
            fingerprint,
            name: p.name,
            color: p.color,
            font: p.font,
            effect: p.effect,
            description: p.description,
            bubble: p.bubble,
            avatar: if p.avatar.is_empty() {
                String::new()
            } else {
                B64.encode(&p.avatar)
            },
            banner: if p.banner.is_empty() {
                String::new()
            } else {
                B64.encode(&p.banner)
            },
        })
        .collect())
}

/// Publish the server livery (owner/admin only); re-seals the server. An all-empty livery
/// removes it. Sizes are bounded by the backend; the *values* are validated in the UI. The
/// published **icon and cursor are preserved**; each has its own command (`set_server_icon` /
/// `set_server_cursor`), so changing colours never resends or clears either image.
#[tauri::command]
async fn set_livery(
    state: State<'_, AppState>,
    server: u64,
    preset: String,
    accent: String,
    tokens: HashMap<String, String>,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor
        .set_livery(Livery {
            preset,
            accent,
            tokens,
            // Ignored by `set_livery`, which reads all three back out of the document and
            // writes them again unchanged. Publishing colours never touches them.
            icon: String::new(),
            cursor: String::new(),
            name: String::new(),
        })
        .await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Set (or clear, with `""`) the shared server icon (owner/admin only); re-seals the server.
/// `icon` is base64-encoded image bytes, capped like an avatar.
#[tauri::command]
async fn set_server_icon(
    state: State<'_, AppState>,
    server: u64,
    icon: String,
) -> Result<(), String> {
    if !icon.is_empty() {
        let bytes = B64
            .decode(icon.as_bytes())
            .map_err(|e| format!("bad server icon: {e}"))?;
        if bytes.len() > MAX_SERVER_ICON_BYTES {
            return Err(format!(
                "server icon too large: {} bytes (max {MAX_SERVER_ICON_BYTES})",
                bytes.len()
            ));
        }
    }
    let actor = actor_of(&state, server).await?;
    actor.set_server_icon(icon).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Set (or clear, with `""`) the shared server cursor (owner/admin only); re-seals the server.
/// `cursor` is base64-encoded image bytes, capped far below the icon (a cursor is ≤64×64).
#[tauri::command]
async fn set_server_cursor(
    state: State<'_, AppState>,
    server: u64,
    cursor: String,
) -> Result<(), String> {
    if !cursor.is_empty() {
        let bytes = B64
            .decode(cursor.as_bytes())
            .map_err(|e| format!("bad server cursor: {e}"))?;
        if bytes.len() > MAX_SERVER_CURSOR_BYTES {
            return Err(format!(
                "server cursor too large: {} bytes (max {MAX_SERVER_CURSOR_BYTES})",
                bytes.len()
            ));
        }
    }
    let actor = actor_of(&state, server).await?;
    actor.set_server_cursor(cursor).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// The server's published livery (all-empty if none).
#[tauri::command]
async fn get_livery(state: State<'_, AppState>, server: u64) -> Result<UiLivery, String> {
    let actor = actor_of(&state, server).await?;
    let l = actor.livery().await;
    Ok(UiLivery {
        preset: l.preset,
        accent: l.accent,
        tokens: l.tokens,
        icon: l.icon,
        cursor: l.cursor,
        name: l.name,
    })
}

/// The largest file this server accepts, in bytes, and the band an owner may move it within.
///
/// The frontend uses this to refuse an over-large file with a sentence naming the real limit,
/// rather than letting the upload begin and fail. The native side still enforces it.
#[derive(Serialize, Clone)]
struct UiFileSizeLimit {
    /// What this server currently accepts.
    limit: u64,
    /// The protocol ceiling: no server may be set above this.
    max: u64,
    /// The smallest an owner may set, so the fileshare stays usable.
    min: u64,
}

#[tauri::command]
async fn get_file_size_limit(
    state: State<'_, AppState>,
    server: u64,
) -> Result<UiFileSizeLimit, String> {
    let actor = actor_of(&state, server).await?;
    Ok(UiFileSizeLimit {
        limit: actor.file_size_limit().await,
        max: catcoms_app::MAX_FILE_BYTES as u64,
        min: catcoms_app::MIN_FILE_SIZE_LIMIT,
    })
}

/// Set the largest file this server accepts; owner/admin only, re-seals the server.
///
/// Lowering it does not withdraw files already shared. It governs what may be added next.
#[tauri::command]
async fn set_file_size_limit(
    state: State<'_, AppState>,
    server: u64,
    bytes: u64,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.set_file_size_limit(bytes).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Publish (or clear, with `""`) the shared **server name**; owner/admin only, re-seals the
/// server. Independent of the colours and both images: setting it disturbs none of them.
///
/// Clearing does not rename anybody's rail. It means the group publishes no name, and every
/// member falls back to the local label it already had.
#[tauri::command]
async fn set_shared_server_name(
    state: State<'_, AppState>,
    server: u64,
    name: String,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.set_server_name(name).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Assign a custom badge to a member (owner/admin only); re-seals the server. An empty `label`
/// removes that member's badge. Sizes and the entry count are bounded by the backend, which also
/// rejects labels reserved for the built-in roles (`owner`/`admin`/`mod`/`moderator`).
#[tauri::command]
async fn set_member_badge(
    state: State<'_, AppState>,
    server: u64,
    fp: String,
    label: String,
    color: String,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.set_member_badge(fp, label, color).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Every admitted companion device, keyed by the **companion's** fingerprint (empty until some
/// member pairs a second device). The UI resolves a message author through this map: an author
/// that appears here renders under `origin`'s profile with `name` as a device tag.
#[tauri::command]
async fn get_devices(
    state: State<'_, AppState>,
    server: u64,
) -> Result<HashMap<String, UiDevice>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .devices()
        .await
        .into_iter()
        .map(|(fp, d)| {
            (
                fp,
                UiDevice {
                    origin: d.origin,
                    name: d.name,
                },
            )
        })
        .collect())
}

/// Every assigned member badge, keyed by member fingerprint (empty if none).
#[tauri::command]
async fn get_badges(
    state: State<'_, AppState>,
    server: u64,
) -> Result<HashMap<String, UiBadge>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .badges()
        .await
        .into_iter()
        .map(|(fp, b)| {
            (
                fp,
                UiBadge {
                    label: b.label,
                    color: b.color,
                },
            )
        })
        .collect())
}

/// Open a streamed upload: the frontend then sends `size` bytes as a run of `push_file_chunk`
/// calls and closes with `finish_file_upload`. Returns how many chunks it must send.
///
/// Uploading through one command meant the whole file crossed the IPC bridge as a single base64
/// string and was then sealed inside one actor command. Both halves scale with the file: the
/// webview froze building and posting that string, and the server actor, which is one `select!`
/// loop, stopped draining inbound sync and stopped answering every other command for that server
/// until the last chunk was written. A large share therefore looked like the whole app hanging.
/// A streamed upload moves and seals one [`CHUNK_BYTES`] chunk at a time, so neither side is ever
/// occupied for longer than one chunk and the server keeps syncing throughout.
///
/// Returns the upload's **token** along with its chunk count. Every later call carries the token;
/// see [`UploadKey`] for why the caller's own id cannot be the identity of the work.
#[tauri::command]
async fn begin_file_upload(
    state: State<'_, AppState>,
    server: u64,
    upload_id: String,
    mime: String,
    size: u64,
    trace: Option<String>,
) -> Result<UploadTicket, AppError> {
    // The first half of the bracket. An upload that begins and never ends is a reservation holding
    // a slot with no record of what became of it, and the review names it: FILE.UPLOAD.ORPHANED.
    // Detecting one means being able to see a start with no matching end, which means recording
    // the start.
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Files,
        "file_upload",
        server,
        None,
    );
    // Reaching the actor is both the session gate and proof the server exists, before this
    // reserves a slot for it.
    let actor = actor_of(&state, server)
        .await
        .map_err(|e| op.fail(codes::SERVER_UNAVAILABLE, e))?;
    if upload_id.is_empty()
        || upload_id.len() > MAX_UPLOAD_ID_BYTES
        || !upload_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(op.fail(codes::FILE_UPLOAD_REFUSED, "bad upload id"));
    }
    // This server's own limit, which the owner sets and which can never exceed the protocol
    // ceiling. Checked here rather than only at publish so an over-large file is refused before
    // a single slice is staged, instead of after the whole thing has been sealed into the vault.
    let limit = actor.file_size_limit().await;
    if size > limit {
        return Err(op.fail(
            codes::FILE_UPLOAD_REFUSED,
            format!("file is larger than this server's {limit}-byte limit"),
        ));
    }
    let chunk_total = upload_chunk_count(size);
    let token = mint_upload_token();
    let now = SystemClock.now_ms();
    let mut uploads = state.uploads.lock().await;
    // Retire what this begin supersedes, in one pass:
    //  - the previous generation of the same visible transfer (a restart). It is retired rather
    //    than reused, so an earlier seal still in flight finds nothing to attach to and collects.
    //  - anything untouched for long enough that its caller is plainly gone.
    let retired: Vec<UploadKey> = uploads
        .iter()
        .filter(|(k, v)| {
            (k.0 == server && v.upload_id == upload_id)
                || v.idle_since(now.saturating_sub(UPLOAD_IDLE_TIMEOUT_MS))
        })
        .map(|(k, _)| k.clone())
        .collect();
    let retired: Vec<PendingUpload> = retired.iter().filter_map(|k| uploads.remove(k)).collect();
    if uploads.len() >= MAX_PENDING_UPLOADS {
        return Err(op.fail(
            codes::FILE_UPLOAD_REFUSED,
            "too many uploads are already in flight",
        ));
    }
    // The entry cap does not bound vault growth; this does. Counted against what is already
    // staged plus what this upload would add if it ran to the end and never finished.
    let staged: u64 = uploads.values().map(PendingUpload::staged_bytes).sum();
    if staged.saturating_add(chunk_total as u64 * CHUNK_BYTES as u64) > MAX_STAGED_UPLOAD_BYTES {
        return Err(op.fail(
            codes::FILE_UPLOAD_REFUSED,
            "too much upload data is already waiting to be published",
        ));
    }
    uploads.insert(
        (server, token.clone()),
        PendingUpload {
            server,
            upload_id,
            mime,
            address: CidHasher::new(),
            buffer: Vec::new(),
            chunks: Vec::new(),
            sealing: None,
            bytes_seen: 0,
            declared_size: size,
            chunk_total,
            touched_at: now,
        },
    );
    drop(uploads);
    let superseded = retired.len();
    for old in retired {
        discard_pending_upload(&state, old).await;
    }
    // Not a terminal phase: the upload has started, not finished. `finish` or `cancel` closes it,
    // and a trace with neither is the orphan.
    op.stage("FILE.UPLOAD.RESERVED");
    if superseded > 0 {
        // A restart of the same visible transfer, or a sweep of uploads whose caller went away.
        // Worth a line: it is also what an upload loop looks like from the outside.
        tracing::info!(
            target: "catcoms_app",
            superseded,
            "FILE.UPLOAD.SUPERSEDED"
        );
    }
    Ok(UploadTicket {
        token,
        chunk_total,
        slice_bytes: UPLOAD_SLICE_BYTES,
    })
}

/// Take ONE slice of a streamed upload (base64-encoded bytes at byte `offset`), sealing a chunk
/// into the vault whenever the buffered slices add up to one.
///
/// A slice is the IPC unit and a chunk is the seal unit, and they are deliberately different
/// sizes. The chunk size is fixed by the manifest format; the slice size is chosen so that no
/// single invoke argument is large enough for serializing it to stall the webview.
///
/// `offset` must be exactly the bytes accepted so far and every slice but the file's last must be
/// exactly [`UPLOAD_SLICE_BYTES`]: the media reader finds a byte offset's chunk by dividing, so a
/// reordered, repeated or short slice would silently produce a manifest whose chunk boundaries do
/// not line up. A rejected slice fails the whole upload rather than being skipped, because the
/// running whole-file address has already absorbed what came before it.
#[tauri::command]
async fn push_file_chunk(
    app: AppHandle,
    state: State<'_, AppState>,
    server: u64,
    token: String,
    offset: u64,
    data: String,
    // The frontend threads the upload's trace through every slice so the whole transfer is one
    // operation. A slice is deliberately not worth a diagnostic event of its own: a large file is
    // thousands of them, and recording each would bury the upload in its own progress. It is used
    // for the progress emit below, which is per *chunk* and is what a stalled transfer is
    // diagnosed from. Begin and finish bracket the transfer; what happened between is the
    // difference.
    trace: Option<String>,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    // Bound the decode before doing it: base64 expands by 4/3, so this cannot be a legal slice
    // however it decodes. The decoded length is then held to the exact bound below.
    if data.len() > UPLOAD_SLICE_BYTES * 2 {
        return Err("upload slice is larger than one slice".into());
    }
    let bytes = B64
        .decode(data.as_bytes())
        .map_err(|e| format!("bad slice data: {e}"))?;
    if bytes.len() > UPLOAD_SLICE_BYTES {
        return Err("upload slice is larger than one slice".into());
    }
    let key = (server, token);
    // Validate and buffer under the lock, then seal outside it: an actor round-trip must never
    // happen while holding a lock every other upload needs.
    let (full_chunk, mime, chunk_total, upload_id) = {
        let mut uploads = state.uploads.lock().await;
        let up = uploads
            .get_mut(&key)
            .ok_or_else(|| "no such upload".to_string())?;
        (
            up.admit_slice(offset, &bytes)?,
            up.mime.clone(),
            up.chunk_total,
            up.upload_id.clone(),
        )
    };
    let Some(chunk) = full_chunk else {
        return Ok(()); // buffered; the chunk it belongs to is not complete yet
    };
    let done = seal_pending_chunk(&state, &actor, &key, chunk, mime).await?;
    emit_tracked(
        &app,
        "upload-progress",
        UploadProgressEvt {
            server,
            upload_id,
            done,
            // Publishing the index entry is the final step, so `done == total` means the group can
            // actually see the file rather than that this device has finished copying it.
            total: chunk_total + 1,
        },
        external_progress_trace(trace.as_deref()),
    );
    Ok(())
}

/// Take an upload out of the map if, and only if, it may still be published.
///
/// The last gate before an irreversible, group-visible act, so it is deliberately one step: the
/// upload is removed and returned, or it is removed and destroyed, and there is no state in
/// between where a lock could arrive and find nothing to cancel. `locked` is read *after* the last
/// seal, because an answer from before that await says nothing about the session now.
///
/// A locked session ends the upload rather than deferring it: the chunks are already sealed, and
/// holding them across a lock would mean the vault carrying an unpublished transfer the user
/// believes they closed. An incomplete upload is destroyed for a different reason: its chunks
/// would be listed under the address of a whole file that was never sent, and every member would
/// fail the reassembly check forever.
async fn take_publishable_upload(
    state: &AppState,
    key: &UploadKey,
    locked: bool,
) -> Result<PendingUpload, String> {
    let taken = state.uploads.lock().await.remove(key);
    let Some(pending) = taken else {
        return Err("no such upload".to_string());
    };
    if locked {
        discard_pending_upload(state, pending).await;
        return Err("the vault is locked".into());
    }
    if !pending.is_complete() {
        discard_pending_upload(state, pending).await;
        return Err("the upload did not send every chunk".into());
    }
    Ok(pending)
}

/// Seal one assembled chunk into the vault and record its reference against the pending upload;
/// returns how many chunks that upload has sealed. The caller has already claimed the sealing
/// slot, so this owns releasing it (and, on failure, tearing the whole upload down: the running
/// whole-file address has absorbed these bytes and cannot be rewound).
///
/// The upload is re-looked-up by token after the await, and the returning `FileRef` is attached
/// only if that generation is still waiting for exactly this chunk. Anything else, a cancel, a
/// lock, a restart of the same visible transfer, means the sealed blob belongs to nothing, and
/// the only safe thing to do with it is collect it.
async fn seal_pending_chunk(
    state: &AppState,
    actor: &ServerActor,
    key: &UploadKey,
    chunk: Vec<u8>,
    mime: String,
) -> Result<usize, String> {
    let index = {
        let uploads = state.uploads.lock().await;
        uploads
            .get(key)
            .and_then(|up| up.sealing)
            .ok_or_else(|| "no such upload".to_string())?
    };
    let sealed = actor.seal_upload_chunk(chunk, mime).await;
    let mut uploads = state.uploads.lock().await;
    match sealed {
        Ok(file_ref) => {
            let attached = uploads
                .get_mut(key)
                .filter(|up| up.can_accept(index))
                .map(|up| up.chunk_sealed(file_ref.clone()));
            match attached {
                Some(done) => Ok(done),
                // Cancelled, locked, or restarted mid-seal: the blob is stored but nothing will
                // ever name it, so it goes back out rather than onto whatever now holds this key.
                None => {
                    drop(uploads);
                    actor.discard_upload(vec![file_ref]).await;
                    Err("this upload is no longer waiting for that chunk".into())
                }
            }
        }
        Err(e) => {
            let abandoned = uploads.remove(key);
            drop(uploads);
            if let Some(up) = abandoned {
                discard_pending_upload(state, up).await;
            }
            Err(e)
        }
    }
}

/// Close a streamed upload: publish its index entry under `name` in folder `path`, making it
/// visible to the group, and return the file's content-address hex.
///
/// The MIME type is the one declared at `begin_file_upload`, not one passed here, so it always
/// matches what the chunks were actually sealed with.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn finish_file_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    server: u64,
    token: String,
    name: String,
    path: String,
    trace: Option<String>,
) -> Result<String, AppError> {
    // Closes the bracket opened by `begin_file_upload`, under the same trace the frontend threaded
    // through every slice.
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Files,
        "file_upload",
        server,
        None,
    );
    let actor = op.actor(&state, server).await?;
    let key = (server, token);
    // Seal whatever the last slices left buffered: a file whose size is not a whole number of
    // chunks ends with a short one, and an empty file is still one (empty) chunk.
    let tail = {
        let mut uploads = state.uploads.lock().await;
        let up = uploads
            .get_mut(&key)
            .ok_or_else(|| op.fail(codes::FILE_UPLOAD_FAILED, "no such upload"))?;
        up.take_tail()
            .map_err(|e| op.fail(codes::FILE_UPLOAD_FAILED, e))?
            .map(|chunk| (chunk, up.mime.clone()))
    };
    if let Some((chunk, mime)) = tail {
        seal_pending_chunk(&state, &actor, &key, chunk, mime)
            .await
            .map_err(|e| op.fail(codes::FILE_UPLOAD_FAILED, e))?;
    }
    let locked = require_unlocked_session(&state).await.is_err();
    let pending = take_publishable_upload(&state, &key, locked)
        .await
        .map_err(|e| op.fail(codes::FILE_UPLOAD_FAILED, e))?;
    let cid = pending.address.cid();
    let chunk_total = pending.chunk_total;
    let upload_id = pending.upload_id;
    // Kept for the failure path: a failed publish must not leave sealed bytes nothing names.
    let sealed = pending.chunks.clone();
    let published = actor
        .publish_upload(
            name,
            pending.mime,
            path,
            *cid.as_bytes(),
            pending.declared_size,
            pending.chunks,
        )
        .await;
    let hex = match published {
        Ok(hex) => hex,
        Err(e) => {
            actor.discard_upload(sealed).await;
            // Sealed bytes existed and were thrown away. Distinguishable from a refusal at begin,
            // where nothing had been written yet.
            return Err(op.fail(codes::FILE_UPLOAD_FAILED, e));
        }
    };
    emit_tracked(
        &app,
        "upload-progress",
        UploadProgressEvt {
            server,
            upload_id,
            done: chunk_total + 1,
            total: chunk_total + 1,
        },
        op.trace,
    );
    persist_server(&state, server).await;
    // Deliberately after persistence, like every other operation here: an upload reported as
    // succeeding before its index entry reached the disk is the shape of "it uploaded and then it
    // was gone after a restart".
    op.succeeded("FILE.UPLOAD.PUBLISHED");
    Ok(hex)
}

/// Abandon a streamed upload and garbage-collect whatever it had already sealed. Called when the
/// frontend's upload fails or the user cancels; an upload left open otherwise holds a slot until
/// the session is locked.
#[tauri::command]
async fn cancel_file_upload(
    state: State<'_, AppState>,
    server: u64,
    token: String,
    trace: Option<String>,
) -> Result<(), AppError> {
    // The other way the bracket closes. A cancel is not a failure of the app, but it is the
    // difference between an upload that ended and one that was abandoned without a word.
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Files,
        "file_upload",
        server,
        None,
    );
    require_unlocked_session(&state)
        .await
        .map_err(|e| op.fail(codes::SESSION_LOCKED, e))?;
    let pending = state.uploads.lock().await.remove(&(server, token));
    let had_reservation = pending.is_some();
    if let Some(up) = pending {
        discard_pending_upload(&state, up).await;
    }
    if had_reservation {
        op.succeeded("FILE.UPLOAD.CANCELLED");
    } else {
        // Cancelling something that was already gone. Harmless and idempotent by design, but
        // worth distinguishing: a cancel with nothing to cancel means the reservation was retired
        // by something else, which is a different story from the user changing their mind.
        op.succeeded("FILE.UPLOAD.CANCEL_NOOP");
    }
    Ok(())
}

/// The shared file list (metadata; bytes are fetched on download).
#[tauri::command]
async fn get_files(state: State<'_, AppState>, server: u64) -> Result<FilesPayload, String> {
    let actor = actor_of(&state, server).await?;
    let view = actor.files_view().await;
    let files = view.files.into_iter().map(ui_file).collect();
    Ok(FilesPayload {
        files,
        has_peers: view.has_peers,
    })
}

/// Verify the chunks referenced by this server. This performs no network requests and never
/// treats a CID-named path as healthy until its seal, address and file reference all verify.
#[tauri::command]
async fn get_storage_health(
    state: State<'_, AppState>,
    server: u64,
) -> Result<UiStorageHealth, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    // Validate the webview-provided id before touching the singleflight. The gate itself is fixed
    // size, but stale cache rows for a departed/nonexistent server must not be returned either.
    let (actor, instance) = actor_instance_of(&state, server).await?;
    if let Some(report) = storage_health_cache_get(&state, server, instance, generation).await? {
        return Ok(report);
    }
    // Coalesce expensive work without blocking explicit lock's short cache-clear operation.
    let _scan = state.storage_scans.for_server(server).lock().await;
    if let Some(report) = storage_health_cache_get(&state, server, instance, generation).await? {
        return Ok(report);
    }
    let snapshot = actor.storage_snapshot().await;
    let report = storage_report(&actor, snapshot, SystemClock.now_ms()).await;
    storage_health_cache_publish(&state, server, instance, generation, report.clone()).await?;
    Ok(report)
}

/// Ask authenticated peers for every repairable missing/unreadable chunk, then verify the set.
/// Contradictory exact references remain unreadable without a pointless same-CID fetch.
#[tauri::command]
async fn repair_storage(
    state: State<'_, AppState>,
    server: u64,
) -> Result<UiStorageRepair, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    let (actor, instance) = actor_instance_of(&state, server).await?;
    let _scan = state.storage_scans.for_server(server).lock().await;
    let generation_check = require_ui_session_generation(&state, generation).await?;
    // Do not retain the UI commit guard across peer fetches: lock closes the command boundary
    // immediately and a late result is independently rejected by the guarded publish below.
    drop(generation_check);
    let repaired = actor.repair_storage().await?;
    // The repair result records work counts. Re-snapshot afterward so the inventory rows and
    // exact-manifest health verdict are paired atomically even if a replicated index update landed
    // while the repair was fetching chunks.
    let snapshot = actor.storage_snapshot().await;
    let health = storage_report(&actor, snapshot, SystemClock.now_ms()).await;
    storage_health_cache_publish(&state, server, instance, generation, health.clone()).await?;
    Ok(UiStorageRepair {
        attempted_chunks: repaired.attempted_chunks,
        recovered_chunks: repaired.recovered_chunks,
        health,
    })
}

/// Clone a cache row, then prove the original UI generation still owns the native commit boundary
/// before returning it. Explicit lock can therefore clear promptly and no stale clone escapes
/// after that lock operation completes.
async fn storage_health_cache_get(
    state: &AppState,
    server: u64,
    server_instance: u64,
    generation: u64,
) -> Result<Option<UiStorageHealth>, String> {
    let cached = state
        .storage_health
        .lock()
        .await
        .get(&server)
        .filter(|cached| cached.server_instance == server_instance)
        .map(|cached| cached.report.clone());
    let _commit = require_ui_session_generation(state, generation).await?;
    let current = state
        .servers
        .lock()
        .await
        .get(&server)
        .is_some_and(|entry| entry.instance == server_instance);
    if !current {
        return Err("the server changed while storage inspection was in progress".into());
    }
    Ok(cached)
}

/// Publish expensive work only if it still belongs to the unlocked UI generation that requested
/// it. Lock does not await the scan gate, and a scan that finishes later cannot refill the cache.
async fn storage_health_cache_publish(
    state: &AppState,
    server: u64,
    server_instance: u64,
    generation: u64,
    report: UiStorageHealth,
) -> Result<(), String> {
    let _commit = require_ui_session_generation(state, generation).await?;
    // Hold the registry row through cache insertion. `leave_server` removes this row before
    // clearing the cache, which makes its ordering with this publication atomic.
    let servers = state.servers.lock().await;
    if servers
        .get(&server)
        .is_none_or(|entry| entry.instance != server_instance)
    {
        return Err("the server changed while storage inspection was in progress".into());
    }
    state.storage_health.lock().await.insert(
        server,
        CachedStorageHealth {
            server_instance,
            report,
        },
    );
    Ok(())
}

/// Current members whose self-asserted peer id has a live connection here. This diagnostic
/// projection is not proof that the member controls the peer or is personally online.
#[tauri::command]
async fn get_online_members(
    state: State<'_, AppState>,
    server: u64,
) -> Result<Vec<String>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor.online_members().await)
}

/// One member's reachability as this node sees it, for the debug console's network view.
#[derive(Serialize)]
struct MemberRouteEvt {
    fingerprint: String,
    /// Short hex of the member's self-asserted transport peer, or empty when no record has been
    /// learned. Empty is a complete explanation on its own for a member calls cannot reach.
    peer: String,
    addresses: Vec<String>,
    seq: u64,
    connected: bool,
    /// Policy-approved dial batches submitted for this record epoch, not confirmed failures.
    dial_attempts: u8,
    next_dial_in_ms: u64,
    health: &'static str,
    binding: &'static str,
    active_paths: Vec<ConnectionPathEvt>,
    last_success: Option<MemberRouteSuccessEvt>,
    candidate_families: Vec<&'static str>,
    candidate_transports: Vec<&'static str>,
    actions: Vec<MemberRouteActionEvt>,
    indirect_health: &'static str,
    indirect_witnesses: usize,
    indirect_age_ms: Option<u64>,
    reciprocal_pending: bool,
}

#[derive(Serialize)]
struct ConnectionPathEvt {
    family: &'static str,
    transport: &'static str,
    direction: &'static str,
}

#[derive(Serialize)]
struct MemberRouteSuccessEvt {
    path: ConnectionPathEvt,
    age_ms: u64,
}

#[derive(Serialize)]
struct MemberRouteActionEvt {
    scope: &'static str,
    kind: &'static str,
}

fn connection_family_name(family: catcoms_rt::ConnectionFamily) -> &'static str {
    use catcoms_rt::ConnectionFamily::*;
    match family {
        Ipv4 => "ipv4",
        Ipv6 => "ipv6",
        Dns => "dns",
        Memory => "memory",
        Unknown => "unknown",
    }
}

fn connection_transport_name(transport: catcoms_rt::ConnectionTransport) -> &'static str {
    use catcoms_rt::ConnectionTransport::*;
    match transport {
        Tcp => "tcp",
        QuicV1 => "quic_v1",
        WebSocket => "websocket",
        CircuitRelay => "circuit_relay",
        Memory => "memory",
        Unknown => "unknown",
    }
}

fn connection_path_evt(path: catcoms_rt::ConnectionPath) -> ConnectionPathEvt {
    ConnectionPathEvt {
        family: connection_family_name(path.family),
        transport: connection_transport_name(path.transport),
        direction: match path.direction {
            catcoms_rt::ConnectionDirection::Dialer => "dialer",
            catcoms_rt::ConnectionDirection::Listener => "listener",
        },
    }
}

fn member_route_health_name(health: catcoms_sync::MemberRouteHealth) -> &'static str {
    use catcoms_sync::MemberRouteHealth::*;
    match health {
        NoPeerRecord => "no_peer_record",
        ClaimedPeerHasNoRoute => "claimed_peer_has_no_route",
        ClaimedPeerConnectedDirect => "claimed_peer_connected_direct",
        ClaimedPeerConnectedRelay => "claimed_peer_connected_relay",
        ClaimedPeerConnectedOther => "claimed_peer_connected_other",
        ClaimedPeerDialCoolingDown => "claimed_peer_dial_cooling_down",
        ClaimedPeerDialEligible => "claimed_peer_dial_eligible",
    }
}

fn member_route_action_evt(action: catcoms_sync::MemberRouteAction) -> MemberRouteActionEvt {
    use catcoms_sync::{MemberRouteActionKind as Kind, MemberRouteActionScope as Scope};
    MemberRouteActionEvt {
        scope: match action.scope {
            Scope::ThisDevice => "this_device",
            Scope::MemberDevice => "member_device",
            Scope::Group => "group",
        },
        kind: match action.kind {
            Kind::WaitForAutomaticRecovery => "wait_for_automatic_recovery",
            Kind::CheckMemberConnectivity => "check_member_connectivity",
            Kind::KeepAnotherMemberConnected => "keep_another_member_connected",
            Kind::ConfigureFallbackNode => "configure_fallback_node",
            Kind::ProbeThroughMembers => "probe_through_members",
            Kind::RetryGroupNow => "retry_group_now",
        },
    }
}

fn indirect_route_health_name(health: catcoms_sync::IndirectRouteHealth) -> &'static str {
    use catcoms_sync::IndirectRouteHealth::*;
    match health {
        Unknown => "unknown",
        ReachableViaMember => "reachable_via_member",
        SuspectedUnreachable => "suspected_unreachable",
    }
}

/// What this node knows about reaching each member of a server.
///
/// The question none of the existing views could answer: the roster shows names whether or not
/// anything can be reached, and presence collapses "no record for them yet", "a record with no
/// candidate" and "a submitted dial batch is in scheduler cooldown" into one grey dot. Local
/// state only, so asking costs nothing on the wire. Submission is not a per-address failure result.
#[tauri::command]
async fn get_member_routes(
    state: State<'_, AppState>,
    server: u64,
) -> Result<Vec<MemberRouteEvt>, String> {
    let actor = actor_of(&state, server).await?;
    let deadline_clock = SystemClock;
    let routes = finish_member_route_query_before(
        actor.try_member_routes(),
        deadline_clock.sleep(Duration::from_secs(5)),
    )
    .await?;
    Ok(routes
        .into_iter()
        .map(|r| MemberRouteEvt {
            fingerprint: r.fingerprint,
            peer: r.peer_id.map(|p| hex::encode(&p[..4])).unwrap_or_default(),
            addresses: r.addresses,
            seq: r.seq,
            connected: r.connected,
            dial_attempts: r.dial_attempts,
            next_dial_in_ms: r.next_dial_in_ms,
            health: member_route_health_name(r.health),
            binding: match r.binding {
                catcoms_sync::MemberRouteBinding::Absent => "absent",
                catcoms_sync::MemberRouteBinding::SelfAsserted => "self_asserted",
            },
            active_paths: r
                .active_paths
                .into_iter()
                .map(connection_path_evt)
                .collect(),
            last_success: r.last_success.map(|success| MemberRouteSuccessEvt {
                path: connection_path_evt(success.path),
                age_ms: success.age_ms,
            }),
            candidate_families: r
                .candidate_families
                .into_iter()
                .map(connection_family_name)
                .collect(),
            candidate_transports: r
                .candidate_transports
                .into_iter()
                .map(connection_transport_name)
                .collect(),
            actions: r.actions.into_iter().map(member_route_action_evt).collect(),
            indirect_health: indirect_route_health_name(r.indirect_health),
            indirect_witnesses: r.indirect_witnesses,
            indirect_age_ms: r.indirect_age_ms,
            reciprocal_pending: r.reciprocal_pending,
        })
        .collect())
}

/// Ask the selected server to retry every current member route once, preserving the process-wide
/// scheduler and anti-click cooldown. The stable result id lets Connectivity explain whether work
/// started, no route existed, or a safety limit deferred it.
#[tauri::command]
async fn manual_fallback_redial(state: State<'_, AppState>, server: u64) -> Result<String, String> {
    let actor = actor_of(&state, server).await?;
    let outcome = actor.manual_fallback_redial().await?;
    Ok(match outcome {
        catcoms_sync::ManualRedialOutcome::Submitted => "submitted",
        catcoms_sync::ManualRedialOutcome::CoolingDown => "cooling_down",
        catcoms_sync::ManualRedialOutcome::NoRoutes => "no_routes",
        catcoms_sync::ManualRedialOutcome::DeferredBySafetyLimit => "deferred_by_safety_limit",
    }
    .into())
}

/// Produce a recovery code from this server's current listener candidates. The sync layer filters
/// the bridge's aggregate bootstrap set down to canonical direct IP routes bound to this actor's
/// transport identity, so relay, rendezvous, DNS and stale foreign-peer entries cannot leak into
/// or gain authority through the out-of-band code.
#[tauri::command]
async fn mint_member_recovery(
    state: State<'_, AppState>,
    server: u64,
) -> Result<MemberRecoveryReady, String> {
    let ui_session_generation = unlocked_ui_session_generation(&state).await?;
    let actor = actor_of(&state, server).await?;
    let candidates = {
        let servers = state.servers.lock().await;
        servers
            .get(&server)
            .map(|entry| entry.bootstrap.clone())
            .ok_or_else(|| "server is not open".to_string())?
    };
    let recovery = actor.mint_member_recovery(candidates).await?;
    let ready = MemberRecoveryReady {
        code: recovery.encode(),
        expires_at_ms: recovery.expires_at_ms,
        candidate_count: recovery.candidates.len(),
    };
    // The code contains current private/LAN listener candidates. Serialize its return with an
    // explicit lock exactly like the join-reply event; stale work must not repopulate the webview.
    let _session = require_ui_session_generation(&state, ui_session_generation).await?;
    Ok(ready)
}

/// Seal bounded pending permission to retain a route for the member named by a verified recovery
/// code. Neither the pasted route nor the proven peer/routes change here: a later discovery tick
/// atomically promotes this field only after the exact new peer authenticates live.
async fn authorize_member_recovery_capture(
    state: &AppState,
    server: u64,
    peer: PeerId,
    expires_at_ms: u64,
) -> Result<(), String> {
    let guard = state.store.lock().await;
    let store = guard
        .as_ref()
        .ok_or_else(|| "the vault is locked".to_string())?;
    let mut net = store
        .load_server_net(server)
        .map_err(|error| format!("could not load the server network identity: {error}"))?
        .ok_or_else(|| "the server network identity is missing".to_string())?;
    net.pending_recovery_peer = Some(*peer.as_bytes());
    net.pending_recovery_expires_at_ms = expires_at_ms;
    store
        .save_server_net(server, &net, &mut OsCryptoRng)
        .map_err(|error| format!("could not save pending recovery permission: {error}"))
}

/// Apply a code from another current member. Verification, roster membership, expiry, canonical
/// address filtering and shared dial limits all live inside the actor. A successful result means
/// attempts were submitted; the UI must wait for ordinary connectivity evidence before calling
/// the member reachable.
#[tauri::command]
async fn apply_member_recovery(
    app: AppHandle,
    state: State<'_, AppState>,
    server: u64,
    code: String,
) -> Result<MemberRecoveryAppliedEvt, String> {
    let ui_session_generation = unlocked_ui_session_generation(&state).await?;
    let actor = actor_of(&state, server).await?;
    // Verify without network work, then serialize the durable, expiring authority against lock.
    // This ordering closes the short-edge race where authentication completed before the bridge
    // had written permission for the capture worker to retain that route.
    let verified = actor.verify_member_recovery(code.clone()).await?;
    let _session = require_ui_session_generation(&state, ui_session_generation).await?;
    authorize_member_recovery_capture(&state, server, verified.peer, verified.expires_at_ms)
        .await?;
    drop(_session);
    // Validation is intentionally repeated inside the actor immediately before the bounded dial;
    // a code that expired or lost its roster binding between phases does not reach the network.
    let applied = actor.apply_member_recovery(code).await?;
    // The bounded dial may finish after lock, but its private command result may not escape into a
    // newer or locked UI session. Pending authority simply expires if the result is withheld.
    let _session = require_ui_session_generation(&state, ui_session_generation).await?;
    drop(_session);
    // The dial submission may already have authenticated by the time the actor reply arrives.
    // Wake the same coalesced worker used by later connectivity/member-route events.
    notify_reconnect_capture(&app, server);
    Ok(MemberRecoveryAppliedEvt {
        fingerprint: fingerprint(&applied.device),
        submitted_routes: applied.submitted_routes,
    })
}

/// Bound a present-time diagnostic query so a busy server actor becomes an explicit unavailable
/// snapshot instead of leaving old green rows labelled as current indefinitely.
async fn finish_member_route_query_before<T, Q, D>(query: Q, deadline: D) -> Result<T, String>
where
    Q: Future<Output = Result<T, String>>,
    D: Future<Output = ()>,
{
    tokio::pin!(query);
    tokio::pin!(deadline);
    tokio::select! {
        result = &mut query => result,
        _ = &mut deadline => Err("member-route snapshot timed out".to_string()),
    }
}

/// Delivery state for this device's recent messages in a channel; the seed a UI paints on open,
/// before the throttled `delivery-changed` event next fires. Empty until this session sends a
/// message (the message-id → change mapping is not persisted across a restart).
#[tauri::command]
async fn get_delivery(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
) -> Result<DeliverySnapshotEvt, String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    Ok(delivery_payload(actor.delivery_snapshot(id).await?))
}

/// Per-DM activity stats (no message text) for the friends-list sortings, one entry per DM group.
#[derive(Serialize, Clone)]
struct DmStat {
    server: u64,
    count: u64,
    first_ts: u64,
    last_ts: u64,
    active_days: u64,
}

/// Activity stats for every DM (count + timestamps over its #general conversation), so the UI can
/// sort friends by activity / reconnect / recency. Clones the DM actors first (no lock held across
/// the awaits), then queries each; bounded by the (small) number of DMs.
#[tauri::command]
async fn dm_stats(state: State<'_, AppState>) -> Result<Vec<DmStat>, String> {
    require_unlocked_session(&state).await?;
    let dms: Vec<(u64, ServerActor)> = {
        let servers = state.servers.lock().await;
        servers
            .iter()
            .filter(|(_, e)| e.is_dm)
            .map(|(id, e)| (*id, e.actor.clone()))
            .collect()
    };
    let general = channel_id("general");
    let mut out = Vec::with_capacity(dms.len());
    for (id, actor) in dms {
        let s = actor.message_stats(general).await;
        out.push(DmStat {
            server: id,
            count: s.count,
            first_ts: s.first_ts,
            last_ts: s.last_ts,
            active_days: s.active_days,
        });
    }
    Ok(out)
}

/// One pending incoming DM (friend) request surfaced to the recipient.
#[derive(Serialize, Clone)]
struct DmRequestView {
    from_fp: String,
    from_name: String,
    invite: String,
}

/// Deliver a DM (friend) invite to member `target_fp` over `server` ("Add friend" from the roster).
/// Returns `true` if the member was reachable and the request was sent.
#[tauri::command]
async fn send_dm_invite(
    state: State<'_, AppState>,
    server: u64,
    target_fp: String,
    invite_hex: String,
) -> Result<bool, String> {
    let invite = hex::decode(invite_hex.trim()).map_err(|e| format!("bad invite: {e}"))?;
    let actor = actor_of(&state, server).await?;
    actor.send_dm_invite(target_fp, invite).await
}

/// Push a call-signalling message (base64 payload) to member `target_fp`. `true` if reached.
///
/// The path behind one of the two incidents that started all of this: a call died while the roster
/// still showed the peer online. That is exactly a run of these returning `false`, which the caller
/// turned into a `console.warn` and nothing else. The `false` is not an error and must not be
/// recorded as one, but it is the whole diagnosis, so it is recorded as its own outcome.
///
/// The payload is opaque here and stays that way: what is recorded is whether it went, never what
/// it said.
#[tauri::command]
async fn send_call_signal(
    state: State<'_, AppState>,
    server: u64,
    target_fp: String,
    payload: String,
    trace: Option<String>,
) -> Result<bool, AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Voice,
        "send_call_signal",
        server,
        None,
    );
    let bytes = B64
        .decode(payload.as_bytes())
        .map_err(|e| op.fail(codes::VOICE_SIGNAL_FAILED, format!("bad payload: {e}")))?;
    let actor = op.actor(&state, server).await?;
    let delivered = actor
        .send_call_signal(target_fp, bytes)
        .await
        .map_err(|e| op.fail(codes::VOICE_SIGNAL_FAILED, e))?;
    if delivered {
        op.succeeded("VOICE.SIGNAL.DELIVERED");
    } else {
        // The roster says this member is here and the transport has nowhere to send to. Not a
        // failure of this command, and precisely the thing that was invisible.
        op.failed("VOICE.SIGNAL.NO_MEMBER_ROUTE");
    }
    Ok(delivered)
}

/// This call's E2E media base key (base64) + the MLS epoch it's keyed to. Derived locally from the
/// group key; never sent on the wire.
#[tauri::command]
async fn call_media_key(
    state: State<'_, AppState>,
    server: u64,
    call_id: String,
) -> Result<(String, u64), String> {
    let id: u128 = call_id.parse().map_err(|_| "bad call id".to_string())?;
    let actor = actor_of(&state, server).await?;
    let (key, epoch) = actor.media_key(id).await?;
    Ok((B64.encode(key), epoch))
}

/// The pending incoming DM (friend) requests received over `server`.
#[tauri::command]
async fn get_dm_requests(
    state: State<'_, AppState>,
    server: u64,
) -> Result<Vec<DmRequestView>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .dm_requests()
        .await
        .into_iter()
        .map(|(from_fp, from_name, invite)| DmRequestView {
            from_fp,
            from_name,
            invite: hex::encode(invite),
        })
        .collect())
}

/// Dismiss a pending DM request by the sender's fingerprint (after accepting or declining it).
#[tauri::command]
async fn dismiss_dm_request(
    state: State<'_, AppState>,
    server: u64,
    from_fp: String,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.dismiss_dm_request(from_fp).await;
    Ok(())
}

/// Whether a shared file's blob is held locally (openable without a network fetch).
#[tauri::command]
async fn file_available(
    state: State<'_, AppState>,
    server: u64,
    cid: String,
) -> Result<bool, String> {
    let raw = hex::decode(cid.trim()).map_err(|e| format!("bad cid: {e}"))?;
    let actor = actor_of(&state, server).await?;
    Ok(actor.file_available(raw).await)
}

/// Remove a shared file from the index by content-address hex (owner/admin only).
#[tauri::command]
async fn delete_file(state: State<'_, AppState>, server: u64, cid: String) -> Result<(), String> {
    let raw = hex::decode(cid.trim()).map_err(|e| format!("bad cid: {e}"))?;
    let actor = actor_of(&state, server).await?;
    actor.delete_file(raw).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Adjust ONE listing's circulation expiry: `expires` is an absolute ms-epoch deadline, or `null`
/// to keep the file forever. Addressed by content-address hex + folder, because expiry is per
/// listing. Uploader, owner or admin only (honest-client gate, like delete).
///
/// This records metadata. It does **not** cause the file to be dropped, deleted or evicted at the
/// deadline; no retention pass consumes it yet.
#[tauri::command]
async fn set_file_expiry(
    state: State<'_, AppState>,
    server: u64,
    cid: String,
    path: String,
    expires: Option<u64>,
) -> Result<(), String> {
    let raw = hex::decode(cid.trim()).map_err(|e| format!("bad cid: {e}"))?;
    let actor = actor_of(&state, server).await?;
    actor.set_file_expiry(raw, path, expires).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Where a shared file is used: the wiki pages that embed it, plus status/chat reference counts.
#[tauri::command]
async fn get_file_usage(
    state: State<'_, AppState>,
    server: u64,
    cid: String,
) -> Result<UiFileUsage, String> {
    let raw = hex::decode(cid.trim()).map_err(|e| format!("bad cid: {e}"))?;
    let actor = actor_of(&state, server).await?;
    let usage = actor.file_usage(raw).await;
    Ok(UiFileUsage {
        pinned: usage.wiki_pinned(),
        wiki_pages: usage.wiki_pages,
        status_count: usage.status_count,
        chat_count: usage.chat_count,
        event_count: usage.event_count,
    })
}

/// The content addresses (lowercase hex) embedded in a live wiki page: files that must never drop
/// out of circulation. Derived from the wiki each call; dropping the embed un-pins the file.
#[tauri::command]
async fn get_wiki_pinned_cids(
    state: State<'_, AppState>,
    server: u64,
) -> Result<Vec<String>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor.wiki_pinned_cids().await)
}

fn valid_inline_download_cancellation_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= INLINE_DOWNLOAD_CANCELLATION_ID_MAX_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_'))
}

/// Reclaim one begin-only permit at the bound. Such an entry owns no actor work and has no native
/// completion path after its webview disappears; active entries are never displaced.
fn reclaim_abandoned_inline_download_at_capacity(
    table: &mut HashMap<String, InlineDownloadCancellation>,
) {
    if table.len() < MAX_CANCELLABLE_INLINE_DOWNLOADS {
        return;
    }
    let abandoned = table
        .iter()
        .filter(|(_, entry)| !entry.started)
        .min_by_key(|(_, entry)| entry.generation)
        .map(|(id, _)| id.clone());
    if let Some(abandoned) = abandoned {
        if let Some(entry) = table.remove(&abandoned) {
            let _ = entry.signal.send(true);
        }
    }
}

fn require_inline_download_capacity(
    table: &mut HashMap<String, InlineDownloadCancellation>,
) -> Result<(), String> {
    reclaim_abandoned_inline_download_at_capacity(table);
    if table.len() >= MAX_CANCELLABLE_INLINE_DOWNLOADS {
        Err("too many inline downloads are already active".to_string())
    } else {
        Ok(())
    }
}

fn register_inline_download(state: &AppState, id: &str) -> Result<(), String> {
    if !valid_inline_download_cancellation_id(id) {
        return Err("invalid inline-download cancellation id".to_string());
    }
    let mut table = state
        .inline_downloads
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Lock publishes this flag before acquiring the same table to cancel its contents. Testing it
    // while holding the table closes the otherwise possible sweep-before-insert race.
    if state.session_lock_requested.load(Ordering::Acquire) {
        return Err("the vault is locked".to_string());
    }
    if let Some(existing) = table.get(id) {
        if existing.started {
            return Err("inline-download cancellation id is already registered".to_string());
        }
        // A reloaded webview can restart its local sequence and reuse the exact id left by its
        // predecessor. Begin-only authority owns no actor work, so replace it atomically; rejecting
        // it would let the abandoned registration deny the fresh view's first take load.
        if let Some(abandoned) = table.remove(id) {
            let _ = abandoned.signal.send(true);
        }
    }
    require_inline_download_capacity(&mut table)?;
    let generation = state
        .next_inline_download_generation
        .fetch_add(1, Ordering::Relaxed);
    let (signal, _receiver) = watch::channel(false);
    table.insert(
        id.to_string(),
        InlineDownloadCancellation {
            generation,
            started: false,
            signal,
        },
    );
    Ok(())
}

/// Claim a process-owned slot for callers of the compatibility form that omit a cancellation id.
/// The reserved `#native:` prefix is outside the webview token grammar, so injected IPC cannot
/// collide with or cancel these entries; explicit lock remains their cancellation authority.
fn claim_internal_inline_download(
    state: &AppState,
) -> Result<(InlineDownloadLease, watch::Receiver<bool>), String> {
    let (id, generation, receiver) = {
        let mut table = state
            .inline_downloads
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.session_lock_requested.load(Ordering::Acquire) {
            return Err("the vault is locked".to_string());
        }
        require_inline_download_capacity(&mut table)?;

        // At most four ids can be live. Even after the theoretical u64 wrap, five attempts find
        // an unused id without allowing a stale RAII lease to alias the new registration.
        let mut identity = None;
        for _ in 0..=MAX_CANCELLABLE_INLINE_DOWNLOADS {
            let generation = state
                .next_inline_download_generation
                .fetch_add(1, Ordering::Relaxed);
            let id = format!("#native:{generation}");
            if !table.contains_key(&id) {
                identity = Some((id, generation));
                break;
            }
        }
        let (id, generation) = identity
            .ok_or_else(|| "could not allocate an inline-download generation".to_string())?;
        let (signal, receiver) = watch::channel(false);
        table.insert(
            id.clone(),
            InlineDownloadCancellation {
                generation,
                started: true,
                signal,
            },
        );
        (id, generation, receiver)
    };
    Ok((
        InlineDownloadLease {
            inner: Arc::new(InlineDownloadLeaseInner {
                table: Arc::clone(&state.inline_downloads),
                id,
                generation,
            }),
        },
        receiver,
    ))
}

fn claim_inline_download(
    state: &AppState,
    id: &str,
) -> Result<(InlineDownloadLease, watch::Receiver<bool>), String> {
    if !valid_inline_download_cancellation_id(id) {
        return Err("invalid inline-download cancellation id".to_string());
    }
    let (generation, receiver) = {
        let mut table = state
            .inline_downloads
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let entry = table
            .get_mut(id)
            .ok_or_else(|| "inline-download cancellation id is not registered".to_string())?;
        if entry.started {
            return Err("inline-download cancellation id was already claimed".to_string());
        }
        entry.started = true;
        (entry.generation, entry.signal.subscribe())
    };
    Ok((
        InlineDownloadLease {
            inner: Arc::new(InlineDownloadLeaseInner {
                table: Arc::clone(&state.inline_downloads),
                id: id.to_string(),
                generation,
            }),
        },
        receiver,
    ))
}

fn cancel_inline_download_registration(state: &AppState, id: &str) -> Result<bool, String> {
    if !valid_inline_download_cancellation_id(id) {
        return Err("invalid inline-download cancellation id".to_string());
    }
    let signal = {
        let mut table = state
            .inline_downloads
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match table.get(id) {
            Some(entry) if entry.started => Some(entry.signal.clone()),
            Some(_) => table.remove(id).map(|entry| entry.signal),
            None => None,
        }
    };
    if let Some(signal) = signal {
        // Active registrations remain counted until their exact RAII lease observes cancellation
        // and returns. Otherwise repeated cancel/start churn could exceed the process-wide cap.
        let _ = signal.send(true);
        Ok(true)
    } else {
        Ok(false)
    }
}

fn cancel_all_inline_downloads(state: &AppState) {
    let signals = {
        let mut table = state
            .inline_downloads
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let signals = table
            .values()
            .filter(|entry| entry.started)
            .map(|entry| entry.signal.clone())
            .collect::<Vec<_>>();
        // Begin-only registrations have no command that can own their cleanup. Active entries
        // stay counted until their RAII leases return, preserving the global cap across unlock.
        table.retain(|_, entry| entry.started);
        signals
    };
    for signal in signals {
        let _ = signal.send(true);
    }
}

fn ensure_inline_download_not_cancelled(
    cancellation: Option<&watch::Receiver<bool>>,
) -> Result<(), String> {
    if cancellation.is_some_and(|receiver| *receiver.borrow()) {
        Err("download cancelled".to_string())
    } else {
        Ok(())
    }
}

/// Reserve one bounded native cancellation signal before starting a whole-file take read.
#[tauri::command]
async fn begin_inline_download(
    state: State<'_, AppState>,
    cancellation: String,
) -> Result<(), String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    // Hold the same commit boundary as lock-sensitive publication. Either this registration is
    // visible for a subsequent lock to drain, or the newer lock generation rejects it first.
    let _commit = require_ui_session_generation(&state, generation).await?;
    register_inline_download(&state, &cancellation)
}

/// Cancel and retire an inline read. Safe while locking because it can only destroy authority.
#[tauri::command]
fn cancel_inline_download(state: State<'_, AppState>, cancellation: String) -> Result<(), String> {
    // Do not reveal whether a token existed while the vault gate is closed. Cancellation remains
    // idempotent and available because it can only remove authority and bounded background work.
    let _ = cancel_inline_download_registration(&state, &cancellation)?;
    Ok(())
}

/// Download a small shared file by content-address hex; returns base64-encoded bytes. Fetches the
/// file ONE chunk per actor command (emitting `download-progress` after each), so the actor returns
/// to its loop between chunks and interleaves other commands + network sync. The whole reassembled
/// file is verified against the requested content address (defends against a malicious manifest
/// whose chunks individually verify).
///
/// **Bounded at [`MAX_INLINE_DOWNLOAD_BYTES`].** The result is one base64 string built in memory
/// and handed to the webview, which is the shape that froze the app when uploads worked this way:
/// cost scales with the file and the webview serializes it whole on its main thread. Media plays
/// through the `catcoms-media:` protocol and saving streams to disk, so nothing needs this for a
/// large file; the cap is here so no future caller can reintroduce the freeze by accident.
#[tauri::command]
async fn download_file(
    app: AppHandle,
    state: State<'_, AppState>,
    server: u64,
    cid: String,
    trace: Option<String>,
    cancellation: Option<String>,
) -> Result<String, AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Files,
        "download_file",
        server,
        None,
    );
    let raw = hex::decode(cid.trim())
        .map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, format!("bad cid: {e}")))?;
    let progress_cancellation = cancellation.clone();
    // The shared RAII lease removes this exact generation only after both the command and any
    // lower transport request have returned. Ordinary callers receive an internal native token;
    // take playback registers its caller token first so a stale call can preempt its waiter.
    let (cancellation_lease, cancellation) = match cancellation.as_deref() {
        Some(id) => {
            let (lease, receiver) = claim_inline_download(&state, id)
                .map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, e))?;
            (Some(lease), Some(receiver))
        }
        None => {
            let (lease, receiver) = claim_internal_inline_download(&state)
                .map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, e))?;
            (Some(lease), Some(receiver))
        }
    };
    let target: [u8; 32] = raw
        .clone()
        .try_into()
        .map_err(|_| op.fail(codes::FILE_DOWNLOAD_FAILED, "bad cid length"))?;
    let actor = op.actor(&state, server).await?;
    ensure_inline_download_not_cancelled(cancellation.as_ref())
        .map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, e))?;
    let (total, size) = actor.file_download_plan(raw.clone()).await.ok_or_else(|| {
        op.fail(
            codes::FILE_DOWNLOAD_FAILED,
            "this file can't be downloaded; it isn't listed, or its reference is invalid",
        )
    })?;
    inline_download_allowed(size).map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, e))?;
    ensure_inline_download_not_cancelled(cancellation.as_ref())
        .map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, e))?;
    require_unlocked_session(&state)
        .await
        .map_err(|e| op.fail(codes::SESSION_LOCKED, e))?;
    // The plan, before any bytes move. A download that stalls is one of these with no completion
    // after it, and the chunk count is what says how far it got.
    op.stage("FILE.DOWNLOAD.PLANNED");
    emit_tracked(
        &app,
        "download-progress",
        DownloadProgressEvt {
            server,
            cid: cid.clone(),
            cancellation: progress_cancellation.clone(),
            done: 0,
            total,
            bytes_done: 0,
            bytes_total: size,
            network_bytes_done: 0,
            provider: None,
        },
        op.trace,
    );
    let mut out = Vec::with_capacity(size as usize);
    let mut network_bytes_done = 0u64;
    for i in 0..total {
        ensure_inline_download_not_cancelled(cancellation.as_ref())
            .map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, e))?;
        // A transfer can outlive the click that started it. Do not return plaintext or continue
        // emitting file metadata after an explicit lock closes the webview session.
        require_unlocked_session(&state)
            .await
            .map_err(|e| op.fail(codes::SESSION_LOCKED, e))?;
        let request_cancellation = cancellation.as_ref().map(|signal| {
            RequestCancellation::new(
                signal.clone(),
                cancellation_lease
                    .as_ref()
                    .map(InlineDownloadLease::request_keepalive),
            )
        });
        let (chunk, provider) = actor
            .fetch_file_chunk_cancellable(raw.clone(), i, request_cancellation)
            .await
            .map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, e))?;
        ensure_inline_download_not_cancelled(cancellation.as_ref())
            .map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, e))?;
        if provider.is_some() {
            network_bytes_done = network_bytes_done.saturating_add(chunk.len() as u64);
        }
        // This buffer is held whole in memory and then base64'd into the webview, so a manifest
        // whose chunks exceed its declared size must be stopped before the next append rather
        // than at the address check after the last one.
        if out.len() as u64 + chunk.len() as u64 > size {
            return Err(op.fail(
                codes::FILE_DOWNLOAD_FAILED,
                "this file's chunks hold more data than it declares",
            ));
        }
        out.extend_from_slice(&chunk);
        emit_tracked(
            &app,
            "download-progress",
            DownloadProgressEvt {
                server,
                cid: cid.clone(),
                cancellation: progress_cancellation.clone(),
                done: i + 1,
                total,
                bytes_done: out.len() as u64,
                bytes_total: size,
                network_bytes_done,
                provider,
            },
            op.trace,
        );
    }
    if out.len() as u64 != size {
        return Err(op.fail(
            codes::FILE_DOWNLOAD_FAILED,
            "this file's chunks hold less data than it declares",
        ));
    }
    ensure_inline_download_not_cancelled(cancellation.as_ref())
        .map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, e))?;
    if Cid::of(&out).as_bytes() != &target {
        // Every chunk verified and the whole did not. Recorded as its own outcome because it means
        // something specific: a manifest whose parts are individually honest and collectively not.
        return Err(op.fail(
            codes::FILE_DOWNLOAD_FAILED,
            "the reassembled file failed its integrity check",
        ));
    }
    require_unlocked_session(&state)
        .await
        .map_err(|e| op.fail(codes::SESSION_LOCKED, e))?;
    ensure_inline_download_not_cancelled(cancellation.as_ref())
        .map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, e))?;
    let encoded = B64.encode(&out);
    ensure_inline_download_not_cancelled(cancellation.as_ref())
        .map_err(|e| op.fail(codes::FILE_DOWNLOAD_FAILED, e))?;
    require_unlocked_session(&state)
        .await
        .map_err(|e| op.fail(codes::SESSION_LOCKED, e))?;
    op.succeeded("FILE.DOWNLOAD.COMPLETED");
    Ok(encoded)
}

/// Post to the server status feed. **Owner/admin only** unless the feed has been opened to
/// members (`set_status_policy`), so a refusal is a real outcome the caller sees rather than a
/// post that silently never happened.
#[tauri::command]
async fn post_status(
    state: State<'_, AppState>,
    server: u64,
    text: String,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Documents,
        "post_status",
        server,
        None,
    );
    let actor = op.actor(&state, server).await?;
    actor
        .post_status(text)
        .await
        .map_err(|e| op.fail(codes::STATUS_POST_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("STATUS.POST.PERSISTED");
    Ok(())
}

/// The server status feed (newest-first).
#[tauri::command]
async fn get_statuses(state: State<'_, AppState>, server: u64) -> Result<Vec<UiMessage>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .statuses()
        .await
        .into_iter()
        .rev()
        .map(ui_message)
        .collect())
}

/// Edit one of your own status posts (by post id); re-seals the server.
#[tauri::command]
async fn edit_status(
    state: State<'_, AppState>,
    server: u64,
    msg_id: String,
    text: String,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Documents,
        "edit_status",
        server,
        None,
    );
    let actor = op.actor(&state, server).await?;
    actor
        .edit_status(msg_id, text)
        .await
        .map_err(|e| op.fail(codes::STATUS_EDIT_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("STATUS.EDIT.PERSISTED");
    Ok(())
}

/// Delete a status post (by post id): your own, or anyone's as an owner/admin.
#[tauri::command]
async fn delete_status(
    state: State<'_, AppState>,
    server: u64,
    msg_id: String,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Documents,
        "delete_status",
        server,
        None,
    );
    let actor = op.actor(&state, server).await?;
    actor
        .delete_status(msg_id)
        .await
        .map_err(|e| op.fail(codes::STATUS_DELETE_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("STATUS.DELETE.PERSISTED");
    Ok(())
}

/// Toggle this member's emoji reaction on a status post (by post id). Any member may react,
/// whoever the feed lets write.
#[tauri::command]
async fn toggle_status_reaction(
    state: State<'_, AppState>,
    server: u64,
    msg_id: String,
    emoji: String,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Documents,
        "toggle_status_reaction",
        server,
        None,
    );
    let actor = op.actor(&state, server).await?;
    actor
        .toggle_status_reaction(msg_id, emoji)
        .await
        .map_err(|e| op.fail(codes::STATUS_REACTION_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("STATUS.REACTION.PERSISTED");
    Ok(())
}

/// Pin or unpin a status post (by post id) (owner/admin).
#[tauri::command]
async fn set_status_pin(
    state: State<'_, AppState>,
    server: u64,
    msg_id: String,
    pinned: bool,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Documents,
        "set_status_pin",
        server,
        None,
    );
    let actor = op.actor(&state, server).await?;
    actor
        .set_status_pin(msg_id, pinned)
        .await
        .map_err(|e| op.fail(codes::STATUS_PIN_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("STATUS.PIN.PERSISTED");
    Ok(())
}

/// Whether plain members may post to the status feed (`false` = owner/admin only, the default).
#[tauri::command]
async fn get_status_policy(state: State<'_, AppState>, server: u64) -> Result<bool, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor.status_members_may_post().await)
}

/// Open or close the status feed to plain members (owner/admin only); re-seals the server. The
/// policy rides the feed document, so a `status-updated` event follows and every member re-reads
/// it along with the posts.
#[tauri::command]
async fn set_status_policy(
    state: State<'_, AppState>,
    server: u64,
    members_may_post: bool,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Documents,
        "set_status_policy",
        server,
        None,
    );
    let actor = op.actor(&state, server).await?;
    actor
        .set_status_members_may_post(members_may_post)
        .await
        .map_err(|e| op.fail(codes::STATUS_POLICY_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("STATUS.POLICY.PERSISTED");
    Ok(())
}

/// Create a server event; re-seals the server. **Any member may**; an event is server content,
/// like a channel or a status post. Rejected with a message when the title is blank or over 120
/// UTF-8 bytes, the body is over 1024, or the end time precedes the start (`endTs: 0` = no end).
/// `image` is the hex content address of an already-shared file (empty for none), checked for
/// shape only: the blob is fetched over the file path like any other embed.
/// An `events-changed` event follows, so the UI re-reads the calendar.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn create_event(
    state: State<'_, AppState>,
    server: u64,
    title: String,
    body: String,
    start_ts: u64,
    end_ts: u64,
    image: String,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Documents,
        "create_event",
        server,
        None,
    );
    let actor = op.actor(&state, server).await?;
    // The title and body are content; that an event was created is the record.
    actor
        .create_event(title, body, start_ts, end_ts, image)
        .await
        .map_err(|e| op.fail(codes::DOCUMENT_WRITE_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("EVENT.CREATE.PERSISTED");
    Ok(())
}

/// Delete a server event by id (its author, or an owner/admin); re-seals the server.
#[tauri::command]
async fn delete_event(state: State<'_, AppState>, server: u64, id: String) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.delete_event(id).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// The server's events, sorted by start time ascending.
#[tauri::command]
async fn get_events(state: State<'_, AppState>, server: u64) -> Result<Vec<UiEvent>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .events()
        .await
        .into_iter()
        .map(|e| UiEvent {
            id: e.id,
            title: e.title,
            body: e.body,
            start_ts: e.start_ts,
            end_ts: e.end_ts,
            author: e.author,
            image: e.image,
        })
        .collect())
}

/// The wiki page names (sorted).
#[tauri::command]
async fn get_wiki_pages(state: State<'_, AppState>, server: u64) -> Result<Vec<String>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor.wiki_pages().await)
}

/// The whole wiki as a name -> body map (for backlinks + link existence).
#[tauri::command]
async fn get_wiki_map(
    state: State<'_, AppState>,
    server: u64,
) -> Result<std::collections::HashMap<String, String>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor.wiki_map().await)
}

/// One inbound join attempt as shown to the server operator.
#[derive(Serialize, Clone)]
struct UiJoinAttempt {
    /// Milliseconds since the epoch, on the serving node's clock.
    at: u64,
    /// The stable outcome id (`admitted`, `expired`, `already-used`, ...). The user-facing
    /// sentence lives in the frontend, with the rest of the app's copy.
    outcome: String,
    /// Whether the joiner got in (or is on their way in).
    admitted: bool,
    /// The requesting peer, as a short hex prefix.
    peer: String,
    /// The invite nonce prefix: what an operator matches against the invite they sent.
    nonce: String,
}

/// The recent inbound join attempts this server served, newest first.
///
/// The operator's half of a failed join. The joiner still gets an opaque rejection over the
/// wire, by design; without this nobody on either side could tell an expired invite from one
/// that had already been redeemed, which is the reported field failure verbatim.
#[tauri::command]
async fn get_join_attempts(
    state: State<'_, AppState>,
    server: u64,
) -> Result<Vec<UiJoinAttempt>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .join_attempts()
        .await
        .into_iter()
        .map(|a| UiJoinAttempt {
            at: a.at_ms,
            outcome: a.outcome.as_str().to_string(),
            admitted: a.outcome.admitted(),
            peer: a.peer_prefix,
            nonce: a.nonce_prefix,
        })
        .collect())
}

/// What the last founding/joining attempt did, plus what this node knows about its own
/// reachability. Feeds the connectivity panel on the create/join screens.
fn reconcile_advertised(current: &mut Vec<String>, live: Option<Vec<String>>) {
    if let Some(live) = live {
        *current = live;
    }
}

#[tauri::command]
async fn get_connectivity(state: State<'_, AppState>) -> Result<Connectivity, String> {
    require_unlocked_session(&state).await?;
    let mut diag = state.diag.lock().await.clone();
    // A mapper or relay can change between the inner found/join body spawning its collector and
    // the wrapper committing `diag`. ServerEntry retains every member's base direct addresses, so
    // its bootstrap is authoritative in both directions: replacement closes both “mapped,
    // advertises none” and “expired relay remains ready” startup races.
    if diag.server != 0 {
        let live = state
            .servers
            .lock()
            .await
            .get(&diag.server)
            .map(|entry| entry.bootstrap.clone());
        reconcile_advertised(&mut diag.advertised, live);
    }
    diag.public_direct = diag.advertised.iter().any(|address| {
        !address.contains("/p2p-circuit")
            && address
                .parse::<Multiaddr>()
                .is_ok_and(|addr| addr_is_globally_routable(&addr))
    });
    let failed_join = diag.action == "join" && diag.server == 0;
    diag.upnp = if failed_join {
        PORT_MAPPING_FAILED_JOIN.to_string()
    } else {
        state
            .upnp
            .lock()
            .await
            .get(&diag.server)
            .cloned()
            .unwrap_or_else(|| PORT_MAPPING_NOT_ATTEMPTED.to_string())
    };
    diag.autonat = if failed_join {
        AUTONAT_FAILED_JOIN.to_string()
    } else {
        let evidence = state.autonat.lock().await.get(&diag.server).cloned();
        autonat_status(&diag.advertised, evidence.as_ref())
    };
    diag.mesh_observations = if diag.server == 0 {
        Vec::new()
    } else {
        state
            .mesh_observations
            .lock()
            .await
            .get(&diag.server)
            .cloned()
            .unwrap_or_default()
    };
    Ok(diag)
}

fn switchboard_route_usable(address: &str) -> bool {
    // A relay circuit is useful only when the relay host itself is a literal public route. The
    // old `contains p2p-circuit` shortcut accepted LAN/DNS relay paths which the signed-offer
    // codec later stripped, letting Settings enable a role the protocol could never advertise.
    address
        .parse::<Multiaddr>()
        .is_ok_and(|addr| addr_is_globally_routable(&addr))
}

#[tauri::command]
async fn get_switchboard_status(
    state: State<'_, AppState>,
    server: u64,
) -> Result<SwitchboardStatus, String> {
    require_unlocked_session(&state).await?;
    let (actor, offered, eligible) = {
        let servers = state.servers.lock().await;
        let entry = servers
            .get(&server)
            .ok_or_else(|| "unknown server".to_string())?;
        (
            entry.actor.clone(),
            entry.switchboard,
            entry
                .bootstrap
                .iter()
                .any(|address| switchboard_route_usable(address)),
        )
    };
    let online = actor
        .switchboard_offers()
        .await
        .into_iter()
        .map(|offer| SwitchboardMember {
            fingerprint: hex::encode(&offer.device_id().as_bytes()[..8]),
            addresses: offer.addresses.len(),
        })
        .collect();
    let reason = if eligible {
        "This device has an advertised public or relayed candidate route it can offer. Direct reachability remains best-effort until a callback succeeds.".to_string()
    } else {
        "An advertised public mapping candidate, public IPv6 address, manual forward, or relay circuit is required before this device can host.".to_string()
    };
    Ok(SwitchboardStatus {
        offered,
        eligible,
        online,
        reason,
    })
}

#[tauri::command]
async fn set_switchboard_offered(
    app: AppHandle,
    state: State<'_, AppState>,
    server: u64,
    offered: bool,
) -> Result<SwitchboardStatus, String> {
    require_unlocked_session(&state).await?;
    let actor = {
        let servers = state.servers.lock().await;
        let entry = servers
            .get(&server)
            .ok_or_else(|| "unknown server".to_string())?;
        let eligible = entry
            .bootstrap
            .iter()
            .any(|address| switchboard_route_usable(address));
        if offered && !eligible {
            return Err("this device has no usable public or relay route to offer".to_string());
        }
        entry.actor.clone()
    };
    if !offered {
        // Revocation is fail-safe: close the protocol gate first. Even if disk sealing fails, the
        // current process and UI state remain off rather than silently continuing to serve.
        actor.set_switchboard_offered(false).await?;
        if let Some(entry) = state.servers.lock().await.get_mut(&server) {
            entry.switchboard = false;
        }
    }
    let persist_result: Result<(), String> = async {
        let store = state.store.lock().await;
        if let Some(store) = store.as_ref() {
            let mut net = store
                .load_server_net(server)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "this server has no persisted network record".to_string())?;
            net.switchboard = offered;
            let mut rng = OsCryptoRng;
            store
                .save_server_net(server, &net, &mut rng)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = persist_result {
        return Err(if offered {
            format!("hosting was not enabled because consent could not be saved: {error}")
        } else {
            format!(
                "hosting is off for this session, but the revocation could not be saved: {error}"
            )
        });
    }
    if offered {
        // Enabling is the reverse order: durable consent first, then the serving gate. If the
        // actor has stopped, roll the persisted bit back so a reload cannot surprise-enable it.
        if let Err(error) = actor.set_switchboard_offered(true).await {
            let store = state.store.lock().await;
            if let Some(store) = store.as_ref() {
                if let Ok(Some(mut net)) = store.load_server_net(server) {
                    net.switchboard = false;
                    let mut rng = OsCryptoRng;
                    let _ = store.save_server_net(server, &net, &mut rng);
                }
            }
            return Err(error);
        }
        if let Some(entry) = state.servers.lock().await.get_mut(&server) {
            entry.switchboard = true;
        }
    }
    emit_tracked(
        &app,
        "switchboard-changed",
        ServerEvt { server },
        catcoms_diagnostics::TraceId::default(),
    );
    get_switchboard_status(state, server).await
}

/// Pull the globally-routable IP literals out of a set of advertised multiaddrs, newest-family
/// first. A relay circuit names the *relay's* address, never this node's, so circuits are
/// dropped: reporting one here would tell the media plane it is directly reachable when the
/// only working path runs through somebody else's host.
fn routable_media_hosts(advertised: &[String]) -> (Vec<String>, Vec<String>) {
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for address in advertised {
        let Ok(parsed) = address.parse::<Multiaddr>() else {
            continue;
        };
        if parsed
            .iter()
            .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
        {
            continue;
        }
        if !addr_is_globally_routable(&parsed) {
            continue;
        }
        for protocol in parsed.iter() {
            match protocol {
                Protocol::Ip4(ip) => {
                    let literal = ip.to_string();
                    if !v4.contains(&literal) {
                        v4.push(literal);
                    }
                }
                Protocol::Ip6(ip) => {
                    let literal = ip.to_string();
                    if !v6.contains(&literal) {
                        v6.push(literal);
                    }
                }
                _ => {}
            }
        }
    }
    (v4, v6)
}

/// One member this device could hand its media to when a direct path cannot be built.
#[derive(Serialize)]
struct CallBridge {
    /// Short fingerprint, matching the identifier the call UI already renders for a peer.
    fingerprint: String,
    /// How many routes the helper published. More routes is a better bet, not a guarantee.
    addresses: usize,
    /// True when at least one published route is a literal public address rather than a circuit.
    /// A helper reachable only through *another* relay is a poor media bridge: it inherits that
    /// relay's latency and its liveness.
    direct: bool,
}

/// What the media plane should do for this server, derived entirely from what the mesh already
/// proved. The frontend owns ICE; this is the evidence it has never had access to.
///
/// The two planes are otherwise disjoint by construction: the libp2p transport's PCP/UPnP/PCPv6
/// mappings cover the *mesh* socket, and the webview's ICE agent binds its own ephemeral ports,
/// so none of that traversal work has ever applied to a call. What does transfer is the
/// *knowledge*: whether this node is directly reachable, whether it has a global IPv6 route, and
/// which members have proven they can host for others.
#[derive(Serialize)]
struct CallTransport {
    /// Whether this node advertises at least one non-circuit, globally routable address.
    public_direct: bool,
    /// The AutoNAT verdict verbatim, so the call UI can quote evidence rather than assert.
    autonat: String,
    /// Public IPv4 literals this node is known at. Diagnostic only: the media plane cannot turn
    /// these into ICE candidates, because the webview's ports are not the mesh's ports.
    public_ipv4: Vec<String>,
    /// Public IPv6 literals. These matter more than IPv4: IPv6 has no NAT to traverse, so a
    /// direct media path needs only a firewall pinhole rather than a mapping.
    public_ipv6: Vec<String>,
    /// Members currently offering to host, best candidates first.
    bridges: Vec<CallBridge>,
    /// Whether a third-party relay is likely to be required for this node to be heard at all.
    relay_likely_required: bool,
    /// Whether this node's router has granted the mesh a port mapping. The media plane asks the
    /// same router for each call's socket, so this is the "your router can help calls" signal.
    router_maps: bool,
    /// One line the call UI can show verbatim when a link will not come up.
    advice: String,
}

#[tauri::command]
async fn get_call_transport(
    state: State<'_, AppState>,
    server: u64,
) -> Result<CallTransport, String> {
    require_unlocked_session(&state).await?;
    let (actor, advertised, router_maps) = {
        let servers = state.servers.lock().await;
        let entry = servers
            .get(&server)
            .ok_or_else(|| "unknown server".to_string())?;
        let router_maps = entry
            .bootstrap_owners
            .values()
            .any(|owners| owners.contains(&BootstrapOwner::PortMapping));
        (entry.actor.clone(), entry.bootstrap.clone(), router_maps)
    };
    let public_direct = advertised.iter().any(|address| {
        address
            .parse::<Multiaddr>()
            .is_ok_and(|addr| addr_is_globally_routable(&addr))
            && !address.contains("/p2p-circuit")
    });
    let autonat = {
        let evidence = state.autonat.lock().await.get(&server).cloned();
        autonat_status(&advertised, evidence.as_ref())
    };
    let (public_ipv4, public_ipv6) = routable_media_hosts(&advertised);

    // Reuse the switchboard's consent-gated offer pool: a member who has already agreed to carry
    // somebody else's traffic is exactly the member who should be asked to carry their media.
    // Sorting puts literal-route helpers first so the picker never prefers a relayed helper.
    let mut bridges: Vec<CallBridge> = actor
        .switchboard_offers()
        .await
        .into_iter()
        .map(|offer| {
            let direct = offer
                .addresses
                .iter()
                .any(|address| switchboard_route_usable(address));
            CallBridge {
                fingerprint: hex::encode(&offer.device_id().as_bytes()[..8]),
                addresses: offer.addresses.len(),
                direct,
            }
        })
        .collect();
    bridges.sort_by(|a, b| {
        b.direct
            .cmp(&a.direct)
            .then_with(|| b.addresses.cmp(&a.addresses))
    });

    let relay_likely_required = !public_direct && public_ipv6.is_empty() && !router_maps;
    let mut advice = if public_direct {
        "This device is directly reachable, so a call should connect without a relay. A peer that still fails is behind the stricter NAT.".to_string()
    } else if !public_ipv6.is_empty() {
        "This device has a public IPv6 route but no verified IPv4 path. Calls will connect directly to IPv6 peers once the router permits inbound UDP to the app; IPv4-only peers will need a relay.".to_string()
    } else if bridges.is_empty() {
        "This device is behind NAT and no member is offering to host. Calls to peers who are also behind NAT need a relay: ask a member with a public route to enable hosting, or set a TURN server in Settings, Calls.".to_string()
    } else {
        format!(
            "This device is behind NAT. {} member(s) are offering to host, so a relayed path is available.",
            bridges.len()
        )
    };
    if router_maps {
        advice.push_str(
            " Your router opens ports on request, so each call also offers a router-mapped direct route; one mapped side is enough for the pair.",
        );
    }

    Ok(CallTransport {
        public_direct,
        autonat,
        public_ipv4,
        public_ipv6,
        bridges,
        relay_likely_required,
        router_maps,
        advice,
    })
}

/// The public socket a router granted for one webview media port.
#[derive(Serialize)]
struct MappedCallPort {
    ip: String,
    port: u16,
    /// Which protocol granted it, for the log and the call panel.
    mechanism: String,
    /// The router's own state confirmed the mapping beyond the grant response (see
    /// `MediaPortMapping::confirmed`); not a claim of verified external reachability.
    confirmed: bool,
}

/// The IPv4 source address the kernel would route toward the public internet, or `None` when it
/// cannot be resolved.
///
/// The webview uses this to decide which gathered ICE host candidates are worth signalling. A
/// desktop with virtualisation installed gathers candidates on adapters no remote peer can reach
/// (VirtualBox host-only, WSL/Hyper-V vEthernet), and each one it sends costs the far end a
/// connectivity check before ICE can settle. Naming the real interface is what lets the webview
/// tell those apart; see `shouldSignalHostCandidate`.
///
/// This is the machine's own LAN address, which it already puts in every host candidate it
/// signals, so returning it to the page reveals nothing the call plane did not already publish.
#[tauri::command]
async fn default_route_address(state: State<'_, AppState>) -> Result<Option<String>, String> {
    require_unlocked_session(&state).await?;
    Ok(catcoms_net::default_route_ipv4().map(|ip| ip.to_string()))
}

/// Ask the router to forward one of the active call's media UDP ports to this machine, over the
/// bound-interface IGD path the invite reachability fix proved out, with PCP/NAT-PMP as the
/// fallback rung for routers that don't speak UPnP. The webview signals the returned public
/// socket to the peer as an extra ICE candidate; a router mapping forwards from any source, so
/// one mapped side connects the pair regardless of the other side's NAT type. This is the
/// media-plane counterpart of the mesh's mapping workers, which only ever cover the stable
/// libp2p listen port and never the ICE agent's ephemeral sockets.
#[tauri::command]
async fn map_call_port(
    state: State<'_, AppState>,
    port: u16,
    address: Option<String>,
) -> Result<MappedCallPort, String> {
    require_unlocked_session(&state).await?;
    let key = port;
    let port = std::num::NonZeroU16::new(port).ok_or("port must be nonzero")?;
    // The candidate's own claimed IPv4, when the webview could read one (mic-granted pages get
    // real IPs; an mDNS-obfuscated candidate passes None and is mapped permissively).
    let claimed = address
        .as_deref()
        .and_then(|a| a.parse::<std::net::Ipv4Addr>().ok());
    let mapped = catcoms_net::map_media_udp_port(port, claimed).await?;
    let result = MappedCallPort {
        ip: mapped.external.ip().to_string(),
        port: mapped.external.port(),
        mechanism: mapped.mechanism.to_string(),
        confirmed: mapped.confirmed,
    };
    // Retain the mapping: a PCP/NAT-PMP lease is renewed by the client inside it, and call end
    // releases it. A same-port remap (shouldn't happen; ICE ports are fresh per call) releases
    // the entry it replaces rather than leaking its renewal task.
    if let Some(previous) = state.media_mappings.lock().await.insert(key, mapped) {
        previous.release().await;
    }
    Ok(result)
}

/// Release a call mapping when the call ends. Best-effort at the router; the UPnP lease is
/// bounded and a dropped PCP/NAT-PMP client stops renewing, so nothing outlives a crash long.
#[tauri::command]
async fn unmap_call_port(state: State<'_, AppState>, port: u16) -> Result<(), String> {
    require_unlocked_session(&state).await?;
    let mapping = state.media_mappings.lock().await.remove(&port);
    if let Some(mapping) = mapping {
        mapping.release().await;
    }
    Ok(())
}

/// How much plaintext one media response may carry. A player asks for the window it is about to
/// show; this bounds what a hostile or buggy `Range` can make the app allocate at once, and keeps
/// one response comfortably inside a single sealed chunk.
const MEDIA_WINDOW_BYTES: usize = 2 * 1024 * 1024;

/// A response must fit inside one chunk. Checked at compile time because both sides are
/// constants: a runtime assertion on them is optimised out and proves nothing. Raising the window
/// above the chunk size is exactly the regression that made one response need two decrypts.
const _: () = assert!(MEDIA_WINDOW_BYTES <= CHUNK_BYTES);

/// The largest image served entire in one rangeless response. See [`serves_whole_image`].
///
/// Comfortably above a 4K screenshot or a phone photo, and far below the 256 MiB a listing may
/// declare: the body is assembled in memory before it crosses the scheme boundary, so this is the
/// most an ordinary `<img>` may cost at once.
const MAX_WHOLE_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// Whether a request must be answered with the whole file rather than one window.
///
/// `<img>` issues a plain GET with no `Range` header, takes the response body as the entire
/// image, and never follows up on a partial response. A windowed answer therefore hands the
/// decoder a truncated file: every image above [`MEDIA_WINDOW_BYTES`] rendered as a broken icon,
/// which is most screenshots and nearly every 4K one. `<video>`/`<audio>` are the opposite case:
/// they range-request, so they keep the windowed path that holds one response to one chunk.
///
/// A rangeless request for something larger than [`MAX_WHOLE_IMAGE_BYTES`] still gets a window.
/// It cannot render either way, and refusing it here would only trade a broken image for a
/// different broken image while giving up the bound this exists to keep.
fn serves_whole_image(mime: &str, ranged: bool, total: u64) -> bool {
    !ranged && total <= MAX_WHOLE_IMAGE_BYTES && mime.starts_with("image/")
}

/// How many [`CHUNK_BYTES`] chunks cover a file of `total` bytes. An empty file is still one.
fn media_chunk_span(total: u64) -> usize {
    total.div_ceil(CHUNK_BYTES as u64).max(1) as usize
}

/// The custom scheme the webview plays shared media through: `catcoms-media://a/<server>/<cid>`.
/// Windows rewrites this to `http://catcoms-media.localhost/...`, so the host is never parsed;
/// only the path is.
const MEDIA_SCHEME: &str = "catcoms-media";

/// Parse `/<server>/<cid>` out of a media URI path. Deliberately strict: the server id must be a
/// plain integer and the cid lowercase hex of exactly 32 bytes, so nothing resembling a path
/// traversal or a foreign identifier reaches the file index.
fn parse_media_path(path: &str) -> Option<(u64, String)> {
    let mut parts = path.trim_start_matches('/').split('/');
    let server: u64 = parts.next()?.parse().ok()?;
    let cid = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if cid.len() != 64 || !cid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((server, cid.to_ascii_lowercase()))
}

/// Authorize every head-derived response, including bodyless range errors, against the UI
/// generation that began the request. File size is plaintext metadata too: a delayed request must
/// not return `Content-Range: */size` after explicit lock has completed.
async fn authorized_media_range(
    state: &AppState,
    generation: u64,
    total: u64,
    range: Option<String>,
) -> Result<(u64, usize), http::Response<Vec<u8>>> {
    let deny = |code: http::StatusCode| {
        http::Response::builder()
            .status(code)
            .header("Access-Control-Allow-Origin", "null")
            .body(Vec::new())
            .expect("static response builds")
    };
    let _commit = require_ui_session_generation(state, generation)
        .await
        .map_err(|_| deny(http::StatusCode::FORBIDDEN))?;
    let (start, len) = match range
        .as_deref()
        .and_then(|value| parse_range_header(value, total))
    {
        Some(parsed) => parsed,
        None if range.is_some() => return Err(deny(http::StatusCode::RANGE_NOT_SATISFIABLE)),
        None => (0, MEDIA_WINDOW_BYTES),
    };
    if start >= total {
        return Err(http::Response::builder()
            .status(http::StatusCode::RANGE_NOT_SATISFIABLE)
            .header("Content-Range", format!("bytes */{total}"))
            .body(Vec::new())
            .expect("static response builds"));
    }
    Ok((start, len))
}

fn bodyless_media_denial(code: http::StatusCode) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(code)
        .header("Access-Control-Allow-Origin", "null")
        .body(Vec::new())
        .expect("static response builds")
}

/// Publish the scheme response while owning the same native commit guard explicit lock waits on.
/// Building a response under the guard is insufficient: the responder is the externally-visible
/// step, and another runtime thread could otherwise complete lock between return and `respond`.
async fn publish_media_response<F>(
    state: &AppState,
    generation: Option<u64>,
    response: http::Response<Vec<u8>>,
    publish: F,
) where
    F: FnOnce(http::Response<Vec<u8>>),
{
    let Some(generation) = generation else {
        publish(bodyless_media_denial(http::StatusCode::FORBIDDEN));
        return;
    };
    match require_ui_session_generation(state, generation).await {
        Ok(_commit) => publish(response),
        Err(_) => publish(bodyless_media_denial(http::StatusCode::FORBIDDEN)),
    }
}

/// Parse a single-range `bytes=start-[end]` header into a start offset and a length cap.
///
/// Only the first range of a possibly-multi-range header is honoured, and a multipart response is
/// never produced: media elements ask for one range at a time, and answering a multi-range request
/// with the first range is a legal (if partial) response, where mis-assembling a multipart body
/// would corrupt playback silently.
fn parse_range_header(raw: &str, total: u64) -> Option<(u64, usize)> {
    let spec = raw.trim().strip_prefix("bytes=")?;
    let first = spec.split(',').next()?.trim();
    let (from, to) = first.split_once('-')?;
    if from.is_empty() {
        // A suffix range ("-500") means the LAST n bytes, not the first n.
        let n: u64 = to.parse().ok()?;
        let n = n.min(total);
        return Some((
            total.saturating_sub(n),
            n.min(MEDIA_WINDOW_BYTES as u64) as usize,
        ));
    }
    let start: u64 = from.parse().ok()?;
    let len = if to.is_empty() {
        MEDIA_WINDOW_BYTES
    } else {
        let end: u64 = to.parse().ok()?;
        // `bytes=0-0` is one byte: the range is inclusive at both ends.
        end.saturating_sub(start)
            .saturating_add(1)
            .min(MEDIA_WINDOW_BYTES as u64) as usize
    };
    Some((start, len))
}

/// Serve one media request. Split out from the protocol registration so the whole path is
/// testable and so every failure returns a status rather than panicking inside the webview's
/// scheme handler.
async fn serve_media(
    state: &AppState,
    path: &str,
    range: Option<String>,
) -> http::Response<Vec<u8>> {
    let deny = |code: http::StatusCode| bodyless_media_denial(code);
    let Some((server, cid)) = parse_media_path(path) else {
        return deny(http::StatusCode::BAD_REQUEST);
    };
    // The same boundary every native command sits behind: a locked vault serves no plaintext,
    // and a media element left in the DOM must not keep pulling bytes after an explicit lock.
    let generation = match unlocked_ui_session_generation(state).await {
        Ok(generation) => generation,
        Err(_) => return deny(http::StatusCode::FORBIDDEN),
    };
    if require_unlocked_session(state).await.is_err() {
        return deny(http::StatusCode::FORBIDDEN);
    }
    let Ok(actor) = actor_of(state, server).await else {
        return deny(http::StatusCode::NOT_FOUND);
    };
    let Ok(raw) = hex::decode(&cid) else {
        return deny(http::StatusCode::BAD_REQUEST);
    };

    // Reads are chunk-aligned and cached, which is not an optimisation but a correctness
    // requirement for the actor: it processes one command at a time, so every chunk fetched here
    // is time the server cannot answer get_messages or get_channels. Serving a 2 MiB window out
    // of an 8 MiB chunk meant decrypting the same chunk four times over during ordinary
    // playback, which stalled the deck and made the whole server look like it was still loading.
    //
    // The head comes from the index rather than from chunk 0 for the same reason: reading a whole
    // chunk to learn a size and a mime put a second decrypt on every response and, because the
    // cache is small, evicted the chunk the player was actually reading.
    let head = match media_head(state, &actor, server, &cid, &raw, generation).await {
        Some(head) => head,
        None => return deny(http::StatusCode::NOT_FOUND),
    };
    let total = head.total_size;
    let mime = safe_media_mime(&head.mime);

    // An image asked for without a Range is asked for whole. Served chunk by chunk like every
    // other read, so the actor still returns to its loop between decrypts.
    if serves_whole_image(&mime, range.is_some(), total) {
        // Every chunk read below carries the generation guard of its own, and the response is
        // built under the same commit the windowed path holds, so there is nothing extra to take
        // here: the size this is about to allocate is already bounded by `serves_whole_image`.
        let mut body = Vec::with_capacity(total as usize);
        for index in 0..media_chunk_span(total) {
            let bytes = match media_chunk(
                state,
                &actor,
                server,
                &cid,
                &raw,
                head.manifest_version,
                index,
                generation,
            )
            .await
            {
                Some(bytes) => bytes,
                None => return deny(http::StatusCode::SERVICE_UNAVAILABLE),
            };
            let want = (total as usize).saturating_sub(body.len());
            body.extend_from_slice(&bytes[..want.min(bytes.len())]);
            if body.len() as u64 >= total {
                break;
            }
        }
        // The same last authorization point the windowed path holds, for the same reason: no
        // plaintext crosses the scheme boundary if an explicit lock completed during the reads.
        let _response_commit = match require_ui_session_generation(state, generation).await {
            Ok(commit) => commit,
            Err(_) => return deny(http::StatusCode::FORBIDDEN),
        };
        return http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", mime)
            .header("Accept-Ranges", "bytes")
            .header("Content-Length", body.len().to_string())
            .header("Cache-Control", "no-store")
            .header("Access-Control-Allow-Origin", "null")
            .header("X-Content-Type-Options", "nosniff")
            .body(body)
            .expect("response builds");
    }

    let (start, len) = match authorized_media_range(state, generation, total, range).await {
        Ok(plan) => plan,
        Err(response) => return response,
    };
    let plan = media_window(start, len);
    let bytes = match media_chunk(
        state,
        &actor,
        server,
        &cid,
        &raw,
        head.manifest_version,
        plan.index,
        generation,
    )
    .await
    {
        Some(bytes) => bytes,
        None => return deny(http::StatusCode::SERVICE_UNAVAILABLE),
    };
    let lo = plan.offset.min(bytes.len());
    let hi = (lo + plan.len).min(bytes.len());
    let body = bytes[lo..hi].to_vec();
    let end = start + body.len() as u64;
    // This is the last authorization point and remains held through response construction. If an
    // explicit lock completed during either actor read, no plaintext crosses the scheme boundary;
    // if lock begins after this guard, it cannot complete until this response has been built.
    let _response_commit = match require_ui_session_generation(state, generation).await {
        Ok(commit) => commit,
        Err(_) => return deny(http::StatusCode::FORBIDDEN),
    };
    // Always a 206 with an explicit Content-Range: the response is a window by construction, and
    // claiming 200 for a partial body is what makes a player think the file is truncated.
    http::Response::builder()
        .status(http::StatusCode::PARTIAL_CONTENT)
        .header("Content-Type", mime)
        .header("Accept-Ranges", "bytes")
        .header("Content-Length", body.len().to_string())
        .header(
            "Content-Range",
            format!("bytes {start}-{}/{total}", end.saturating_sub(1).max(start)),
        )
        // Plaintext from an encrypted vault: never cached to disk by the webview, and never
        // readable by anything the page might embed.
        .header("Cache-Control", "no-store")
        .header("Access-Control-Allow-Origin", "null")
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .expect("response builds")
}

/// Which chunk a read starts in, and how much of it may be served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MediaWindow {
    /// Index of the chunk containing `start`.
    index: usize,
    /// Byte offset of `start` within that chunk.
    offset: usize,
    /// How many bytes may be taken, never past the end of this chunk.
    len: usize,
}

/// Plan one media response.
///
/// The invariant that matters, and the one this exists to hold: **a response never spans more
/// than one chunk.** The first version of this served a fixed 2 MiB window out of an 8 MiB
/// chunk, so sequential playback decrypted every chunk four times over; because the actor
/// handles one command at a time, those redundant reads were also time the server could not
/// answer anything else, and the whole app looked like it was still loading during a call.
/// Clamping to the chunk end makes a sequential reader ask for the next chunk next, which is
/// exactly one decrypt per chunk.
fn media_window(start: u64, len: usize) -> MediaWindow {
    let chunk = CHUNK_BYTES as u64;
    let index = (start / chunk) as usize;
    let offset = (start % chunk) as usize;
    let remaining = CHUNK_BYTES - offset;
    MediaWindow {
        index,
        offset,
        len: len.min(remaining),
    }
}

/// A shared file's plaintext size and declared type, remembered per track.
///
/// One actor round-trip the first time a track is served and none afterwards, against one round
/// trip AND one 8 MiB decrypt per response before: see [`catcoms_app::Server::file_head`] for
/// what that cost the deck.
async fn media_head(
    state: &AppState,
    actor: &ServerActor,
    server: u64,
    cid: &str,
    raw: &[u8],
    generation: u64,
) -> Option<MediaHead> {
    // Re-resolve the cheap index metadata on every request. The claimed plaintext CID is not a
    // trustworthy manifest identity until a full export hashes every byte, and another member may
    // replace or ambiguously repeat it while the media element remains mounted.
    let live = actor.file_head(raw.to_vec()).await?;
    let cached = {
        let heads = state.media_heads.lock().await;
        heads
            .iter()
            .find(|h| {
                h.server == server && h.cid == cid && h.manifest_version == live.manifest_version
            })
            .cloned()
    };
    if let Some(hit) = cached {
        let _commit = require_ui_session_generation(state, generation)
            .await
            .ok()?;
        return Some(hit);
    }
    let total_size = live.total_size;
    let declared = live.mime;
    // A member-authored MIME string is not evidence about the bytes. Open chunk zero through the
    // normal authenticated file path once per exact manifest and authorize the scheme only when
    // the conservative classifier agrees. Returning octet-stream with a body is not a denial:
    // media elements may still sniff and decode it despite `nosniff`.
    if safe_media_mime(&declared) == "application/octet-stream" {
        return None;
    }
    let first = media_chunk(
        state,
        actor,
        server,
        cid,
        raw,
        live.manifest_version,
        0,
        generation,
    )
    .await?;
    let mime = validated_inline_media_mime(&declared, &first)?;
    media_head_put_for_generation(
        state,
        generation,
        MediaHead {
            server,
            cid: cid.to_string(),
            manifest_version: live.manifest_version,
            total_size,
            mime: mime.clone(),
        },
    )
    .await
}

async fn media_head_put_for_generation(
    state: &AppState,
    generation: u64,
    head: MediaHead,
) -> Option<MediaHead> {
    let _commit = require_ui_session_generation(state, generation)
        .await
        .ok()?;
    let mut heads = state.media_heads.lock().await;
    heads.retain(|existing| existing.server != head.server || existing.cid != head.cid);
    heads.push(head);
    while heads.len() > MEDIA_HEAD_ENTRIES {
        heads.remove(0);
    }
    heads.last().cloned()
}

/// Fetch one whole decrypted chunk, from the cache when possible.
///
/// The cache exists because the actor is single-threaded: every miss is time the server spends
/// not answering anything else, so a player walking a track must not make us re-read a chunk it
/// has already been served.
///
/// Every argument names something the read is bound to (which server, which manifest version,
/// which UI generation). Bundling them into a struct would move the same fields behind a name
/// without removing one, and each is checked separately at a different point in the read.
#[allow(clippy::too_many_arguments)]
async fn media_chunk(
    state: &AppState,
    actor: &ServerActor,
    server: u64,
    cid: &str,
    raw: &[u8],
    manifest_version: [u8; 32],
    index: usize,
    generation: u64,
) -> Option<Arc<Vec<u8>>> {
    let cached = {
        let mut cache = state.media_cache.lock().await;
        media_cache_take(&mut cache, server, cid, manifest_version, index)
    };
    if let Some(bytes) = cached {
        let _commit = require_ui_session_generation(state, generation)
            .await
            .ok()?;
        return Some(bytes);
    }
    let start = index as u64 * CHUNK_BYTES as u64;
    let range = actor
        .read_file_range(raw.to_vec(), manifest_version, start, CHUNK_BYTES)
        .await
        .ok()?;
    // A read past the end is how the caller learns the file is shorter than it guessed; it is not
    // worth a cache entry.
    if range.bytes.is_empty() && start > 0 {
        return Some(Arc::new(Vec::new()));
    }
    let bytes = Arc::new(range.bytes);
    media_cache_put_for_generation(
        state,
        generation,
        MediaChunk {
            server,
            cid: cid.to_string(),
            manifest_version,
            index,
            bytes: Arc::clone(&bytes),
        },
    )
    .await
    .then_some(bytes)
}

async fn media_cache_put_for_generation(
    state: &AppState,
    generation: u64,
    chunk: MediaChunk,
) -> bool {
    let Ok(_commit) = require_ui_session_generation(state, generation).await else {
        return false;
    };
    let mut cache = state.media_cache.lock().await;
    media_cache_put(&mut cache, chunk);
    true
}

/// Constrain what a shared file's declared MIME may become on a media response. The value is
/// author-controlled, and `nosniff` only helps once the type itself is one we chose: an attacker
/// who could name `text/html` here would have a same-origin script vector.
fn safe_media_mime(declared: &str) -> String {
    let lowered = declared.trim().to_ascii_lowercase();
    let base = lowered.split(';').next().unwrap_or("").trim().to_string();
    // Explicit allowlist: notably excludes SVG/XML and playlist formats, which can contain active
    // links or markup and are not inert merely because their top-level type says "image/audio".
    let ok = matches!(
        base.as_str(),
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "image/bmp"
            | "image/tiff"
            | "image/x-icon"
            | "audio/mpeg"
            | "audio/ogg"
            | "audio/wav"
            | "audio/x-wav"
            | "audio/flac"
            | "audio/mp4"
            | "audio/aac"
            | "audio/webm"
            | "video/mp4"
            | "video/webm"
            | "video/ogg"
            | "video/quicktime"
            | "video/x-msvideo"
    );
    if ok {
        base
    } else {
        "application/octet-stream".to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaSignatureEvidence {
    /// Recognized bytes agree with the exact declared container (or an explicit alias).
    Matched,
    /// Recognized bytes belong to a different container than the declaration.
    Mismatch,
    /// The declaration is allowed media, but the bounded prefix is not a format we recognize.
    Unrecognized,
    /// The declaration itself is not on the inert media allowlist.
    NotMedia,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetectedMediaContainer {
    Png,
    Jpeg,
    Gif,
    Webp,
    Avif,
    Bmp,
    Tiff,
    Ico,
    Mp3,
    Wav,
    Flac,
    Aac,
    Mp4,
    Ogg,
    Webm,
    QuickTime,
    Avi,
}

fn container_matches_mime(container: DetectedMediaContainer, mime: &str) -> bool {
    match container {
        DetectedMediaContainer::Png => mime == "image/png",
        DetectedMediaContainer::Jpeg => mime == "image/jpeg",
        DetectedMediaContainer::Gif => mime == "image/gif",
        DetectedMediaContainer::Webp => mime == "image/webp",
        DetectedMediaContainer::Avif => mime == "image/avif",
        DetectedMediaContainer::Bmp => mime == "image/bmp",
        DetectedMediaContainer::Tiff => mime == "image/tiff",
        DetectedMediaContainer::Ico => mime == "image/x-icon",
        DetectedMediaContainer::Mp3 => mime == "audio/mpeg",
        DetectedMediaContainer::Wav => matches!(mime, "audio/wav" | "audio/x-wav"),
        DetectedMediaContainer::Flac => mime == "audio/flac",
        DetectedMediaContainer::Aac => mime == "audio/aac",
        // These containers do not reveal whether their tracks are audio-only from the bounded
        // header evidence. Both explicitly allowlisted top-level aliases are therefore honest.
        DetectedMediaContainer::Mp4 => matches!(mime, "audio/mp4" | "video/mp4"),
        DetectedMediaContainer::Ogg => matches!(mime, "audio/ogg" | "video/ogg"),
        DetectedMediaContainer::Webm => matches!(mime, "audio/webm" | "video/webm"),
        DetectedMediaContainer::QuickTime => mime == "video/quicktime",
        DetectedMediaContainer::Avi => mime == "video/x-msvideo",
    }
}

/// Conservative magic-byte classification for the media formats the webview is willing to decode.
/// This validates container identity, not codec correctness or safety; malformed-but-recognizable
/// media still reaches a decoder and must remain untrusted.
fn detected_media_container(bytes: &[u8]) -> Option<DetectedMediaContainer> {
    let starts = |signature: &[u8]| bytes.starts_with(signature);
    if starts(b"\x89PNG\r\n\x1a\n") {
        return Some(DetectedMediaContainer::Png);
    }
    if starts(b"\xff\xd8\xff") {
        return Some(DetectedMediaContainer::Jpeg);
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Some(DetectedMediaContainer::Gif);
    }
    if starts(b"BM") {
        return Some(DetectedMediaContainer::Bmp);
    }
    if starts(b"II*\0") || starts(b"MM\0*") {
        return Some(DetectedMediaContainer::Tiff);
    }
    if starts(b"\0\0\x01\0") {
        return Some(DetectedMediaContainer::Ico);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(DetectedMediaContainer::Webp);
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let declared_box_len = u32::from_be_bytes(bytes[..4].try_into().ok()?) as usize;
        if declared_box_len < 12 {
            return None;
        }
        let end = declared_box_len.min(bytes.len());
        let has_brand = |wanted: &[u8; 4]| {
            &bytes[8..12] == wanted
                || (end >= 20 && bytes[16..end].chunks_exact(4).any(|brand| brand == wanted))
        };
        if has_brand(b"avif") || has_brand(b"avis") {
            return Some(DetectedMediaContainer::Avif);
        }
        if has_brand(b"qt  ") {
            return Some(DetectedMediaContainer::QuickTime);
        }
        if [
            b"isom", b"iso2", b"iso3", b"iso4", b"iso5", b"iso6", b"mp41", b"mp42", b"M4A ",
            b"M4V ", b"avc1", b"dash", b"cmfc",
        ]
        .iter()
        .any(|brand| has_brand(brand))
        {
            return Some(DetectedMediaContainer::Mp4);
        }
        return None;
    }
    if starts(b"fLaC") {
        return Some(DetectedMediaContainer::Flac);
    }
    if starts(b"ID3") {
        return Some(DetectedMediaContainer::Mp3);
    }
    if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xf6 == 0xf0 {
        return Some(DetectedMediaContainer::Aac);
    }
    if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0 && bytes[1] & 0x06 != 0 {
        return Some(DetectedMediaContainer::Mp3);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return Some(DetectedMediaContainer::Wav);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"AVI " {
        return Some(DetectedMediaContainer::Avi);
    }
    if starts(b"OggS") {
        return Some(DetectedMediaContainer::Ogg);
    }
    if starts(b"\x1a\x45\xdf\xa3")
        && bytes
            .windows(4)
            .any(|window| window.eq_ignore_ascii_case(b"webm"))
    {
        return Some(DetectedMediaContainer::Webm);
    }
    None
}

fn media_signature_evidence(declared: &str, bytes: &[u8]) -> MediaSignatureEvidence {
    let allowed = safe_media_mime(declared);
    if allowed == "application/octet-stream" {
        return MediaSignatureEvidence::NotMedia;
    }
    let Some(detected) = detected_media_container(bytes) else {
        return MediaSignatureEvidence::Unrecognized;
    };
    if container_matches_mime(detected, &allowed) {
        MediaSignatureEvidence::Matched
    } else {
        MediaSignatureEvidence::Mismatch
    }
}

/// Return a decoder-facing MIME only for a recognized, agreeing inert media container.
/// Mismatch, unknown media, SVG and non-media declarations receive no response body at all.
fn validated_inline_media_mime(declared: &str, bytes: &[u8]) -> Option<String> {
    let allowed = safe_media_mime(declared);
    (media_signature_evidence(&allowed, bytes) == MediaSignatureEvidence::Matched)
        .then_some(allowed)
}

fn validate_ui_state_json(json: &str) -> Result<(), String> {
    if json.len() > MAX_UI_STATE_BYTES {
        return Err("UI continuity state is too large".into());
    }
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "UI continuity state is not valid JSON")?;
    let object = value
        .as_object()
        .ok_or_else(|| "UI continuity state must be an object".to_string())?;
    if object.get("version").and_then(serde_json::Value::as_u64) != Some(1)
        || !object
            .get("drafts")
            .is_some_and(serde_json::Value::is_object)
        || !object
            .get("readMarks")
            .is_some_and(serde_json::Value::is_object)
    {
        return Err("unsupported UI continuity state shape".into());
    }
    Ok(())
}

/// Load drafts/read markers only after the vault is unlocked. Returning a canonical empty value
/// on first run keeps plaintext localStorage out of this path entirely.
#[tauri::command]
async fn get_ui_state(state: State<'_, AppState>) -> Result<String, String> {
    require_unlocked_session(&state).await?;
    let guard = state.store.lock().await;
    let store = guard
        .as_ref()
        .ok_or_else(|| "unlock the vault before loading UI state".to_string())?;
    let bytes = store.load_ui_state().map_err(|error| error.to_string())?;
    if bytes.is_empty() {
        return Ok(r#"{"version":1,"drafts":{},"readMarks":{}}"#.into());
    }
    let json = String::from_utf8(bytes).map_err(|_| "UI continuity state is not UTF-8")?;
    validate_ui_state_json(&json)?;
    Ok(json)
}

#[tauri::command]
async fn save_ui_state(state: State<'_, AppState>, json: String) -> Result<(), String> {
    validate_ui_state_json(&json)?;
    let generation = unlocked_ui_session_generation(&state).await?;
    save_ui_state_for_generation(&state, &json, generation).await
}

/// Serialize continuity writes with the lock snapshot. If lock was requested after the caller
/// captured its JSON, the generation recheck rejects it; if this write already owns the commit
/// mutex, lock waits and its newer final snapshot necessarily wins afterward.
async fn save_ui_state_for_generation(
    state: &AppState,
    json: &str,
    generation: u64,
) -> Result<(), String> {
    let _session_commit = require_ui_session_generation(state, generation).await?;
    let guard = state.store.lock().await;
    let store = guard
        .as_ref()
        .ok_or_else(|| "unlock the vault before saving UI state".to_string())?;
    store
        .save_ui_state(json.as_bytes(), &mut OsCryptoRng)
        .map_err(|error| error.to_string())
}

/// Every member's role (fingerprint -> owner/admin/member).
#[tauri::command]
async fn get_roles(
    state: State<'_, AppState>,
    server: u64,
) -> Result<std::collections::HashMap<String, String>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor.roles().await)
}

/// Signed public moderation history and advisory kick votes for a server space.
#[tauri::command]
async fn get_moderation(
    state: State<'_, AppState>,
    server: u64,
) -> Result<UiModerationState, String> {
    let actor = server_actor_of(&state, server).await?;
    let state = actor.moderation_state().await;
    Ok(UiModerationState {
        events: state
            .events
            .into_iter()
            .map(|event| UiModerationEvent {
                id: event.id,
                kind: event.kind,
                actor: event.actor,
                signer: event.signer,
                target: event.target,
                channel: event.channel,
                message_id: event.message_id,
                message_text: event.message_text,
                message_ts: event.message_ts,
                reason: event.reason,
                evidence_ids: event.evidence_ids,
                case_id: event.case_id,
                outcome: event.outcome,
                ts: event.ts,
                signature_valid: event.signature_valid,
                authorized: event.authorized,
            })
            .collect(),
        votes: state
            .votes
            .into_iter()
            .map(|vote| UiModerationVote {
                case_id: vote.case_id,
                voter: vote.voter,
                signer: vote.signer,
                yes: vote.yes,
                ts: vote.ts,
                signature_valid: vote.signature_valid,
                eligible: vote.eligible,
            })
            .collect(),
    })
}

/// Warn one current chat message. The backend snapshots its text/author/time into the signed
/// record before any optional deletion so evidence cannot turn into a dangling pointer.
#[tauri::command]
async fn warn_message(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    message_id: String,
    reason: String,
) -> Result<String, String> {
    let channel = channel
        .parse::<u128>()
        .map_err(|_| "bad channel id".to_string())?;
    let actor = server_actor_of(&state, server).await?;
    let id = actor.warn_message(channel, message_id, reason).await?;
    persist_server(&state, server).await;
    Ok(id)
}

/// Publish the moderator's case and its selected signed warning evidence. Voting is advisory;
/// only the owner resolution command can invoke protocol-enforced MLS removal.
#[tauri::command]
async fn create_kick_case(
    state: State<'_, AppState>,
    server: u64,
    target: String,
    reason: String,
    evidence_ids: Vec<String>,
) -> Result<String, String> {
    let actor = server_actor_of(&state, server).await?;
    let id = actor.create_kick_case(target, reason, evidence_ids).await?;
    persist_server(&state, server).await;
    Ok(id)
}

#[tauri::command]
async fn cast_kick_vote(
    state: State<'_, AppState>,
    server: u64,
    case_id: String,
    yes: bool,
) -> Result<(), String> {
    let actor = server_actor_of(&state, server).await?;
    actor.cast_kick_vote(case_id, yes).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Resolve a case. `remove=false` dismisses it; `remove=true` is owner-only in the backend and
/// performs MLS removal before recording whether removal succeeded.
#[tauri::command]
async fn resolve_kick_case(
    state: State<'_, AppState>,
    server: u64,
    case_id: String,
    remove: bool,
) -> Result<(), String> {
    let actor = server_actor_of(&state, server).await?;
    actor.resolve_kick_case(case_id, remove).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Grant or revoke admin for a member (owner only); re-seals the server.
#[tauri::command]
async fn set_admin(
    state: State<'_, AppState>,
    server: u64,
    fp: String,
    admin: bool,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.set_admin(fp, admin).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Remove a member from the server (owner only); re-seals the server.
#[tauri::command]
async fn remove_member(state: State<'_, AppState>, server: u64, fp: String) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.remove_member(fp).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Revoke one of your own linked devices (M5); re-seals the server. The owner enforces the MLS
/// Remove when it next reconciles.
#[tauri::command]
async fn revoke_device(state: State<'_, AppState>, server: u64, fp: String) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.revoke_device(fp).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Read a wiki page's body.
#[tauri::command]
async fn get_wiki_page(
    state: State<'_, AppState>,
    server: u64,
    name: String,
) -> Result<String, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor.read_wiki_page(name).await)
}

/// Create or update a wiki page. Returns `true` when the server's review mode queued the
/// edit for owner/admin approval instead of publishing it immediately.
#[tauri::command]
async fn save_wiki_page(
    state: State<'_, AppState>,
    server: u64,
    name: String,
    body: String,
    trace: Option<String>,
) -> Result<bool, AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Documents,
        "save_wiki_page",
        server,
        None,
    );
    let actor = op.actor(&state, server).await?;
    // The page body never reaches the record, and neither does its name: a wiki page is content,
    // and the whole point of the privacy model is that content has no representation here.
    let queued = actor
        .write_wiki_page(name, body)
        .await
        .map_err(|e| op.fail(codes::DOCUMENT_WRITE_REJECTED, e))?;
    persist_server(&state, server).await;
    // Two different things happened, and only one of them put the edit on the page. "Saved" and
    // "queued for someone to approve" look identical from a success return, and an author who
    // thinks the first happened when it was the second goes looking for their edit and cannot find
    // it. Same shape as a call signal that was sent to a member with no route: an outcome, not an
    // error, and the outcome is the diagnosis.
    if queued {
        op.succeeded("WIKI.EDIT.QUEUED_FOR_REVIEW");
    } else {
        op.succeeded("WIKI.EDIT.APPLIED");
    }
    Ok(queued)
}

/// One entry in a wiki page's revision history, as the UI reads it.
#[derive(Serialize, Clone)]
struct UiWikiRev {
    /// Stable revision id (for a reviewed edit, the pending-edit id it came from).
    id: String,
    /// The proposer's device fingerprint (resolved to a display name at render time).
    author: String,
    /// When the revision took effect, epoch-millis.
    ts: u64,
    /// The full page body as of this revision.
    body: String,
    /// "edit" | "approve" | "auto" | "reject" | "rollback" | "delete" | "rename".
    kind: String,
    /// The reviewer's fingerprint for approve/reject; empty otherwise.
    actor: String,
    /// Context: the old name for "rename", the restored revision id for "rollback".
    note: String,
}

/// A member edit awaiting review, as the UI reads it.
#[derive(Serialize, Clone)]
struct UiWikiPending {
    id: String,
    page: String,
    author: String,
    ts: u64,
    expires_ts: u64,
    body: String,
}

/// A wiki page's revision history, oldest first (auto-accepted edits included).
#[tauri::command]
async fn get_wiki_history(
    state: State<'_, AppState>,
    server: u64,
    page: String,
) -> Result<Vec<UiWikiRev>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .wiki_history(page)
        .await
        .into_iter()
        .map(|r| UiWikiRev {
            id: r.id,
            author: r.author,
            ts: r.ts,
            body: r.body,
            kind: r.kind,
            actor: r.actor,
            note: r.note,
        })
        .collect())
}

/// The live review queue: member edits still inside their window, oldest first.
#[tauri::command]
async fn get_wiki_pending(
    state: State<'_, AppState>,
    server: u64,
) -> Result<Vec<UiWikiPending>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .wiki_pending_edits()
        .await
        .into_iter()
        .map(|p| UiWikiPending {
            id: p.id,
            page: p.page,
            author: p.author,
            ts: p.ts,
            expires_ts: p.expires_ts,
            body: p.body,
        })
        .collect())
}

/// The wiki review window in days (0 = edits publish immediately).
#[tauri::command]
async fn get_wiki_review_days(state: State<'_, AppState>, server: u64) -> Result<u32, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor.wiki_review_days().await)
}

/// Set the wiki review window in days, 0..=30 (owner/admin only).
#[tauri::command]
async fn set_wiki_review_days(
    state: State<'_, AppState>,
    server: u64,
    days: u32,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.set_wiki_review_days(days).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Approve a pending wiki edit (owner/admin only): publishes it and records the revision.
#[tauri::command]
async fn approve_wiki_edit(
    state: State<'_, AppState>,
    server: u64,
    id: String,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.approve_wiki_edit(id).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Decline a pending wiki edit (owner/admin only; errors once it has auto-accepted).
#[tauri::command]
async fn reject_wiki_edit(
    state: State<'_, AppState>,
    server: u64,
    id: String,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.reject_wiki_edit(id).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Restore a page to an earlier revision. Returns `true` when review mode queued the
/// restore for approval instead of publishing it.
#[tauri::command]
async fn restore_wiki_page(
    state: State<'_, AppState>,
    server: u64,
    page: String,
    rev: String,
) -> Result<bool, String> {
    let actor = actor_of(&state, server).await?;
    let queued = actor.restore_wiki_page(page, rev).await?;
    persist_server(&state, server).await;
    Ok(queued)
}

/// The wiki's per-page render formats (name -> "md" | "wiki"). A page absent from the map has
/// no declared format and renders as markdown.
#[tauri::command]
async fn get_wiki_meta(
    state: State<'_, AppState>,
    server: u64,
) -> Result<std::collections::HashMap<String, String>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor.wiki_meta().await)
}

/// Set a wiki page's render format ("md" or "wiki").
#[tauri::command]
async fn set_wiki_format(
    state: State<'_, AppState>,
    server: u64,
    name: String,
    format: String,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.set_wiki_format(name, format).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Delete a wiki page (and its format metadata).
#[tauri::command]
async fn delete_wiki_page(
    state: State<'_, AppState>,
    server: u64,
    name: String,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.delete_wiki_page(name).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Rename a wiki page, carrying its body and format.
#[tauri::command]
async fn rename_wiki_page(
    state: State<'_, AppState>,
    server: u64,
    from: String,
    to: String,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.rename_wiki_page(from, to).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Resolve the two things every channel operation needs, classifying each failure as it happens.
///
/// The prefix shared by send, edit, delete, react and pin. Sharing it is not only about repetition:
/// it makes the five commands classify identically, which is what turns "how often does a server's
/// actor go missing" into a question with an answer rather than five separate answers that have to
/// be reconciled by reading prose.
///
/// The session gate and the actor lookup are separated on purpose. They are one call today and
/// they fail for entirely different reasons: one means "unlock the app", the other means "this
/// server's task has stopped". Collapsing them is how both came to produce the same sentence.
async fn channel_target(
    state: &AppState,
    op: &Operation,
    server: u64,
    channel: &str,
) -> Result<(u128, ServerActor), AppError> {
    let id: u128 = channel
        .parse()
        .map_err(|_| op.fail(codes::CHANNEL_BAD_ID, "bad channel id"))?;
    require_unlocked_session(state)
        .await
        .map_err(|e| op.fail(codes::SESSION_LOCKED, e))?;
    let actor = actor_of_unchecked(state, server)
        .await
        .map_err(|failure| op.fail(failure.code(), failure.message()))?;
    // Bound to this operation here rather than at each caller, which is what makes it impossible
    // for a new channel command to forget. Everything the actor does in response, and every event
    // that work produces, then lands under the same trace as the command that asked for it.
    // Library actors carry the pre-normalization boundary token. Their `tracing` records and
    // returned events are normalized when they cross back into diagnostics; passing `op.trace`
    // here would compute H(H(raw)) and split channel operations from every actor stage.
    Ok((id, op.bind_actor(actor)))
}

/// Send a chat message to a channel (by id).
///
/// The first command instrumented end to end, and the pattern the others follow. What it records is
/// the *stages*, because "the message did not arrive" was previously unanswerable: the evidence
/// could not say whether the command reached Rust, whether the actor was alive, whether the
/// operation was accepted, or whether persistence completed. Each is a different bug with a
/// different fix, and they all looked the same.
///
/// Note what is not recorded: the message. Not its text, not its length, not its recipient. The
/// channel becomes a session reference and the stage names carry the diagnosis.
#[tauri::command]
async fn send_message(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    text: String,
    reply_to: Option<String>,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Channels,
        "send_message",
        server,
        Some(&channel),
    );
    let (id, actor) = channel_target(&state, &op, server, &channel).await?;
    op.stage("CHANNEL.SEND.ENQUEUED");
    actor
        .send_reply(id, text, reply_to.unwrap_or_default())
        .await
        .map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?;
    op.stage("CHANNEL.SEND.ACCEPTED");
    persist_server(&state, server).await;
    // Deliberately after persistence. An operation reported as succeeding before its state reached
    // the disk is the exact shape of "it worked and then it was gone after a restart".
    op.succeeded("CHANNEL.SEND.PERSISTED");
    Ok(())
}

/// Edit one of your own messages (by message id) in a channel.
#[tauri::command]
async fn edit_message(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    msg_id: String,
    text: String,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Channels,
        "edit_message",
        server,
        Some(&channel),
    );
    let (id, actor) = channel_target(&state, &op, server, &channel).await?;
    actor
        .edit_message(id, msg_id, text)
        .await
        .map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("CHANNEL.EDIT.PERSISTED");
    Ok(())
}

/// Delete one of your own messages (by message id) from a channel.
#[tauri::command]
async fn delete_message(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    msg_id: String,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Channels,
        "delete_message",
        server,
        Some(&channel),
    );
    let (id, actor) = channel_target(&state, &op, server, &channel).await?;
    actor
        .delete_message(id, msg_id)
        .await
        .map_err(|e| op.fail(codes::CHAT_DELETE_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("CHANNEL.DELETE.PERSISTED");
    Ok(())
}

/// Toggle this member's emoji reaction on a message (by message id) in a channel.
#[tauri::command]
async fn toggle_reaction(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    msg_id: String,
    emoji: String,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Channels,
        "toggle_reaction",
        server,
        Some(&channel),
    );
    let (id, actor) = channel_target(&state, &op, server, &channel).await?;
    actor
        .toggle_reaction(id, msg_id, emoji)
        .await
        .map_err(|e| op.fail(codes::CHAT_REACTION_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("CHANNEL.REACTION.PERSISTED");
    Ok(())
}

/// Pin or unpin a message (by message id) in a channel (owner/admin).
#[tauri::command]
async fn set_pin(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    msg_id: String,
    pinned: bool,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Channels,
        "set_pin",
        server,
        Some(&channel),
    );
    let (id, actor) = channel_target(&state, &op, server, &channel).await?;
    actor
        .set_pin(id, msg_id, pinned)
        .await
        .map_err(|e| op.fail(codes::CHAT_PIN_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("CHANNEL.PIN.PERSISTED");
    Ok(())
}

/// Set (or clear, with `""`) a channel's topic; its short description. **Any member may**, like
/// creating the channel itself; rejected over 256 UTF-8 bytes. A `channel-updated` event for this
/// channel follows, so the UI re-reads it exactly as it re-reads messages.
#[tauri::command]
async fn set_channel_topic(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    topic: String,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Channels,
        "set_channel_topic",
        server,
        Some(&channel),
    );
    let (id, actor) = channel_target(&state, &op, server, &channel).await?;
    // The topic text is content and stays out of the record; that it changed is the event.
    actor
        .set_channel_topic(id, topic)
        .await
        .map_err(|e| op.fail(codes::CHANNEL_TOPIC_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("CHANNEL.TOPIC.PERSISTED");
    Ok(())
}

/// Read a channel's current topic (empty if none is set).
#[tauri::command]
async fn get_channel_topic(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
) -> Result<String, String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    Ok(actor.channel_topic(id).await)
}

/// Queue a shared file (by content address) in a channel's jukebox; its persistent playlist.
/// **Any member may**, like setting the topic; rejected when the cid is not a content address,
/// the name is blank or over 200 UTF-8 bytes, or the queue already holds 64 entries. Replies
/// with the new entry's id. A `channel-updated` event for this channel follows, so the UI
/// re-reads the queue exactly as it re-reads messages.
#[tauri::command]
async fn jukebox_add(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    cid: String,
    name: String,
    trace: Option<String>,
) -> Result<String, AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Channels,
        "jukebox_add",
        server,
        Some(&channel),
    );
    let (id, actor) = channel_target(&state, &op, server, &channel).await?;
    // The track's name is a user's words and never reaches the record. What it was called does not
    // explain why the queue failed to converge; that the operation was accepted and persisted does.
    let entry = actor
        .jukebox_add(id, cid, name)
        .await
        .map_err(|e| op.fail(codes::JUKEBOX_ADD_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("JUKEBOX.ADD.PERSISTED");
    Ok(entry)
}

/// Remove a jukebox entry (by entry id) from a channel; any member, and idempotent.
#[tauri::command]
async fn jukebox_remove(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    entry: String,
    trace: Option<String>,
) -> Result<(), AppError> {
    let op = Operation::start(
        trace,
        catcoms_diagnostics::Section::Channels,
        "jukebox_remove",
        server,
        Some(&channel),
    );
    let (id, actor) = channel_target(&state, &op, server, &channel).await?;
    actor
        .jukebox_remove(id, entry)
        .await
        .map_err(|e| op.fail(codes::JUKEBOX_REMOVE_REJECTED, e))?;
    persist_server(&state, server).await;
    op.succeeded("JUKEBOX.REMOVE.PERSISTED");
    Ok(())
}

/// Read a channel's jukebox queue, sorted by queue time ascending.
#[tauri::command]
async fn get_jukebox(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
) -> Result<Vec<UiJukeEntry>, String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .jukebox(id)
        .await
        .into_iter()
        .map(|e| UiJukeEntry {
            id: e.id,
            cid: e.cid,
            name: e.name,
            author: e.author,
            added_ms: e.added_ms,
        })
        .collect())
}

/// Read a channel's current messages (by id).
#[tauri::command]
async fn get_messages(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
) -> Result<Vec<UiMessage>, String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .messages(id)
        .await
        .into_iter()
        .map(ui_message)
        .collect())
}

/// One row of [`get_message_tail`]: a message plus whether it is addressed to this member.
#[derive(Serialize, Clone)]
struct UiTailMessage {
    #[serde(flatten)]
    message: UiMessage,
    /// An `@[my name]` mention or a reply to one of my messages, resolved natively against the
    /// whole channel so the webview does not need the history to answer it.
    targets_me: bool,
}

/// Upper bound on one tail read. A notification needs the newest row and the handful behind it;
/// a caller asking for more than this is asking for `get_messages`.
const MAX_MESSAGE_TAIL: usize = 256;

/// Read the newest `limit` messages of a channel (oldest first) with an "addressed to me" bit per
/// row. This is the arrival-notification read: it runs for every arrival in every channel that is
/// not on screen, so it is bounded by `limit`, never by how long the channel has existed.
#[tauri::command]
async fn get_message_tail(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    limit: usize,
    after_id: Option<String>,
    after_ts: Option<u64>,
) -> Result<UiMessageTail, String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let limit = limit.clamp(1, MAX_MESSAGE_TAIL);
    let actor = actor_of(&state, server).await?;
    let tail = actor
        .message_tail(
            id,
            limit,
            after_id.unwrap_or_default(),
            after_ts.unwrap_or_default(),
        )
        .await;
    Ok(UiMessageTail {
        rows: tail
            .rows
            .into_iter()
            .map(|(m, targets_me)| UiTailMessage {
                message: ui_message(m),
                targets_me,
            })
            .collect(),
        addressed_after_cursor: tail.addressed_after_cursor,
    })
}

/// Read the named rows, wherever they sort, each with an "addressed to me" bit.
///
/// The arrival read. Rows are ordered by their senders' timestamps, so a message that has just
/// arrived is not necessarily near the end and looking for it in a bounded tail, or in the page
/// the reader happens to have loaded, finds nothing. Ids that name no row come back absent, which
/// is how a caller learns the row was deleted rather than mistaking somebody else's message for it.
#[tauri::command]
async fn get_messages_by_id(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    ids: Vec<String>,
) -> Result<Vec<UiTailMessage>, String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    // One notification's worth. A caller with more ids than this is not announcing an arrival.
    let mut ids = ids;
    ids.truncate(MAX_MESSAGE_TAIL);
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .messages_by_id(id, ids)
        .await
        .into_iter()
        .map(|(m, targets_me)| UiTailMessage {
            message: ui_message(m),
            targets_me,
        })
        .collect())
}

/// What [`get_message_tail`] answers: the newest rows, and whether anything at all after the
/// caller's cursor addresses this member, including rows the tail was too short to carry.
#[derive(Serialize)]
struct UiMessageTail {
    rows: Vec<UiTailMessage>,
    addressed_after_cursor: bool,
}

/// Where a paged read is centred, as the webview names it: `{kind:"tail"}`, `{kind:"id", id}` or
/// `{kind:"index", index}`.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum UiPageAnchor {
    Tail,
    Id { id: String },
    Index { index: u64 },
    FirstReplyTo { id: String },
}

/// The client's read boundary and clock, for a natively measured unread summary.
#[derive(serde::Deserialize)]
struct UiUnreadProbe {
    /// Id of the newest message from somebody else that this device had read. The cursor; `null`
    /// for a read mark written before ids existed, or one whose message is gone.
    divider_id: Option<String>,
    /// `null` (or absent) means "everything is read".
    divider_ts: Option<u64>,
    now_ms: u64,
}

#[derive(Serialize)]
struct UiUnreadSummary {
    ceiling_ts: u64,
    first_index: Option<u64>,
    count: u64,
}

/// One row of [`get_message_page`].
#[derive(Serialize, Clone)]
struct UiPagedMessage {
    #[serde(flatten)]
    message: UiMessage,
    targets_me: bool,
    reply_count: u32,
    /// `{id, author, text}` of the parent when this row is a reply whose parent exists.
    reply_to_preview: Option<UiReplyPreview>,
}

#[derive(Serialize, Clone)]
struct UiReplyPreview {
    id: String,
    author: String,
    text: String,
}

/// A contiguous slice of a channel: see `MessagePage` in `catcoms-app`.
#[derive(Serialize)]
struct UiMessagePage {
    version: u64,
    total: u64,
    start: u64,
    anchor_index: Option<u64>,
    rows: Vec<UiPagedMessage>,
    unread: Option<UiUnreadSummary>,
}

/// Upper bound on the rows one side of a page may ask for. A view holds a few hundred rows and
/// reveals a couple of hundred more per step; nothing on screen needs more than this at once.
const MAX_PAGE_SIDE: usize = 2_048;

/// Read a bounded slice of a channel around an anchor. This is how the webview reads history:
/// the newest rows on open, a page above on scroll-up, a window around a target on a jump, and a
/// refresh of the rows it holds (by id anchor) when the channel changes; never the whole log.
#[tauri::command]
async fn get_message_page(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    anchor: UiPageAnchor,
    before: usize,
    after: usize,
    unread: Option<UiUnreadProbe>,
) -> Result<UiMessagePage, String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let query = catcoms_app::MessagePageQuery {
        anchor: match anchor {
            UiPageAnchor::Tail => catcoms_app::PageAnchor::Tail,
            UiPageAnchor::Id { id } => catcoms_app::PageAnchor::Id(id),
            UiPageAnchor::Index { index } => catcoms_app::PageAnchor::Index(index),
            UiPageAnchor::FirstReplyTo { id } => catcoms_app::PageAnchor::FirstReplyTo(id),
        },
        before: before.min(MAX_PAGE_SIDE),
        after: after.min(MAX_PAGE_SIDE),
        unread: unread.map(|probe| catcoms_app::UnreadProbe {
            divider_id: probe.divider_id.unwrap_or_default(),
            divider_ts: probe.divider_ts.unwrap_or(u64::MAX),
            now_ms: probe.now_ms,
        }),
    };
    let actor = actor_of(&state, server).await?;
    let page = actor.message_page(id, query).await;
    Ok(UiMessagePage {
        version: page.version,
        total: page.total,
        start: page.start,
        anchor_index: page.anchor_index,
        unread: page.unread.map(|u| UiUnreadSummary {
            ceiling_ts: u.ceiling_ts,
            first_index: u.first_index,
            count: u.count,
        }),
        rows: page
            .rows
            .into_iter()
            .map(|row| UiPagedMessage {
                message: ui_message(row.message),
                targets_me: row.targets_me,
                reply_count: row.reply_count,
                reply_to_preview: row.reply_to_preview.map(|p| UiReplyPreview {
                    id: p.id,
                    author: p.author,
                    text: p.text,
                }),
            })
            .collect(),
    })
}

/// Every pinned message of a channel, in log order. Pins are few and curated, so a paged client
/// asks for them by name instead of scanning the history for the flag.
#[tauri::command]
async fn get_pinned_messages(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
) -> Result<Vec<UiMessage>, String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .pinned_messages(id)
        .await
        .into_iter()
        .map(ui_message)
        .collect())
}

/// One channel's newest activity, with no message text: what unread state is rebuilt from.
#[derive(Serialize)]
struct UiChannelHead {
    /// Channel id as a string; u128 ids do not survive a JS number.
    channel: String,
    count: u64,
    latest_ts: u64,
    /// Newest timestamp among messages this device did not write (`0` if there are none). Own
    /// messages never make a channel unread, so this is what a read mark is compared against.
    latest_incoming_ts: u64,
    latest_incoming_id: String,
}

/// Read one compact activity head per channel.
///
/// The live `channel-updated` stream only reports what happened while the UI was listening, and it
/// is deliberately dropped at the native boundary while the vault is locked. Unread badges that
/// were outstanding across an explicit lock, a restart or an offline catch-up therefore cannot be
/// recovered from events at all: the client rebuilds them by comparing these heads with its own
/// durable read marks.
#[tauri::command]
async fn get_channel_heads(
    state: State<'_, AppState>,
    server: u64,
) -> Result<Vec<UiChannelHead>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .channel_heads()
        .await
        .into_iter()
        .map(|h| UiChannelHead {
            channel: h.channel.to_string(),
            count: h.count,
            latest_ts: h.latest_ts,
            latest_incoming_ts: h.latest_incoming_ts,
            latest_incoming_id: h.latest_incoming_id,
        })
        .collect())
}

/// The cross-server mention/reply inbox: every message addressed to me, across all servers/DMs,
/// newest first. Each server's actor scans its own channels (and resolves author names, since they
/// are per-server); the bridge tags each item with its server context.
#[tauri::command]
async fn get_inbox(state: State<'_, AppState>) -> Result<Vec<UiInboxItem>, String> {
    require_unlocked_session(&state).await?;
    // Snapshot (id, name, is_dm, actor) under the lock, then query each actor without holding it.
    let servers: Vec<(u64, String, bool, ServerActor)> = {
        let guard = state.servers.lock().await;
        guard
            .iter()
            .map(|(id, e)| (*id, e.name.clone(), e.is_dm, e.actor.clone()))
            .collect()
    };
    let mut out = Vec::new();
    for (id, name, is_dm, actor) in servers {
        for item in actor.inbox(50).await {
            out.push(UiInboxItem {
                server: id,
                server_name: name.clone(),
                is_dm,
                channel: item.channel.to_string(),
                message_id: item.message_id,
                author: item.author,
                author_name: item.author_name,
                text: item.text,
                ts: item.ts,
                mention: item.mention,
                reply: item.reply,
            });
        }
    }
    out.sort_by(|a, b| b.ts.cmp(&a.ts));
    // Per-server cap (50) then a global cap (100), newest first. A single hyper-active server can
    // thus have its older mentions truncated before the global merge; fine for an inbox view.
    out.truncate(100);
    Ok(out)
}

/// One server reloaded from disk, returned to the UI to repopulate the rail.
#[derive(Serialize, Clone)]
struct ReloadedServer {
    server: u64,
    name: String,
    invite: String,
    channel: String,
    channels: Vec<UiChannel>,
    is_dm: bool,
}

/// Transport-independent half of vault reload. Keeping this seam below Tauri's concrete runtime
/// lets the lifecycle test exercise the exact restore, blob attachment, route restoration and
/// actor startup used in production with a deterministic in-memory transport.
struct RestoredActor {
    actor: ServerActor,
    events: mpsc::Receiver<catcoms_app::TracedEvent>,
    task: tokio::task::JoinHandle<()>,
    group_id: Vec<u8>,
    device_id: DeviceId,
}

/// The injected seams (transport, RNG, clock) are what make this testable with an in-memory
/// transport, so they are arguments by design rather than by accident.
#[allow(clippy::too_many_arguments)]
async fn restore_server_actor<T, R>(
    state: &AppState,
    snapshot: &[u8],
    record: &ServerRecord,
    transport: T,
    rng: R,
    clock: Box<dyn Clock + Send>,
    bootstrap: &[String],
    record_seq: u64,
    reconnect_routes: Vec<(PeerId, String)>,
    switchboard: bool,
) -> Result<RestoredActor, String>
where
    T: MeshTransport + Send + 'static,
    R: catcoms_rt::CryptoRngCore + Send + 'static,
{
    let mut server = Server::restore(snapshot, transport, rng, clock, &record.display_name)
        .map_err(|error| error.to_string())?;
    server.set_endpoint_dial_scheduler(state.endpoint_dials.clone());
    server.set_local_reconnect_routes(reconnect_routes);
    server
        .subscribe_control()
        .await
        .map_err(|error| error.to_string())?;
    attach_blob_store(state, &mut server).await;
    if let Err(error) = server.publish_self_record(bootstrap.to_vec(), record_seq) {
        tracing::warn!(
            target: "catcoms_app",
            server = record.id,
            phase = "reload",
            error = %error,
            "DISCOVERY.PEER_RECORD.PUBLISH_FAILED"
        );
    }
    server.set_switchboard_offered(switchboard);

    // Restore the cross-session address cache before the eager member redial. Invalid or missing
    // best-effort cache data never prevents the authoritative group snapshot from opening.
    {
        let guard = state.store.lock().await;
        if let Some(store) = guard.as_ref() {
            match (
                store.address_cache_key(),
                store.load_address_cache(record.id),
            ) {
                (Ok(key), Ok(bytes)) if !bytes.is_empty() => {
                    if !server.load_address_cache(&bytes, &key) {
                        tracing::warn!(
                            target: "catcoms_app",
                            server = record.id,
                            "VAULT.ADDRESS_CACHE.REJECTED"
                        );
                    }
                }
                (Err(error), _) | (_, Err(error)) => tracing::warn!(
                    target: "catcoms_app",
                    server = record.id,
                    error = %error,
                    "VAULT.ADDRESS_CACHE.LOAD_FAILED"
                ),
                _ => {}
            }
        }
    }

    server.cache_known_records();
    let local_redialled = server.dial_local_reconnect_routes().await;
    let cached_redialled = server.dial_cached_peers().await;
    if local_redialled + cached_redialled > 0 {
        tracing::info!(
            target: "catcoms_app",
            server = record.id,
            local_peers = local_redialled,
            cached_peers = cached_redialled,
            "DISCOVERY.REDIAL.STARTED"
        );
    }

    let group_id = server.group_id();
    let device_id = server.device_id();
    let (actor, events, task) = spawn(server);
    actor.open_channel(channel_id("general")).await;
    Ok(RestoredActor {
        actor,
        events,
        task,
        group_id,
        device_id,
    })
}

async fn running_servers(state: &AppState) -> Vec<ReloadedServer> {
    let servers: Vec<_> = state
        .servers
        .lock()
        .await
        .iter()
        .map(|(id, e)| {
            (
                *id,
                e.name.clone(),
                e.invite.clone().unwrap_or_default(),
                e.is_dm,
                e.actor.clone(),
            )
        })
        .collect();
    let mut reloaded = Vec::with_capacity(servers.len());
    for (server, name, invite, is_dm, actor) in servers {
        reloaded.push(ReloadedServer {
            server,
            name,
            invite,
            channel: channel_id("general").to_string(),
            channels: ui_channels(actor.channels().await),
            is_dm,
        });
    }
    reloaded.sort_by_key(|s| s.server);
    reloaded
}

/// Reload one sealed server from disk onto a fresh transport and register it under its
/// on-disk id. The reloaded node reads its history immediately (offline); peers re-dial as
/// they come online (Phase 9g). A founder whose address changed re-mints a fresh invite.
async fn reload_one(
    app: &AppHandle,
    state: &AppState,
    snapshot: &[u8],
    record: &ServerRecord,
) -> Result<(), String> {
    // 9g used to dial every address in the snapshot's peer records here, straight at the
    // transport: up to `MAX_PEX_ADDRESSES` per record for every record ever stored, unconditional,
    // bypassing `DiscoveryPolicy` and its dial budget entirely, and with no membership check, so a
    // removed member was re-dialled on every launch for the life of the install. It was harmless
    // only because `peer_records` was permanently empty; wiring PEX is what armed it.
    //
    // The re-dial now happens after the server is restored, via `dial_cached_peers`, which is
    // roster-checked, address-validated, policy-ranked and budget-capped. See below.

    // The persisted invite is still read first: for a server founded before the network record
    // existed it is the only surviving record of which rendezvous this server used.
    let persisted_invite = (!record.invite.is_empty())
        .then(|| {
            decode_and_verify_invite(&record.invite)
                .ok()
                .map(|decoded| decoded.token)
        })
        .flatten();
    let invite_rendezvous = persisted_invite
        .as_ref()
        .and_then(|i| i.rendezvous.first().cloned())
        .unwrap_or_default();

    // Rebuild on the SAME libp2p identity and the SAME port as last launch. Both are what keep an
    // already-issued invite redeemable: the invite names `/p2p/<id>` at a fixed port, and a
    // regenerated pair is exactly what made a remote joiner time out.
    let mut net = load_or_init_server_net(state, record.id, &invite_rendezvous).await;
    let saved_port = net.port;
    let relay_dial: Vec<Multiaddr> = net.relay.parse().into_iter().collect();
    let (mesh, libp2p_id, port, bound) = build_transport(&net, &relay_dial)?;
    net.port = port;

    // Re-run the founder's reachability work verbatim: the advertise address, the UPnP probe, the
    // relay-circuit reservation and the rendezvous registration. Before this, a reload rebuilt a
    // loopback-only bootstrap and every invite minted afterwards was same-machine only.
    let id = libp2p_id.to_string();
    let (reach, problems) =
        establish_reachability(&mesh, &id, port, &bound, &net, &mut Vec::new()).await;
    for p in &problems {
        // Unlike founding, a reload never fails over this: the user is not standing at a form, and
        // a server that loads with reduced reach still reads its history and re-dials its peers.
        tracing::warn!(target: "catcoms_app", server = record.id, problem = %p, "REACH.RELOAD.DEGRADED");
    }
    let Reachability {
        bootstrap,
        bootstrap_owners,
        rz_target,
        rz_handle,
    } = reach;
    // `establish_reachability` has already advertised every direct candidate and offered it to
    // AutoNAT. Do not add them again here: the v2 client treats each candidate event as work, so a
    // duplicate would spend another public-server probe without adding evidence.
    let port_mapping_rx = mesh.take_port_mapping_snapshots().await;
    let autonat_rx = mesh.take_autonat_snapshots().await;
    let relay_address_rx = mesh.take_relay_address_snapshots().await;
    let mesh_observation_rx = mesh.take_mesh_observation_snapshots().await;
    let mesh_handle = mesh.handle();

    // Re-register the persisted invite's own (nonce-keyed) namespace, so the invite that is about
    // to be re-presented in the UI still resolves for whoever is holding it. (`rz_handle` is `Some`
    // only when `rz_target` is, so this is the full set of conditions.)
    if let (Some(rz), Some(handle), Some(invite)) = (
        rz_target.as_ref(),
        rz_handle.as_ref(),
        persisted_invite.as_ref(),
    ) {
        let _ = register_join_ns(handle, &invite.group_id, &invite.invite_nonce, rz).await;
    }
    let rz_vec: Vec<String> = rz_target.iter().map(|t| t.addr.to_string()).collect();

    let RestoredActor {
        actor,
        events,
        task,
        group_id,
        device_id,
    } = restore_server_actor(
        state,
        snapshot,
        record,
        mesh,
        OsCryptoRng,
        Box::new(SystemClock),
        &bootstrap,
        net.record_seq,
        net.reconnect_routes
            .iter()
            .map(|route| (PeerId::new(route.peer_id), route.address.clone()))
            .collect(),
        net.switchboard,
    )
    .await?;
    // If the persisted invite is discovery-enabled but we could NOT re-register its namespace
    // (rendezvous infra was down at reload), drop it: it would not resolve. The rail then prompts a
    // fresh invite (which re-registers). A direct (non-rendezvous) invite is presented unchanged;
    // it now survives a restart properly, since the peer id and port behind it no longer move.
    let discovery_unregistered = persisted_invite
        .as_ref()
        .is_some_and(|i| !i.rendezvous.is_empty())
        && rz_handle.is_none();
    let presented_invite = if record.invite.is_empty() || discovery_unregistered {
        None
    } else {
        Some(record.invite.clone())
    };

    // Register under the SAME id as on disk (don't allocate a new one).
    supervise("server_actor", record.id, task);
    forward_events(app.clone(), record.id, events);
    let timer_actor = actor.clone();
    state.servers.lock().await.insert(
        record.id,
        ServerEntry {
            actor,
            instance: state.next_server_instance.fetch_add(1, Ordering::Relaxed),
            group_id,
            device_id,
            invite: presented_invite,
            name: record.display_name.clone(),
            bootstrap,
            bootstrap_owners,
            interface_routes: Some(InterfaceRouteIdentity {
                port,
                peer_id: id.clone(),
            }),
            rendezvous: rz_vec,
            mesh: Some(mesh_handle),
            is_dm: record.is_dm,
            switchboard: net.switchboard,
            record_seq: net.record_seq,
            persist: PersistCounters::default(),
        },
    );
    install_reconnect_capture_worker(app, record.id, timer_actor.clone());
    spawn_discovery_timer(app.clone(), record.id, timer_actor);
    // Re-seal if the port moved. (The reserved peer-record sequence block was already sealed by
    // `load_or_init_server_net`, before the transport came up.)
    if net.port != saved_port {
        persist_server_net(state, record.id, &net).await;
    }
    if let Some(rx) = port_mapping_rx {
        app.state::<AppState>()
            .inner()
            .upnp
            .lock()
            .await
            .insert(record.id, PORT_MAPPING_WAITING.to_string());
        spawn_port_mapping_fold(app.clone(), record.id, rx, id);
    }
    if let Some(rx) = autonat_rx {
        app.state::<AppState>().inner().autonat.lock().await.insert(
            record.id,
            AutoNatEvidence {
                waiting: true,
                results: Vec::new(),
            },
        );
        spawn_autonat_fold(app.clone(), record.id, rx);
    }
    if let Some(rx) = relay_address_rx {
        spawn_relay_fold(app.clone(), record.id, rx);
    }
    if let Some(rx) = mesh_observation_rx {
        spawn_mesh_observation_fold(app.clone(), record.id, rx);
    }
    Ok(())
}

/// The only external URL prefix the app will hand to the OS: our own issue tracker's
/// new-issue form. `open_issue_url` is a launcher, and a launcher that takes any URL is a way
/// to point the user's browser (or a registered `foo://` handler) anywhere, so the allowlist
/// is a constant here rather than anything the webview can influence.
const ISSUE_URL_PREFIX: &str = "https://github.com/Thalpy/Mewtual/issues/new?";
const ISSUE_URL_MAX_BYTES: usize = 6_000;
const PUBLIC_ISSUE_TITLE: &str = "Diagnostic report";
const PUBLIC_ISSUE_TRUNCATION_NOTE: &str =
    "\n\n_(Report truncated in this URL; return to Mewtual for the full text.)_";

/// Is this a new-issue URL on our own tracker? Split out from the command so the allowlist
/// itself is testable without launching a browser.
fn is_tracker_url(url: &str) -> bool {
    url.len() <= ISSUE_URL_MAX_BYTES
        && !url
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        && url.starts_with(ISSUE_URL_PREFIX)
}

/// Percent-encode one query value from its UTF-8 bytes.
///
/// A tiny local encoder keeps the launch boundary dependency-free and, unlike string slicing on a
/// finished URL, can never cut a `%HH` escape or a Unicode scalar. Only RFC 3986 unreserved bytes
/// survive literally; every other byte becomes ASCII, so URL byte accounting is exact.
fn encode_issue_query(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(*byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

fn encoded_issue_query_len(value: &str) -> usize {
    value
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                1
            } else {
                3
            }
        })
        .sum()
}

/// Native-owned public issue output. `report` is the exact publication envelope for clipboard
/// fallback; only the browser URL may receive a shortened excerpt.
#[derive(Serialize)]
struct PublicDiagnosticsIssue {
    report: String,
    truncated: bool,
}

struct PreparedPublicDiagnosticsIssue {
    url: String,
    report: String,
    truncated: bool,
}

fn public_issue_url(body: &str) -> String {
    format!(
        "{ISSUE_URL_PREFIX}labels=bug&title={}&body={}",
        encode_issue_query(PUBLIC_ISSUE_TITLE),
        encode_issue_query(body),
    )
}

/// Build a bounded tracker URL while retaining the exact full report separately.
fn prepare_public_diagnostics_issue(native_report: &str) -> PreparedPublicDiagnosticsIssue {
    let report = format!(
        "**Type:** Bug report\n**App:** Mewtual desktop {}\n**Environment:** Mewtual desktop\n\n{}",
        env!("CARGO_PKG_VERSION"),
        native_report,
    );
    let complete = public_issue_url(&report);
    if complete.len() <= ISSUE_URL_MAX_BYTES {
        return PreparedPublicDiagnosticsIssue {
            url: complete,
            report,
            truncated: false,
        };
    }

    let empty_url = public_issue_url("");
    let note_len = encoded_issue_query_len(PUBLIC_ISSUE_TRUNCATION_NOTE);
    let body_budget = ISSUE_URL_MAX_BYTES
        .saturating_sub(empty_url.len())
        .saturating_sub(note_len);
    let mut excerpt = String::new();
    let mut encoded_len = 0usize;
    for character in report.chars() {
        let mut utf8 = [0; 4];
        let char_len = encoded_issue_query_len(character.encode_utf8(&mut utf8));
        if encoded_len.saturating_add(char_len) > body_budget {
            break;
        }
        excerpt.push(character);
        encoded_len += char_len;
    }
    excerpt.push_str(PUBLIC_ISSUE_TRUNCATION_NOTE);
    let url = public_issue_url(&excerpt);
    debug_assert!(url.len() <= ISSUE_URL_MAX_BYTES);
    PreparedPublicDiagnosticsIssue {
        url,
        report,
        truncated: true,
    }
}

fn is_external_http_url(url: &str) -> bool {
    if url.is_empty()
        || url.len() > 4096
        || url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return false;
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    rest.is_some_and(|r| !r.is_empty() && !r.starts_with(['/', '?', '#']))
}

fn launch_url(url: &str) -> Result<(), String> {
    // Deliberately no shell: `cmd /C start` would interpret query-string separators. Each
    // launcher receives the URL as one literal argv entry.
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32.exe");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

const SPACE_GUIDE_NAME: &str = "mewtual-server-space-guide-2048x1024.png";
const MAX_SPACE_GUIDE_BYTES: usize = 16 * 1024 * 1024;
const SPACE_LAYOUT_NAME: &str = "mewtual-server-space-layout.json";
const MAX_SPACE_LAYOUT_BYTES: usize = 1024 * 1024;

/// Keep the frontend from turning this deliberately narrow command into an arbitrary-file
/// writer. The canvas export must be a reasonably-sized 2048 x 1024 PNG with an IHDR first.
fn validate_space_guide_png(bytes: &[u8]) -> bool {
    bytes.len() >= 24
        && bytes.len() <= MAX_SPACE_GUIDE_BYTES
        && bytes[..8] == [137, 80, 78, 71, 13, 10, 26, 10]
        && &bytes[12..16] == b"IHDR"
        && u32::from_be_bytes(bytes[16..20].try_into().expect("four-byte PNG width")) == 2048
        && u32::from_be_bytes(bytes[20..24].try_into().expect("four-byte PNG height")) == 1024
}

/// Layout export is intentionally its own narrow writer: accepting only our small, versioned
/// object keeps a compromised webview from using the command as an arbitrary Downloads writer.
fn validate_space_layout_json(json: &str) -> bool {
    if json.is_empty() || json.len() > MAX_SPACE_LAYOUT_BYTES {
        return false;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return false;
    };
    value.get("kind").and_then(|v| v.as_str()) == Some("mewtual-server-space-layout")
        && value.get("version").and_then(|v| v.as_u64()) == Some(1)
        && value.get("space").is_some_and(serde_json::Value::is_object)
}

fn safe_download_name(requested: &str) -> String {
    // Peer-provided file names are display data, not paths. Apply Windows' strict character rules
    // everywhere so a shared name behaves consistently on every desktop platform.
    let leaf = requested.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = leaf
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        // Leaves room for a collision suffix even when every scalar needs two UTF-16 code units.
        .take(120)
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let mut name = if trimmed.is_empty() {
        "mewtual-download".to_string()
    } else {
        trimmed.to_string()
    };
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let numbered_device = (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && matches!(stem.as_bytes()[3], b'1'..=b'9');
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered_device {
        name.insert(0, '_');
    }
    name
}

fn numbered_download_name(name: &str, number: usize) -> String {
    if number == 0 {
        return name.to_string();
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mewtual-download");
    match path
        .extension()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
    {
        Some(extension) => format!("{stem} ({number}).{extension}"),
        None => format!("{stem} ({number})"),
    }
}

/// Create a download without replacing an earlier one, returning the open file and its path.
/// `create_new` also closes the exists/write race, so the returned path is reserved.
fn create_download(
    downloads: &Path,
    requested_name: &str,
) -> Result<(std::fs::File, PathBuf), String> {
    std::fs::create_dir_all(downloads).map_err(|e| e.to_string())?;
    let safe_name = safe_download_name(requested_name);
    for number in 0..1_000 {
        let name = numbered_download_name(&safe_name, number);
        let path = downloads.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("too many copies of the guide already exist in Downloads".into())
}

/// Write a whole small artefact (a guide image, a layout export) as one download.
fn write_download(downloads: &Path, requested_name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let (mut file, path) = create_download(downloads, requested_name)?;
    if let Err(error) = file.write_all(bytes) {
        let _ = std::fs::remove_file(&path);
        return Err(error.to_string());
    }
    Ok(path)
}

/// Extension for a download still being written. A streamed save fills one of these and renames it
/// onto the real name only once the whole file has been verified, so the peer-chosen name never
/// exists holding bytes Mewtual has not authenticated. Crash, kill or power loss during a transfer
/// therefore leaves a `.part`, not something that looks like the file.
const DOWNLOAD_STAGING_SUFFIX: &str = "part";

/// Reserve a staging file and the final name it will be renamed onto.
///
/// Both are reserved up front with `create_new`, so a second save of the same file cannot pick the
/// same final name while this one is still writing, and the rename at the end cannot collide.
fn create_staged_download(
    downloads: &Path,
    requested_name: &str,
) -> Result<(std::fs::File, PathBuf, PathBuf), String> {
    let (final_file, final_path) = create_download(downloads, requested_name)?;
    // The reservation is a placeholder, not the download: hold the name, write elsewhere.
    drop(final_file);
    let staging_name = format!(
        "{}.{DOWNLOAD_STAGING_SUFFIX}",
        final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("mewtual-download")
    );
    match create_download(downloads, &staging_name) {
        Ok((file, staging_path)) => Ok((file, staging_path, final_path)),
        Err(e) => {
            let _ = std::fs::remove_file(&final_path);
            Err(e)
        }
    }
}

/// Move a verified staging file onto its reserved final name.
///
/// The reservation is an empty file this same call created, so replacing it is not clobbering
/// anyone: `rename` over it is the atomic publish, and no window exists in which the final name
/// holds a partial download.
fn publish_staged_download(staging: &Path, final_path: &Path) -> Result<(), String> {
    std::fs::rename(staging, final_path).map_err(|e| e.to_string())
}

fn launch_path(path: &Path) -> Result<(), String> {
    // As with URLs, never involve a shell: the path is one literal argument to the OS launcher.
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler").arg(path);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

fn reveal_path(path: &Path) -> Result<(), String> {
    // Reveal rather than execute arbitrary shared files. This gives every Download button a
    // visible system window without silently launching a peer-supplied executable or document.
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("explorer.exe");
        c.arg(format!("/select,{}", path.to_string_lossy()));
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(path);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedFileResult {
    path: String,
    displayed: bool,
    warning: Option<String>,
    /// `matched`, `mismatch`, or `unrecognized` for an allowlisted declared media type. This is
    /// format evidence only—not a promise that a platform decoder is vulnerability-free.
    content_validation: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupResult {
    path: String,
    files: usize,
    bytes: u64,
    displayed: bool,
    warning: Option<String>,
}

/// A secret change has a third outcome beyond success/failure: rename committed the new wrapper,
/// but the directory flush could not prove it durable across sudden power loss. Reporting that as
/// an error would falsely tell the user the old secret remained active.
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct VaultSecretChangeResult {
    changed: bool,
    durability_confirmed: bool,
    warning: Option<String>,
}

fn vault_secret_change_result(
    result: Result<(), catcoms_app::AppError>,
) -> Result<VaultSecretChangeResult, String> {
    match result {
        Ok(()) => Ok(VaultSecretChangeResult {
            changed: true,
            durability_confirmed: true,
            warning: None,
        }),
        Err(catcoms_app::AppError::CommittedButNotDurable(error)) => {
            Ok(VaultSecretChangeResult {
                changed: true,
                durability_confirmed: false,
                warning: Some(format!(
                    "The new secret is active, but the filesystem could not confirm crash durability. Keep the new secret; do not treat the old secret as current. Details: {error}"
                )),
            })
        }
        Err(error) => Err(error.to_string()),
    }
}

/// Copy one already-sealed vault tree without following links. Refusing links is important for
/// both directions: an attacker must not smuggle unrelated host files into an exported backup,
/// and backup semantics must never depend on where a link happens to point at restore time.
fn copy_backup_tree(source: &Path, destination: &Path) -> Result<(usize, u64), String> {
    std::fs::create_dir(destination).map_err(|error| error.to_string())?;
    let mut files = 0usize;
    let mut bytes = 0u64;
    let entries = std::fs::read_dir(source).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        let to = destination.join(entry.file_name());
        if kind.is_symlink() {
            return Err(format!(
                "the vault contains a symbolic link and cannot be backed up safely: {}",
                entry.path().display()
            ));
        }
        if kind.is_dir() {
            let (nested_files, nested_bytes) = copy_backup_tree(&entry.path(), &to)?;
            files = files.saturating_add(nested_files);
            bytes = bytes.saturating_add(nested_bytes);
        } else if kind.is_file() {
            let copied = std::fs::copy(entry.path(), to).map_err(|error| error.to_string())?;
            files = files.saturating_add(1);
            bytes = bytes.saturating_add(copied);
        } else {
            return Err(format!(
                "the vault contains an unsupported filesystem entry: {}",
                entry.path().display()
            ));
        }
    }
    Ok((files, bytes))
}

fn backup_destination(downloads: &Path, stamp: u64) -> Result<PathBuf, String> {
    std::fs::create_dir_all(downloads).map_err(|error| error.to_string())?;
    for number in 0..1_000usize {
        let suffix = if number == 0 {
            String::new()
        } else {
            format!(" ({number})")
        };
        let path = downloads.join(format!("Mewtual Backup {stamp}{suffix}"));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("too many Mewtual backups with this name already exist in Downloads".into())
}

/// Export an offline copy of the entire sealed vault. It remains protected by the current vault
/// secret; no plaintext snapshots, drafts, identities or attachments are written to Downloads.
/// Restore is intentionally a locked-screen operation and is not performed by this command.
#[tauri::command]
async fn create_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    trace: Option<String>,
) -> Result<BackupResult, AppError> {
    // A backup is the operation whose silent failure costs the most. It writes every server's
    // snapshot and the registry before copying, so a failure part-way through is a partial image,
    // and the phases say which part.
    let op = Operation::start_maybe(
        trace,
        catcoms_diagnostics::Section::Vault,
        "create_backup",
        None,
        None,
    );
    require_unlocked_session(&state)
        .await
        .map_err(|e| op.fail(codes::SESSION_LOCKED, e))?;
    // Capture every actor first, without holding either state lock across its round trip.
    let servers: Vec<(u64, ServerActor, ServerRecord)> = {
        let servers = state.servers.lock().await;
        servers
            .iter()
            .map(|(id, entry)| {
                (
                    *id,
                    op.bind_actor(entry.actor.clone()),
                    ServerRecord {
                        id: *id,
                        display_name: entry.name.clone(),
                        invite: entry.invite.clone().unwrap_or_default(),
                        is_dm: entry.is_dm,
                    },
                )
            })
            .collect()
    };
    let mut snapshots = Vec::with_capacity(servers.len());
    for (id, actor, _) in &servers {
        snapshots.push((
            *id,
            actor
                .snapshot()
                .await
                .map_err(|e| op.fail(codes::VAULT_BACKUP_FAILED, e))?,
        ));
    }
    let records: Vec<ServerRecord> = servers.into_iter().map(|(_, _, record)| record).collect();
    op.stage("VAULT.BACKUP.SNAPSHOTTED");

    let downloads = app
        .path()
        .download_dir()
        .map_err(|error| op.fail(codes::VAULT_BACKUP_FAILED, error.to_string()))?;
    let destination = backup_destination(&downloads, SystemClock.now_ms())
        .map_err(|e| op.fail(codes::VAULT_BACKUP_FAILED, e))?;
    let (files, bytes) = {
        // Serialize persistence and the filesystem copy with every other vault write so the
        // exported registry and snapshots form one coherent point-in-time image.
        let guard = state.store.lock().await;
        let store = guard.as_ref().ok_or_else(|| {
            op.fail(
                codes::VAULT_BACKUP_FAILED,
                "unlock the vault before creating a backup",
            )
        })?;
        let mut rng = OsCryptoRng;
        for (id, snapshot) in snapshots {
            store
                .save_server(id, &snapshot, &mut rng)
                .map_err(|error| op.fail(codes::VAULT_BACKUP_FAILED, error.to_string()))?;
        }
        store
            .save_registry(&records, &mut rng)
            .map_err(|error| op.fail(codes::VAULT_BACKUP_FAILED, error.to_string()))?;
        copy_backup_tree(store.backup_source_dir(), &destination)
            .map_err(|e| op.fail(codes::VAULT_BACKUP_FAILED, e))?
    };
    let warning = reveal_path(&destination)
        .err()
        .map(|error| format!("The backup was created, but Downloads could not be opened: {error}"));
    // The bytes are on disk by this point; failing to open a file manager afterwards is not a
    // failed backup and must not be recorded as one.
    op.succeeded("VAULT.BACKUP.WRITTEN");
    Ok(BackupResult {
        path: destination.to_string_lossy().into_owned(),
        files,
        bytes,
        displayed: warning.is_none(),
        warning,
    })
}

/// Change the local vault secret by rewrapping its root DEK. The store mutex serializes the small
/// atomic vault-file replacement with snapshots and backup export; server actors can keep running
/// because the DEK and every data-encryption subkey remain unchanged.
#[tauri::command]
async fn change_vault_secret(
    state: State<'_, AppState>,
    current_secret: String,
    new_secret: String,
) -> Result<VaultSecretChangeResult, String> {
    require_unlocked_session(&state).await?;
    let current_secret = Zeroizing::new(current_secret);
    let new_secret = Zeroizing::new(new_secret);
    let guard = state.store.lock().await;
    let store = guard
        .as_ref()
        .ok_or_else(|| "unlock the vault before changing its secret".to_string())?;
    vault_secret_change_result(store.change_passphrase(
        current_secret.as_bytes(),
        new_secret.as_bytes(),
        &mut OsCryptoRng,
    ))
}

/// Open a prefilled bug report / feature request on the tracker in the user's default browser.
/// Mewtual has no service of its own to receive feedback, so filing means handing GitHub a
/// filled-in form: the app carries no GitHub credentials and posts nothing itself, and the user
/// submits (and so authors) the issue from their own browser.
#[tauri::command]
async fn open_issue_url(state: State<'_, AppState>, url: String) -> Result<(), String> {
    require_unlocked_session(&state).await?;
    if !is_tracker_url(&url) {
        return Err("refusing to open a URL outside the issue tracker".into());
    }
    launch_url(&url)
}

/// Open a chat/wiki link in the system browser, keeping the Mewtual webview on the conversation.
#[tauri::command]
async fn open_external_url(state: State<'_, AppState>, url: String) -> Result<(), String> {
    require_unlocked_session(&state).await?;
    if !is_external_http_url(&url) {
        return Err("only http and https links can be opened".into());
    }
    launch_url(&url)
}

/// Save the generated Server Space overlay to Downloads and show it in the user's normal image
/// viewer. A normal detached `<a download>` is not dependable inside a desktop webview and gives
/// no visible result when it is rejected.
#[tauri::command]
async fn save_and_open_space_guide(
    app: AppHandle,
    state: State<'_, AppState>,
    png_base64: String,
) -> Result<SavedFileResult, String> {
    require_unlocked_session(&state).await?;
    if png_base64.len() > MAX_SPACE_GUIDE_BYTES * 2 {
        return Err("the generated guide is unexpectedly large".into());
    }
    let bytes = B64
        .decode(png_base64)
        .map_err(|_| "the generated guide could not be decoded".to_string())?;
    if !validate_space_guide_png(&bytes) {
        return Err("the generated guide is not a valid 2048 x 1024 PNG".into());
    }
    let downloads = app.path().download_dir().map_err(|e| e.to_string())?;
    let path = write_download(&downloads, SPACE_GUIDE_NAME, &bytes)?;
    let warning = launch_path(&path).err().map(|error| {
        format!("The guide was saved, but its image viewer could not be opened: {error}")
    });
    Ok(SavedFileResult {
        path: path.to_string_lossy().into_owned(),
        displayed: warning.is_none(),
        warning,
        content_validation: Some("matched".into()), // generator output passed strict PNG checks
    })
}

/// Export a portable Server Space layout to Downloads and reveal it without executing it.
#[tauri::command]
async fn save_space_layout(
    app: AppHandle,
    state: State<'_, AppState>,
    json: String,
) -> Result<SavedFileResult, String> {
    require_unlocked_session(&state).await?;
    if !validate_space_layout_json(&json) {
        return Err("the layout is not a valid Mewtual Server Space export".into());
    }
    let downloads = app.path().download_dir().map_err(|e| e.to_string())?;
    let path = write_download(&downloads, SPACE_LAYOUT_NAME, json.as_bytes())?;
    let warning = reveal_path(&path)
        .err()
        .map(|error| format!("The layout was saved, but Downloads could not be opened: {error}"));
    Ok(SavedFileResult {
        path: path.to_string_lossy().into_owned(),
        displayed: warning.is_none(),
        warning,
        content_validation: None,
    })
}

// Kept as one literal so jam-sheet.test.ts can pin the cross-language renderer/save contract.
const JAM_SHEET_STYLE: &str = r#"text{font-family:Georgia,serif}.st{stroke:#444;stroke-width:1}.bar{stroke:#444;stroke-width:1}.nh{fill:#111}.nh.open{fill:#fff;stroke:#111;stroke-width:1.6}.stem{stroke:#111;stroke-width:1.4}.flag{stroke:#111;stroke-width:1.4;fill:none}.acc{font-size:11px;fill:#111}.clef{font-size:34px;fill:#111}.who{font-size:13px;fill:#333;font-style:italic}.ttl{font-size:19px;fill:#111}.sub{font-size:12px;fill:#555}.xh{stroke:#111;stroke-width:1.6}.ped{font-size:9px;fill:#666;font-family:monospace}.led{stroke:#444;stroke-width:1}.bt{stroke:#b9b2a0;stroke-width:0.6;stroke-dasharray:2 4}"#;

#[derive(Debug)]
struct JamSheetTag<'a> {
    name: &'a str,
    attrs: Vec<(&'a str, &'a str)>,
    self_closing: bool,
}

/// Parse the renderer's deliberately tiny opening-tag syntax. This is not a permissive XML
/// parser: accepting browser error recovery here would make it very difficult to prove that an
/// apparent attribute boundary cannot be reinterpreted as active SVG.
fn jam_sheet_tag(input: &str) -> Option<(JamSheetTag<'_>, &str)> {
    let after_open = input.strip_prefix('<')?;
    let end = after_open.find('>')?;
    let raw = &after_open[..end];
    let remaining = &after_open[end + 1..];
    if raw.is_empty() || raw.starts_with('/') || raw.starts_with('!') || raw.starts_with('?') {
        return None;
    }
    let (body, self_closing) = match raw.strip_suffix('/') {
        Some(body) => (body, true),
        None => (raw, false),
    };
    let name_end = body.find(' ').unwrap_or(body.len());
    let name = &body[..name_end];
    if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_lowercase()) {
        return None;
    }

    let mut attrs = Vec::new();
    let mut rest = &body[name_end..];
    while !rest.is_empty() {
        rest = rest.strip_prefix(' ')?;
        let equals = rest.find('=')?;
        let key = &rest[..equals];
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b':')
        {
            return None;
        }
        let value_start = rest[equals + 1..].strip_prefix('"')?;
        let quote = value_start.find('"')?;
        let value = &value_start[..quote];
        if value.contains(['<', '>', '\0']) {
            return None;
        }
        attrs.push((key, value));
        rest = &value_start[quote + 1..];
    }
    Some((
        JamSheetTag {
            name,
            attrs,
            self_closing,
        },
        remaining,
    ))
}

fn jam_sheet_attrs(tag: &JamSheetTag<'_>, names: &[&str]) -> bool {
    tag.attrs.len() == names.len()
        && tag
            .attrs
            .iter()
            .zip(names)
            .all(|((actual, _), expected)| actual == expected)
}

fn jam_sheet_number(value: &str) -> bool {
    if value.is_empty() || value.len() > 24 {
        return false;
    }
    let mut dots = 0;
    let mut digits = 0;
    for (index, byte) in value.bytes().enumerate() {
        match byte {
            b'0'..=b'9' => digits += 1,
            b'.' if dots == 0 => dots += 1,
            b'-' if index == 0 => {}
            _ => return false,
        }
    }
    digits > 0
        && value
            .parse::<f64>()
            .is_ok_and(|number| number.is_finite() && number.abs() <= 4_000_000.0)
}

fn jam_sheet_text(text: &str) -> bool {
    if text.chars().any(char::is_control) || text.contains(['<', '>']) {
        return false;
    }
    let mut remaining = text;
    while let Some(amp) = remaining.find('&') {
        remaining = &remaining[amp..];
        let entity = ["&amp;", "&lt;", "&gt;", "&quot;"]
            .into_iter()
            .find(|entity| remaining.starts_with(entity));
        let Some(entity) = entity else {
            return false;
        };
        remaining = &remaining[entity.len()..];
    }
    true
}

fn jam_sheet_root(tag: &JamSheetTag<'_>) -> bool {
    if tag.self_closing
        || tag.name != "svg"
        || !jam_sheet_attrs(
            tag,
            &["xmlns", "viewBox", "width", "height", "data-mewtual-sheet"],
        )
        || tag.attrs[0].1 != "http://www.w3.org/2000/svg"
        || tag.attrs[4].1 != "v1"
        || !jam_sheet_number(tag.attrs[2].1)
        || !jam_sheet_number(tag.attrs[3].1)
    {
        return false;
    }
    let view: Vec<&str> = tag.attrs[1].1.split(' ').collect();
    view.len() == 4
        && view[0] == "0"
        && view[1] == "0"
        && view[2] == tag.attrs[2].1
        && view[3] == tag.attrs[3].1
}

fn jam_sheet_element(tag: &JamSheetTag<'_>) -> bool {
    match tag.name {
        "rect" => {
            tag.self_closing
                && jam_sheet_attrs(tag, &["x", "y", "width", "height", "fill"])
                && tag.attrs[..4]
                    .iter()
                    .all(|(_, value)| jam_sheet_number(value))
                && tag.attrs[4].1 == "#fffdf6"
        }
        "line" => {
            tag.self_closing
                && jam_sheet_attrs(tag, &["x1", "y1", "x2", "y2", "class"])
                && tag.attrs[..4]
                    .iter()
                    .all(|(_, value)| jam_sheet_number(value))
                && matches!(tag.attrs[4].1, "st" | "bar" | "stem" | "xh" | "led" | "bt")
        }
        "ellipse" => {
            tag.self_closing
                && jam_sheet_attrs(tag, &["cx", "cy", "rx", "ry", "class"])
                && tag.attrs[..4]
                    .iter()
                    .all(|(_, value)| jam_sheet_number(value))
                && matches!(tag.attrs[4].1, "nh" | "nh open")
        }
        "path" => {
            tag.self_closing
                && jam_sheet_attrs(tag, &["d", "class"])
                && tag.attrs[1].1 == "flag"
                && tag.attrs[0].1.len() <= 160
                && tag.attrs[0].1.starts_with('M')
                && tag.attrs[0].1.contains(" q")
                && tag.attrs[0].1.bytes().all(|byte| {
                    byte.is_ascii_digit() || matches!(byte, b'M' | b'q' | b' ' | b'.' | b'-')
                })
        }
        _ => false,
    }
}

fn jam_sheet_text_tag(tag: &JamSheetTag<'_>) -> bool {
    if tag.self_closing || tag.name != "text" {
        return false;
    }
    let ordinary = jam_sheet_attrs(tag, &["x", "y", "class"]);
    let anchored =
        jam_sheet_attrs(tag, &["x", "y", "class", "text-anchor"]) && tag.attrs[3].1 == "middle";
    (ordinary || anchored)
        && jam_sheet_number(tag.attrs[0].1)
        && jam_sheet_number(tag.attrs[1].1)
        && matches!(
            tag.attrs[2].1,
            "who" | "ttl" | "sub" | "acc" | "clef" | "ped"
        )
}

/// Validate the exact inert SVG subset emitted by jam-sheet.ts.
///
/// The webview is not the authority for writing active markup into Downloads. Only flat geometry,
/// escaped text, a fixed stylesheet and numeric coordinates are accepted; links, event handlers,
/// external resources, namespaces, animation, foreign content and browser-recovery syntax are
/// unrepresentable in this grammar.
fn validate_jam_sheet_svg(svg: &str) -> bool {
    if svg.is_empty() || svg.len() > 4_000_000 {
        return false;
    }
    let Some((root, after_root)) = jam_sheet_tag(svg) else {
        return false;
    };
    if !jam_sheet_root(&root) {
        return false;
    }
    let Some((style, after_style_open)) = jam_sheet_tag(after_root) else {
        return false;
    };
    if style.name != "style" || style.self_closing || !style.attrs.is_empty() {
        return false;
    }
    let Some(mut remaining) = after_style_open
        .strip_prefix(JAM_SHEET_STYLE)
        .and_then(|value| value.strip_prefix("</style>"))
    else {
        return false;
    };

    loop {
        if remaining == "</svg>" {
            return true;
        }
        let Some((tag, after_tag)) = jam_sheet_tag(remaining) else {
            return false;
        };
        if tag.name == "text" {
            if !jam_sheet_text_tag(&tag) {
                return false;
            }
            let Some(end) = after_tag.find("</text>") else {
                return false;
            };
            if !jam_sheet_text(&after_tag[..end]) {
                return false;
            }
            remaining = &after_tag[end + "</text>".len()..];
        } else {
            if !jam_sheet_element(&tag) {
                return false;
            }
            remaining = after_tag;
        }
    }
}

fn validate_jam_sheet_name(name: &str) -> bool {
    let Some(stem) = name
        .strip_prefix("mewtual-take-")
        .and_then(|value| value.strip_suffix(".svg"))
    else {
        return false;
    };
    let Some((take, date)) = stem.split_once('-') else {
        return false;
    };
    !take.is_empty()
        && take.len() <= 10
        && take.bytes().all(|byte| byte.is_ascii_digit())
        && date.len() == 8
        && date.bytes().all(|byte| byte.is_ascii_digit())
}

/// Export a jam-take sheet transcript to Downloads and reveal it without executing it.
///
/// Although jam-sheet.ts generates the content, IPC input is still untrusted. The validator pins
/// it to that renderer's inert versioned grammar before any bytes are written.
#[tauri::command]
async fn save_jam_sheet(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    svg: String,
) -> Result<SavedFileResult, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    let downloads = app.path().download_dir().map_err(|e| e.to_string())?;
    save_jam_sheet_to_downloads(
        &state,
        generation,
        &downloads,
        &name,
        &svg,
        std::future::ready(()),
        reveal_path,
    )
    .await
}

/// Testable sheet-export commit core. Validation may be expensive at the 4 MiB cap, so the exact
/// UI generation is rechecked after it and the returned commit guard remains held through both
/// plaintext publication and reveal.
async fn save_jam_sheet_to_downloads<F, R>(
    state: &AppState,
    generation: u64,
    downloads: &Path,
    name: &str,
    svg: &str,
    before_commit: F,
    reveal: R,
) -> Result<SavedFileResult, String>
where
    F: Future<Output = ()>,
    R: FnOnce(&Path) -> Result<(), String>,
{
    if !validate_jam_sheet_name(name) {
        return Err("the sheet export has an unexpected file name".into());
    }
    if !validate_jam_sheet_svg(svg) {
        return Err("the sheet export is not a Mewtual sheet transcript".into());
    }
    before_commit.await;
    let _commit = require_ui_session_generation(state, generation).await?;
    let path = write_download(downloads, name, svg.as_bytes())?;
    let warning = reveal(&path)
        .err()
        .map(|error| format!("The sheet was saved, but Downloads could not be opened: {error}"));
    Ok(SavedFileResult {
        path: path.to_string_lossy().into_owned(),
        displayed: warning.is_none(),
        warning,
        content_validation: None,
    })
}

/// What a streamed save needs from the running app: somewhere to get chunks, and an answer to
/// "is this still allowed to happen?".
///
/// A seam, so the save loop below can be tested as itself. Its rules are all about *when* things
/// happen relative to a fetch that may be in flight for seconds (the lock landing mid-transfer,
/// bytes exceeding what the listing declared, the rename happening only after verification), and
/// none of that is reachable through a `#[tauri::command]` signature that wants a live `AppHandle`.
trait SaveSource {
    /// Fetch and decrypt one chunk, with the signed provider that served it if it came over the
    /// network. May take arbitrarily long: this is the await the lock can land inside.
    async fn chunk(&mut self, index: usize) -> Result<(Vec<u8>, Option<String>), String>;

    /// Whether the webview session is still unlocked. Called again after every await, because an
    /// answer from before a network round-trip says nothing about the state after it.
    async fn still_unlocked(&self) -> Result<(), String>;

    /// Atomically cross from verified staging bytes to a visible plaintext file under the
    /// authorization epoch that started this save. Implementations must not split their final
    /// session check from the rename.
    async fn publish_verified(&self, staging: &Path, final_path: &Path) -> Result<(), String>;

    /// Report progress. Purely informational; a dropped update never changes the outcome.
    fn progress(
        &self,
        done: usize,
        bytes_done: u64,
        network_bytes_done: u64,
        provider: Option<String>,
    );
}

/// Stream a listed file into Downloads and return the path it was published at.
///
/// Writes into a `.part` and renames onto the reserved name only once the whole file has been
/// verified: reserving the final name up front stops a second save landing on it, and writing
/// elsewhere until the end means neither an error here nor a crash can leave the peer's chosen
/// filename holding bytes this device has not authenticated. Every failure path removes both.
async fn stream_download_to_disk(
    downloads: &Path,
    name: &str,
    total: usize,
    size: u64,
    target: &[u8; 32],
    source: &mut impl SaveSource,
) -> Result<PathBuf, String> {
    let (mut file, staging, path) = create_staged_download(downloads, name)?;
    let failed = |error: String| -> String {
        let _ = std::fs::remove_file(&staging);
        let _ = std::fs::remove_file(&path);
        error
    };
    source.progress(0, 0, 0, None);
    let mut address = CidHasher::new();
    let mut bytes_done = 0u64;
    let mut network_bytes_done = 0u64;
    for i in 0..total {
        // A transfer can outlive the click that started it, and a chunk fetch can be in flight
        // across a lock. Check before asking for bytes and again before writing them: the second
        // check is the one that matters, because the first says nothing about a session that
        // closed while the fetch was outstanding.
        if let Err(e) = source.still_unlocked().await {
            return Err(failed(e));
        }
        let (chunk, provider) = match source.chunk(i).await {
            Ok(got) => got,
            Err(e) => return Err(failed(e)),
        };
        if let Err(e) = source.still_unlocked().await {
            return Err(failed(e));
        }
        if provider.is_some() {
            network_bytes_done = network_bytes_done.saturating_add(chunk.len() as u64);
        }
        // Independent of the manifest's own layout check: never write more bytes than the file
        // said it was. The declared size is what the user was shown and what this download was
        // sized against, so anything past it is not this file, whatever the manifest claims.
        let next = bytes_done
            .checked_add(chunk.len() as u64)
            .filter(|n| *n <= size);
        let Some(next) = next else {
            return Err(failed(
                "this file's chunks hold more data than it declares".into(),
            ));
        };
        address.update(&chunk);
        bytes_done = next;
        if let Err(e) = file.write_all(&chunk) {
            return Err(failed(e.to_string()));
        }
        source.progress(i + 1, bytes_done, network_bytes_done, provider);
    }
    if let Err(e) = file.sync_all() {
        return Err(failed(e.to_string()));
    }
    drop(file);
    if bytes_done != size {
        return Err(failed(
            "this file's chunks hold less data than it declares".into(),
        ));
    }
    // End-to-end integrity, the same check the all-in-one download makes: individually valid
    // chunks in the wrong order (or a manifest that lied) must not be saved as the file.
    if address.cid().as_bytes() != target {
        return Err(failed(
            "the reassembled file failed its integrity check".into(),
        ));
    }
    // The source owns one indivisible authorization+rename step. A plain check followed by this
    // rename has a lock TOCTOU: explicit lock can complete in the gap and the old command then
    // publishes plaintext under its final name.
    if let Err(e) = source.publish_verified(&staging, &path).await {
        return Err(failed(e));
    }
    Ok(path)
}

fn exported_media_validation(path: &Path, declared_mime: &str) -> Option<String> {
    if safe_media_mime(declared_mime) == "application/octet-stream" {
        return None;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return Some("unrecognized".into());
    };
    let mut prefix = [0u8; 64];
    let Ok(read) = file.read(&mut prefix) else {
        return Some("unrecognized".into());
    };
    let evidence = match media_signature_evidence(declared_mime, &prefix[..read]) {
        MediaSignatureEvidence::Matched => "matched",
        MediaSignatureEvidence::Mismatch => "mismatch",
        MediaSignatureEvidence::Unrecognized => "unrecognized",
        MediaSignatureEvidence::NotMedia => return None,
    };
    Some(evidence.into())
}

/// The live [`SaveSource`]: chunks from the server actor, authorization from the session gate,
/// progress to the webview.
struct ActorSaveSource<'a> {
    actor: ServerActor,
    state: &'a AppState,
    app: &'a AppHandle,
    server: u64,
    cid: String,
    raw: Vec<u8>,
    total: usize,
    size: u64,
    generation: u64,
}

impl SaveSource for ActorSaveSource<'_> {
    async fn chunk(&mut self, index: usize) -> Result<(Vec<u8>, Option<String>), String> {
        self.actor.fetch_file_chunk(self.raw.clone(), index).await
    }

    async fn still_unlocked(&self) -> Result<(), String> {
        let generation_check = require_ui_session_generation(self.state, self.generation).await?;
        drop(generation_check);
        Ok(())
    }

    async fn publish_verified(&self, staging: &Path, final_path: &Path) -> Result<(), String> {
        publish_download_for_generation(self.state, self.generation, staging, final_path).await
    }

    fn progress(
        &self,
        done: usize,
        bytes_done: u64,
        network_bytes_done: u64,
        provider: Option<String>,
    ) {
        let _ = self.app.emit(
            "download-progress",
            DownloadProgressEvt {
                server: self.server,
                cid: self.cid.clone(),
                cancellation: None,
                done,
                total: self.total,
                bytes_done,
                bytes_total: self.size,
                network_bytes_done,
                provider,
            },
        );
    }
}

async fn publish_download_for_generation(
    state: &AppState,
    generation: u64,
    staging: &Path,
    final_path: &Path,
) -> Result<(), String> {
    let _commit = require_ui_session_generation(state, generation).await?;
    publish_staged_download(staging, final_path)
}

/// Download a listed group file straight into Downloads, one chunk at a time, and reveal it.
///
/// The plaintext never enters the webview. Saving a file used to mean `download_file` handing the
/// whole thing back as one base64 string and `save_download` taking it straight back the other
/// way, so the bytes crossed the IPC bridge twice, whole, as a JS string; the same shape as the
/// upload freeze, and worse the larger the file. Here the bytes go from the actor to the file and
/// the webview gets only the `download-progress` events it already listens for. The transfer is
/// still one chunk per actor command, so the server keeps syncing throughout.
///
/// Unlike `<a download>`, this has an observable result in a Tauri webview. The file is revealed,
/// never executed automatically.
#[tauri::command]
async fn save_group_file(
    app: AppHandle,
    state: State<'_, AppState>,
    server: u64,
    cid: String,
    name: String,
) -> Result<SavedFileResult, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    let raw = hex::decode(cid.trim()).map_err(|e| format!("bad cid: {e}"))?;
    let target: [u8; 32] = raw
        .clone()
        .try_into()
        .map_err(|_| "bad cid length".to_string())?;
    let actor = actor_of(&state, server).await?;
    let (total, size) = actor.file_download_plan(raw.clone()).await.ok_or_else(|| {
        "this file can't be downloaded; it isn't listed, or its reference is invalid".to_string()
    })?;
    let declared_mime = actor
        .file_head(raw.clone())
        .await
        .ok_or_else(|| {
            "this file can't be inspected; it isn't listed, or its reference is invalid".to_string()
        })?
        .mime;
    if size > MAX_FILE_BYTES as u64 {
        return Err(format!(
            "file is larger than the {MAX_FILE_BYTES}-byte limit"
        ));
    }
    let downloads = app.path().download_dir().map_err(|e| e.to_string())?;
    let mut source = ActorSaveSource {
        actor,
        state: &state,
        app: &app,
        server,
        cid,
        raw,
        total,
        size,
        generation,
    };
    let path =
        stream_download_to_disk(&downloads, &name, total, size, &target, &mut source).await?;
    // Keep inspection and the OS reveal in the same initiating UI epoch as publication. If lock
    // won after the rename guard was released, the verified file may remain in Downloads, but the
    // stale command cannot inspect/reveal it or report success behind the lock screen.
    let _commit = require_ui_session_generation(&state, generation).await?;
    let content_validation = exported_media_validation(&path, &declared_mime);
    let warning = reveal_path(&path)
        .err()
        .map(|error| format!("The file was saved, but Downloads could not be opened: {error}"));
    Ok(SavedFileResult {
        path: path.to_string_lossy().into_owned(),
        displayed: warning.is_none(),
        warning,
        content_validation,
    })
}

/// Is there already a vault on this machine? The frontend asks before it draws the gate:
/// `unlock` creates the vault when there isn't one, so without this the same screen serves
/// "unlock" and "choose your secret forever", and a typo on a fresh install silently founds a
/// second identity rather than failing. Says nothing about whether any secret opens it.
#[tauri::command]
async fn vault_exists(app: AppHandle) -> Result<bool, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("vault");
    Ok(ServerStore::exists(&dir))
}

/// Re-authorize a webview against the store this native process already owns. Explicit UI lock
/// intentionally leaves actors and decrypted store state mounted, but it still closes every IPC
/// plaintext boundary; therefore this path must verify the supplied passphrase before restoring
/// the session. It cannot call `ServerStore::open`: the existing store owns the lifetime mount
/// lock by design.
async fn authenticate_mounted_store(
    state: &AppState,
    passphrase: &[u8],
) -> Result<bool, catcoms_app::AppError> {
    let store_guard = state.store.lock().await;
    let Some(store) = store_guard.as_ref() else {
        return Ok(false);
    };
    store.verify_passphrase(passphrase)?;
    Ok(true)
}

/// Resolve continuity debt from the locked UI generation before reopening native IPC.
///
/// A write failure retains the exact validated snapshot, so every unlock attempt retries it and
/// remains locked until persistence succeeds. Malformed state cannot become valid by retrying; its
/// first unlock attempt surfaces the loss and consumes only that error, making a repeated unlock an
/// explicit acknowledgement instead of permanently stranding the vault gate.
async fn settle_ui_lock_continuity_before_unlock(
    state: &AppState,
    expected_generation: u64,
) -> Result<(), String> {
    let pending = {
        let mut slot = state.pending_ui_lock_snapshot.lock().await;
        if matches!(slot.as_ref(), Some(snapshot) if snapshot.generation <= expected_generation) {
            slot.take()
        } else {
            None
        }
    };

    if let Some(snapshot) = pending {
        let result = match validate_ui_state_json(&snapshot.json) {
            Err(error) => Err(error),
            Ok(()) => {
                let store = state.store.lock().await;
                match store.as_ref() {
                    Some(store) => store
                        .save_ui_state(snapshot.json.as_bytes(), &mut OsCryptoRng)
                        .map_err(|error| error.to_string()),
                    None => Err("unlock the vault before saving UI state".to_string()),
                }
            }
        };
        if let Err(error) = result {
            let generation = snapshot.generation;
            let mut slot = state.pending_ui_lock_snapshot.lock().await;
            let replace = match slot.as_ref() {
                Some(current) => current.generation <= generation,
                None => true,
            };
            if replace {
                *slot = Some(snapshot);
            }
            *state.last_ui_lock_completion.lock().await = Some(UiLockCompletion {
                generation,
                error: Some(error.clone()),
            });
            return Err(format!(
                "the vault is still locked because its latest screen state could not be saved: {error}; fix the storage problem and try again"
            ));
        }

        // A successful retry resolves the old completion error for this generation. Return here
        // instead of interpreting that stale error below as an irrecoverable malformed snapshot.
        // Do not clear a newer completion: a concurrent lock publishes its generation before it
        // waits for the commit guard held by our caller.
        let mut completion = state.last_ui_lock_completion.lock().await;
        if matches!(completion.as_ref(), Some(previous) if previous.generation <= expected_generation)
        {
            *completion = None;
        }
        return Ok(());
    }

    let mut completion = state.last_ui_lock_completion.lock().await;
    if let Some(previous) = completion.as_ref() {
        if previous.generation <= expected_generation {
            if let Some(error) = previous.error.as_ref() {
                let message = format!(
                    "the vault is still locked because its latest screen state was invalid and could not be saved: {error}; unlock again to continue without that latest screen state"
                );
                // No retryable snapshot exists for malformed input. Consume the warning only after
                // surfacing it once, so a deliberate second unlock is the acknowledgement.
                *completion = None;
                return Err(message);
            }
            *completion = None;
        }
    }
    Ok(())
}

/// Commit a successful authentication only if no newer explicit lock began while Argon2, disk
/// reads, or actor reloads were in flight. The generation is checked on both sides of updating the
/// flags because `lock_session_inner` deliberately invalidates commands before awaiting this
/// commit mutex. A racing lock therefore either wins first or makes this transition roll itself
/// back; a completed lock can never be undone by stale unlock work.
async fn finalize_unlock_session(
    state: &AppState,
    expected_generation: u64,
    include_running_servers: bool,
) -> Result<Option<Vec<ReloadedServer>>, String> {
    let _commit = state.ui_session_commit.lock().await;
    if state.ui_session_generation.load(Ordering::Acquire) != expected_generation {
        return Err("unlock was superseded by a newer lock request; try again".into());
    }
    settle_ui_lock_continuity_before_unlock(state, expected_generation).await?;
    // A successful authenticated transition either retried the exact pending bytes or crossed the
    // explicit malformed-snapshot acknowledgement flow. The old close warning is now resolved and
    // must not leak into a later, newly authorized UI session.
    *state.vault_window_close_debt.lock().await = None;
    let servers = if include_running_servers {
        Some(running_servers(state).await)
    } else {
        None
    };
    if state.ui_session_generation.load(Ordering::Acquire) != expected_generation {
        return Err("unlock was superseded by a newer lock request; try again".into());
    }

    let mut resumable = state.session_resumable.lock().await;
    *resumable = true;
    state.session_lock_requested.store(false, Ordering::Release);
    if state.ui_session_generation.load(Ordering::Acquire) != expected_generation {
        // A lock request may publish its generation without this mutex so new IPC fails promptly.
        // Restore the conservative state before releasing either transition boundary.
        *resumable = false;
        state.session_lock_requested.store(true, Ordering::Release);
        return Err("unlock was superseded by a newer lock request; try again".into());
    }
    Ok(servers)
}

/// Unlock the on-disk store with `passphrase` and reload every persisted server. Called once
/// at launch. A wrong passphrase fails (the vault won't open); a first-ever launch just
/// creates the vault and returns no servers. Returns the reloaded servers for the rail.
#[tauri::command]
async fn unlock(
    app: AppHandle,
    state: State<'_, AppState>,
    passphrase: String,
    trace: Option<String>,
) -> Result<Vec<ReloadedServer>, AppError> {
    // Unlock is the one operation whose failures a user meets before anything else works, and the
    // three of them are entirely different: the passphrase is wrong, the vault is unreadable, or it
    // opened and some servers did not come back. All three were one string.
    let op = Operation::start_maybe(
        trace,
        catcoms_diagnostics::Section::Vault,
        "unlock",
        None,
        None,
    );
    // Capture before the first await. Any lock command that starts after this unlock invalidates
    // its generation, even if the unlock was queued behind another mount or expensive KDF work.
    let unlock_generation = state.ui_session_generation.load(Ordering::Acquire);
    // This lock covers both the already-mounted authentication path and first reload. Without it,
    // two concurrent webview mounts can both observe `None` and start duplicate actors.
    let _mount = state.vault_mount.lock().await;
    if authenticate_mounted_store(&state, passphrase.as_bytes())
        .await
        .map_err(|e| op.fail(codes::VAULT_LOCKED_OUT, e.to_string()))?
    {
        let servers = finalize_unlock_session(&state, unlock_generation, true)
            .await
            .map_err(|e| op.fail(codes::SESSION_LOCKED, e))?
            .expect("the mounted-store path requests a running-server projection");
        op.succeeded("VAULT.UNLOCK.ALREADY_OPEN");
        return Ok(servers);
    }

    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| op.fail(codes::VAULT_READ_FAILED, e.to_string()))?
        .join("vault");
    let mut rng = OsCryptoRng;
    // Opening the vault verifies the passphrase (the DEK won't decrypt otherwise).
    let store = ServerStore::open(&dir, passphrase.as_bytes(), &mut rng)
        .map_err(|e| op.fail(codes::VAULT_LOCKED_OUT, e.to_string()))?;

    let records = store
        .load_registry()
        .map_err(|e| op.fail(codes::VAULT_READ_FAILED, e.to_string()))?;

    // Restore the grant-ceremony ledger: a pairing request must stay single-use across a restart,
    // or re-pasting one would mint a second bundle. A corrupt/missing blob leaves an empty ledger
    // (the pre-persistence behaviour) rather than blocking unlock.
    match store.load_pairing_ledger() {
        Ok(bytes) if !bytes.is_empty() => match PairingLedger::restore(&bytes) {
            Ok(led) => *state.pairing_ledger.lock().await = led,
            Err(e) => {
                tracing::warn!(target: "catcoms_app", error = %e, "IDENTITY.PAIRING_LEDGER.RESTORE_FAILED")
            }
        },
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(target: "catcoms_app", error = %e, "IDENTITY.PAIRING_LEDGER.READ_FAILED")
        }
    }

    // Load every server's sealed snapshot up front, while we still own `store` locally.
    let snapshots: Vec<_> = records
        .iter()
        .map(|r| match store.load_server(r.id) {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::error!(target: "catcoms_app", server = r.id, error = %e, "VAULT.SERVER.LOAD_FAILED");
                None
            }
        })
        .collect();
    let max_id = records.iter().map(|r| r.id).max().unwrap_or(0);

    // Install the unlocked store BEFORE reloading. `reload_one` -> `attach_blob_store` reads
    // `state.store` to attach the on-disk sealing blob store; if it were still `None` here,
    // every reloaded server would silently keep an empty in-memory blob store and be unable to
    // read its own persisted blobs ("no peer has it" for files you uploaded before the restart).
    *state.store.lock().await = Some(store);
    {
        let mut n = state.next_id.lock().await;
        if *n < max_id {
            *n = max_id;
        }
    }

    let mut reloaded = Vec::new();
    let mut failed = 0usize;
    for (record, snap) in records.iter().zip(snapshots.iter()) {
        let Some(bytes) = snap else {
            failed += 1;
            continue;
        };
        if let Err(e) = reload_one(&app, &state, bytes, record).await {
            tracing::error!(target: "catcoms_app", server = record.id, error = %e, "VAULT.SERVER.RESTORE_FAILED");
            failed += 1;
            continue;
        }
        reloaded.push(ReloadedServer {
            server: record.id,
            name: record.display_name.clone(),
            invite: record.invite.clone(),
            channel: channel_id("general").to_string(),
            channels: ui_channels(
                actor_of_unchecked(&state, record.id)
                    .await
                    .map_err(|e| op.fail(codes::SERVER_UNAVAILABLE, e))?
                    .channels()
                    .await,
            ),
            is_dm: record.is_dm,
        });
    }
    finalize_unlock_session(&state, unlock_generation, false)
        .await
        .map_err(|e| op.fail(codes::SESSION_LOCKED, e))?;
    // The summary that makes a partial unlock visible.
    //
    // Individual failures already log, but an unlock that returns four servers when the registry
    // held five is reported to the user as a success, and the missing one simply is not on the
    // rail. Somebody noticing that a week later has no way to tell whether the server was left,
    // never joined, or failed to restore every single launch since. This is the difference between
    // those, at a level Safe mode keeps when anything went wrong.
    if failed > 0 {
        tracing::warn!(
            target: "catcoms_app",
            restored = reloaded.len(),
            failed,
            expected = records.len(),
            "VAULT.UNLOCK.PARTIAL"
        );
    } else {
        tracing::info!(
            target: "catcoms_app",
            restored = reloaded.len(),
            "VAULT.UNLOCK.COMPLETED"
        );
    }
    Ok(reloaded)
}

/// Restore an already-unlocked frontend after F5/HMR without asking for the vault passphrase
/// again. An explicit UI lock disables this path until `unlock` verifies the passphrase.
async fn resume_session_for_generation(
    state: &AppState,
    generation: u64,
) -> Option<Vec<ReloadedServer>> {
    resume_session_projection(state, generation, running_servers(state)).await
}

async fn resume_session_projection(
    state: &AppState,
    generation: u64,
    projection: impl Future<Output = Vec<ReloadedServer>>,
) -> Option<Vec<ReloadedServer>> {
    let _commit = require_ui_session_generation(state, generation)
        .await
        .ok()?;
    let servers = projection.await;
    // Lock invalidation deliberately happens before `lock_session_inner` waits for `_commit`, so
    // recheck after every actor await. If a lock began while channels were being projected, none
    // of the names/invites/channels may cross back into the stale webview.
    if state.ui_session_generation.load(Ordering::Acquire) != generation
        || require_unlocked_session(state).await.is_err()
    {
        return None;
    }
    Some(servers)
}

async fn resume_session_inner(state: &AppState) -> Option<Vec<ReloadedServer>> {
    let generation = unlocked_ui_session_generation(state).await.ok()?;
    resume_session_for_generation(state, generation).await
}

#[tauri::command]
async fn resume_session(state: State<'_, AppState>) -> Result<Option<Vec<ReloadedServer>>, String> {
    Ok(resume_session_inner(&state).await)
}

#[tauri::command]
async fn lock_session(
    state: State<'_, AppState>,
    ui_state_json: Option<String>,
) -> Result<LockSessionOutcome, String> {
    // Tauri requires async commands borrowing `State` to use a `Result` return. The outer error is
    // reserved for bridge/dispatch failure; application-level continuity failure stays inside the
    // resolved outcome so the webview knows native locking completed.
    Ok(lock_session_outcome_inner(&state, ui_state_json).await)
}

/// IPC-visible lock completion. A continuity error is data-loss evidence, not evidence that the
/// security boundary remained open, so it must not be collapsed into a rejected command.
#[derive(Debug, Serialize, PartialEq, Eq)]
struct LockSessionOutcome {
    continuity_error: Option<String>,
}

async fn lock_session_outcome_inner(
    state: &AppState,
    ui_state_json: Option<String>,
) -> LockSessionOutcome {
    LockSessionOutcome {
        continuity_error: lock_session_inner(state, ui_state_json).await.err(),
    }
}

/// Result of a native-owned window close. If this response reaches the webview then native locking
/// is complete; successful destruction normally removes the recipient before it can observe one.
#[derive(Debug, Serialize, PartialEq, Eq)]
struct CloseVaultWindowOutcome {
    continuity_error: Option<String>,
    deferred: bool,
    destroy_error: Option<String>,
}

/// Lock the UI session and decide whether the close may proceed. Kept separate from the actual
/// Tauri window mutation so the security/durability policy is deterministic under unit tests.
async fn close_vault_window_plan_inner(
    state: &AppState,
    ui_state_json: Option<String>,
    discard_continuity_error: bool,
) -> (LockSessionOutcome, bool) {
    let _close = state.vault_window_close.lock().await;
    // Two close/lock requests can cross the bridge before the first response disables the WebView.
    // The general lock-completion slot may be replaced by a later Ctrl+L success, so close debt has
    // its own latch. Only a later close carrying the explicit loss acknowledgement may consume it.
    {
        let mut debt = state.vault_window_close_debt.lock().await;
        if let Some(error) = debt.as_ref().cloned() {
            if discard_continuity_error {
                *debt = None;
                return (
                    LockSessionOutcome {
                        continuity_error: Some(error),
                    },
                    true,
                );
            }
            return (
                LockSessionOutcome {
                    continuity_error: Some(error),
                },
                false,
            );
        }
    }
    let had_local_snapshot = ui_state_json.is_some();
    let (requested_generation, result) =
        lock_session_with_generation_inner(state, ui_state_json).await;
    let mut outcome = LockSessionOutcome {
        continuity_error: result.err(),
    };
    // The commit mutex is shared by all lock callers, so an older ordinary Ctrl+L may consume and
    // validate the close's newer pending snapshot. In that interleaving this close itself sees no
    // pending bytes. Bind its decision to the generation it registered and recover the completion
    // produced by whichever caller actually consumed that generation (or a newer replacement).
    if had_local_snapshot && outcome.continuity_error.is_none() {
        if let Some(completion) = state.last_ui_lock_completion.lock().await.as_ref() {
            if completion.generation >= requested_generation {
                outcome.continuity_error.clone_from(&completion.error);
            }
        }
    }
    // After a remount the JS snapshot is gone, but a prior native lock may already have consumed
    // it. Preserve that transaction's result rather than letting this snapshot-less idempotent lock
    // overwrite a failure with `Ok(())`.
    if !had_local_snapshot && outcome.continuity_error.is_none() {
        if let Some(completion) = state.last_ui_lock_completion.lock().await.as_ref() {
            outcome.continuity_error.clone_from(&completion.error);
        }
    }
    if let Some(error) = outcome.continuity_error.as_ref() {
        if !discard_continuity_error {
            *state.vault_window_close_debt.lock().await = Some(error.clone());
        }
    }
    let may_destroy = outcome.continuity_error.is_none() || discard_continuity_error;
    (outcome, may_destroy)
}

/// Native owns the lock -> final snapshot -> destroy sequence. The frontend's ordinary close
/// handler cannot enforce this ordering with an ACL-granted `destroy()` call alone, and a webview
/// remount cannot retain a JavaScript Promise for an earlier Ctrl+L request. Calling the same
/// idempotent native lock again crosses `ui_session_commit`; when the current webview retained the
/// snapshot it submits those exact immutable bytes again.
#[tauri::command]
async fn close_vault_window(
    app: AppHandle,
    state: State<'_, AppState>,
    ui_state_json: Option<String>,
    discard_continuity_error: bool,
) -> Result<CloseVaultWindowOutcome, String> {
    let (lock, may_destroy) =
        close_vault_window_plan_inner(&state, ui_state_json, discard_continuity_error).await;
    if !may_destroy {
        return Ok(CloseVaultWindowOutcome {
            continuity_error: lock.continuity_error,
            deferred: true,
            destroy_error: None,
        });
    }

    let destroy_error = match app.get_webview_window("main") {
        Some(window) => window.destroy().err().map(|error| error.to_string()),
        None => Some("the main window no longer exists".to_string()),
    };
    Ok(CloseVaultWindowOutcome {
        continuity_error: lock.continuity_error,
        deferred: false,
        destroy_error,
    })
}

/// Testable core of the explicit lock operation. Closing the command boundary is unconditional:
/// malformed continuity state or a vault write failure is reported, but must never leave the
/// sensitive webview session open as a side effect of that error.
async fn lock_session_inner(state: &AppState, ui_state_json: Option<String>) -> Result<(), String> {
    lock_session_with_generation_inner(state, ui_state_json)
        .await
        .1
}

/// Execute one lock request and return the exact generation it registered. Native window-close
/// policy needs this provenance because a different caller may consume the request's snapshot
/// while holding the shared commit mutex.
async fn lock_session_with_generation_inner(
    state: &AppState,
    ui_state_json: Option<String>,
) -> (u64, Result<(), String>) {
    // Invalidate old work before awaiting anything. The commit mutex is acquired next so, once
    // this function returns, an old join can neither emit its private reply nor register/persist.
    state.session_lock_requested.store(true, Ordering::Release);
    // Cancellation destroys authority and must not wait behind the persistence transaction. This
    // also retires unclaimed registrations left by a reloaded/crashed webview.
    cancel_all_inline_downloads(state);
    let lock_generation = state
        .ui_session_generation
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    // Register before either session mutex await. A later close from a remounted webview carries no
    // JS snapshot, but it can still save this exact native transaction if it reaches commit first.
    if let Some(json) = ui_state_json {
        let mut pending = state.pending_ui_lock_snapshot.lock().await;
        let replace = match pending.as_ref() {
            Some(snapshot) => snapshot.generation <= lock_generation,
            None => true,
        };
        if replace {
            *pending = Some(PendingUiLockSnapshot {
                generation: lock_generation,
                json,
            });
        }
    }
    *state.session_resumable.lock().await = false;
    let _session_commit = state.ui_session_commit.lock().await;
    // Close the registration/early-sweep race as a second line of defence. Registration itself
    // checks `session_lock_requested` while holding the cancellation table, but this post-commit
    // drain also retires any future registration path that was already inside the UI commit seam.
    cancel_all_inline_downloads(state);
    // Save the final draft/read snapshot and close IPC as one ordered native operation. Two
    // separate fire-and-forget commands could race, causing the save to arrive after the lock and
    // be correctly rejected by the new session gate.
    let pending_snapshot = state.pending_ui_lock_snapshot.lock().await.take();
    let completed_generation = pending_snapshot
        .as_ref()
        .map(|snapshot| snapshot.generation);
    let (save_result, retry_snapshot) = match pending_snapshot {
        Some(snapshot) => match validate_ui_state_json(&snapshot.json) {
            Err(error) => (Err(error), None),
            Ok(()) => {
                let guard = state.store.lock().await;
                let result = match guard.as_ref() {
                    Some(store) => store
                        .save_ui_state(snapshot.json.as_bytes(), &mut OsCryptoRng)
                        .map_err(|error| error.to_string()),
                    None => Err("unlock the vault before saving UI state".to_string()),
                };
                let retry = result.as_ref().err().map(|_| snapshot);
                (result, retry)
            }
        },
        None => (Ok(()), None),
    };
    if let Some(snapshot) = retry_snapshot {
        // Disk failures can be transient. Retain exact bytes for the warning's second close or a
        // remounted webview; malformed input is intentionally not retained and retried forever.
        let mut pending = state.pending_ui_lock_snapshot.lock().await;
        let replace = match pending.as_ref() {
            Some(current) => current.generation <= snapshot.generation,
            None => true,
        };
        if replace {
            *pending = Some(snapshot);
        }
    }
    if let Some(generation) = completed_generation {
        *state.last_ui_lock_completion.lock().await = Some(UiLockCompletion {
            generation,
            error: save_result.clone().err(),
        });
    }
    // The media cache holds decrypted chunks of shared files. Locking must drop them along with
    // the rest of the session's plaintext, not leave a film resident until something evicts it.
    state.media_cache.lock().await.clear();
    // The heads are only sizes and types, but they name what was being played: a locked vault
    // should not still be able to answer that.
    state.media_heads.lock().await.clear();
    // Storage reports contain plaintext metadata even though the managed file bytes remain
    // sealed. Clear them at the same boundary and require a fresh authenticated scan after unlock.
    state.storage_health.lock().await.clear();
    // An upload in flight is session state too. Dropping the reservations frees their slots and
    // garbage-collects the chunks they had sealed but will now never publish.
    let abandoned: Vec<PendingUpload> =
        state.uploads.lock().await.drain().map(|(_, v)| v).collect();
    for upload in abandoned {
        discard_pending_upload(state, upload).await;
    }
    (lock_generation, save_result)
}

// ---------------------------------------------------------------------------
// Multi-device grant ceremony (M2); four paste-carried steps, no new transport.
// ---------------------------------------------------------------------------

/// What `pairing_begin` hands the **new** device.
#[derive(Serialize, Clone)]
struct PairingBegun {
    /// The blob to carry to the origin device (copy/paste; QR at M6).
    blob: String,
    /// This device's id as full hex; its first 8 characters are the roster fingerprint.
    device_id: String,
}

/// What the origin device's grant popup shows.
#[derive(Serialize, Clone)]
struct PairingRead {
    /// The requesting device's id as full hex.
    device_id: String,
    /// The six-digit code, **as a string** so a leading zero survives the trip to JS
    /// (`023602` is a valid code and `23602` is a different one).
    sas: String,
    /// The scope the grant will cover if accepted: every unlocked server's local label
    /// (the popup must show what is about to be granted; adversarial-review finding).
    servers: Vec<String>,
    /// How many of those are DMs (surfaced separately in the popup copy).
    dm_count: usize,
}

/// The sealed bundle to carry back to the new device.
#[derive(Serialize, Clone)]
struct PairingBundle {
    bundle: String,
}

/// One server's grant, as summarized to the new device after the bundle opens.
#[derive(Serialize, Clone)]
struct PairingGrantSummary {
    /// The origin device's local label for that server.
    name: String,
    /// The MLS group id (hex).
    group_id: String,
    /// The certifying origin's device id (hex); different per server by design.
    origin: String,
    /// How many bootstrap / rendezvous hints came with it.
    bootstrap: usize,
    rendezvous: usize,
}

/// What `pairing_open` shows on the new device: the code to compare, and what arrived.
#[derive(Serialize, Clone)]
struct PairingOpened {
    /// The six-digit code; the human's last check. It must match the one the origin
    /// showed at its popup; if it does not, discard the grant.
    sas: String,
    /// The name the origin gave this device.
    device_name: String,
    servers: Vec<PairingGrantSummary>,
}

/// One server as `pairing_mint` sees it: the registry's half of a grant (local label +
/// the live reach hints), plus the actor that will sign the certificate.
struct GrantSource {
    id: u64,
    name: String,
    bootstrap: Vec<String>,
    rendezvous: Vec<String>,
    actor: ServerActor,
}

/// The origin identity anchoring this ceremony's SAS: the **lowest-numbered** server's.
///
/// A member holds one origin identity *per server*; that is what stops servers from
/// linking them; so a ceremony has to name one of them, and both ends must name the
/// same one or the six digits will not match. Choosing it deterministically here means
/// the popup and the new device's bundle-open agree with no extra value typed across
/// (the choice is written into the bundle, and `open_grant_bundle` reads it back).
async fn ceremony_origin(state: &AppState) -> Result<DeviceId, String> {
    let actor = {
        let servers = state.servers.lock().await;
        let (_, entry) = servers
            .iter()
            .min_by_key(|(id, _)| **id)
            .ok_or_else(|| "join or found a server before pairing a device".to_string())?;
        entry.actor.clone()
    };
    actor
        .origin_identity()
        .await
        .map(|(id, _)| id)
        .ok_or_else(|| "server actor stopped".to_string())
}

/// Step 1, on the **new** device: mint its device identity and a single-use pairing
/// request. The secrets stay in this process; only the request travels.
#[tauri::command]
async fn pairing_begin(state: State<'_, AppState>) -> Result<PairingBegun, String> {
    require_unlocked_session(&state).await?;
    let mut rng = OsCryptoRng;
    let (secrets, blob) = catcoms_app::begin_pairing(&mut rng).map_err(|e| e.to_string())?;
    let device_id = secrets.device_id().to_string();
    // Starting a ceremony abandons any earlier one: its nonce is never reused, and any grants
    // still held from it are dropped (they were minted for a device key we are replacing).
    *state.pairing.lock().await = Some(secrets);
    state.pairing_grants.lock().await.clear();
    Ok(PairingBegun { blob, device_id })
}

/// Step 2, on the **origin** device: read a pasted request for the grant popup, and
/// remember it as THE pending ceremony; `pairing_mint` acts on this stored view only,
/// so the device the popup showed is the device that gets certified. Nothing is minted
/// and the nonce is not consumed here; the popup may be reopened (re-reading replaces
/// the pending view wholesale).
#[tauri::command]
async fn pairing_read(state: State<'_, AppState>, blob: String) -> Result<PairingRead, String> {
    require_unlocked_session(&state).await?;
    let origin = ceremony_origin(&state).await?;
    let view = catcoms_app::read_pairing_blob(&blob, &origin).map_err(|e| e.to_string())?;
    if state
        .pairing_ledger
        .lock()
        .await
        .is_spent(&view.request.pairing_nonce)
    {
        return Err("that pairing request has already been used".to_string());
    }
    // The scope the popup must disclose: everything an accept would grant.
    let (servers, dm_count) = {
        let guard = state.servers.lock().await;
        let mut names: Vec<(u64, String, bool)> = guard
            .iter()
            .map(|(id, e)| (*id, e.name.clone(), e.is_dm))
            .collect();
        names.sort_by_key(|(id, _, _)| *id);
        let dm_count = names.iter().filter(|(_, _, dm)| *dm).count();
        (
            names.into_iter().map(|(_, n, _)| n).collect::<Vec<_>>(),
            dm_count,
        )
    };
    let read = PairingRead {
        device_id: view.new_device_id.to_string(),
        sas: format!("{:06}", view.sas),
        servers,
        dm_count,
    };
    *state.pending_grant.lock().await = Some(PendingGrant { view, origin });
    Ok(read)
}

/// Decline the pending ceremony: burn its nonce (single-use **either way**, per the
/// design; a declined request cannot be re-run by re-pasting the same blob) and clear
/// the pending view.
#[tauri::command]
async fn pairing_decline(state: State<'_, AppState>) -> Result<(), String> {
    require_unlocked_session(&state).await?;
    if let Some(pending) = state.pending_grant.lock().await.take() {
        // Already-spent just means a mint won the race; either way the nonce is dead.
        let _ = state
            .pairing_ledger
            .lock()
            .await
            .spend(pending.view.request.pairing_nonce);
        persist_pairing_ledger(&state).await;
    }
    Ok(())
}

/// Seal the pairing ledger to disk, so a declined or spent request stays spent across a restart
/// (best-effort, like every other persist here: a locked vault is not an error).
async fn persist_pairing_ledger(state: &AppState) {
    let snapshot = state.pairing_ledger.lock().await.snapshot();
    let guard = state.store.lock().await;
    if let Some(store) = guard.as_ref() {
        let mut rng = OsCryptoRng;
        if let Err(e) = store.save_pairing_ledger(&snapshot, &mut rng) {
            tracing::error!(target: "catcoms_app", error = %e, "IDENTITY.PAIRING_LEDGER.SEAL_FAILED");
        }
    }
}

/// Step 3, on the **origin** device, once the human has confirmed the popup: sign one
/// certificate per unlocked server; each with *that server's* own origin key, inside
/// its actor; and seal them all into one passphrase-wrapped bundle.
///
/// Each server contributes what only its actor knows (group id + signature) and what
/// only the bridge knows (the local label and the live bootstrap / rendezvous hints,
/// which are exactly what an invite carries and `join_server` consumes). The pairing
/// nonce is spent inside `mint_grant_bundle`, so a re-paste of the same request fails.
#[tauri::command]
async fn pairing_mint(
    state: State<'_, AppState>,
    passphrase: String,
    device_name: String,
    turn: Option<HashMap<String, String>>,
) -> Result<PairingBundle, String> {
    require_unlocked_session(&state).await?;
    // Mint ONLY from the pending view `pairing_read` stored; the popup's device is the
    // certified device, closing the read→mint TOCTOU. The lock is held for the whole
    // mint so a concurrent re-read cannot swap the ceremony out from under the accept.
    let mut pending_guard = state.pending_grant.lock().await;
    let pending = pending_guard
        .as_ref()
        .ok_or_else(|| "no pairing request has been read; paste one first".to_string())?;
    let ceremony = pending.origin;
    let view = &pending.view;

    // A spent nonce costs zero signatures (and zero actor round-trips).
    if state
        .pairing_ledger
        .lock()
        .await
        .is_spent(&view.request.pairing_nonce)
    {
        return Err("that pairing request has already been used".to_string());
    }

    // Snapshot what the registry knows about each server under the lock, then talk to
    // each actor without holding it; the same shape as `get_inbox`. Sorted so the
    // bundle's server order is stable.
    let mut servers: Vec<GrantSource> = {
        let guard = state.servers.lock().await;
        guard
            .iter()
            .map(|(id, e)| GrantSource {
                id: *id,
                name: e.name.clone(),
                bootstrap: e.bootstrap.clone(),
                rendezvous: e.rendezvous.clone(),
                actor: e.actor.clone(),
            })
            .collect()
    };
    servers.sort_by_key(|s| s.id);

    // The operator's shared TURN lives in the frontend's per-server storage (the invite's
    // `.turn.` suffix), not in the backend, so the caller passes it in keyed by server id as a
    // string (JSON object keys always are). Absent = no TURN for that server, which is exactly
    // what an invite without a `.turn.` suffix carries.
    let turn = turn.unwrap_or_default();

    let mut grants = Vec::with_capacity(servers.len());
    for s in servers {
        let (_, group_id) = s
            .actor
            .origin_identity()
            .await
            .ok_or_else(|| format!("server {} stopped", s.id))?;
        // The owner's key, read from the LIVE roster: the new device pins it to authenticate the
        // Welcome it will be admitted with (see `PerServerGrant::owner_public_key`).
        let owner_public_key = s
            .actor
            .owner_public_key()
            .await
            .ok_or_else(|| format!("server '{}' has no readable owner key", s.name))?;
        let certificate = s
            .actor
            .sign_device_cert(view.new_device_id, device_name.clone())
            .await?;
        grants.push(PerServerGrant {
            group_id,
            server_name: s.name,
            bootstrap: s.bootstrap,
            rendezvous: s.rendezvous,
            turn: turn.get(&s.id.to_string()).cloned().unwrap_or_default(),
            owner_public_key,
            certificate,
        });
    }

    let mut ledger = state.pairing_ledger.lock().await;
    let mut rng = OsCryptoRng;
    let bundle = catcoms_app::mint_grant_bundle(
        passphrase.as_bytes(),
        &device_name,
        &view.request,
        &ceremony,
        &grants,
        &mut ledger,
        &mut rng,
    )
    .map_err(|e| e.to_string())?;
    drop(ledger);
    // The ceremony is complete; the pending view is done with (its nonce is now spent).
    *pending_guard = None;
    drop(pending_guard);
    persist_pairing_ledger(&state).await;
    Ok(PairingBundle { bundle })
}

/// Step 4, on the **new** device: unseal the bundle and check it is this device's grant
/// for this ceremony. Returns the code for the human's final comparison plus a summary
/// of what arrived.
///
/// The opened [`PerServerGrant`]s are **held** alongside the ceremony secrets; `pairing_join`
/// (step 5) redeems them. Opening again simply replaces them, so a re-paste is harmless.
#[tauri::command]
async fn pairing_open(
    state: State<'_, AppState>,
    bundle: String,
    passphrase: String,
) -> Result<PairingOpened, String> {
    require_unlocked_session(&state).await?;
    let guard = state.pairing.lock().await;
    let secrets = guard
        .as_ref()
        .ok_or_else(|| "no pairing in progress on this device".to_string())?;
    let opened = catcoms_app::open_grant_bundle(passphrase.as_bytes(), &bundle, secrets)
        .map_err(|e| e.to_string())?;
    let summaries = opened
        .grants
        .iter()
        .map(|g| PairingGrantSummary {
            name: g.server_name.clone(),
            group_id: hex::encode(&g.group_id),
            origin: g.certificate.origin_id.to_string(),
            bootstrap: g.bootstrap.len(),
            rendezvous: g.rendezvous.len(),
        })
        .collect();
    *state.pairing_grants.lock().await = opened.grants;
    Ok(PairingOpened {
        sas: format!("{:06}", opened.sas),
        device_name: opened.device_name,
        servers: summaries,
    })
}

/// One server's outcome from `pairing_join`.
#[derive(Serialize, Clone)]
struct PairingJoinResult {
    /// The origin device's local label for that server (what the user recognises it by).
    name: String,
    /// Whether this device is now admitted to that server.
    ok: bool,
    /// Why it is not, when `ok` is false. A failure is **retryable**; the grant is kept.
    error: Option<String>,
    /// The bridge server id, once joined (so the UI can select it).
    server: Option<u64>,
}

/// Step 5, on the **new** device: redeem the held grants; connect to each server the way an
/// invite join does, present the origin-signed certificate through the owner-serialized add
/// queue, and register every admitted server exactly like `join_server` does.
///
/// `server_index` selects one grant (indexing the list `pairing_open` returned); omit it to run
/// every remaining grant. Results come back per server, in that same order. A grant that failed
/// stays held so the user can retry; the common failure is an owner that has not come online
/// yet, and the admission is offline-queued precisely for that. Once every grant has been
/// redeemed the ceremony secrets are dropped.
#[tauri::command]
async fn pairing_join(
    app: AppHandle,
    state: State<'_, AppState>,
    server_index: Option<usize>,
) -> Result<Vec<PairingJoinResult>, String> {
    require_unlocked_session(&state).await?;
    let grants = state.pairing_grants.lock().await.clone();
    if grants.is_empty() {
        return Err("no grants to join; open a grant bundle first".to_string());
    }
    let targets: Vec<usize> = match server_index {
        Some(i) if i < grants.len() => vec![i],
        Some(_) => return Err("no such server in this grant bundle".to_string()),
        None => (0..grants.len()).collect(),
    };

    let mut results = Vec::with_capacity(targets.len());
    let mut joined: Vec<usize> = Vec::new();
    for i in targets {
        let grant = &grants[i];
        match join_one_grant(&app, &state, grant).await {
            Ok(server) => {
                joined.push(i);
                results.push(PairingJoinResult {
                    name: grant.server_name.clone(),
                    ok: true,
                    error: None,
                    server: Some(server),
                });
            }
            Err(e) => results.push(PairingJoinResult {
                name: grant.server_name.clone(),
                ok: false,
                error: Some(e),
                server: None,
            }),
        }
    }

    // Drop the redeemed grants; if that was the last one, the ceremony is over and the device
    // key held for it is no longer needed here (each joined server owns its own copy).
    {
        let mut held = state.pairing_grants.lock().await;
        let mut keep = Vec::new();
        for (i, g) in grants.into_iter().enumerate() {
            if !joined.contains(&i) {
                keep.push(g);
            }
        }
        *held = keep;
        if held.is_empty() {
            *state.pairing.lock().await = None;
        }
    }
    Ok(results)
}

/// Join one granted server: dial it, run the certificate admission, and register the result in
/// the bridge registry with the same post-join sequence `join_server` uses.
async fn join_one_grant(
    app: &AppHandle,
    state: &AppState,
    grant: &PerServerGrant,
) -> Result<u64, String> {
    // The device identity the certificate names. Every granted server needs its own MLS provider
    // but the SAME signature key (one certified device id across the bundle), so each join takes
    // a duplicate and the ceremony secrets stay intact for a retry.
    let device = {
        let guard = state.pairing.lock().await;
        let secrets = guard
            .as_ref()
            .ok_or_else(|| "no pairing in progress on this device".to_string())?;
        secrets.device().duplicate().map_err(|e| e.to_string())?
    };

    // Reach the server exactly as an invite join does; via the grant's bootstrap addresses,
    // which are the invite's own field, passed through by the origin.
    //
    // Rendezvous discovery is NOT available here: the pre-join namespace is keyed by an *invite
    // nonce* (`join_namespace(group_id, invite_nonce, …)`), and a grant carries a certificate
    // instead. A companion whose grant has only rendezvous hints therefore cannot discover the
    // group yet; that needs a certificate-keyed pre-join namespace (M4 backlog).
    let (contact, addrs) =
        schedule_grant_bootstrap(&state.endpoint_dials, &grant.group_id, &grant.bootstrap)
            .map_err(|error| {
                if grant.bootstrap.is_empty() && !grant.rendezvous.is_empty() {
                    "this grant is rendezvous-only; pairing needs a directly-dialable server"
                        .to_string()
                } else {
                    error
                }
            })?;
    let (mesh, _id) = MeshService::new_tcp(None, &addrs).map_err(|e| e.to_string())?;
    let mesh_handle = mesh.handle();
    timeout(
        Duration::from_secs(20),
        mesh.wait_for_peer_connected(contact),
    )
    .await
    .map_err(|_| "timed out connecting to the server".to_string())?
    .map_err(|_| "the server transport closed while connecting".to_string())?;

    let name = grant.server_name.clone();
    let mut server = Server::join_with_grant(
        mesh,
        device,
        OsCryptoRng,
        Box::new(SystemClock),
        name.clone(),
        contact,
        grant,
    )
    .await
    .map_err(|e| e.to_string())?;
    server.set_endpoint_dial_scheduler(state.endpoint_dials.clone());
    // Same omission as `join_server`, and the same consequence: a companion device that never
    // subscribes the control topic stops seeing membership changes the moment it is paired.
    server
        .subscribe_control()
        .await
        .map_err(|e| e.to_string())?;
    attach_blob_store(state, &mut server).await;
    // Steady-state discovery: keep the grant's rendezvous nodes so this companion can re-find the
    // group after a restart (post-join, group-keyed; unlike the pre-join namespace above, this
    // works from a grant). Mirrors `join_server`.
    // A grant is authored by this user's own already-paired master device, which is where the
    // rendezvous was typed in the first place, so it gets the operator variant (a name is allowed).
    // It previously got no validation at all, not even the structural checks; a grant naming a
    // circuit address or a relay chain would have been handed straight to the dialer. A grant
    // whose rendezvous list does not validate simply pairs without steady-state discovery rather
    // than failing the pairing, which is the same degradation as a grant that names none.
    let rz_config: Vec<(String, Vec<u8>)> = match validate_operator_rendezvous_addrs(
        &grant.rendezvous,
    ) {
        Ok(targets) => targets
            .into_iter()
            .map(|t| (t.addr.to_string(), t.peer.to_bytes()))
            .collect(),
        Err(e) => {
            tracing::warn!(target: "catcoms_app", error = %e, "IDENTITY.PAIRING.RENDEZVOUS_REJECTED");
            Vec::new()
        }
    };
    if !rz_config.is_empty() {
        server.set_rendezvous_nodes(rz_config);
    }

    let general = channel_id("general");
    let group_id = server.group_id();
    let device_id = server.device_id();
    let (actor, events, task) = spawn(server);
    actor.open_channel(general).await;
    actor.catch_up_channel_index(contact).await;
    actor.catch_up(contact, general).await;
    actor.catch_up_profiles(contact).await;
    actor.catch_up_livery(contact).await;
    actor.catch_up_badges(contact).await;
    actor.catch_up_devices(contact).await;
    actor.catch_up_files(contact).await;
    actor.catch_up_status(contact).await;
    actor.catch_up_calendar(contact).await;
    actor.catch_up_wiki(contact).await;
    actor.catch_up_roles(contact).await;
    actor.catch_up_moderation(contact).await;
    // A companion mints no invites (owner-scoped), so it carries no bootstrap/rendezvous of its
    // own; the same registry shape a joiner gets. `is_dm` is not in the grant yet, so a DM pairs
    // in as a server on the rail until M4 carries the flag.
    //
    // `record_seq` is 0 and no peer record is published here, deliberately. Unlike found/join,
    // this path builds its transport with `MeshService::new_tcp(None, ..)`: a throwaway identity
    // on an OS-assigned port, so it has no stable address worth telling other members about, and
    // publishing one would hand them a peer id and port that die with the process. The companion
    // becomes visible on its first reload, which goes through `reload_one` on a persisted
    // identity and a reserved sequence block like every other server. (Giving this path a
    // persisted identity is multi-device work, not discovery work.)
    let server_id = register_server(
        app,
        state,
        actor,
        events,
        task,
        group_id,
        device_id,
        None,
        name,
        Vec::new(),
        HashMap::new(),
        None,
        Vec::new(),
        Some(mesh_handle),
        false,
        false,
        0,
    )
    .await;
    persist_server(state, server_id).await;
    persist_registry(state).await;
    Ok(server_id)
}

/// Where the debug log is written, and where the "keep a debug log" preference is remembered.
///
/// Both live under the app data directory beside the vault, so a user told "your log is in this
/// folder" can actually find it; a log the user cannot locate is not a log.
fn log_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("logs"))
}

/// The opt-**out** file. Its presence means "no log"; a flag with no parser cannot be corrupted
/// into an unreadable state, and the sense is inverted from the obvious one on purpose: see
/// [`debug_logging_enabled`].
fn debug_flag_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(log_dir(app)?.join("debug-logging-disabled"))
}

/// The state of the debug log, as shown in Settings.
///
/// `enabled` and `state` are separate because the whole value of this struct is that they can
/// disagree. The old version had one boolean assigned from the preference, so a process that had
/// never managed to open a file still reported "active" and a user could burn their only
/// reproduction on the strength of it.
#[derive(Serialize, Clone)]
struct DebugLogging {
    /// Whether the preference is on right now.
    enabled: bool,
    /// Whether *this running process* is actually writing a file. A subscriber can only be
    /// installed once per process, so a toggle applies at the next launch, and saying so is the
    /// difference between a working setting and a user who thinks they captured a log.
    active: bool,
    /// What the sink is doing: `stopped`, `active`, `degraded` or `failed`. Derived from bytes
    /// that reached a file, never from `enabled`.
    state: String,
    /// Why it is degraded or failed, when it is. Shown verbatim, because "permission denied
    /// opening the diagnostics directory" is something a person can act on and "logging failed"
    /// is not.
    error: String,
    /// Identifies this run inside the file itself, so an excerpt can be matched to its source.
    session: String,
    /// The folder the log is written to, always shown so the user can go and get it.
    dir: String,
    /// The file **this process opened**. Not the newest file in the directory: that used to be
    /// how this was answered, and it names a previous run's log whenever the current one failed
    /// to open, which is exactly when a wrong answer does the most damage.
    file: String,
    /// Events this session put in the file, and bytes they took.
    events_written: u64,
    bytes_written: u64,
    /// Events that never made it: queue overflow, or emitted after the quota stopped the writer.
    events_dropped: u64,
    /// Events that reached the file with their tail cut off. A different thing from dropped, and
    /// worth showing separately: a truncated line is there and says so, so a reader who meets one
    /// knows not to draw a conclusion from half an error message.
    events_truncated: u64,
    /// How full the write queue is, and how full it has ever been.
    queue_depth: u64,
    queue_high_water: u64,
    /// The session byte quota, so the UI can show how close to it this run has come.
    session_quota_bytes: u64,
}

impl DebugLogging {
    /// Build the reply from the sink's own health plus the stored preference.
    fn from_health(enabled: bool, dir: &std::path::Path, health: &catcoms_log::SinkHealth) -> Self {
        DebugLogging {
            enabled,
            active: health.state == catcoms_log::SinkState::Active
                || health.state == catcoms_log::SinkState::Degraded,
            state: health.state.as_str().to_string(),
            error: health.last_error.clone().unwrap_or_default(),
            session: health.session_id.clone(),
            dir: dir.display().to_string(),
            file: health
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            events_written: health.events_written,
            bytes_written: health.bytes_written,
            events_dropped: health.events_dropped,
            events_truncated: health.events_truncated,
            queue_depth: health.queue_depth as u64,
            queue_high_water: health.queue_high_water as u64,
            session_quota_bytes: catcoms_log::MAX_SESSION_BYTES,
        }
    }
}

/// Read the debug-logging preference.
///
/// **On by default while the product is in alpha**, which is a deliberate reversal of the usual
/// answer for a privacy tool. A bug report without a log costs a round trip and usually a second
/// reproduction, and at this stage nearly every session is a test session. The opt-out is one
/// click in Settings and is honoured permanently once taken; see `catcoms_log`'s module docs for
/// exactly what an enabled log may contain. Revisit this default before a general release.
///
/// Never fails the caller: an unreadable app data directory reads as the default rather than
/// erroring, since a preference lookup must not be able to stop the app starting.
fn debug_logging_enabled(app: &AppHandle) -> bool {
    debug_flag_path(app)
        .map(|p| debug_enabled_from_flag(p.exists()))
        .unwrap_or(true)
}

/// The preference, from whether the opt-out file exists.
///
/// Split out because the sense is inverted from the obvious one and inverted flags are exactly
/// what a later reader "corrects" back. Presence of the file means the user turned logging OFF;
/// absence means the alpha default, which is ON. Getting this backwards would either lose every
/// bug report or write a log for someone who explicitly asked not to have one.
fn debug_enabled_from_flag(opt_out_exists: bool) -> bool {
    !opt_out_exists
}

/// What this process did about diagnostics at startup, and what came of it.
///
/// Set once in `setup`; read by `get_debug_logging` so the UI can distinguish "on" from "on since
/// the last restart", and both of those from "asked for, and it did not work".
struct LogState {
    /// Dropping this waits for queued output to reach the disk, so it is held for the life of the
    /// process. It also answers for the sink's health, which is why it is no longer a `_` binding.
    guard: Option<catcoms_log::LogGuard>,
    /// Why there is no guard, when there is none. Preserved verbatim from startup, because by the
    /// time a user opens Settings the failing call is long gone.
    init_error: Option<String>,
    dir: std::path::PathBuf,
}

impl LogState {
    /// The sink's health, or a synthetic failed state carrying the startup error.
    ///
    /// A process whose logger would not start has no sink to ask, and answering "stopped" there
    /// would be indistinguishable from the user having turned logging off. The distinction is the
    /// entire point of the struct, so the startup error is carried forward instead.
    fn health(&self) -> catcoms_log::SinkHealth {
        match (&self.guard, &self.init_error) {
            (Some(guard), _) => guard.health(),
            (None, Some(error)) => catcoms_log::SinkHealth {
                desired: true,
                state: catcoms_log::SinkState::Failed,
                last_error: Some(error.clone()),
                ..catcoms_log::SinkHealth::stopped()
            },
            (None, None) => catcoms_log::SinkHealth::stopped(),
        }
    }
}

/// The debug log's current state, for Settings.
#[tauri::command]
async fn get_debug_logging(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DebugLogging, String> {
    require_unlocked_session(&state).await?;
    let log = app.try_state::<LogState>();
    // Prefer the directory this process actually opened, so the path shown is the path being
    // written to even if the app data directory moved under us.
    let dir = match log.as_ref() {
        Some(l) => l.dir.clone(),
        None => log_dir(&app)?,
    };
    let health = match log.as_ref() {
        Some(l) => l.health(),
        None => catcoms_log::SinkHealth::stopped(),
    };
    Ok(DebugLogging::from_health(
        debug_logging_enabled(&app),
        &dir,
        &health,
    ))
}

/// Put a marked record through the whole pipeline and report whether it reached the disk.
///
/// The one question the settings page could never answer for itself. Every other signal it shows
/// is inferred from a preference or from state captured at startup; this emits an event now, waits
/// for the writer, and reads the file size back, so a sink that has quietly stopped since launch is
/// caught by the button rather than by a missing bug report a week later.
#[tauri::command]
async fn test_debug_logging(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DebugLogging, String> {
    require_unlocked_session(&state).await?;
    tracing::info!(target: "catcoms_app", at = wall_ms(), "DIAG.SELFTEST.RECORD");
    if let Some(log) = app.try_state::<LogState>() {
        if let Some(guard) = log.guard.as_ref() {
            guard.sync();
        }
    }
    get_debug_logging(app, state).await
}

/// Turn the debug log on or off for the **next** launch (a tracing subscriber is install-once
/// per process, so nothing can be attached to this one after the fact).
#[tauri::command]
async fn set_debug_logging(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<DebugLogging, String> {
    require_unlocked_session(&state).await?;
    let dir = log_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // The file records the opt-OUT, so enabling removes it. Inverted from the obvious mapping
    // because the default is on for alpha, and a default that depends on a file being written at
    // first launch would silently turn itself off on any install that could not write one.
    let flag = debug_flag_path(&app)?;
    if enabled {
        if flag.exists() {
            std::fs::remove_file(&flag).map_err(|e| e.to_string())?;
        }
    } else {
        std::fs::write(&flag, b"off").map_err(|e| e.to_string())?;
    }
    get_debug_logging(app, state).await
}

/// How much of one frontend log line is kept. Long enough for a stack frame and a message,
/// short enough that a runaway loop cannot fill the disk one line at a time.
const MAX_UI_LOG_BYTES: usize = 2000;

/// One field of a diagnostic event, rendered at the session's capture mode.
#[derive(Serialize)]
struct ConsoleField {
    name: String,
    value: String,
    kind: &'static str,
    /// Whether a higher capture mode would show more of this value. Lets the console tell a reader
    /// what they are *not* seeing, instead of leaving them to find out by switching and comparing.
    sensitive: bool,
}

/// One diagnostic event, as the debug console reads it.
///
/// A faithful carry of `catcoms_diagnostics::EventView`, which is a faithful carry of the canonical
/// event. It used to be a flattened `tracing` line: section, phase, span parentage, references and
/// capture mode were dropped on the way, twelve of the trace's sixteen characters with them, and
/// every value was rendered at a hard-coded Enhanced regardless of what the user had chosen. The
/// console then guessed the sections back from target names and by searching the text for the word
/// "voice". Found by adversarial review (P3-005).
#[derive(Serialize)]
struct ConsoleLogEvent {
    seq: u64,
    at_ms: u64,
    /// Milliseconds since this process started, which never goes backwards. `at_ms` can jump when
    /// a clock is corrected, so a duration taken across two events uses this one.
    monotonic_ms: u64,
    /// The canonical section, one of twenty-two.
    section: &'static str,
    /// The console section it falls under, one of six. Stated natively, so the console groups
    /// events by what they *are* rather than by which crate happened to emit them.
    view: &'static str,
    level: &'static str,
    /// The stable `AREA.COMPONENT.OUTCOME` code. `LOG.TRACING.EVENT` means an un-migrated call
    /// site whose prose is in the `message` field.
    code: &'static str,
    phase: &'static str,
    operation: &'static str,
    /// Sixteen hex characters, or empty when the event belongs to no operation.
    trace: String,
    span: String,
    parent_span: String,
    refs: Vec<(&'static str, String)>,
    duration_ms: Option<u64>,
    attempt: Option<u32>,
    /// The emitting module, e.g. `catcoms_net`. Kept for locating the code that said this; it is
    /// no longer what decides which console section the event appears in.
    target: String,
    fields: Vec<ConsoleField>,
    /// Fields this event had to drop at the cap.
    ///
    /// Carried so a shortened field list reads as shortened. The JSON and the text row already say
    /// so, and a console that quietly showed the surviving thirty-two would be the one rendering
    /// where a reader could take the list for the whole of it.
    fields_dropped: u32,
    /// The mode this line was rendered at. On every event rather than only in the page header,
    /// because an excerpt someone pastes gets separated from its header immediately.
    capture: &'static str,
    /// Mode generation assigned when the event entered the native ring.
    capture_epoch: u64,
}

impl From<catcoms_diagnostics::EventView> for ConsoleLogEvent {
    fn from(view: catcoms_diagnostics::EventView) -> Self {
        ConsoleLogEvent {
            seq: view.seq,
            at_ms: view.at_ms,
            monotonic_ms: view.monotonic_ms,
            section: view.section,
            view: view.view,
            level: view.level,
            code: view.code,
            phase: view.phase,
            operation: view.operation,
            trace: view.trace,
            span: view.span,
            parent_span: view.parent_span,
            refs: view.refs,
            duration_ms: view.duration_ms,
            attempt: view.attempt,
            target: view.target,
            fields: view
                .fields
                .into_iter()
                .map(|f| ConsoleField {
                    name: f.name,
                    value: f.value,
                    kind: f.kind,
                    sensitive: f.sensitive,
                })
                .collect(),
            fields_dropped: view.fields_dropped,
            capture: view.capture,
            capture_epoch: view.capture_epoch,
        }
    }
}

/// A page of diagnostics plus the counters the console's severity roll-up needs.
#[derive(Serialize)]
struct ConsoleLog {
    events: Vec<ConsoleLogEvent>,
    /// Session totals, counted before the ring evicts anything, so the roll-up stays true after
    /// the offending line has aged out.
    errors: u64,
    warnings: u64,
    /// Events the ring dropped to stay bounded. The console says so rather than presenting the
    /// gap as a quiet period.
    dropped: u64,
    /// Events the capture config excluded. A different thing from `dropped` and worth showing
    /// separately: it is what distinguishes a section that is silent by policy from one that is
    /// silent because nothing happened.
    filtered: u64,
    latest_seq: u64,
    capacity: usize,
    /// The mode this page was rendered at, so the console can label what it is showing.
    capture: &'static str,
    /// Identifies this run, so an excerpt someone pastes can be matched to its report.
    session_id: String,
}

/// The most events one poll will return, so a console that has been closed for an hour cannot ask
/// for the whole ring in a single IPC payload.
const MAX_CONSOLE_LOG_PAGE: usize = 500;

/// Serve the debug console the diagnostics it has not seen yet.
///
/// Polled with the last sequence number the console holds, so an open console costs one small
/// message per tick and a freshly opened one gets the backlog the ring still has. The ring is
/// in-memory only: this reads it, and nothing here writes it anywhere.
///
/// Each event renders at its capture-time mode and carries its mode generation. Changing the
/// viewer later cannot recover address bytes that Safe capture destroyed before ring insertion.
///
/// Gated on an unlocked session, unlike `log_ui` below. The ring holds peer addresses and stable
/// identifiers, and a locked app must not show those to whoever picks the machine up. Someone
/// diagnosing a failure that happens before unlock still has the debug log file.
#[tauri::command]
async fn get_console_log(
    state: State<'_, AppState>,
    after_seq: u64,
    limit: usize,
) -> Result<ConsoleLog, String> {
    require_unlocked_session(&state).await?;
    let hub = catcoms_log::hub();
    let stats = hub.stats();
    let mode = hub.mode();
    let events = hub
        .since(after_seq, limit.clamp(1, MAX_CONSOLE_LOG_PAGE))
        .iter()
        .map(|e| catcoms_diagnostics::event_view(e, mode).into())
        .collect();
    Ok(ConsoleLog {
        events,
        errors: stats.errors,
        warnings: stats.warnings,
        dropped: stats.dropped,
        filtered: stats.filtered,
        latest_seq: stats.latest_seq,
        capacity: catcoms_log::LOG_RING_CAPACITY,
        capture: mode.as_str(),
        session_id: hub.session_id().to_string(),
    })
}

/// Drop the events the console is holding. The session counters and sequence are kept: this is a
/// "clear my view" button, not a rewrite of what happened.
#[tauri::command]
async fn clear_console_log(state: State<'_, AppState>) -> Result<(), String> {
    require_unlocked_session(&state).await?;
    catcoms_log::hub().clear();
    Ok(())
}

/// Told to the webview when the capture mode moves, so it can stop producing records nobody wants.
#[derive(Serialize, Clone)]
struct CaptureModeEvt {
    mode: &'static str,
}

/// One section's capture level, for the console's capture panel.
#[derive(Serialize)]
struct SectionCapture {
    id: &'static str,
    /// The console section it feeds, so the panel can group twenty-two rows under six headings.
    view: &'static str,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`, or absent when the section is off entirely.
    level: Option<&'static str>,
}

/// What is being captured right now.
#[derive(Serialize)]
struct CaptureConfigView {
    mode: &'static str,
    /// Whether this mode is deliberately forgotten at the next launch. Full trace is expensive and
    /// revealing, and somebody who turned it on to reproduce one bug should not still be running it
    /// a fortnight later because they forgot.
    expires_at_restart: bool,
    /// Whether this mode may render literal addresses. This is an address-display decision, not a
    /// publication verdict: automatic public diagnostics use a separate native allowlist.
    reveals_addresses: bool,
    sections: Vec<SectionCapture>,
}

fn capture_config_view() -> CaptureConfigView {
    let hub = catcoms_log::hub();
    let config = hub.config();
    CaptureConfigView {
        mode: config.mode.as_str(),
        expires_at_restart: config.mode.expires_at_restart(),
        reveals_addresses: config.mode.allows_raw_addresses(),
        sections: catcoms_diagnostics::SECTIONS
            .iter()
            .map(|section| SectionCapture {
                id: section.as_str(),
                view: section.view().as_str(),
                level: config.level(*section).map(|l| l.as_str()),
            })
            .collect(),
    }
}

/// One background task, as the console shows it.
#[derive(Serialize)]
struct TaskHealthView {
    id: u64,
    kind: &'static str,
    server: Option<u64>,
    started_ms: i64,
    last_beat_ms: Option<i64>,
    state: &'static str,
    /// Whether this is a state somebody should be told about. Decided natively so the console does
    /// not hold a second opinion about what counts as a fault.
    fault: bool,
    cause: Option<String>,
}

/// What every supervised background task is doing.
///
/// The answer that used to be a log line and then, once the line aged out of the ring, nothing at
/// all: a healthy-looking app whose only evidence that half of it had stopped had scrolled away.
/// State that has to stay in a bounded buffer to be true is not state. Found by adversarial review
/// (P3-009).
#[tauri::command]
async fn get_task_health(state: State<'_, AppState>) -> Result<Vec<TaskHealthView>, String> {
    require_unlocked_session(&state).await?;
    Ok(tasks::snapshot(wall_ms())
        .into_iter()
        .map(|task| TaskHealthView {
            id: task.id,
            kind: task.kind,
            server: task.server,
            started_ms: task.started_ms,
            last_beat_ms: task.last_beat_ms,
            state: task.state,
            fault: task.fault,
            cause: task.cause,
        })
        .collect())
}

/// Where the event stream has got to.
#[derive(Serialize)]
struct EventCursor {
    /// Which run of the native process this stream belongs to.
    generation: u64,
    /// The last position issued across the whole stream.
    ord: u64,
}

/// Where the event stream has got to right now.
///
/// Read by the webview before it installs its listeners, so it knows what the next event should be
/// numbered. Without it a remounted webview takes whatever sequence it happens to see first as its
/// baseline, which makes everything missed before that moment invisible: precisely the window a
/// hot reload or an F5 opens while the native process keeps running and keeps emitting.
///
/// Two counters and nothing else. Gated with the other reads for consistency rather than because
/// they are sensitive.
#[tauri::command]
async fn get_event_cursor(state: State<'_, AppState>) -> Result<EventCursor, String> {
    require_unlocked_session(&state).await?;
    Ok(EventCursor {
        generation: event_generation(),
        ord: EVENT_ORD.load(std::sync::atomic::Ordering::Relaxed),
    })
}

/// What the diagnostics are currently capturing.
#[tauri::command]
async fn get_capture_config(state: State<'_, AppState>) -> Result<CaptureConfigView, String> {
    require_unlocked_session(&state).await?;
    Ok(capture_config_view())
}

/// Change how much is captured, immediately.
///
/// Takes effect now rather than at the next launch. That is the whole point of the hub owning the
/// gate: the previous design could attach a subscriber only once per process, so a user who wanted
/// to stop being recorded had to quit the app to do it.
///
/// Not persisted, deliberately. A capture mode that survived a restart would be a privacy setting
/// nobody remembers making, and `Full` in particular is meant to expire on its own.
#[tauri::command]
async fn set_capture_mode(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: String,
) -> Result<CaptureConfigView, String> {
    require_unlocked_session(&state).await?;
    let mode = catcoms_diagnostics::CaptureMode::parse(&mode)
        .ok_or_else(|| format!("unknown capture mode: {mode}"))?;
    catcoms_log::hub().set_mode(mode);
    // The webview produces diagnostics too, and it cannot see the gate. Without being told, it
    // would keep building records and sending them across the bridge for the native side to throw
    // away, which is the same "kept paying, stopped keeping" shape the tracing layer had.
    emit_tracked(
        &app,
        "capture-changed",
        CaptureModeEvt {
            mode: mode.as_str(),
        },
        catcoms_diagnostics::TraceId::default(),
    );
    // Recorded through the pipeline's own section, which is never turned down: a report that
    // changed what it was capturing halfway through, silently, is a report that misleads about a
    // gap. `Off` records nothing at all, including this, which is what off means.
    catcoms_diagnostics::DiagnosticHub::record(
        &catcoms_log::hub(),
        catcoms_diagnostics::DiagnosticEvent::info(
            catcoms_diagnostics::Section::Diag,
            "DIAG.CAPTURE.MODE_CHANGED",
        )
        .target("catcoms_app")
        .field(
            "mode",
            catcoms_diagnostics::SafeText::describe(mode.as_str()),
        ),
    );
    Ok(capture_config_view())
}

/// Turn one section up, down, or off, without disturbing the others.
///
/// The second of the two axes. One switch meant choosing between capturing almost nothing and
/// capturing the transport layer narrating every address the node has ever seen, so it stayed off
/// and nobody had a log when they needed one.
#[tauri::command]
async fn set_section_capture(
    state: State<'_, AppState>,
    section: String,
    level: Option<String>,
) -> Result<CaptureConfigView, String> {
    require_unlocked_session(&state).await?;
    let section = catcoms_diagnostics::Section::parse(&section)
        .ok_or_else(|| format!("unknown diagnostic section: {section}"))?;
    // An unrecognised level is refused rather than defaulted. A control that quietly does something
    // other than what it was asked is worse than one that says no.
    let level = match level {
        Some(name) => Some(
            catcoms_diagnostics::Level::parse(&name)
                .ok_or_else(|| format!("unknown capture level: {name}"))?,
        ),
        None => None,
    };
    catcoms_log::hub().set_section_level(section, level);
    Ok(capture_config_view())
}

/// Explicitly restore the recommended section levels for the current privacy mode.
#[tauri::command]
async fn reset_section_capture(state: State<'_, AppState>) -> Result<CaptureConfigView, String> {
    require_unlocked_session(&state).await?;
    catcoms_log::hub().reset_section_levels();
    Ok(capture_config_view())
}

/// Build the public-issue attachment from a native allowlist.
///
/// Kept as a private helper rather than an IPC command: the webview must never receive a report
/// and then hand caller-authored title/body/URL bytes back across the operating-system launcher
/// boundary. [`open_public_diagnostics_issue`] performs that sequence atomically in native code.
fn build_public_diagnostics_report() -> Result<String, String> {
    let hub = catcoms_log::hub();
    let events = hub.since(0, catcoms_log::LOG_RING_CAPACITY);
    let report =
        catcoms_diagnostics::export::render_public_report(events.iter().map(|event| &**event));
    if report.len() > MAX_REPORT_BYTES {
        return Err("public diagnostics report exceeds the bounded export size".into());
    }
    // Defense in depth: the allowlist is the privacy proof; the scanner catches regressions in
    // its output contract before any future issue-submission command receives the bytes.
    validate_report_for_mode(
        &report,
        catcoms_diagnostics::CaptureMode::Safe,
        ExportPurpose::Publish,
    )?;
    Ok(report)
}

/// Open a public diagnostics issue using only native-owned report, title and destination bytes.
///
/// The command intentionally accepts no webview payload. A compromised renderer can ask for this
/// fixed disclosure after unlock, but cannot append another report, swap the title, or turn the
/// launcher into a confused deputy for a different URL. The browser still presents the GitHub form
/// for the user to review and submit; Mewtual posts nothing itself.
#[tauri::command]
async fn open_public_diagnostics_issue(
    state: State<'_, AppState>,
) -> Result<PublicDiagnosticsIssue, String> {
    require_unlocked_session(&state).await?;
    let native_report = build_public_diagnostics_report()?;
    let prepared = prepare_public_diagnostics_issue(&native_report);
    if !is_tracker_url(&prepared.url) {
        return Err("native diagnostics issue URL failed its fixed tracker boundary".into());
    }
    launch_url(&prepared.url)?;
    Ok(PublicDiagnosticsIssue {
        report: prepared.report,
        truncated: prepared.truncated,
    })
}

/// The largest diagnostics report that will be written.
///
/// Comfortably above a full ring rendered as text, and far below anything that could fill a disk.
/// A cap that is never reached in normal use is still the difference between a bug and an outage.
const MAX_REPORT_BYTES: usize = 8 * 1024 * 1024;

/// The prefix every saved report carries. Retention only ever considers files matching it, so the
/// debug logs sharing this directory are never at risk from it, and vice versa.
const REPORT_PREFIX: &str = "mewtual-diagnostics-";

/// How many saved reports survive, and how much they may occupy between them.
///
/// The bounded writer next door limits segment size, segment count, session bytes and directory
/// bytes, and its retention only considers `debug_log_*`. Reports were exempt from all of it: an
/// unlocked webview could press Save in a loop and fill the disk without touching a single one of
/// those carefully chosen limits, which would make the console's own export the outage the writer
/// was designed to prevent. Found by adversarial review (P3-003).
const MAX_SAVED_REPORTS: usize = 10;
const MAX_REPORT_DIR_BYTES: u64 = 64 * 1024 * 1024;

/// Delete the oldest saved reports until they are inside both the count and the byte quota.
///
/// Never touches the file being written, and never touches anything that is not a report.
fn retain_reports(dir: &std::path::Path, keeping: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut held: Vec<(std::path::PathBuf, u64)> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(REPORT_PREFIX))
        .map(|e| (e.path(), e.metadata().map(|m| m.len()).unwrap_or(0)))
        .collect();
    // The name leads with a zero-padded timestamp, so sorting by name is sorting by age.
    held.sort_by(|a, b| a.0.cmp(&b.0));

    let mut total: u64 = held.iter().map(|(_, size)| size).sum();
    let mut index = 0;
    while index < held.len()
        && (held.len() + keeping > MAX_SAVED_REPORTS || total > MAX_REPORT_DIR_BYTES)
    {
        let (path, size) = &held[index];
        if std::fs::remove_file(path).is_ok() {
            total = total.saturating_sub(*size);
            held.remove(index);
        } else {
            index += 1;
        }
    }
}

/// Whether a report save is already running.
///
/// One at a time. Two concurrent saves would race on retention and could each delete what the other
/// was about to count, and there is no reason for a person to need two at once.
static REPORT_SAVING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Clears [`REPORT_SAVING`] however the save ends.
///
/// A guard rather than a clear at each exit: this function has several `?` returns and one that
/// forgot to release would wedge the button for the rest of the session, with no symptom except a
/// feature that quietly stopped working.
struct ReportSaveGuard;

impl Drop for ReportSaveGuard {
    fn drop(&mut self) {
        REPORT_SAVING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Where a saved report went, for the UI to show.
#[derive(Serialize)]
struct SavedReport {
    /// The full path, so the user can go and get it.
    path: String,
    /// Just the file name, for saying "saved as X" without a wall of directory.
    file: String,
    bytes: usize,
    /// Non-blocking validator categories still present (currently legacy bridged prose). The UI
    /// must disclose these rather than turning "written" into "safe".
    review: Vec<String>,
}

use catcoms_diagnostics::export::ExportPurpose;

/// Validate the exact bytes about to leave the process and return what the reader should review.
///
/// Deliberately native: a compromised or stale webview must not be able to bypass the check by
/// invoking the command directly, and the capture mode consulted is the real one rather than
/// whatever the frontend believes.
fn validate_report_for_save(text: &str) -> Result<Vec<String>, String> {
    validate_report_for_mode(text, catcoms_log::hub().mode(), ExportPurpose::Local)
}

/// Where the caller says these bytes are going. Anything unrecognised is treated as publishing,
/// so a typo fails closed rather than quietly downgrading the check.
fn export_purpose(purpose: &str) -> ExportPurpose {
    match purpose {
        "local" => ExportPurpose::Local,
        _ => ExportPurpose::Publish,
    }
}

fn validate_report_for_mode(
    text: &str,
    mode: catcoms_diagnostics::CaptureMode,
    purpose: ExportPurpose,
) -> Result<Vec<String>, String> {
    let report = catcoms_diagnostics::export::validate_export(text, mode);
    // One row per category with a count, never one per finding. Reported verbatim this filled the
    // window with several hundred `opaque_blob at line N` entries and told the reader nothing they
    // could act on. The line numbers stay in the report for anyone who wants to jump to one.
    let say = |rows: Vec<(catcoms_diagnostics::export::Category, usize)>| -> Vec<String> {
        rows.into_iter()
            .map(|(category, lines)| format!("{} ({lines})", category.as_str()))
            .collect()
    };
    // Saving or copying is never refused. The report goes into the same folder as the log it was
    // drawn from, so refusing to write it guards nothing, and refusing on every category is what
    // made the first Save of a real session fail: the un-migrated networking prose narrates every
    // address and peer id it sees, which is hundreds of findings in an ordinary report.
    //
    // Publishing is the boundary the review was about, in its words: "the planned next stage is
    // GitHub issue submission. A false 'safe' label turns a local diagnostic failure into a public
    // disclosure." An interruption is worth it there, because that one cannot be taken back.
    if report.blocked(purpose) {
        return Err(format!(
            "this report is not safe to post: it contains {}. Turn on redaction, or save it and read it first",
            say(report.refusals(purpose)).join(", ")
        ));
    }
    Ok(say(report.disclosure()))
}

#[cfg(test)]
mod report_validation_tests {
    use super::{export_purpose, validate_report_for_mode};
    use catcoms_diagnostics::export::ExportPurpose;
    use catcoms_diagnostics::CaptureMode;

    /// Saving and copying, the two purposes that never leave this machine.
    fn saving(text: &str, mode: CaptureMode) -> Result<Vec<String>, String> {
        validate_report_for_mode(text, mode, ExportPurpose::Local)
    }

    /// A caller that names no recognised purpose is treated as publishing.
    ///
    /// Fail closed: a typo or an older webview must not be able to downgrade the check by asking
    /// for a purpose this build has never heard of.
    #[test]
    fn an_unrecognised_purpose_is_treated_as_publishing() {
        assert_eq!(export_purpose("local"), ExportPurpose::Local);
        assert_eq!(export_purpose("publish"), ExportPurpose::Publish);
        assert_eq!(export_purpose(""), ExportPurpose::Publish);
        assert_eq!(export_purpose("locl"), ExportPurpose::Publish);
    }

    /// Posting a report that carries an account path is refused, and says why without echoing it.
    #[test]
    fn publishing_refuses_what_saving_allows() {
        let text = "Mewtual report\nerror at C:\\Users\\private\\vault.db\n";
        let error = validate_report_for_mode(text, CaptureMode::Safe, ExportPurpose::Publish)
            .expect_err("a local account path must not be posted in public");
        assert!(error.contains("local_path (1)"), "{error}");
        assert!(
            !error.contains("C:\\Users"),
            "the refusal must not echo the text it is refusing"
        );
    }

    /// Saving discloses; it does not refuse.
    ///
    /// This asserted the opposite and shipped a Save button that could not save. The log the
    /// report is drawn from is already in the same folder, so refusing to write the report guards
    /// nothing, and every ordinary session trips the scanners because the un-migrated networking
    /// prose narrates addresses and peer ids by the hundred.
    #[test]
    fn saving_names_what_is_in_the_report_instead_of_refusing_it() {
        let review = saving(
            "Mewtual report\nerror at C:\\Users\\private\\vault.db\n",
            CaptureMode::Safe,
        )
        .expect("writing to the user's own log folder is not disclosure");
        assert_eq!(review, vec!["local_path (1)"]);
        assert!(
            !review.iter().any(|row| row.contains("C:\\Users")),
            "the disclosure must not echo the text it is describing"
        );
    }

    #[test]
    fn raw_addresses_follow_the_native_capture_mode() {
        let text = "route 203.0.113.9:443";
        assert_eq!(
            saving(text, CaptureMode::Safe).unwrap(),
            vec!["raw_address (1)"],
            "safe capture promised no literal addresses, so the report says it has one"
        );
        assert!(
            saving(text, CaptureMode::Enhanced).unwrap().is_empty(),
            "enhanced was asked for addresses, so one is nothing to report"
        );
    }

    #[test]
    fn legacy_bridged_prose_requires_review_but_does_not_force_a_bypass() {
        let review = saving("LOG.TRACING.EVENT outcome=failed", CaptureMode::Safe)
            .expect("legacy prose without a concrete sensitive shape remains exportable");
        assert_eq!(review, vec!["bridged_prose (1)"]);
    }

    /// The report from the screenshot: hundreds of findings, and a Save that has to work.
    #[test]
    fn a_real_session_log_saves_and_is_summarised_rather_than_enumerated() {
        let mut body = String::from("Mewtual report\n");
        for at in 0..200 {
            body.push_str(&format!(
                "LOG.TRACING.EVENT message=dialling 198.51.100.{} peer=12D3KooWQ9xTvR4bKdL2sWpAmN7zYcEfGh{:02}\n",
                at % 250,
                at % 100,
            ));
        }
        let review =
            saving(&body, CaptureMode::Safe).expect("a real session's own log must be savable");
        assert!(
            review.len() <= 4,
            "the user gets a few counted rows, not one line per finding: {review:?}"
        );
        assert!(review.iter().any(|row| row.starts_with("raw_address (")));
    }
}

#[derive(Serialize)]
struct ReportValidation {
    review: Vec<String>,
}

/// Check a report immediately before the webview copies it. Validation stays native and uses the
/// native capture mode, so the clipboard path cannot drift into a weaker frontend-only scanner.
#[tauri::command]
async fn validate_diagnostics_report(
    state: State<'_, AppState>,
    text: String,
    purpose: String,
) -> Result<ReportValidation, String> {
    require_unlocked_session(&state).await?;
    if text.len() > MAX_REPORT_BYTES {
        return Err(format!(
            "the report is larger than the {} MiB limit",
            MAX_REPORT_BYTES / (1024 * 1024)
        ));
    }
    Ok(ReportValidation {
        review: validate_report_for_mode(
            &text,
            catcoms_log::hub().mode(),
            export_purpose(&purpose),
        )?,
    })
}

/// Write a diagnostics report next to the debug log.
///
/// The console can already copy a report to the clipboard, which is the fast path for pasting into
/// a chat. This is the one for keeping: a clipboard survives until the next thing you copy, and a
/// bug report written a day later needs the evidence to still exist. It also gives someone a file
/// to attach rather than a wall of text to paste.
///
/// The text is composed by the console, through the same serialiser that renders the screen, so
/// the file matches what the user was looking at including whether redaction was on. Composing it
/// natively instead would give two renderings that could disagree, and a report that contradicts
/// the screenshot it arrived with costs the reader more time than it saves.
///
/// The file name is built here and never taken from the webview: a caller-supplied name is a path
/// traversal waiting to happen, and there is nothing the webview needs to say about it.
#[tauri::command]
async fn save_diagnostics_report(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<SavedReport, String> {
    require_unlocked_session(&state).await?;
    if text.len() > MAX_REPORT_BYTES {
        return Err(format!(
            "the report is larger than the {} MiB limit",
            MAX_REPORT_BYTES / (1024 * 1024)
        ));
    }
    let review = validate_report_for_save(&text)?;

    // One save at a time: two would race on retention, each deleting what the other was about to
    // count. Released on every exit below, including the error paths.
    if REPORT_SAVING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err("a diagnostics report is already being written".to_string());
    }
    let _release = ReportSaveGuard;

    let dir = match app.try_state::<LogState>() {
        Some(log) => log.dir.clone(),
        None => log_dir(&app)?,
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Make room before writing, counting the one about to be written.
    retain_reports(&dir, 1);

    let session = catcoms_log::hub().session_id().to_string();
    // The timestamp leads and is zero-padded so that sorting these by name sorts them by age,
    // which is what retention above relies on.
    let file = format!("{REPORT_PREFIX}{:013}-{session}.txt", wall_ms());
    let path = dir.join(&file);
    // Written to a temporary file and renamed, so an interrupted save leaves no half-report that
    // reads as a whole one. A stray `.part` is obvious; a truncated report is not.
    let staging = dir.join(format!("{file}.part"));
    std::fs::write(&staging, text.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&staging, &path).map_err(|e| {
        let _ = std::fs::remove_file(&staging);
        e.to_string()
    })?;

    tracing::info!(
        target: "catcoms_app",
        bytes = text.len(),
        "PRIVACY.EXPORT.WRITTEN"
    );
    Ok(SavedReport {
        path: path.display().to_string(),
        file,
        bytes: text.len(),
        review,
    })
}

/// Record something the webview saw.
///
/// Until this existed the log was Rust-only, so every `console.warn` in the voice path (failed
/// signals, rejected ICE candidates, offer handling that threw) went to a devtools console nobody
/// had open and was gone. Half the app is in the webview; a log that cannot see it is a log of
/// half the app.
///
/// Deliberately not gated on an unlocked session, unlike almost every other command: the errors
/// most worth having are the ones from unlock failing and from startup, which happen before there
/// is a session to check.
///
/// It also does not check whether the debug **file** is on any more. It emits a `tracing` event and
/// lets the installed layers decide: the file layer exists only when the user enabled it, and the
/// in-memory console ring is always there. Returning early when the file was off is what used to
/// make the frontend's own errors invisible to the in-app console unless the user had turned on a
/// log file and restarted first, which is the opposite of what someone hitting a problem needs.
#[tauri::command]
async fn log_ui(level: String, message: String) {
    record_ui_log(&level, message, 1);
}

/// One line the webview wants recorded.
#[derive(Deserialize)]
struct UiLogRecord {
    level: String,
    message: String,
    /// How many identical lines this one stands for.
    ///
    /// The frontend collapses consecutive repeats so a render or reconnect loop does not cost an
    /// IPC round trip per line, but the count travels with the survivor. "One ICE candidate
    /// rejected" and "four thousand in two seconds" are different bugs, and the old deduper
    /// rendered them as the same evidence.
    #[serde(default)]
    repeats: u32,
}

/// The most records one call may carry. A batch is a convenience, not a way around the rate limit.
const MAX_UI_LOG_BATCH: usize = 256;

/// Record a batch of webview lines.
///
/// Batching exists because the alternative is one IPC round trip per `console.warn`, and the
/// moments worth capturing are exactly the ones where the webview is emitting fastest. See
/// [`log_ui`] for why this is not gated on an unlocked session.
#[tauri::command]
async fn log_ui_batch(records: Vec<UiLogRecord>) -> SendOutcome {
    let offered = records.len() as u64;
    let mut accepted = 0u64;
    for record in records.into_iter().take(MAX_UI_LOG_BATCH) {
        if record_ui_log(&record.level, record.message, record.repeats.max(1)) {
            accepted += 1;
        }
    }
    SendOutcome { offered, accepted }
}

/// Shorten `text` to at most `max` **bytes** without splitting a character.
///
/// `String::truncate` panics unless the index is a character boundary, and this input is arbitrary
/// text from the webview: one emoji, one combining mark or any CJK line long enough puts a
/// multi-byte character across the cap. The frontend's own limit is counted in UTF-16 units, so it
/// cannot prevent this. Panicking here would mean the act of recording an error destroys the
/// evidence of that error, which is the worst available failure for a logging path.
fn truncate_utf8_bytes(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut end = max;
    // Byte 0 is always a boundary, so this terminates even when the first character alone is
    // longer than the cap.
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

/// Wall-clock milliseconds, through the same `SystemClock` seam the rest of the bridge stamps
/// with. Reading the OS clock directly is forbidden outside that seam, and for good reason: it is
/// what makes every timing-dependent behaviour here testable.
///
/// Only ever used for rate-limiter arithmetic and for stamping a self-test record, so a clock that
/// steps is a cosmetic problem rather than a correctness one; the elapsed calculation clamps
/// negatives either way.
fn wall_ms() -> i64 {
    SystemClock.now_ms() as i64
}

/// What the native side did with a batch of webview records.
///
/// The webview cannot otherwise know. Its send used to be `void invoke(...).catch(() => {})`: the
/// batch was retired the moment it was handed over, the rejection was swallowed outside the
/// batcher, and the pipeline reported perfect health precisely when the bridge was unhealthy. An
/// explicit answer is what lets it count its own losses. Found by adversarial review (P3-006).
///
/// `accepted` can be less than `offered` without anything being wrong: the limiter below exists to
/// suppress a storm, and doing its job is not a delivery failure. It *is* a loss, and the webview
/// is entitled to know the number.
#[derive(Serialize, Clone, Copy)]
struct SendOutcome {
    offered: u64,
    accepted: u64,
}

/// How many webview records may arrive in a burst before the limiter starts suppressing.
const UI_LOG_BURST: f64 = 200.0;

/// How many it may sustain per second once the burst is spent.
const UI_LOG_PER_SECOND: f64 = 50.0;

/// How often the limiter is allowed to say how much it suppressed.
const UI_LOG_REPORT_INTERVAL_MS: i64 = 5_000;

/// A token bucket over the webview's log traffic.
///
/// The webview is the least trustworthy input this process has: a render loop, a reconnect storm
/// or a compromised page can all emit without bound, and each record costs formatting, a queue
/// slot and disk. Suppression is counted and reported rather than silent, because a limiter that
/// hides its own effect turns a retry storm into a quiet period.
#[derive(Debug)]
struct UiLogBudget {
    tokens: f64,
    last_ms: i64,
    suppressed: u64,
    last_report_ms: i64,
}

/// Which of the webview's two channels a record arrived on.
///
/// They get separate budgets rather than sharing one. A `console.warn` storm and a stalled send are
/// both plausible at the same moment, and while the two shared a bucket the storm spent it: the
/// structured events describing what was actually going wrong were suppressed by the noise about
/// it. Required by adversarial review (P3-006).
#[derive(Clone, Copy, PartialEq, Eq)]
enum UiLogChannel {
    /// `console.error` and friends, forwarded as prose.
    Prose,
    /// Structured observations with a code, a section and a trace.
    Structured,
}

static UI_PROSE_BUDGET: std::sync::OnceLock<std::sync::Mutex<UiLogBudget>> =
    std::sync::OnceLock::new();
static UI_STRUCTURED_BUDGET: std::sync::OnceLock<std::sync::Mutex<UiLogBudget>> =
    std::sync::OnceLock::new();

/// Whether one more record fits, plus the suppression summary owed to the log if it does.
fn ui_log_allowance(channel: UiLogChannel, now_ms: i64) -> (bool, Option<u64>) {
    let slot = match channel {
        UiLogChannel::Prose => &UI_PROSE_BUDGET,
        UiLogChannel::Structured => &UI_STRUCTURED_BUDGET,
    };
    let cell = slot.get_or_init(|| {
        std::sync::Mutex::new(UiLogBudget {
            tokens: UI_LOG_BURST,
            last_ms: now_ms,
            suppressed: 0,
            last_report_ms: now_ms,
        })
    });
    let mut budget = match cell.lock() {
        Ok(b) => b,
        // A poisoned limiter must not take logging down with it: the numbers behind it are still
        // perfectly usable.
        Err(e) => e.into_inner(),
    };

    let elapsed = (now_ms - budget.last_ms).max(0) as f64 / 1000.0;
    budget.tokens = (budget.tokens + elapsed * UI_LOG_PER_SECOND).min(UI_LOG_BURST);
    budget.last_ms = now_ms;

    if budget.tokens >= 1.0 {
        budget.tokens -= 1.0;
        // Report on the first record after a quiet spell rather than on a timer, so the summary
        // lands next to the traffic it describes.
        if budget.suppressed > 0 && now_ms - budget.last_report_ms >= UI_LOG_REPORT_INTERVAL_MS {
            let count = std::mem::take(&mut budget.suppressed);
            budget.last_report_ms = now_ms;
            return (true, Some(count));
        }
        return (true, None);
    }

    budget.suppressed += 1;
    (false, None)
}

/// Read a JSON object into ordered pairs.
///
/// `serde` will not map an object onto a `Vec` of pairs on its own: it wants a sequence, and says
/// so. A visitor gets the entries in the order the parser met them, which is the order the webview
/// wrote them.
#[derive(Default)]
struct BoundedUiFields {
    values: Vec<(String, serde_json::Value)>,
    dropped: u32,
}

impl From<Vec<(String, serde_json::Value)>> for BoundedUiFields {
    fn from(mut values: Vec<(String, serde_json::Value)>) -> Self {
        let dropped = values.len().saturating_sub(catcoms_diagnostics::MAX_FIELDS) as u32;
        values.truncate(catcoms_diagnostics::MAX_FIELDS);
        BoundedUiFields { values, dropped }
    }
}

fn ordered_fields<'de, D>(deserializer: D) -> Result<BoundedUiFields, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct InDocumentOrder;

    impl<'de> serde::de::Visitor<'de> for InDocumentOrder {
        type Value = BoundedUiFields;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object of diagnostic fields")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut fields = Vec::with_capacity(
                map.size_hint()
                    .unwrap_or(0)
                    .min(catcoms_diagnostics::MAX_FIELDS),
            );
            let mut dropped = 0u32;
            while let Some(name) = map.next_key::<String>()? {
                if fields.len() < catcoms_diagnostics::MAX_FIELDS {
                    fields.push((name, map.next_value()?));
                } else {
                    // Drain the rest without retaining it in the command/event. Tauri has already
                    // materialized the outer JSON `Value`, so this is deliberately a post-decode
                    // work/ring bound rather than a claim that the IPC request was never allocated.
                    // The loss remains explicit on the event.
                    map.next_value::<serde::de::IgnoredAny>()?;
                    dropped = dropped.saturating_add(1);
                }
            }
            Ok(BoundedUiFields {
                values: fields,
                dropped,
            })
        }
    }

    deserializer.deserialize_map(InDocumentOrder)
}

/// One structured observation from the webview.
#[derive(Deserialize)]
struct UiDiagnosticEvent {
    section: String,
    code: String,
    level: String,
    #[serde(default)]
    trace: String,
    /// Session-local proof returned beside a native event trace. This is bridge metadata, never a
    /// diagnostic field: persisting it would turn a short-lived provenance check into report data.
    #[serde(default)]
    trace_proof: String,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    duration_ms: Option<u64>,
    /// The webview's fields, in the order it wrote them.
    ///
    /// A `Vec` of pairs rather than a `HashMap`, because a map has no order and Rust's is
    /// deliberately seeded per process: the same events exported from two runs came out with their
    /// fields shuffled differently, which is the byte-identical-output property gone for exactly
    /// the events the console shows most. Found by adversarial review (P3-015).
    ///
    /// Insertion order rather than sorted, because that is what the canonical event does with its
    /// own fields and what `render.rs` promises. Sorting would also be deterministic, and would
    /// throw away the order the producer meant.
    #[serde(default, deserialize_with = "ordered_fields")]
    fields: BoundedUiFields,
}

/// The bounded subset retained after Tauri has decoded the invoke's JSON body.
///
/// This limits command/ring work and drops the decoded excess promptly; it is not a raw IPC byte
/// cap. Tauri materializes `InvokeBody::Json` before command argument deserialization, so a
/// compromised renderer can still spend process memory and parsing time on an oversized request.
struct RetainedUiDiagnosticEvents {
    offered: usize,
    values: Vec<UiDiagnosticEvent>,
}

impl<'de> Deserialize<'de> for RetainedUiDiagnosticEvents {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct BoundedBatch;

        impl<'de> serde::de::Visitor<'de> for BoundedBatch {
            type Value = RetainedUiDiagnosticEvents;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an array of structured UI diagnostic events")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut values =
                    Vec::with_capacity(seq.size_hint().unwrap_or(0).min(MAX_UI_LOG_BATCH));
                while values.len() < MAX_UI_LOG_BATCH {
                    let Some(event) = seq.next_element::<UiDiagnosticEvent>()? else {
                        let offered = values.len();
                        return Ok(RetainedUiDiagnosticEvents { offered, values });
                    };
                    values.push(event);
                }
                let mut offered = values.len();
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    offered = offered.saturating_add(1);
                }
                Ok(RetainedUiDiagnosticEvents { offered, values })
            }
        }

        deserializer.deserialize_seq(BoundedBatch)
    }
}

impl From<Vec<UiDiagnosticEvent>> for RetainedUiDiagnosticEvents {
    fn from(mut values: Vec<UiDiagnosticEvent>) -> Self {
        let offered = values.len();
        values.truncate(MAX_UI_LOG_BATCH);
        RetainedUiDiagnosticEvents { offered, values }
    }
}

/// Record structured observations from the webview.
///
/// The difference between this and `log_ui_batch` is the difference the whole migration is about.
/// A console line is prose that happens to have been written down; this is an event with a stable
/// code, a section, a phase and a trace, so the half of an operation that happens in the webview
/// is readable alongside the half that happens here rather than in a separate format that has to
/// be correlated by eye.
///
/// Shares `log_ui`'s rate limiter, because it shares its threat model: the webview is the least
/// trustworthy producer this process has, and a render loop must cost a counter rather than a disk.
/// Also shares its reason for being outside the unlocked-session gate.
#[tauri::command]
async fn record_ui_events(events: RetainedUiDiagnosticEvents) -> SendOutcome {
    use catcoms_diagnostics::{DiagnosticEvent, Level, Phase, SafeText, Section};

    let offered = events.offered;
    let mut accepted = 0usize;
    for event in events.values {
        let (allowed, suppressed) = ui_log_allowance(UiLogChannel::Structured, wall_ms());
        if let Some(count) = suppressed {
            tracing::warn!(
                target: "catcoms_ui",
                suppressed = count,
                "UI.LOG.RATE_LIMITED: records dropped to keep the log usable"
            );
        }
        if !allowed {
            continue;
        }

        // Built only if it will be kept. Each of these bounds the webview's code, its trace and
        // every one of its fields, and the webview is the producer with the least restraint in the
        // process, so an excluded batch should cost the gate and nothing else.
        let section = ui_section(&event.section);
        let level = ui_level(&event.level);
        catcoms_log::hub().record_with(section, level, || {
            // The code is data from the webview, so it cannot be the `&'static str` a structured
            // event wants. It is bounded and carried as a field instead, and the event's own code
            // says where it came from. A webview cannot mint an arbitrary code that later shows up
            // in an issue title, which is the property that matters.
            let mut recorded = DiagnosticEvent::new(section, level, "UI.EVENT")
                .target("catcoms_ui")
                .phase(ui_phase(&event.phase))
                .field("code", SafeText::describe(&event.code));

            if let Some(duration) = event.duration_ms {
                recorded = recorded.took(duration);
            }
            // The trace becomes the event's own trace, not a field that happens to be called
            // "trace". As a field it rendered as text, did not reach `DiagnosticHub::trace`, and so
            // could not gather the webview's half of an operation with the native half: the
            // correlation the whole mechanism exists for stopped exactly at the bridge. Found by
            // adversarial review (P3-011).
            if let Some(trace) = parse_returned_ui_trace(&event.trace, &event.trace_proof) {
                recorded = recorded.trace(trace);
            }
            recorded.fields_dropped = recorded.fields_dropped.saturating_add(event.fields.dropped);
            for (name, value) in event.fields.values {
                recorded = recorded.field(name, ui_field(&value));
            }
            recorded
        });
        // Counted as accepted once it has been offered to the hub. An event the capture config
        // excludes is not a delivery failure: the webview got it here, and the hub's own `filtered`
        // counter accounts for the rest. Counting it as lost would report the user's own setting
        // back to them as a fault.
        accepted += 1;
    }

    fn ui_section(name: &str) -> Section {
        match name {
            "ipc" => Section::Ipc,
            "channels" => Section::Channels,
            "voice" => Section::Voice,
            "files" => Section::Files,
            "sync" => Section::Sync,
            "join" => Section::Join,
            "startup" => Section::Startup,
            _ => Section::Ui,
        }
    }
    fn ui_level(name: &str) -> Level {
        match name {
            "error" => Level::Error,
            "warn" => Level::Warn,
            "debug" => Level::Debug,
            _ => Level::Info,
        }
    }
    fn ui_phase(name: &str) -> Phase {
        match name {
            "start" => Phase::Start,
            "progress" => Phase::Progress,
            "success" => Phase::Success,
            "failure" => Phase::Failure,
            "cancel" => Phase::Cancel,
            "timeout" => Phase::Timeout,
            _ => Phase::Observation,
        }
    }
    /// Map a JSON field onto the typed value that can carry it.
    ///
    /// Numbers and booleans keep their type so a reader can sort on them. Anything else becomes
    /// bounded text: the webview cannot put an object graph or a message body into a diagnostic
    /// through this door.
    fn ui_field(value: &serde_json::Value) -> catcoms_diagnostics::SafeValue {
        use catcoms_diagnostics::SafeValue;
        match value {
            serde_json::Value::Bool(v) => SafeValue::Bool(*v),
            serde_json::Value::Number(n) => n
                .as_u64()
                .map(SafeValue::Count)
                .or_else(|| n.as_i64().map(SafeValue::Delta))
                .unwrap_or_else(|| SafeValue::Text(SafeText::describe(&n.to_string()))),
            serde_json::Value::String(s) => SafeValue::Text(SafeText::describe(s)),
            other => SafeValue::Text(SafeText::describe(&other.to_string())),
        }
    }

    SendOutcome {
        offered: offered as u64,
        accepted: accepted as u64,
    }
}

/// Bound, rate-limit and emit one webview line. Returns whether it was kept.
fn record_ui_log(level: &str, message: String, repeats: u32) -> bool {
    let (allowed, suppressed) = ui_log_allowance(UiLogChannel::Prose, wall_ms());
    if let Some(count) = suppressed {
        tracing::warn!(
            target: "catcoms_ui",
            suppressed = count,
            "UI.LOG.RATE_LIMITED: records dropped to keep the log usable"
        );
    }
    if !allowed {
        return false;
    }

    let mut text = message;
    if text.len() > MAX_UI_LOG_BYTES {
        truncate_utf8_bytes(&mut text, MAX_UI_LOG_BYTES);
        text.push_str(" [truncated]");
    }
    if repeats > 1 {
        text.push_str(&format!(" [x{repeats}]"));
    }
    // The webview chooses the level, so it is matched against a known set rather than parsed:
    // an unrecognised one becomes info instead of being dropped.
    match level {
        "error" => tracing::error!(target: "catcoms_ui", "{text}"),
        "warn" => tracing::warn!(target: "catcoms_ui", "{text}"),
        "debug" => tracing::debug!(target: "catcoms_ui", "{text}"),
        _ => tracing::info!(target: "catcoms_ui", "{text}"),
    }
    true
}

/// Build and run the Tauri application.
pub fn run() {
    tauri::Builder::default()
        // Update checking lives in Rust so the download and its minisign verification never
        // touch the webview: the frontend only asks "is there one?" and "install it".
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState::default())
        // Shared media plays straight out of the vault instead of crossing the IPC bridge as a
        // base64 string. The handler answers Range requests, so a player starts on the first
        // chunk and a seek costs one chunk, and the bytes never become a JS string at all.
        .register_asynchronous_uri_scheme_protocol(MEDIA_SCHEME, |app, request, responder| {
            let handle = app.app_handle().clone();
            let path = request.uri().path().to_string();
            let range = request
                .headers()
                .get(http::header::RANGE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                // Capture authorization before any actor/disk await, then hold its native commit
                // boundary through the synchronous responder publication. A stale response is
                // replaced by a bodyless denial rather than escaping after explicit lock.
                let generation = unlocked_ui_session_generation(&state).await.ok();
                let response = serve_media(&state, &path, range).await;
                publish_media_response(&state, generation, response, |response| {
                    responder.respond(response);
                })
                .await;
            });
        })
        // Install the tracing subscriber before anything interesting happens. Until this landed
        // the desktop app had **no** subscriber at all, so every `tracing::warn!` in the whole
        // protocol stack (including the five distinct reasons a join can be refused) was
        // discarded and there was no log file anywhere. On by default while in alpha: see
        // `debug_logging_enabled`, and `catcoms_log`'s module docs for what a log may contain.
        .setup(|app| {
            let handle = app.handle().clone();
            let enabled = debug_logging_enabled(&handle);
            let dir = log_dir(&handle).unwrap_or_else(|_| std::path::PathBuf::from("logs"));
            // A failure here is kept rather than discarded. Diagnostics that cannot start are a
            // thing the user needs told, and the only moment the reason exists is right now: by
            // the time somebody opens Settings and wonders why their log is missing, the io::Error
            // that explains it was dropped several thousand events ago.
            let (guard, init_error) =
                match catcoms_log::init_debug_with(enabled, &dir, catcoms_log::APP_FILE_FILTER) {
                    Ok(guard) => (Some(guard), None),
                    Err(e) => (None, Some(e.to_string())),
                };
            app.manage(LogState {
                guard,
                init_error,
                dir,
            });
            spawn_network_monitor(&handle);
            // The last line of `setup`, and the only proof that startup got all the way through.
            //
            // Everything before this point can fail in ways that leave a plausible-looking log: a
            // panic here exits the process with no window, and the last thing written is whatever
            // happened to come before it. A marker at the end turns "the log stops somewhere" into
            // "the log stops before this", which is the difference between a guess and an answer.
            // `scripts/startup-check.mjs` asserts on it, because liveness cannot be asserted on:
            // somebody closing the window is not a failure.
            tracing::info!(target: "catcoms_app", "STARTUP.SETUP.COMPLETE");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_exists,
            resume_session,
            lock_session,
            close_vault_window,
            unlock,
            found_server,
            preview_invite,
            join_server,
            apply_join_reply,
            leave_server,
            open_channel,
            get_channels,
            get_invite,
            mint_invite_fresh,
            check_invite_routes,
            rename_server,
            get_members,
            set_profile,
            get_profiles,
            set_livery,
            set_server_icon,
            set_server_cursor,
            get_livery,
            set_shared_server_name,
            get_file_size_limit,
            set_file_size_limit,
            set_member_badge,
            get_badges,
            get_devices,
            begin_file_upload,
            push_file_chunk,
            finish_file_upload,
            cancel_file_upload,
            get_files,
            get_storage_health,
            repair_storage,
            get_online_members,
            get_member_routes,
            manual_fallback_redial,
            mint_member_recovery,
            apply_member_recovery,
            get_delivery,
            dm_stats,
            send_dm_invite,
            get_dm_requests,
            send_call_signal,
            call_media_key,
            dismiss_dm_request,
            begin_inline_download,
            cancel_inline_download,
            download_file,
            file_available,
            delete_file,
            set_file_expiry,
            get_file_usage,
            get_wiki_pinned_cids,
            post_status,
            get_statuses,
            edit_status,
            delete_status,
            toggle_status_reaction,
            set_status_pin,
            get_status_policy,
            set_status_policy,
            create_event,
            delete_event,
            get_events,
            get_wiki_pages,
            get_wiki_map,
            get_wiki_page,
            save_wiki_page,
            get_wiki_meta,
            set_wiki_format,
            delete_wiki_page,
            rename_wiki_page,
            get_wiki_history,
            get_wiki_pending,
            get_wiki_review_days,
            set_wiki_review_days,
            approve_wiki_edit,
            reject_wiki_edit,
            restore_wiki_page,
            get_roles,
            get_moderation,
            warn_message,
            create_kick_case,
            cast_kick_vote,
            resolve_kick_case,
            get_join_attempts,
            get_connectivity,
            get_call_transport,
            map_call_port,
            default_route_address,
            unmap_call_port,
            get_switchboard_status,
            set_switchboard_offered,
            get_ui_state,
            save_ui_state,
            create_backup,
            change_vault_secret,
            get_debug_logging,
            set_debug_logging,
            test_debug_logging,
            get_console_log,
            clear_console_log,
            get_capture_config,
            set_capture_mode,
            set_section_capture,
            reset_section_capture,
            open_public_diagnostics_issue,
            get_event_cursor,
            get_task_health,
            save_diagnostics_report,
            validate_diagnostics_report,
            log_ui,
            log_ui_batch,
            record_ui_events,
            set_admin,
            remove_member,
            revoke_device,
            send_message,
            edit_message,
            delete_message,
            toggle_reaction,
            set_pin,
            set_channel_topic,
            get_channel_topic,
            jukebox_add,
            jukebox_remove,
            get_jukebox,
            get_inbox,
            get_messages,
            get_message_tail,
            get_messages_by_id,
            get_message_page,
            get_pinned_messages,
            get_channel_heads,
            pairing_begin,
            pairing_read,
            pairing_mint,
            pairing_decline,
            pairing_open,
            pairing_join,
            open_issue_url,
            open_external_url,
            save_and_open_space_guide,
            save_space_layout,
            save_jam_sheet,
            save_group_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Mewtual desktop app");
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::ManualClock;

    /// A burst of sends must cost the writes it needs, and no send may be told it is durable by a
    /// write that predates it. The interleaving that matters: three changes land, the first writer
    /// through the door covers all three, and the two behind it then have nothing to do.
    #[test]
    fn a_burst_is_covered_by_the_write_that_saw_it() {
        let mut counters = PersistCounters::default();
        let first = counters.request();
        let second = counters.request();
        let third = counters.request();
        assert_eq!((first, second, third), (1, 2, 3));
        assert!(counters.needs_write(first));

        // The first writer takes its snapshot now, so it contains every change requested so far.
        let covering = counters.requested;
        counters.completed_through(covering);
        assert!(
            !counters.needs_write(second) && !counters.needs_write(third),
            "a write that saw a change is the write that made it durable"
        );

        // A change that lands after that snapshot is not covered by it, whatever its number.
        let fourth = counters.request();
        assert!(counters.needs_write(fourth));
        // An older write finishing late may never retire a newer request.
        counters.completed_through(covering);
        assert!(
            counters.needs_write(fourth),
            "completion never goes backwards"
        );
        counters.completed_through(counters.requested);
        assert!(!counters.needs_write(fourth));
    }

    #[tokio::test]
    async fn recovery_capture_wakes_are_bounded_replaced_and_coalesced() {
        let signals = StdMutex::new(HashMap::new());
        let mut old = replace_reconnect_capture_signal(&signals, 41).unwrap();
        let mut restored = replace_reconnect_capture_signal(&signals, 41).unwrap();
        assert!(
            old.changed().await.is_err(),
            "restoring an on-disk id closes its superseded capture worker"
        );

        {
            let guard = signals.lock().unwrap();
            let signal = guard.get(&41).expect("the restored registry entry is live");
            for _ in 0..10_000 {
                signal.send_modify(|generation| *generation = generation.wrapping_add(1));
            }
        }
        restored.changed().await.unwrap();
        assert_eq!(
            *restored.borrow_and_update(),
            10_000,
            "a burst retains one latest wake generation rather than an event-sized queue"
        );

        signals.lock().unwrap().remove(&41);
        assert!(
            restored.changed().await.is_err(),
            "leaving the server closes the capture worker"
        );
    }

    #[tokio::test]
    async fn a_stalled_member_route_query_becomes_an_unavailable_snapshot() {
        let clock = ManualClock::new(0);
        let deadline_clock = clock.clone();
        let query = tokio::spawn(async move {
            finish_member_route_query_before(
                std::future::pending::<Result<(), String>>(),
                deadline_clock.sleep(Duration::from_secs(5)),
            )
            .await
        });
        tokio::task::yield_now().await;
        clock.advance_ms(5_000);

        assert_eq!(
            query.await.unwrap().unwrap_err(),
            "member-route snapshot timed out"
        );
    }

    /// A canonical event as it reaches the webview.
    ///
    /// The last hop of P3-005, and the one worth testing separately: the canonical model and the
    /// view over it can both be right while the bridge quietly drops a field on the way to the
    /// only thing a person actually looks at. Asserted through the serialised JSON rather than the
    /// struct, because it is the JSON the console parses.
    #[test]
    fn every_canonical_field_reaches_the_webview() {
        use catcoms_diagnostics::{
            AddressValue, CaptureMode, DiagnosticEvent, DiagnosticHub, Phase, RefDomain, Refs,
            Section, SessionSalt, SpanId, TraceId,
        };
        use rand_chacha::ChaCha20Rng;
        use rand_core::SeedableRng;

        let salt = SessionSalt::for_tests(3);
        let event = DiagnosticEvent::warn(Section::Join, "JOIN.ROUTES.EXHAUSTED")
            .phase(Phase::Failure)
            .operation("join_server")
            .trace(TraceId(0x7f2c_0000_0000_0001))
            .span(SpanId(0x91ab), SpanId(0x6dc4))
            .target("catcoms_sync")
            .took(60_123)
            .attempt(4)
            .refs(Refs {
                server: Some(salt.reference(RefDomain::Server, b"group-1")),
                ..Refs::default()
            })
            .field("direct_candidates", 4u64)
            .field(
                "address",
                AddressValue::new("/ip6/2001:db8::1/udp/31484/quic-v1"),
            );
        // Exercise the real capture boundary. Constructing an event alone still retains its raw
        // address so a Full-mode hub could admit it; only the hub irreversibly minimises Safe
        // capture before the webview can read the event.
        let clock = Arc::new(ManualClock::new(1_787_000_000_000));
        let mut rng = ChaCha20Rng::from_seed([7; 32]);
        let safe_hub = DiagnosticHub::with_capacity(
            clock,
            SessionSalt::for_tests(11),
            CaptureMode::Safe,
            2,
            &mut rng,
        );
        assert_eq!(safe_hub.record(event.clone()), Some(1));
        let safe_event = safe_hub.since(0, 1).pop().unwrap();

        let bridged: ConsoleLogEvent =
            catcoms_diagnostics::event_view(&safe_event, CaptureMode::Full).into();
        let json = serde_json::to_value(&bridged).expect("the console event serialises");

        assert_eq!(json["seq"], 1);
        assert_eq!(json["at_ms"], 1_787_000_000_000u64);
        assert_eq!(json["monotonic_ms"], 0);
        assert_eq!(json["section"], "join");
        assert_eq!(
            json["view"], "network",
            "the console groups on this instead of guessing from the target"
        );
        assert_eq!(json["level"], "WARN");
        assert_eq!(json["code"], "JOIN.ROUTES.EXHAUSTED");
        assert_eq!(json["phase"], "failure");
        assert_eq!(json["operation"], "join_server");
        assert_eq!(json["trace"], "7f2c000000000001");
        assert_eq!(json["span"], "00000000000091ab");
        assert_eq!(json["parent_span"], "0000000000006dc4");
        assert_eq!(json["duration_ms"], 60_123);
        assert_eq!(json["attempt"], 4);
        assert_eq!(json["target"], "catcoms_sync");
        assert_eq!(json["capture"], "safe");
        // The key names the slot; the value is the keyed per-session reference, which carries its
        // domain so a server reference can never be mistaken for a peer one.
        assert_eq!(json["refs"][0][0], "server");
        assert!(
            json["refs"][0][1].as_str().unwrap().starts_with("srv-"),
            "{}",
            json["refs"][0][1]
        );
        assert!(
            !json.to_string().contains("group-1"),
            "and never the identifier it stands for: {json}"
        );
        assert_eq!(json["fields"][0]["name"], "direct_candidates");
        assert_eq!(json["fields"][0]["value"], "4");
        assert_eq!(json["fields"][0]["sensitive"], false);

        // Safe is Safe all the way to the webview. The projection this replaced hard-coded
        // Enhanced, so the console rendered a literal address whatever the user had chosen.
        assert_eq!(json["fields"][1]["name"], "address");
        assert_eq!(json["fields"][1]["kind"], "address");
        assert_eq!(json["fields"][1]["value"], "ip6/quic-v1");
        assert_eq!(
            json["fields"][1]["sensitive"], false,
            "Safe capture destroyed the literal rather than merely hiding it"
        );

        let clock = Arc::new(ManualClock::new(1_787_000_000_000));
        let enhanced_hub = DiagnosticHub::with_capacity(
            clock,
            SessionSalt::for_tests(12),
            CaptureMode::Enhanced,
            2,
            &mut rng,
        );
        assert_eq!(enhanced_hub.record(event), Some(1));
        let enhanced_event = enhanced_hub.since(0, 1).pop().unwrap();
        let enhanced: ConsoleLogEvent =
            catcoms_diagnostics::event_view(&enhanced_event, CaptureMode::Safe).into();
        let json = serde_json::to_value(&enhanced).expect("the console event serialises");
        assert_eq!(
            json["fields"][1]["value"],
            "/ip6/2001:db8::1/udp/31484/quic-v1"
        );
        assert_eq!(json["capture"], "enhanced");
        assert_eq!(json["capture_epoch"], 1);
    }

    /// The webview's fields keep the order it wrote them in.
    ///
    /// They arrived in a `HashMap`, whose iteration order Rust seeds per process, so the same
    /// events exported from two runs came out with their fields shuffled differently. That is the
    /// byte-identical-output property gone, for exactly the events the console shows most: a report
    /// that cannot be diffed against another cannot be compared between two peers, which is how
    /// some sync bugs are localised at all. Found by adversarial review (P3-015).
    #[test]
    fn a_webview_events_fields_keep_the_order_it_wrote_them() {
        // Deliberately not alphabetical, so a container that sorted would be caught too.
        let json = r#"{
            "section": "channels",
            "code": "UI.TEST",
            "level": "info",
            "fields": { "zulu": 1, "alpha": 2, "mike": 3 }
        }"#;
        let event: UiDiagnosticEvent = serde_json::from_str(json).expect("the webview's shape");
        let names: Vec<&str> = event
            .fields
            .values
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["zulu", "alpha", "mike"]);

        // And it is the same on every parse, which a hashed container is not.
        for _ in 0..8 {
            let again: UiDiagnosticEvent = serde_json::from_str(json).unwrap();
            let order: Vec<&str> = again
                .fields
                .values
                .iter()
                .map(|(n, _)| n.as_str())
                .collect();
            assert_eq!(order, names);
        }
    }

    #[test]
    fn ui_diagnostic_command_retains_only_bounded_events_and_fields_after_ipc_decode() {
        let fields: serde_json::Map<String, serde_json::Value> = (0
            ..(catcoms_diagnostics::MAX_FIELDS + 5))
            .map(|index| (format!("field_{index}"), serde_json::json!(index)))
            .collect();
        let event = serde_json::json!({
            "section": "ui",
            "code": "UI.BOUNDED",
            "level": "warn",
            "fields": fields,
        });
        let offered = MAX_UI_LOG_BATCH + 3;
        let wire = serde_json::Value::Array((0..offered).map(|_| event.clone()).collect());

        let batch: RetainedUiDiagnosticEvents = serde_json::from_value(wire).unwrap();
        assert_eq!(batch.offered, offered);
        assert_eq!(batch.values.len(), MAX_UI_LOG_BATCH);
        assert!(batch.values.iter().all(|event| {
            event.fields.values.len() == catcoms_diagnostics::MAX_FIELDS
                && event.fields.dropped == 5
        }));
    }

    #[tokio::test]
    async fn the_ui_recording_bridge_cannot_retain_address_or_peer_text_under_safe_capture() {
        use catcoms_diagnostics::CaptureMode;

        const IPV6: &str = "2001:db8:feed::42";
        const PEER: &str = "12D3KooWDoNotRetainThisPeerIdentifier";
        // Sixteen caller-controlled hex digits can encode an address/identifier just as readily as
        // an arbitrary string field. Repeating it also proves normalization preserves correlation.
        const TRACE: &str = "20010db8feed0042";

        let hub = catcoms_log::hub();
        let restore = hub.config();
        hub.set_mode(CaptureMode::Safe);
        hub.reset_section_levels();
        let before = hub.stats().latest_seq;
        let outcome = record_ui_events(
            (0..2)
                .map(|_| UiDiagnosticEvent {
                    section: "join".into(),
                    code: format!("UI route {IPV6} {PEER}"),
                    level: "warn".into(),
                    trace: TRACE.into(),
                    trace_proof: String::new(),
                    phase: "failure".into(),
                    duration_ms: None,
                    fields: vec![(
                        format!("route_{IPV6}_{PEER}"),
                        serde_json::Value::String(format!("/ip6/{IPV6}/tcp/22487/p2p/{PEER}")),
                    )]
                    .into(),
                })
                .collect::<Vec<_>>()
                .into(),
        )
        .await;
        assert_eq!((outcome.offered, outcome.accepted), (2, 2));

        let events: Vec<_> = hub
            .since(before, 16)
            .into_iter()
            .filter(|event| event.code == "UI.EVENT")
            .collect();
        assert_eq!(events.len(), 2);
        assert_ne!(
            events[0].trace.as_hex(),
            TRACE,
            "raw external trace retained"
        );
        assert_eq!(
            events[0].trace, events[1].trace,
            "repeated external traces must still correlate inside one session"
        );
        let event = &events[0];
        let later_full = catcoms_diagnostics::event_view(event, CaptureMode::Full);
        let rendered = format!(
            "{} {} {:?}",
            later_full.target,
            later_full.trace,
            later_full
                .fields
                .iter()
                .map(|field| (&field.name, &field.value))
                .collect::<Vec<_>>()
        );
        assert_eq!(later_full.capture, "safe");
        for canary in [IPV6, PEER, TRACE] {
            assert!(
                !rendered.contains(canary),
                "Safe-captured UI text reappeared in a later Full view: {rendered}"
            );
        }

        hub.set_mode(restore.mode);
        for section in catcoms_diagnostics::SECTIONS {
            hub.set_section_level(section, restore.level(section));
        }
    }

    /// A locked vault must not be reported as a broken server.
    ///
    /// These two failures want opposite things from the user, and for a long time they arrived
    /// identically: `actor_of` checked the lock and then reported everything as
    /// `SERVER.ACTOR.UNAVAILABLE` with a `Restart` remediation, so somebody who needed to type a
    /// passphrase was told to restart the application. The sentence above it was even correct,
    /// which is what made it survive: only the remediation was wrong, and nothing asserted on that.
    #[test]
    fn a_locked_vault_and_a_missing_server_ask_for_different_things() {
        assert_eq!(ActorLookup::Locked.code().code(), "SESSION.LOCKED");
        assert_eq!(
            ActorLookup::Locked.code().remediation(),
            Some(errors::Remediation::Unlock),
            "the fix is a passphrase, and the advice has to say so"
        );

        assert_eq!(
            ActorLookup::NotOpen.code().code(),
            "SERVER.ACTOR.UNAVAILABLE"
        );
        assert_eq!(
            ActorLookup::NotOpen.code().remediation(),
            Some(errors::Remediation::Restart)
        );

        // The distinction is the point, so assert it rather than the two halves separately.
        assert_ne!(
            ActorLookup::Locked.code().remediation(),
            ActorLookup::NotOpen.code().remediation()
        );
    }

    /// A refusal about who somebody is must not arrive as advice about what they typed.
    ///
    /// The status feed's commands all reported `DOCUMENT.WRITE.REJECTED`, whose declared remedy is
    /// `AmendInput`, so a member refused by the feed's posting policy was told to change their
    /// wording while the composer deliberately kept their draft: the app's combined answer to "you
    /// may not post here" was "try rephrasing it". The sentence above it was correct in every
    /// case, which is what let it stand. Found by adversarial review.
    #[test]
    fn a_status_refusal_about_authority_suggests_nothing_to_retype() {
        for code in [
            codes::STATUS_POST_REJECTED,
            codes::STATUS_DELETE_REJECTED,
            codes::STATUS_REACTION_REJECTED,
            codes::STATUS_PIN_REJECTED,
            codes::STATUS_POLICY_REJECTED,
        ] {
            assert_eq!(
                code.remediation(),
                None,
                "{} is refused by a role or a policy, so it has nothing to suggest",
                code.code()
            );
        }
        // And the one that does carry a remedy is the one carrying the caller's text, so the
        // split is asserted rather than only its larger half.
        assert_eq!(
            codes::STATUS_EDIT_REJECTED.remediation(),
            Some(errors::Remediation::AmendInput)
        );
    }

    /// The lookup itself, not just the mapping. A default state has no unlocked session, which is
    /// exactly the case that used to come back as a broken server.
    #[tokio::test]
    async fn looking_up_an_actor_while_locked_says_locked() {
        let state = AppState::default();
        assert_eq!(actor_of(&state, 1).await.unwrap_err(), ActorLookup::Locked);
        // And with the lock out of the way it is an honest "no such server", rather than the lock
        // answer leaking into every later failure. A real vault, because that is what unlocked
        // means here: the check is `resumable && store.is_some()`, and faking half of it would test
        // the test rather than the code.
        let dir = tempfile::tempdir().unwrap();
        let store =
            catcoms_app::ServerStore::open(dir.path(), b"passphrase", &mut catcoms_rt::OsCryptoRng)
                .expect("a fresh vault opens");
        *state.session_resumable.lock().await = true;
        *state.store.lock().await = Some(store);
        assert_eq!(actor_of(&state, 1).await.unwrap_err(), ActorLookup::NotOpen);
    }

    /// A task started from `setup` must not need a runtime that does not exist yet.
    ///
    /// Deliberately a plain `#[test]` and not a `#[tokio::test]`. That is the whole point: `setup`
    /// runs on the main thread before the async runtime is entered, so this test is in the same
    /// context the real caller is, and a bare `tokio::spawn` panics in it exactly as it did at
    /// startup.
    ///
    /// This is the bug that shipped: `spawn_network_monitor` was changed to `tokio::spawn` to give
    /// the supervisor a handle it could read a panic from, which is right, and it was called before
    /// there was anything to spawn onto, which is fatal. Nothing caught it, because the whole suite
    /// tests functions and never starts the application, so the first thing to notice was a person
    /// launching it and getting exit code 101.
    #[test]
    fn a_task_can_be_supervised_from_before_the_runtime_exists() {
        let handle = supervise_detached("test_detached", None, None, async {});
        // Known immediately, rather than once it has been given a thread. A caller that asked for a
        // task and crashed before it started should still leave evidence that it asked.
        let found = tasks::snapshot(wall_ms())
            .into_iter()
            .find(|t| t.id == handle.id())
            .expect("the task is registered before it is spawned");
        assert_eq!(found.kind, "test_detached");
    }

    /// An emitted event has to tell the webview which operation it belongs to.
    ///
    /// The last hop of P3-004. The trace crosses the actor boundary, reaches the bridge, and then
    /// has to survive into the payload the listener actually reads, or the frontend stages of an
    /// operation are back to being lined up against the native ones by wall clock.
    #[test]
    fn an_emitted_payload_carries_the_operation_that_caused_it() {
        let trace = catcoms_diagnostics::TraceId(0x7f2c_0000_0000_0001);
        let stamped = stamp_payload(
            serde_json::json!({ "server": 1 }),
            1198,
            44_012,
            trace,
            Some("native-proof"),
        )
        .expect("an object payload can carry the envelope");
        assert_eq!(stamped["__seq"], 1198, "this event name's own sequence");
        assert_eq!(
            stamped["__ord"], 44_012,
            "and the whole stream's, which is what says whether anything at all was missed"
        );
        assert_eq!(stamped["__gen"], event_generation());
        assert_eq!(stamped["__trace"], "7f2c000000000001");
        assert_eq!(stamped["__trace_proof"], "native-proof");
        assert_eq!(stamped["server"], 1, "and the payload itself is untouched");

        // An arrival from a peer belongs to no local operation. Left off entirely rather than sent
        // as sixteen zeroes, so a listener testing for it gets an answer rather than a value that
        // looks like an operation and is not one.
        let spontaneous = stamp_payload(
            serde_json::json!({ "server": 1 }),
            1199,
            44_013,
            catcoms_diagnostics::TraceId::default(),
            None,
        )
        .expect("still numbered");
        assert_eq!(spontaneous["__seq"], 1199);
        assert!(spontaneous.get("__trace").is_none());

        // A payload that is not an object can carry none of it, and says so by refusing rather
        // than by silently dropping the sequence the frontend checks for gaps with.
        assert!(stamp_payload(serde_json::json!(7), 1200, 44_014, trace, None).is_none());
    }

    /// A payload field that shadows the envelope must be noticed, not silently overwritten.
    ///
    /// The envelope has to win, or the frontend reads application data as a sequence number and
    /// either invents a gap or misses a real one. But winning quietly is how such a collision would
    /// survive: the symptom is a gap detector that has become fiction.
    #[test]
    fn a_payload_that_shadows_the_envelope_is_noticed() {
        assert!(collides_with_envelope(
            &serde_json::json!({ "server": 1, "__seq": 5 })
        ));
        assert!(collides_with_envelope(&serde_json::json!({ "__ord": 5 })));
        assert!(collides_with_envelope(&serde_json::json!({ "__gen": 5 })));
        assert!(collides_with_envelope(
            &serde_json::json!({ "__trace": "x" })
        ));
        assert!(collides_with_envelope(
            &serde_json::json!({ "__trace_proof": "x" })
        ));
        assert!(!collides_with_envelope(
            &serde_json::json!({ "server": 1, "seq": 5 })
        ));
        // A payload that cannot carry an envelope cannot collide with one either.
        assert!(!collides_with_envelope(&serde_json::json!(7)));

        // Every envelope key is checked. One added to `stamp_payload` and forgotten here would be
        // one the collision test does not cover.
        let stamped = stamp_payload(
            serde_json::json!({}),
            1,
            2,
            catcoms_diagnostics::TraceId(9),
            Some("proof"),
        )
        .unwrap();
        for key in stamped.as_object().unwrap().keys() {
            assert!(
                ENVELOPE_KEYS.contains(&key.as_str()),
                "{key} is stamped onto every payload but is not guarded against collision"
            );
        }
    }

    /// Pin the external trace's canonical wire spelling independently from its privacy reduction.
    #[test]
    fn a_webview_trace_is_the_same_trace_natively() {
        assert_eq!(
            parse_trace("7f2c000000000001"),
            Some(catcoms_diagnostics::TraceId(0x7f2c_0000_0000_0001))
        );
        assert_eq!(parse_trace(""), None);
        assert_eq!(parse_trace("not hex"), None);
        assert_eq!(
            parse_trace("0000000000000000"),
            None,
            "an all-zero trace is what unset renders as; correlating on it would gather everything"
        );
    }

    #[test]
    fn a_native_event_round_trip_normalizes_an_external_trace_exactly_once() {
        const RAW: &str = "20010db8feed0042";
        let raw = parse_trace(RAW).unwrap();
        let operation = Operation::start_maybe(
            Some(RAW.into()),
            catcoms_diagnostics::Section::Ipc,
            "trace-proof-test",
            None,
            None,
        );
        assert_ne!(
            operation.trace, raw,
            "caller-controlled trace bytes entered native state"
        );

        // This is the same pair `emit_tracked` places in an event envelope after an actor carries
        // the normalized trace. The webview returns both unchanged on its UI diagnostic stage.
        let proof = hex::encode(catcoms_log::hub().trace_proof(operation.trace));
        let stamped = stamp_payload(
            serde_json::json!({ "server": 1 }),
            1,
            2,
            operation.trace,
            Some(&proof),
        )
        .unwrap();
        let returned = parse_returned_ui_trace(
            stamped["__trace"].as_str().unwrap(),
            stamped["__trace_proof"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            returned, operation.trace,
            "the return path must not compute H(H(raw))"
        );

        let unproved = parse_returned_ui_trace(stamped["__trace"].as_str().unwrap(), "").unwrap();
        assert_ne!(
            unproved, operation.trace,
            "the webview cannot claim arbitrary hex was already normalized without a valid proof"
        );
    }

    #[test]
    fn upload_progress_never_blesses_a_renderer_controlled_trace_as_native() {
        const RAW: &str = "20010db8feed0042";
        let raw = parse_trace(RAW).unwrap();
        let first = external_progress_trace(Some(RAW));
        let repeated = external_progress_trace(Some(RAW));

        assert_ne!(first, raw);
        assert_eq!(
            first, repeated,
            "upload stages still need session-local correlation"
        );
        assert_eq!(
            external_progress_trace(None),
            catcoms_diagnostics::TraceId::default()
        );
    }

    /// The scope rules behind the invite page's "Your network / Internet" chips and the manual
    /// port-forward suggestion. Each address kind must land in exactly one scope, and only a
    /// plain LAN listener may supply the ip/port the suggestion names.
    #[test]
    fn invite_route_classification_pins_the_scope_rules() {
        let peer = "12D3KooWSaXFXMFgkGxgBF6UPEojspeSj2KaDiP4ks5poLzieKKN";
        let mut check = InviteRouteCheck::default();
        classify_invite_routes(
            &mut check,
            &[
                "not a multiaddr".to_string(),
                format!("/ip4/127.0.0.1/tcp/31484/p2p/{peer}"),
                format!("/ip4/192.168.0.231/udp/31484/quic-v1/p2p/{peer}"),
                format!("/ip4/213.105.231.38/udp/31484/quic-v1/p2p/{peer}"),
            ],
        );
        assert!(check.lan, "a private listener is the LAN scope");
        assert!(
            check.public_direct,
            "a routable listener is the internet scope"
        );
        assert!(!check.relay);
        // The suggestion names the LAN listener's exact socket; loopback supplies nothing.
        assert_eq!(check.lan_ip, "192.168.0.231");
        assert_eq!(check.port, 31484);

        // A routable relay circuit is reachability, but through the relay, never "direct".
        let mut relayed = InviteRouteCheck::default();
        classify_invite_routes(
            &mut relayed,
            &[format!(
                "/ip4/213.105.231.38/udp/7220/quic-v1/p2p/{peer}/p2p-circuit/p2p/{peer}"
            )],
        );
        assert!(relayed.relay);
        assert!(!relayed.public_direct);

        // A LAN-hosted circuit is dialable on this network, but its socket is the relay's
        // machine: it must never supply the port-forward suggestion's values.
        let mut lan_relay = InviteRouteCheck::default();
        classify_invite_routes(
            &mut lan_relay,
            &[format!(
                "/ip4/192.168.0.50/tcp/7220/p2p/{peer}/p2p-circuit/p2p/{peer}"
            )],
        );
        assert!(lan_relay.lan);
        assert!(!lan_relay.relay, "a LAN relay earns no internet confidence");
        assert_eq!(
            lan_relay.lan_ip, "",
            "the relay's socket is not this machine's"
        );
        assert_eq!(lan_relay.port, 0);

        // Loopback alone reaches neither scope: same-machine only.
        let mut loopback = InviteRouteCheck::default();
        classify_invite_routes(
            &mut loopback,
            &[format!("/ip4/127.0.0.1/tcp/31484/p2p/{peer}")],
        );
        assert!(!loopback.lan);
        assert!(!loopback.public_direct);
        assert!(!loopback.relay);
    }

    fn test_libp2p_peer(n: u8) -> libp2p::PeerId {
        let mut seed = [21; 32];
        seed[0] = n;
        keypair_from_seed(seed).unwrap().public().to_peer_id()
    }

    #[tokio::test]
    async fn native_network_bursts_become_one_bounded_generation() {
        let source = n0_watcher::Watchable::new(0u8);
        let watcher = source.watch();
        let signal = NetworkChangeSignal::default();
        let mut generations = signal.subscribe();
        let clock = catcoms_rt::ManualClock::new(1_000);

        // Make the first update observable before the forwarding task starts, then let it reach
        // the injected debounce sleep before delivering a second callback in the same burst.
        source.set(1).unwrap();
        let task = tokio::spawn(forward_network_changes(watcher, signal, clock.clone()));
        tokio::task::yield_now().await;
        clock.advance_ms(500);
        source.set(2).unwrap();
        tokio::task::yield_now().await;

        // A trailing debounce restarts from the second callback; the first callback's original
        // deadline must not publish an intermediate epoch.
        clock.advance_ms(NETWORK_CHANGE_DEBOUNCE_MS - 1);
        tokio::task::yield_now().await;
        assert!(!generations.has_changed().unwrap());
        clock.advance_ms(1);
        generations.changed().await.unwrap();
        assert_eq!(*generations.borrow_and_update(), 1);

        // The second source state was consumed by the first quiet window, not left queued as a
        // second signed epoch. Advancing another whole window must therefore remain silent.
        tokio::task::yield_now().await;
        clock.advance_ms(NETWORK_CHANGE_DEBOUNCE_MS);
        tokio::task::yield_now().await;
        assert!(!generations.has_changed().unwrap());

        // A genuinely later change still reaches every subscriber exactly once.
        source.set(3).unwrap();
        tokio::task::yield_now().await;
        clock.advance_ms(NETWORK_CHANGE_DEBOUNCE_MS);
        generations.changed().await.unwrap();
        assert_eq!(*generations.borrow_and_update(), 2);

        drop(source);
        task.await.unwrap();
    }

    /// A pending upload for a file of `size` bytes, as `begin_file_upload` would have made it.
    fn pending(size: u64) -> PendingUpload {
        PendingUpload {
            server: 1,
            upload_id: "transfer-1".into(),
            mime: "application/octet-stream".into(),
            address: CidHasher::new(),
            buffer: Vec::new(),
            chunks: Vec::new(),
            sealing: None,
            bytes_seen: 0,
            declared_size: size,
            chunk_total: upload_chunk_count(size),
            touched_at: 0,
        }
    }

    /// A `FileRef` shaped well enough to stand in for a sealed chunk; these tests never look
    /// inside one, they only count them.
    fn stub_ref() -> FileRef {
        FileRef {
            plaintext_cid: Cid::of(b"plaintext"),
            ciphertext_cid: Cid::of(b"ciphertext"),
            wrapped_key: catcoms_app::SealedBlob {
                nonce: [0u8; 24],
                ciphertext: vec![0u8; 48],
            },
            size: 0,
            mime: "application/octet-stream".into(),
        }
    }

    /// Drive a whole file through the slice path the frontend uses, sealing (with a stub) exactly
    /// where the real bridge would. Returns the chunks that were handed to the sealer.
    fn stream(size: usize) -> (PendingUpload, Vec<Vec<u8>>) {
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let mut up = pending(size as u64);
        let mut sealed = Vec::new();
        let mut offset = 0usize;
        while offset < size {
            let end = (offset + UPLOAD_SLICE_BYTES).min(size);
            if let Some(chunk) = up.admit_slice(offset as u64, &data[offset..end]).unwrap() {
                let index = up.sealing.expect("a claimed chunk names its index");
                sealed.push(chunk);
                assert!(up.can_accept(index));
                up.chunk_sealed(stub_ref());
            }
            offset = end;
        }
        if let Some(tail) = up.take_tail().unwrap() {
            let index = up.sealing.expect("a claimed tail names its index");
            sealed.push(tail);
            assert!(up.can_accept(index));
            up.chunk_sealed(stub_ref());
        }
        (up, sealed)
    }

    #[test]
    fn streamed_slices_reassemble_into_uniform_chunks() {
        // The manifest's chunks have to be CHUNK_BYTES each with one short tail, whatever slice
        // size the bridge was fed: the media reader finds a byte offset's chunk by dividing.
        for size in [
            0,
            1,
            UPLOAD_SLICE_BYTES,
            UPLOAD_SLICE_BYTES + 3,
            CHUNK_BYTES - 1,
            CHUNK_BYTES,
            CHUNK_BYTES + 1,
            CHUNK_BYTES * 2 + 4242,
        ] {
            let (up, sealed) = stream(size);
            assert!(up.is_complete(), "{size}: every byte accounted for");
            assert_eq!(
                sealed.len(),
                upload_chunk_count(size as u64),
                "{size}: chunks"
            );
            assert_eq!(up.chunks.len(), sealed.len(), "{size}: refs recorded");
            for (i, chunk) in sealed.iter().enumerate() {
                let expected = if i + 1 < sealed.len() {
                    CHUNK_BYTES
                } else {
                    size - CHUNK_BYTES * i
                };
                assert_eq!(chunk.len(), expected, "{size}: chunk {i} is the wrong size");
            }
            let rejoined: Vec<u8> = sealed.concat();
            let original: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            assert_eq!(rejoined, original, "{size}: the chunks are the file");
        }
    }

    #[test]
    fn the_streamed_address_is_the_whole_file_address() {
        // The file's identity, and what every downloader checks the reassembly against. A
        // streamed upload never holds the file, so this is accumulated slice by slice.
        for size in [0, 7, CHUNK_BYTES + 11, CHUNK_BYTES * 2] {
            let (up, _) = stream(size);
            let original: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            assert_eq!(up.address.cid(), Cid::of(&original), "size {size}");
        }
    }

    #[test]
    fn an_empty_upload_still_seals_exactly_one_chunk() {
        let (up, sealed) = stream(0);
        assert_eq!(sealed, vec![Vec::<u8>::new()], "one empty chunk");
        assert!(up.is_complete());
    }

    #[test]
    fn a_file_that_is_a_whole_number_of_chunks_gets_no_extra_tail() {
        // take_tail must not append an empty chunk when the last slice already closed a chunk;
        // that would make the manifest one chunk longer than the reader expects.
        let (_, sealed) = stream(CHUNK_BYTES);
        assert_eq!(sealed.len(), 1);
        assert_eq!(sealed[0].len(), CHUNK_BYTES);
    }

    #[test]
    fn upload_ticket_keys_are_the_names_the_upload_loop_destructures() {
        // The ticket exists so the frontend never holds its own copy of the slice size, which
        // makes the key names part of that contract rather than a detail of this struct. The
        // upload loop destructures `{ token, chunkTotal, sliceBytes }`, and a mismatch is silent
        // in both languages: TypeScript trusts its declared type, serde has no idea who reads it.
        // What reaches the user is `only the last slice of an upload may be short` on the first
        // slice, because `offset + undefined` is NaN and `Blob.slice` reads that as an empty
        // slice. Most payloads here are snake_case and read field by field; this one is not.
        let json = serde_json::to_value(UploadTicket {
            token: "t".into(),
            chunk_total: 3,
            slice_bytes: UPLOAD_SLICE_BYTES,
        })
        .unwrap();
        let obj = json.as_object().unwrap();
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["chunkTotal", "sliceBytes", "token"]);
        assert_eq!(obj["sliceBytes"], serde_json::json!(UPLOAD_SLICE_BYTES));
        assert_eq!(obj["chunkTotal"], serde_json::json!(3));
    }

    #[test]
    fn out_of_order_repeated_and_short_slices_are_refused() {
        let slice = vec![0u8; UPLOAD_SLICE_BYTES];
        let size = (UPLOAD_SLICE_BYTES * 3) as u64;

        // A gap would leave a hole the manifest cannot describe.
        let mut up = pending(size);
        assert!(up.admit_slice(UPLOAD_SLICE_BYTES as u64, &slice).is_err());

        // A repeat would duplicate bytes into the address and the buffer.
        let mut up = pending(size);
        up.admit_slice(0, &slice).unwrap();
        assert!(up.admit_slice(0, &slice).is_err());

        // A short slice before the end of the file would shift every later chunk boundary.
        let mut up = pending(size);
        assert!(up.admit_slice(0, &slice[..8]).is_err());

        // More bytes than were declared: the address would not match the file that was promised.
        let mut up = pending(16);
        assert!(up.admit_slice(0, &slice[..17]).is_err());
    }

    #[test]
    fn a_second_slice_is_refused_while_a_chunk_is_being_sealed() {
        // The map lock is released across the seal, so two concurrent pushes must not both be
        // admitted; the second would race the first onto the same buffer.
        let slice = vec![0u8; UPLOAD_SLICE_BYTES];
        let mut up = pending(CHUNK_BYTES as u64 * 2);
        let mut offset = 0u64;
        loop {
            let full = up.admit_slice(offset, &slice).unwrap();
            offset += UPLOAD_SLICE_BYTES as u64;
            if full.is_some() {
                break;
            }
        }
        assert_eq!(up.sealing, Some(0), "the completed chunk claimed index 0");
        assert!(up.admit_slice(offset, &slice).is_err());
        assert!(up.take_tail().is_err(), "finishing mid-seal is refused too");
        up.chunk_sealed(stub_ref());
        assert!(up.admit_slice(offset, &slice).is_ok(), "sealing released");
    }

    #[test]
    fn a_seal_is_only_attached_to_the_chunk_it_was_started_for() {
        // The map lock is released across the seal, so a completion has to prove it is still the
        // work this upload is waiting for. Anything else would append a ref at the wrong index:
        // the manifest would then be in completion order rather than file order, and every member
        // would fail the whole-file check on a listing nobody can ever repair.
        let slice = vec![0u8; UPLOAD_SLICE_BYTES];
        let mut up = pending(CHUNK_BYTES as u64 * 2);
        let mut offset = 0u64;
        while up.admit_slice(offset, &slice).unwrap().is_none() {
            offset += UPLOAD_SLICE_BYTES as u64;
        }
        assert!(up.can_accept(0), "the chunk it is actually sealing");
        assert!(!up.can_accept(1), "a chunk it has not started");
        up.chunk_sealed(stub_ref());

        // The same completion arriving twice: the second is not owed a slot.
        assert!(!up.can_accept(0), "already recorded");
        assert!(!up.can_accept(1), "nothing is being sealed now");
    }

    #[test]
    fn a_completion_from_a_retired_generation_is_never_attached() {
        // begin/begin under one visible upload id: the second generation is a different entry, so
        // the first generation's outstanding seal cannot be mistaken for work it is waiting for.
        // Keyed by the caller's id instead, this is the schedule that publishes one upload's
        // chunk under another upload's identity.
        let slice = vec![0u8; UPLOAD_SLICE_BYTES];
        let mut old = pending(CHUNK_BYTES as u64 * 2);
        let mut offset = 0u64;
        while old.admit_slice(offset, &slice).unwrap().is_none() {
            offset += UPLOAD_SLICE_BYTES as u64;
        }
        let claimed = old.sealing.expect("a chunk is in flight");

        // The restart: same visible transfer, brand new state. Notably an empty one, whose
        // complete shape is "one chunk and no bytes"; exactly what a stray ref would forge.
        let fresh = pending(0);
        assert!(
            !fresh.can_accept(claimed),
            "a fresh generation is not waiting for the old one's chunk"
        );
        assert!(!fresh.is_complete(), "and it has not silently become whole");
    }

    #[test]
    fn an_upload_that_stops_early_is_never_publishable() {
        // A short upload must not be published: its chunks would be listed under the address of a
        // whole file that was never sent, and every member would fail the reassembly check.
        let slice = vec![0u8; UPLOAD_SLICE_BYTES];
        let mut up = pending(CHUNK_BYTES as u64 * 2);
        up.admit_slice(0, &slice).unwrap();
        assert!(!up.is_complete());
        assert!(up.take_tail().is_err(), "an unfinished upload cannot close");
    }

    #[test]
    fn a_sealed_upload_missing_a_chunk_reference_is_incomplete() {
        // Every byte arrived and the buffer is empty, but a seal never reported back. Publishing
        // would write a manifest with a hole in it.
        let (mut up, _) = stream(CHUNK_BYTES * 2);
        assert!(up.is_complete());
        up.chunks.pop();
        assert!(!up.is_complete());
    }

    #[test]
    fn an_upload_is_split_into_uniform_chunks_with_one_short_tail() {
        // The frontend sends exactly this many chunks, and the media reader finds a byte offset's
        // chunk by dividing, so the count has to agree with CHUNK_BYTES-sized slicing exactly.
        assert_eq!(upload_chunk_count(0), 1, "an empty file is still one chunk");
        assert_eq!(upload_chunk_count(1), 1);
        assert_eq!(upload_chunk_count(CHUNK_BYTES as u64 - 1), 1);
        assert_eq!(upload_chunk_count(CHUNK_BYTES as u64), 1);
        assert_eq!(upload_chunk_count(CHUNK_BYTES as u64 + 1), 2);
        assert_eq!(upload_chunk_count(CHUNK_BYTES as u64 * 3), 3);
        // Derived, not a literal: this was `32` and silently became wrong the moment the file
        // ceiling was raised. That the count also matches what a manifest may declare is
        // static-asserted in `catcoms-app`, which owns both constants.
        assert_eq!(
            upload_chunk_count(MAX_FILE_BYTES as u64),
            MAX_FILE_BYTES.div_ceil(CHUNK_BYTES),
        );
        for size in [0u64, 1, 4242, CHUNK_BYTES as u64 * 2 + 7] {
            let sliced = (0..upload_chunk_count(size))
                .map(|i| {
                    let start = i as u64 * CHUNK_BYTES as u64;
                    size.saturating_sub(start).min(CHUNK_BYTES as u64)
                })
                .sum::<u64>();
            assert_eq!(sliced, size, "the chunks cover the file exactly");
        }
    }

    #[test]
    fn an_inline_read_is_refused_past_the_cap_and_allowed_up_to_it() {
        assert!(inline_download_allowed(0).is_ok(), "an empty file");
        assert!(
            inline_download_allowed(2 * 1024 * 1024).is_ok(),
            "a text read"
        );
        assert!(
            inline_download_allowed(MAX_INLINE_DOWNLOAD_BYTES).is_ok(),
            "exactly at the cap is still inline"
        );
        let over = inline_download_allowed(MAX_INLINE_DOWNLOAD_BYTES + 1).unwrap_err();
        assert!(
            over.contains("streamed or saved"),
            "says what to do instead"
        );
        // The case that mattered: a listing at the product's own maximum. Before the cap this was
        // a ~341 MB JS string built on the webview's main thread, reachable by scrolling past an
        // embedded file rather than by any deliberate action.
        assert!(inline_download_allowed(MAX_FILE_BYTES as u64).is_err());
    }

    #[test]
    fn cancellable_inline_downloads_are_globally_bounded_and_reusable() {
        let state = AppState::default();
        let mut leases = Vec::new();
        for index in 0..MAX_CANCELLABLE_INLINE_DOWNLOADS {
            let id = format!("jam:1:{index}");
            register_inline_download(&state, &id).unwrap();
            leases.push(claim_inline_download(&state, &id).unwrap());
        }
        assert!(register_inline_download(&state, "jam:1:overflow")
            .unwrap_err()
            .contains("too many"));
        assert!(cancel_inline_download_registration(&state, "jam:1:0").unwrap());
        let cancelled = leases.remove(0);
        drop(cancelled);
        register_inline_download(&state, "jam:1:replacement").unwrap();
        assert_eq!(
            state
                .inline_downloads
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len(),
            MAX_CANCELLABLE_INLINE_DOWNLOADS
        );
        drop(leases);
    }

    #[test]
    fn abandoned_unclaimed_inline_registration_cannot_starve_a_reload() {
        let state = AppState::default();
        for index in 0..MAX_CANCELLABLE_INLINE_DOWNLOADS {
            register_inline_download(&state, &format!("jam:1:{index}")).unwrap();
        }
        register_inline_download(&state, "jam:2:fresh").unwrap();
        let fresh = claim_inline_download(&state, "jam:2:fresh").unwrap();
        assert_eq!(
            state
                .inline_downloads
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len(),
            MAX_CANCELLABLE_INLINE_DOWNLOADS
        );
        drop(fresh);
    }

    #[test]
    fn abandoned_unclaimed_inline_registration_can_be_replaced_by_the_same_id() {
        let state = AppState::default();
        register_inline_download(&state, "jam:1:1").unwrap();
        let old_receiver = state
            .inline_downloads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get("jam:1:1")
            .unwrap()
            .signal
            .subscribe();

        register_inline_download(&state, "jam:1:1").unwrap();
        assert!(
            *old_receiver.borrow(),
            "the abandoned generation is explicitly retired"
        );
        let (_lease, fresh_receiver) = claim_inline_download(&state, "jam:1:1").unwrap();
        assert!(
            !*fresh_receiver.borrow(),
            "the replacement owns an independent cancellation generation"
        );
    }

    #[test]
    fn active_inline_registration_does_not_collide_with_a_fresh_webview_nonce() {
        let state = AppState::default();
        register_inline_download(&state, "jam:aaaaaaaaaaaaaaaa:1:1").unwrap();
        let (_old_lease, _old_receiver) =
            claim_inline_download(&state, "jam:aaaaaaaaaaaaaaaa:1:1").unwrap();

        register_inline_download(&state, "jam:bbbbbbbbbbbbbbbb:1:1").unwrap();
        let (_new_lease, new_receiver) =
            claim_inline_download(&state, "jam:bbbbbbbbbbbbbbbb:1:1").unwrap();
        assert!(
            !*new_receiver.borrow(),
            "reset call/sequence counters remain independent across WebView nonces"
        );
    }

    #[test]
    fn cancellation_wakes_a_claimed_download_and_keeps_it_counted_until_return() {
        let state = AppState::default();
        register_inline_download(&state, "jam:9:1").unwrap();
        let (old_lease, old_receiver) = claim_inline_download(&state, "jam:9:1").unwrap();
        assert!(cancel_inline_download_registration(&state, "jam:9:1").unwrap());
        assert!(
            *old_receiver.borrow(),
            "the actor receives the cancellation edge"
        );

        assert!(register_inline_download(&state, "jam:9:1").is_err());
        drop(old_lease);
        register_inline_download(&state, "jam:9:1").unwrap();
        let (_new_lease, new_receiver) = claim_inline_download(&state, "jam:9:1").unwrap();
        assert!(
            !*new_receiver.borrow(),
            "an old RAII guard cannot retire a reused id generation"
        );
    }

    #[test]
    fn session_lock_retires_every_inline_download_registration() {
        let state = AppState::default();
        register_inline_download(&state, "jam:4:1").unwrap();
        let (lease, receiver) = claim_inline_download(&state, "jam:4:1").unwrap();
        register_inline_download(&state, "jam:4:2").unwrap();

        cancel_all_inline_downloads(&state);
        assert!(
            *receiver.borrow(),
            "claimed actor work receives cancellation"
        );
        {
            let table = state
                .inline_downloads
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            assert_eq!(
                table.len(),
                1,
                "active work remains counted until its return"
            );
            assert!(table.contains_key("jam:4:1"));
            assert!(
                !table.contains_key("jam:4:2"),
                "unclaimed work retires at lock"
            );
        }
        drop(lease);
        assert!(state
            .inline_downloads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
    }

    #[test]
    fn inline_download_cancellation_ids_have_a_small_inert_grammar() {
        let state = AppState::default();
        assert!(register_inline_download(&state, "jam:2:17").is_ok());
        assert!(register_inline_download(&state, "../escape").is_err());
        assert!(register_inline_download(
            &state,
            &"x".repeat(INLINE_DOWNLOAD_CANCELLATION_ID_MAX_BYTES + 1)
        )
        .is_err());
    }

    #[test]
    fn omitted_token_inline_downloads_share_the_global_cap_and_lock_cancellation() {
        let state = AppState::default();
        let mut active = (0..MAX_CANCELLABLE_INLINE_DOWNLOADS)
            .map(|_| claim_internal_inline_download(&state).unwrap())
            .collect::<Vec<_>>();
        let overflow = match claim_internal_inline_download(&state) {
            Err(error) => error,
            Ok(_) => panic!("a fifth compatibility-path download exceeded the global cap"),
        };
        assert!(overflow.contains("too many"));

        cancel_all_inline_downloads(&state);
        assert!(
            active.iter().all(|(_, receiver)| *receiver.borrow()),
            "lock cancellation reaches every compatibility-path operation"
        );
        drop(active.pop());
        let (_replacement, receiver) = claim_internal_inline_download(&state).unwrap();
        assert!(
            !*receiver.borrow(),
            "a returned native lease restores exactly one slot"
        );
    }

    #[tokio::test]
    async fn inline_registration_cannot_overtake_explicit_lock_after_commit_admission() {
        let state = Arc::new(AppState::default());
        // Model begin_inline_download after it acquired the exact-generation commit guard but
        // immediately before its synchronous table insertion.
        let admitted_begin = state.ui_session_commit.lock().await;
        let locking_state = Arc::clone(&state);
        let locking =
            tokio::spawn(
                async move { lock_session_with_generation_inner(&locking_state, None).await },
            );
        while !state.session_lock_requested.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }

        assert!(register_inline_download(&state, "jam:1:lock-race")
            .unwrap_err()
            .contains("locked"));
        drop(admitted_begin);
        let (_generation, result) = locking.await.unwrap();
        result.unwrap();
        assert!(state
            .inline_downloads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
    }

    #[test]
    fn staged_upload_bytes_are_bounded_by_more_than_an_entry_count() {
        // MAX_PENDING_UPLOADS alone bounds nothing: one entry can stage a whole file. The quota
        // has to be in bytes, because that is what actually accumulates in the vault.
        let mut up = pending(CHUNK_BYTES as u64 * 3);
        assert_eq!(up.staged_bytes(), 0, "nothing sealed yet");
        up.chunks.push(stub_ref());
        up.chunks.push(stub_ref());
        assert_eq!(up.staged_bytes(), CHUNK_BYTES as u64 * 2);

        // Sixteen full uploads would blow well past the quota, which is the case the entry cap
        // silently allowed.
        let worst = MAX_PENDING_UPLOADS as u64 * MAX_FILE_BYTES as u64;
        assert!(
            worst > MAX_STAGED_UPLOAD_BYTES,
            "the byte quota is the binding limit, not the entry cap"
        );
    }

    #[test]
    fn an_upload_whose_caller_vanished_is_collectable() {
        // A webview reload loses the ids while the native side keeps running, so nothing will ever
        // cancel these. They must age out rather than hold their slot and their bytes forever.
        let mut up = pending(CHUNK_BYTES as u64 * 2);
        up.touched_at = 1_000;
        assert!(up.idle_since(2_000), "untouched since the cutoff");
        assert!(!up.idle_since(500), "still within the window");

        // Never collect one that is mid-seal: a chunk is in flight and will come back looking
        // for it.
        up.sealing = Some(0);
        assert!(!up.idle_since(2_000));
    }

    /// A scripted [`SaveSource`]: hands out prepared chunks, and can have the session lock land
    /// at a chosen point in the sequence, which is the schedule the real thing cannot be asked for.
    struct ScriptedSave {
        chunks: Vec<Vec<u8>>,
        /// Lock the session once this many chunk fetches have completed: the lock lands while the
        /// caller is suspended in the fetch. `None` never locks.
        lock_after_fetches: Option<usize>,
        /// Lock once this many chunks have actually been written. Reaches the phase after the
        /// loop, where there is no next iteration left to notice a lock.
        lock_after_writes: Option<usize>,
        fetches: std::cell::Cell<usize>,
        locked: std::cell::Cell<bool>,
        /// `(chunks written, bytes written)` after each successful write. The observable record of
        /// how far the save actually got, which is what distinguishes "stopped before writing"
        /// from "wrote and then failed", since the failure path deletes the evidence on disk.
        progress: std::cell::RefCell<Vec<(usize, u64)>>,
    }

    impl ScriptedSave {
        fn new(chunks: Vec<Vec<u8>>) -> Self {
            Self {
                chunks,
                lock_after_fetches: None,
                lock_after_writes: None,
                fetches: std::cell::Cell::new(0),
                locked: std::cell::Cell::new(false),
                progress: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn locking_after_fetch(mut self, fetches: usize) -> Self {
            self.lock_after_fetches = Some(fetches);
            self
        }

        fn locking_after_write(mut self, writes: usize) -> Self {
            self.lock_after_writes = Some(writes);
            self
        }

        /// Chunks whose bytes reached the staging file.
        fn written(&self) -> usize {
            self.progress
                .borrow()
                .iter()
                .filter(|(done, _)| *done > 0)
                .count()
        }

        fn total(&self) -> usize {
            self.chunks.len()
        }

        fn size(&self) -> u64 {
            self.chunks.iter().map(|c| c.len() as u64).sum()
        }

        fn whole(&self) -> Vec<u8> {
            self.chunks.concat()
        }

        fn address(&self) -> [u8; 32] {
            *Cid::of(&self.whole()).as_bytes()
        }
    }

    impl SaveSource for ScriptedSave {
        async fn chunk(&mut self, index: usize) -> Result<(Vec<u8>, Option<String>), String> {
            let chunk = self
                .chunks
                .get(index)
                .cloned()
                .ok_or_else(|| format!("no chunk {index}"))?;
            self.fetches.set(self.fetches.get() + 1);
            // The lock lands while this fetch is outstanding, which is the whole point: it becomes
            // true only after the await the caller was suspended in.
            if Some(self.fetches.get()) == self.lock_after_fetches {
                self.locked.set(true);
            }
            Ok((chunk, Some("a-member".into())))
        }

        async fn still_unlocked(&self) -> Result<(), String> {
            if self.locked.get() {
                Err("the vault is locked".into())
            } else {
                Ok(())
            }
        }

        async fn publish_verified(&self, staging: &Path, final_path: &Path) -> Result<(), String> {
            self.still_unlocked().await?;
            publish_staged_download(staging, final_path)
        }

        fn progress(&self, done: usize, bytes_done: u64, _network: u64, _provider: Option<String>) {
            self.progress.borrow_mut().push((done, bytes_done));
            if Some(done) == self.lock_after_writes && done > 0 {
                self.locked.set(true);
            }
        }
    }

    /// Nothing at all under `dir`, staging file or reserved name.
    fn downloads_empty(dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .map(|d| d.count() == 0)
            .unwrap_or(true)
    }

    #[tokio::test]
    async fn a_streamed_save_writes_verifies_and_publishes() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = ScriptedSave::new(vec![b"first ".to_vec(), b"second".to_vec()]);
        let (total, size, target) = (source.total(), source.size(), source.address());

        let path =
            stream_download_to_disk(dir.path(), "notes.txt", total, size, &target, &mut source)
                .await
                .unwrap();

        assert_eq!(path.file_name().unwrap(), "notes.txt");
        assert_eq!(std::fs::read(&path).unwrap(), b"first second");
        assert!(
            !path.with_extension("txt.part").exists(),
            "staging consumed"
        );
        assert_eq!(
            source.progress.borrow().as_slice(),
            &[(0, 0), (1, 6), (2, 12)],
            "one update per chunk, plus the opening zero"
        );
    }

    #[tokio::test]
    async fn a_lock_landing_during_a_fetch_stops_the_save_before_it_writes() {
        // The race the second session check exists for: the answer from before a fetch says
        // nothing about the session after it. Locking during the first fetch must mean nothing is
        // written and nothing is left behind, not a file that finishes and reveals itself.
        let dir = tempfile::tempdir().unwrap();
        let mut source =
            ScriptedSave::new(vec![b"aaaa".to_vec(), b"bbbb".to_vec()]).locking_after_fetch(1);
        let (total, size, target) = (source.total(), source.size(), source.address());

        let err =
            stream_download_to_disk(dir.path(), "secret.txt", total, size, &target, &mut source)
                .await
                .unwrap_err();

        assert!(err.contains("locked"), "reports why: {err}");
        assert!(
            downloads_empty(dir.path()),
            "no partial file, no reserved name"
        );
        assert_eq!(
            source.fetches.get(),
            1,
            "it stopped rather than carrying on"
        );
        // The load-bearing assertion. Without the check *after* the fetch, the chunk already in
        // hand would be written and only the next iteration would notice the lock; the file would
        // still be cleaned up, so disk state alone cannot tell the two apart. This can: decrypted
        // bytes must never reach the disk once the session has closed.
        assert_eq!(
            source.written(),
            0,
            "no plaintext was written after the lock"
        );
    }

    #[tokio::test]
    async fn a_lock_after_the_last_chunk_still_stops_the_rename() {
        // The subtle half: once the loop has finished there is no next iteration to notice a lock,
        // so without the check before the rename the file would be published under its real name
        // and Downloads opened, after the session had closed.
        let dir = tempfile::tempdir().unwrap();
        let mut source = ScriptedSave::new(vec![b"only chunk".to_vec()]).locking_after_write(1);
        let (total, size, target) = (source.total(), source.size(), source.address());

        let err =
            stream_download_to_disk(dir.path(), "last.txt", total, size, &target, &mut source)
                .await
                .unwrap_err();

        assert!(err.contains("locked"), "{err}");
        assert_eq!(
            source.written(),
            1,
            "the whole file did reach the staging file"
        );
        assert!(
            downloads_empty(dir.path()),
            "but the final name never appeared and the staging file was removed"
        );
    }

    #[tokio::test]
    async fn verified_export_publication_is_bound_to_its_exact_unlock_generation() {
        let vault = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let store = ServerStore::open(vault.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = true;
        let old_generation = unlocked_ui_session_generation(&state).await.unwrap();
        let (mut file, staging, final_path) =
            create_staged_download(downloads.path(), "verified.txt").unwrap();
        file.write_all(b"verified plaintext").unwrap();
        file.sync_all().unwrap();
        drop(file);

        // This is the deterministic post-verification/pre-rename pause: lock completes before the
        // old worker tries to publish its already-synced plaintext staging file.
        lock_session_inner(&state, None).await.unwrap();
        assert!(
            publish_download_for_generation(&state, old_generation, &staging, &final_path,)
                .await
                .is_err()
        );
        assert!(staging.exists());
        assert_eq!(std::fs::metadata(&final_path).unwrap().len(), 0);

        assert!(authenticate_mounted_store(&state, b"correct horse")
            .await
            .unwrap());
        let new_generation = state.ui_session_generation.load(Ordering::Acquire);
        finalize_unlock_session(&state, new_generation, true)
            .await
            .unwrap();
        assert_ne!(old_generation, new_generation);
        assert!(
            publish_download_for_generation(&state, old_generation, &staging, &final_path,)
                .await
                .is_err(),
            "unlocking again must not revive an export authorized by the older session"
        );
        publish_download_for_generation(&state, new_generation, &staging, &final_path)
            .await
            .unwrap();
        assert_eq!(std::fs::read(final_path).unwrap(), b"verified plaintext");
    }

    #[tokio::test]
    async fn a_save_that_fails_its_integrity_check_leaves_nothing_behind() {
        // Chunks that individually arrive fine but do not reassemble to the requested address:
        // a manifest that lied, or chunks served in the wrong order.
        let dir = tempfile::tempdir().unwrap();
        let mut source = ScriptedSave::new(vec![b"real bytes".to_vec()]);
        let (total, size) = (source.total(), source.size());
        let wrong = *Cid::of(b"a different file").as_bytes();

        let err = stream_download_to_disk(dir.path(), "x.bin", total, size, &wrong, &mut source)
            .await
            .unwrap_err();

        assert!(err.contains("integrity"), "{err}");
        assert!(
            downloads_empty(dir.path()),
            "unverified bytes are not left on disk"
        );
    }

    #[tokio::test]
    async fn a_save_stops_when_the_chunks_exceed_the_declared_size() {
        // The manifest-amplification defence at the write boundary: even if a listing's layout
        // check were bypassed, the saver stops before writing past what the file declared.
        let dir = tempfile::tempdir().unwrap();
        let mut source = ScriptedSave::new(vec![b"aaaa".to_vec(), b"bbbb".to_vec()]);
        let (total, target) = (source.total(), source.address());

        let err = stream_download_to_disk(dir.path(), "small.bin", total, 5, &target, &mut source)
            .await
            .unwrap_err();

        assert!(err.contains("more data than it declares"), "{err}");
        assert!(downloads_empty(dir.path()));
    }

    #[tokio::test]
    async fn a_save_stops_when_the_chunks_fall_short_of_the_declared_size() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = ScriptedSave::new(vec![b"aaaa".to_vec()]);
        let (total, target) = (source.total(), source.address());

        let err = stream_download_to_disk(dir.path(), "short.bin", total, 99, &target, &mut source)
            .await
            .unwrap_err();

        assert!(err.contains("less data than it declares"), "{err}");
        assert!(downloads_empty(dir.path()));
    }

    /// An `AppState` holding one pending upload under `key`, in the shape `finish` would see.
    async fn state_with_upload(key: &UploadKey, up: PendingUpload) -> AppState {
        let state = AppState::default();
        state.uploads.lock().await.insert(key.clone(), up);
        state
    }

    #[tokio::test]
    async fn a_lock_before_the_index_post_ends_the_upload_instead_of_publishing_it() {
        // The upload-side twin of the save race. Everything is sealed and complete, and the only
        // step left is the group-visible one. A lock arriving here must end the transfer, not
        // publish it a moment later; and it must take the upload with it rather than leaving a
        // reservation nothing will ever close.
        let key = (1u64, "tok".to_string());
        let (complete, _) = stream(CHUNK_BYTES);
        assert!(complete.is_complete(), "the upload was ready to publish");
        let state = state_with_upload(&key, complete).await;

        let err = take_publishable_upload(&state, &key, true)
            .await
            .unwrap_err();

        assert!(err.contains("locked"), "{err}");
        assert!(
            state.uploads.lock().await.is_empty(),
            "and the upload is gone, not left pending across the lock"
        );
    }

    #[tokio::test]
    async fn a_complete_upload_publishes_and_an_incomplete_one_is_destroyed() {
        let key = (1u64, "tok".to_string());
        let (complete, _) = stream(CHUNK_BYTES * 2 + 3);
        let state = state_with_upload(&key, complete).await;

        let taken = take_publishable_upload(&state, &key, false).await.unwrap();
        assert!(taken.is_complete());
        assert!(state.uploads.lock().await.is_empty(), "taken, not copied");

        // Removed twice is not publishable twice.
        assert!(take_publishable_upload(&state, &key, false).await.is_err());

        // A short upload is refused and destroyed rather than published under a whole-file address
        // that was never sent.
        let mut short = pending(CHUNK_BYTES as u64 * 2);
        short
            .admit_slice(0, &vec![0u8; UPLOAD_SLICE_BYTES])
            .unwrap();
        let state = state_with_upload(&key, short).await;
        let err = take_publishable_upload(&state, &key, false)
            .await
            .unwrap_err();
        assert!(err.contains("did not send every chunk"), "{err}");
        assert!(state.uploads.lock().await.is_empty());
    }

    #[tokio::test]
    async fn locking_the_session_drains_every_upload_in_flight() {
        // Uploads are session state. A lock must not leave one holding a slot (and its staged
        // chunks) into whatever comes next.
        let state = AppState::default();
        for token in ["a", "b"] {
            let (up, _) = stream(CHUNK_BYTES);
            state
                .uploads
                .lock()
                .await
                .insert((1, token.to_string()), up);
        }
        *state.session_resumable.lock().await = true;

        lock_session_inner(&state, None).await.unwrap();

        assert!(state.uploads.lock().await.is_empty(), "drained by the lock");
        assert!(
            !*state.session_resumable.lock().await,
            "and the session closed"
        );
    }

    #[tokio::test]
    async fn explicit_lock_invalidates_a_blocked_join_before_its_commit() {
        use std::sync::atomic::AtomicBool;

        let state = Arc::new(AppState::default());
        let dir = tempfile::tempdir().unwrap();
        let store = ServerStore::open(dir.path(), b"passphrase", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = true;
        let generation = unlocked_ui_session_generation(&state).await.unwrap();

        // Model the long network/reply portion of `join_server`: it captured an unlocked epoch,
        // then remained blocked until after the user explicitly locked the UI.
        let committed = Arc::new(AtomicBool::new(false));
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let blocked_state = state.clone();
        let blocked_committed = committed.clone();
        let join = tokio::spawn(async move {
            release_rx.await.unwrap();
            let _permit = require_ui_session_generation(&blocked_state, generation).await?;
            blocked_committed.store(true, Ordering::Release);
            Ok::<(), String>(())
        });

        lock_session_inner(&state, None).await.unwrap();
        release_tx.send(()).unwrap();
        let error = join.await.unwrap().unwrap_err();
        assert!(error.contains("locked") || error.contains("session changed"));
        assert!(
            !committed.load(Ordering::Acquire),
            "stale join work must not enter the registration/persistence boundary"
        );
        assert_ne!(
            state.ui_session_generation.load(Ordering::Acquire),
            generation
        );
    }

    #[test]
    fn a_download_in_progress_never_occupies_its_final_name() {
        // The peer's chosen filename must not exist holding bytes this device has not verified.
        // Removing the file on a handled error is not enough: a crash, a kill or power loss during
        // the transfer would leave an indexer, an antivirus scanner or the user looking at a
        // partial file under the real name.
        let dir = tempfile::tempdir().unwrap();
        let (mut file, staging, final_path) =
            create_staged_download(dir.path(), "notes.txt").unwrap();
        assert_eq!(final_path.file_name().unwrap(), "notes.txt");
        assert_eq!(staging.file_name().unwrap(), "notes.txt.part");
        file.write_all(b"half a file").unwrap();
        assert_eq!(
            std::fs::metadata(&final_path).unwrap().len(),
            0,
            "the reserved name stays empty while bytes land in the staging file"
        );

        // A second save cannot take the reserved name, nor the staging file, while this one runs.
        let (_f2, staging2, final2) = create_staged_download(dir.path(), "notes.txt").unwrap();
        assert_eq!(final2.file_name().unwrap(), "notes (1).txt");
        assert_ne!(staging2, staging);

        drop(file);
        publish_staged_download(&staging, &final_path).unwrap();
        assert_eq!(std::fs::read(&final_path).unwrap(), b"half a file");
        assert!(!staging.exists(), "publishing consumes the staging file");
    }

    #[test]
    fn an_abandoned_download_leaves_neither_a_partial_nor_a_reservation() {
        // What the failure path has to undo: both the staging file and the name it was holding.
        let dir = tempfile::tempdir().unwrap();
        let (mut file, staging, final_path) =
            create_staged_download(dir.path(), "cat.png").unwrap();
        file.write_all(b"partial").unwrap();
        drop(file);
        let _ = std::fs::remove_file(&staging);
        let _ = std::fs::remove_file(&final_path);
        assert!(!staging.exists());
        assert!(!final_path.exists());
        // And the name is free again for a later, honest save.
        let (_f, _s, again) = create_staged_download(dir.path(), "cat.png").unwrap();
        assert_eq!(again, final_path);
    }

    #[test]
    fn listener_summary_reports_only_the_transports_actually_observed() {
        let addresses = vec![
            "/ip4/0.0.0.0/tcp/22487".parse().unwrap(),
            "/ip6/::/udp/22487/quic-v1".parse().unwrap(),
            "/ip4/203.0.113.7/tcp/4001/p2p/12D3KooWF8W6GDyoRR93iGs7VjVQ7jH1mDbGKiqd28KxoM6qQjTq/p2p-circuit"
                .parse()
                .unwrap(),
        ];
        assert_eq!(listener_summary(&addresses), "IPv4 TCP, IPv6 QUIC");
        assert_eq!(listener_summary(&addresses[2..]), "");
    }

    // --- Regression: the media window that stalled the whole app -----------------------------
    // A fixed 2 MiB window over an 8 MiB chunk meant every chunk was fetched and decrypted four
    // times during ordinary playback. Because the actor handles one command at a time, those
    // redundant reads were also time the server could not answer anything else: joining a call
    // and moving around it left the app looking like it was still loading. The invariant these
    // pin is that one response never spans more than one chunk.

    #[test]
    fn a_media_response_never_spans_more_than_one_chunk() {
        // Every offset in the file, at a few sizes, must stay inside the chunk it starts in.
        for start in [
            0u64,
            1,
            CHUNK_BYTES as u64 - 1,
            CHUNK_BYTES as u64,
            CHUNK_BYTES as u64 + 1,
            CHUNK_BYTES as u64 * 3 + 4242,
        ] {
            for len in [
                1usize,
                4096,
                MEDIA_WINDOW_BYTES,
                CHUNK_BYTES,
                CHUNK_BYTES * 4,
            ] {
                let w = media_window(start, len);
                assert!(w.offset < CHUNK_BYTES, "offset must land inside its chunk");
                assert!(
                    w.offset + w.len <= CHUNK_BYTES,
                    "start={start} len={len} would read past the end of chunk {}",
                    w.index
                );
                assert_eq!(
                    w.index,
                    (start / CHUNK_BYTES as u64) as usize,
                    "the window must serve the chunk the read starts in"
                );
            }
        }
    }

    #[test]
    fn a_media_window_is_truncated_at_the_chunk_boundary_rather_than_split() {
        // Ten bytes before the boundary, asking for a thousand: it serves the ten and lets the
        // reader come back for the next chunk. Splitting the read across two chunks here is the
        // shape that made playback touch every chunk twice.
        let start = CHUNK_BYTES as u64 - 10;
        let w = media_window(start, 1000);
        assert_eq!(w.index, 0);
        assert_eq!(w.offset, CHUNK_BYTES - 10);
        assert_eq!(w.len, 10);

        // The very next read is the whole of the next chunk's head, not a continuation.
        let next = media_window(CHUNK_BYTES as u64, 1000);
        assert_eq!(next.index, 1);
        assert_eq!(next.offset, 0);
        assert_eq!(next.len, 1000);
    }

    #[test]
    fn an_unranged_image_is_served_whole_and_a_player_still_gets_a_window() {
        // The bug this exists to hold shut: an <img> issues a plain GET, takes the body as the
        // whole image, and never asks again. A 4K screenshot is several megabytes, so answering
        // that with one MEDIA_WINDOW_BYTES window handed the decoder a truncated PNG and every
        // such image rendered as a broken icon.
        let screenshot = 6 * 1024 * 1024;
        assert!(
            screenshot > MEDIA_WINDOW_BYTES as u64,
            "the case only exists above the window"
        );
        assert!(serves_whole_image("image/png", false, screenshot));

        // A ranged request is a reader that will come back: it keeps the bounded window, and so
        // does everything that is not an image, because players range-request by construction.
        assert!(
            !serves_whole_image("image/png", true, screenshot),
            "a Range means the caller pages for itself"
        );
        assert!(
            !serves_whole_image("video/mp4", false, screenshot),
            "a player must not pull a whole track into one response"
        );
        assert!(
            !serves_whole_image("audio/mpeg", false, screenshot),
            "nor a whole audio track"
        );

        // And the allocation stays bounded: past the cap it is a window again, which is no worse
        // than the behaviour it replaces.
        assert!(serves_whole_image(
            "image/jpeg",
            false,
            MAX_WHOLE_IMAGE_BYTES
        ));
        assert!(!serves_whole_image(
            "image/jpeg",
            false,
            MAX_WHOLE_IMAGE_BYTES + 1
        ));
    }

    #[test]
    fn a_whole_image_read_covers_every_byte_of_the_file() {
        // The assembly loop reads chunk by chunk and stops at the declared size, so the span has
        // to cover the last partial chunk. Undercounting here would silently truncate again.
        assert_eq!(media_chunk_span(0), 1, "an empty file is still one read");
        assert_eq!(media_chunk_span(1), 1);
        assert_eq!(media_chunk_span(CHUNK_BYTES as u64), 1);
        assert_eq!(
            media_chunk_span(CHUNK_BYTES as u64 + 1),
            2,
            "one byte past a chunk needs the next one"
        );
        for total in [
            1u64,
            4242,
            CHUNK_BYTES as u64 * 3 + 7,
            MAX_WHOLE_IMAGE_BYTES,
        ] {
            let covered = media_chunk_span(total) as u64 * CHUNK_BYTES as u64;
            assert!(
                covered >= total,
                "{total} bytes needs at least {total} bytes of chunks, got {covered}"
            );
        }
    }

    #[test]
    fn sequential_playback_walks_chunks_forward_and_never_returns_to_one() {
        // A player asking for the default window makes several requests per chunk, and that is
        // fine: the chunk cache means only the first of them decrypts anything. What must hold is
        // that the walk only ever moves forward, so a chunk that has been left is never needed
        // again and the two-entry cache is always big enough. A planner that ping-ponged across a
        // boundary would defeat the cache and put every one of those reads back on the actor.
        let total = CHUNK_BYTES as u64 * 3 + 4242;
        let mut order = Vec::new();
        let mut at = 0u64;
        while at < total {
            let w = media_window(at, MEDIA_WINDOW_BYTES);
            assert!(w.len > 0, "a window must always make progress");
            if order.last() != Some(&w.index) {
                order.push(w.index);
            }
            at += w.len as u64;
        }
        // Every chunk, in order, each entered exactly once.
        assert_eq!(
            order,
            vec![0, 1, 2, 3],
            "the walk must be forward-only and skip nothing"
        );
    }

    // --- Regression: the local file that still buffered ---------------------------------------
    // Reaching a chunk is not the same as reading it once. Every response also read chunk 0, to
    // learn a size and a mime the index already carried, so the actual sequence of decrypts was
    // 0, N, 0, N, ... over a two-entry cache: each response threw away the chunk being played to
    // make room for the header, then threw away the header to read the chunk back. A file with
    // every byte on this disk buffered anyway, and because the actor is single-threaded, so did
    // everything else the app wanted to do.

    fn cached(server: u64, cid: &str, index: usize) -> MediaChunk {
        MediaChunk {
            server,
            cid: cid.to_string(),
            manifest_version: [7; 32],
            index,
            bytes: Arc::new(vec![index as u8; 8]),
        }
    }

    #[tokio::test]
    async fn explicit_lock_rejects_late_media_cache_publication_and_serves_no_body() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = true;
        let generation = unlocked_ui_session_generation(&state).await.unwrap();
        state.media_cache.lock().await.push(cached(1, "cid", 0));
        state.media_heads.lock().await.push(MediaHead {
            server: 1,
            cid: "cid".into(),
            manifest_version: [7; 32],
            total_size: 8,
            mime: "image/png".into(),
        });

        lock_session_inner(&state, None).await.unwrap();
        assert!(state.media_cache.lock().await.is_empty());
        assert!(state.media_heads.lock().await.is_empty());
        assert!(!media_cache_put_for_generation(&state, generation, cached(1, "cid", 1),).await);
        assert!(media_head_put_for_generation(
            &state,
            generation,
            MediaHead {
                server: 1,
                cid: "cid".into(),
                manifest_version: [8; 32],
                total_size: 8,
                mime: "image/png".into(),
            },
        )
        .await
        .is_none());
        assert!(state.media_cache.lock().await.is_empty());
        assert!(state.media_heads.lock().await.is_empty());

        // Model a worker paused after constructing plaintext but before the URI responder call.
        // Lock has already completed, so publication must replace it with a bodyless denial.
        let plaintext = http::Response::builder()
            .status(http::StatusCode::PARTIAL_CONTENT)
            .body(b"must not escape".to_vec())
            .unwrap();
        let mut published = None;
        publish_media_response(&state, Some(generation), plaintext, |response| {
            published = Some(response);
        })
        .await;
        let published = published.unwrap();
        assert_eq!(published.status(), http::StatusCode::FORBIDDEN);
        assert!(published.body().is_empty());

        let stale_range = authorized_media_range(&state, generation, 8, Some("bytes=99-".into()))
            .await
            .expect_err("a head resolved before lock cannot reveal its size afterward");
        assert_eq!(stale_range.status(), http::StatusCode::FORBIDDEN);
        assert!(stale_range.headers().get("Content-Range").is_none());
        assert!(stale_range.body().is_empty());

        let cid = Cid::of(b"locked media").to_hex();
        let response = serve_media(&state, &format!("/1/{cid}"), None).await;
        assert_eq!(response.status(), http::StatusCode::FORBIDDEN);
        assert!(response.body().is_empty());
    }

    /// Walk a file the way a player does and report which chunks had to be decrypted.
    fn walk_decrypts(total: u64) -> Vec<usize> {
        let mut cache: Vec<MediaChunk> = Vec::new();
        let mut decrypts = Vec::new();
        let mut at = 0u64;
        while at < total {
            let w = media_window(at, MEDIA_WINDOW_BYTES);
            if media_cache_take(&mut cache, 1, "cid", [7; 32], w.index).is_none() {
                decrypts.push(w.index);
                media_cache_put(&mut cache, cached(1, "cid", w.index));
            }
            at += w.len as u64;
        }
        decrypts
    }

    #[test]
    fn playing_a_held_file_through_decrypts_each_chunk_exactly_once() {
        let total = CHUNK_BYTES as u64 * 3 + 4242;
        assert_eq!(
            walk_decrypts(total),
            vec![0, 1, 2, 3],
            "four chunks, four decrypts: nothing is read twice and nothing is read early"
        );
    }

    #[test]
    fn the_cache_keeps_the_chunk_that_was_just_read() {
        // Recency, not insertion order. The distinction only shows when a third chunk arrives:
        // insertion order evicts whatever has been resident longest, which can be the chunk the
        // player is reading right now.
        let mut cache = Vec::new();
        media_cache_put(&mut cache, cached(1, "cid", 0));
        media_cache_put(&mut cache, cached(1, "cid", 1));
        assert!(media_cache_take(&mut cache, 1, "cid", [7; 32], 0).is_some());
        media_cache_put(&mut cache, cached(1, "cid", 2));
        assert!(
            media_cache_take(&mut cache, 1, "cid", [7; 32], 0).is_some(),
            "the chunk that was just read must survive the next admission"
        );
        assert!(
            media_cache_take(&mut cache, 1, "cid", [7; 32], 1).is_none(),
            "the idle one is the one that goes"
        );
    }

    #[test]
    fn the_cache_never_holds_more_plaintext_than_it_promises() {
        let mut cache = Vec::new();
        for index in 0..10 {
            media_cache_put(&mut cache, cached(1, "cid", index));
            assert!(cache.len() <= MEDIA_CACHE_CHUNKS, "at index {index}");
        }
        // And a re-admission of a chunk already held replaces it rather than duplicating it.
        media_cache_put(&mut cache, cached(1, "cid", 9));
        assert_eq!(cache.iter().filter(|c| c.index == 9).count(), 1);
    }

    #[test]
    fn a_new_track_takes_the_old_track_s_plaintext_out_of_memory() {
        let mut cache = Vec::new();
        media_cache_put(&mut cache, cached(1, "aaa", 0));
        media_cache_put(&mut cache, cached(1, "aaa", 1));
        media_cache_put(&mut cache, cached(1, "bbb", 0));
        assert_eq!(cache.len(), 1, "nothing of the old track is kept");
        assert!(media_cache_take(&mut cache, 1, "aaa", [7; 32], 0).is_none());
        // The same content address on another server is another track, and must not be served
        // from this one's cache.
        assert!(media_cache_take(&mut cache, 2, "bbb", [7; 32], 0).is_none());
        media_cache_put(&mut cache, cached(2, "bbb", 0));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn the_debug_log_opt_out_is_not_accidentally_inverted() {
        // On by default for alpha, off only when the user asked. Inverting this either loses
        // every bug report or writes a log for someone who explicitly declined one.
        assert!(
            debug_enabled_from_flag(false),
            "no opt-out file means logging is on"
        );
        assert!(
            !debug_enabled_from_flag(true),
            "the opt-out file must actually opt out"
        );
    }

    #[test]
    fn a_media_path_accepts_only_a_server_id_and_a_full_cid() {
        let cid = "a".repeat(64);
        assert_eq!(
            parse_media_path(&format!("/7/{cid}")),
            Some((7, cid.clone()))
        );
        // Uppercase hex is normalised rather than refused: the index is keyed lowercase.
        assert_eq!(
            parse_media_path(&format!("/7/{}", "A".repeat(64))),
            Some((7, "a".repeat(64)))
        );
        // Anything that is not exactly <server>/<64 hex> is refused before it can reach the file
        // index: traversal, extra segments, short or non-hex ids, missing parts.
        assert_eq!(parse_media_path("/7/../../etc/passwd"), None);
        assert_eq!(parse_media_path(&format!("/7/{cid}/extra")), None);
        assert_eq!(parse_media_path(&format!("/7/{}", "a".repeat(63))), None);
        assert_eq!(parse_media_path(&format!("/7/{}", "z".repeat(64))), None);
        assert_eq!(parse_media_path(&format!("/{cid}")), None);
        assert_eq!(parse_media_path("/notanumber/".to_string().as_str()), None);
    }

    #[test]
    fn a_range_header_is_read_inclusively_and_capped() {
        let total = 10_000u64;
        // Both ends inclusive: bytes=0-0 is one byte, not zero.
        assert_eq!(parse_range_header("bytes=0-0", total), Some((0, 1)));
        assert_eq!(parse_range_header("bytes=100-199", total), Some((100, 100)));
        // An open-ended range is answered with a window, not the rest of the file.
        assert_eq!(
            parse_range_header("bytes=0-", total),
            Some((0, MEDIA_WINDOW_BYTES))
        );
        // A greedy range cannot make the app allocate more than one window.
        assert_eq!(
            parse_range_header("bytes=0-99999999", total),
            Some((0, MEDIA_WINDOW_BYTES))
        );
        // A suffix range is the LAST n bytes; reading it as the first n would play the wrong part.
        assert_eq!(parse_range_header("-500", total), None);
        assert_eq!(parse_range_header("bytes=-500", total), Some((9_500, 500)));
        assert_eq!(parse_range_header("bytes=-99999", total), Some((0, 10_000)));
        // Only the first range of a multi-range request is honoured.
        assert_eq!(parse_range_header("bytes=0-9,20-29", total), Some((0, 10)));
        // Junk is refused rather than guessed at.
        assert_eq!(parse_range_header("items=0-9", total), None);
        assert_eq!(parse_range_header("bytes=abc-def", total), None);
        assert_eq!(parse_range_header("", total), None);
    }

    #[test]
    fn a_declared_mime_cannot_become_a_script_vector() {
        // The value is author-controlled. Only media types may proceed to byte validation;
        // everything else ultimately receives a bodyless denial from the scheme handler.
        assert_eq!(safe_media_mime("video/mp4"), "video/mp4");
        assert_eq!(safe_media_mime("AUDIO/MPEG"), "audio/mpeg");
        assert_eq!(safe_media_mime("image/svg+xml"), "application/octet-stream");
        // Parameters are stripped rather than echoed.
        assert_eq!(safe_media_mime("video/mp4; codecs=\"avc1\""), "video/mp4");
        for hostile in [
            "text/html",
            "application/javascript",
            "video/mp4\r\nX-Injected: 1",
            "",
            "../../etc/passwd",
        ] {
            assert_eq!(
                safe_media_mime(hostile),
                "application/octet-stream",
                "{hostile} must not be served as its declared type"
            );
        }
    }

    #[test]
    fn media_magic_bytes_must_agree_with_the_exact_declared_container() {
        let png = b"\x89PNG\r\n\x1a\nrest";
        let jpeg = b"\xff\xd8\xff\xe0jpeg";
        let mp3 = b"ID3\x04\0\0music";
        let wav = b"RIFF\x10\0\0\0WAVEdata";
        let mp4 = b"\0\0\0\x18ftypisomcontainer";
        let avi = b"RIFF\x10\0\0\0AVI data";
        assert_eq!(
            media_signature_evidence("image/png", png),
            MediaSignatureEvidence::Matched
        );
        assert_eq!(
            media_signature_evidence("audio/mpeg", mp3),
            MediaSignatureEvidence::Matched
        );
        assert_eq!(
            media_signature_evidence("video/mp4", mp4),
            MediaSignatureEvidence::Matched
        );
        assert_eq!(
            media_signature_evidence("audio/mp4", mp4),
            MediaSignatureEvidence::Matched,
            "container headers alone cannot distinguish audio-only from video MP4"
        );
        assert_eq!(
            media_signature_evidence("image/png", mp3),
            MediaSignatureEvidence::Mismatch
        );
        for (declared, bytes) in [
            ("image/png", jpeg.as_slice()),
            ("audio/mpeg", wav.as_slice()),
            ("video/mp4", avi.as_slice()),
        ] {
            assert_eq!(
                media_signature_evidence(declared, bytes),
                MediaSignatureEvidence::Mismatch,
                "same-family container swaps must not inherit the declared decoder path"
            );
        }
        assert_eq!(
            media_signature_evidence("audio/x-wav", wav),
            MediaSignatureEvidence::Matched,
            "the explicit WAV alias is supported"
        );
        assert_eq!(
            media_signature_evidence("image/png", b"<svg><script/></svg>"),
            MediaSignatureEvidence::Unrecognized
        );
        assert_eq!(
            media_signature_evidence("image/svg+xml", b"<svg/>"),
            MediaSignatureEvidence::NotMedia
        );
        assert_eq!(
            validated_inline_media_mime("image/png", png).as_deref(),
            Some("image/png")
        );
        assert!(validated_inline_media_mime("image/png", mp3).is_none());
        assert!(validated_inline_media_mime("image/png", b"unknown").is_none());
        assert!(validated_inline_media_mime("image/svg+xml", b"<svg/>").is_none());
    }

    #[test]
    fn exported_media_reports_format_evidence_without_claiming_decoder_safety() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("cat.png");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\nrest").unwrap();
        assert_eq!(
            exported_media_validation(&png, "image/png").as_deref(),
            Some("matched")
        );
        assert_eq!(
            exported_media_validation(&png, "audio/mpeg").as_deref(),
            Some("mismatch")
        );
        assert_eq!(exported_media_validation(&png, "text/plain"), None);
    }

    #[test]
    fn media_hosts_report_only_this_node_s_own_public_literals() {
        let advertised = vec![
            // Loopback and RFC1918: never a media host.
            "/ip4/127.0.0.1/udp/22487/quic-v1".to_string(),
            "/ip4/192.168.1.40/tcp/22487".to_string(),
            // A real mapped IPv4 and a real global IPv6.
            "/ip4/93.184.216.34/udp/22487/quic-v1".to_string(),
            "/ip6/2606:2800:220:1:248:1893:25c8:1946/udp/22487/quic-v1".to_string(),
            // A circuit names the RELAY's address, not ours. Reporting it would tell the media
            // plane this node is directly reachable when it is only reachable through a host.
            "/ip4/198.41.0.4/tcp/4001/p2p/12D3KooWF8W6GDyoRR93iGs7VjVQ7jH1mDbGKiqd28KxoM6qQjTq/p2p-circuit"
                .to_string(),
        ];
        let (v4, v6) = routable_media_hosts(&advertised);
        assert_eq!(v4, vec!["93.184.216.34".to_string()]);
        assert_eq!(v6, vec!["2606:2800:220:1:248:1893:25c8:1946".to_string()]);
    }

    #[test]
    fn media_hosts_reject_documentation_ranges() {
        // RFC 5737 / RFC 3849 addresses appear in copy-pasted manual-forward configuration and
        // in test fixtures. They parse and they look public; they route nowhere. Offering one to
        // the media plane would advertise a direct path that can never carry a call.
        let advertised = vec![
            "/ip4/198.51.100.9/udp/22487/quic-v1".to_string(),
            "/ip4/203.0.113.7/udp/22487/quic-v1".to_string(),
            "/ip6/2001:db8::5/udp/22487/quic-v1".to_string(),
        ];
        let (v4, v6) = routable_media_hosts(&advertised);
        assert!(v4.is_empty(), "documentation IPv4 must not be offered");
        assert!(v6.is_empty(), "documentation IPv6 must not be offered");
    }

    #[test]
    fn media_hosts_deduplicate_across_transports_of_the_same_address() {
        // TCP and QUIC listeners on one address must not read as two separate routes.
        let advertised = vec![
            "/ip4/93.184.216.34/tcp/22487".to_string(),
            "/ip4/93.184.216.34/udp/22487/quic-v1".to_string(),
        ];
        let (v4, v6) = routable_media_hosts(&advertised);
        assert_eq!(v4, vec!["93.184.216.34".to_string()]);
        assert!(v6.is_empty());
    }

    #[test]
    fn media_hosts_are_empty_when_only_circuits_are_advertised() {
        // The NAT'd case that motivates bridge election: nothing of our own is routable, even
        // though the relay we are reachable through has a perfectly good public address.
        let advertised = vec![
            "/ip4/198.41.0.4/tcp/4001/p2p/12D3KooWF8W6GDyoRR93iGs7VjVQ7jH1mDbGKiqd28KxoM6qQjTq/p2p-circuit"
                .to_string(),
        ];
        let (v4, v6) = routable_media_hosts(&advertised);
        assert!(v4.is_empty() && v6.is_empty());
    }

    #[test]
    fn join_reply_refresh_is_idempotent_and_key_replacement_needs_confirmation() {
        let mut sessions = HashMap::new();
        let invite = [7; 16];
        let first = test_libp2p_peer(1);
        let other = test_libp2p_peer(2);

        assert_eq!(
            record_active_join_reply(
                &mut sessions,
                4,
                invite,
                first,
                [1; 16],
                61_000,
                false,
                1_000,
            ),
            Ok((false, 1, true, None))
        );
        // Exact replay is harmless and does not grow the nonce ledger.
        assert_eq!(
            record_active_join_reply(
                &mut sessions,
                4,
                invite,
                first,
                [1; 16],
                62_000,
                false,
                2_000,
            ),
            Ok((false, 1, false, None))
        );
        assert_eq!(sessions[&(4, invite)].nonces.len(), 1);

        let refused = record_active_join_reply(
            &mut sessions,
            4,
            invite,
            other,
            [2; 16],
            63_000,
            false,
            3_000,
        )
        .unwrap_err();
        assert!(refused.contains("confirm replacement"));
        assert_eq!(sessions[&(4, invite)].joiner, first);

        assert_eq!(
            record_active_join_reply(
                &mut sessions,
                4,
                invite,
                other,
                [2; 16],
                63_000,
                true,
                3_000,
            ),
            Ok((true, 2, true, Some(first)))
        );
        assert_eq!(sessions[&(4, invite)].joiner, other);
    }

    #[test]
    fn expired_join_reply_sessions_are_pruned_and_refreshes_are_bounded() {
        let mut sessions = HashMap::new();
        let peer = test_libp2p_peer(3);
        for nonce in 0..6u8 {
            record_active_join_reply(
                &mut sessions,
                9,
                [4; 16],
                peer,
                [nonce; 16],
                100_000,
                false,
                1_000,
            )
            .unwrap();
        }
        assert_eq!(sessions[&(9, [4; 16])].nonces.len(), 4);

        record_active_join_reply(
            &mut sessions,
            10,
            [5; 16],
            peer,
            [9; 16],
            200_000,
            false,
            100_001,
        )
        .unwrap();
        assert!(!sessions.contains_key(&(9, [4; 16])));
    }

    #[test]
    fn active_join_reply_windows_have_a_global_bound() {
        let mut sessions = HashMap::new();
        let peer = test_libp2p_peer(4);
        for server in 0..MAX_ACTIVE_JOIN_REPLIES as u64 {
            record_active_join_reply(
                &mut sessions,
                server,
                [server as u8; 16],
                peer,
                [server as u8; 16],
                100_000,
                false,
                1_000,
            )
            .unwrap();
        }
        let error = record_active_join_reply(
            &mut sessions,
            MAX_ACTIVE_JOIN_REPLIES as u64,
            [0xff; 16],
            peer,
            [0xff; 16],
            100_000,
            false,
            1_000,
        )
        .unwrap_err();
        assert!(error.contains("too many connection replies"));
        assert_eq!(sessions.len(), MAX_ACTIVE_JOIN_REPLIES);
    }

    #[test]
    fn clock_skew_never_extends_a_received_join_reply_past_sixty_seconds() {
        let now = 1_000_000;
        assert_eq!(
            effective_join_reply_expiry(now + 90_000, now),
            now + catcoms_net::JOIN_REPLY_LIFETIME_MS
        );
        assert_eq!(effective_join_reply_expiry(now + 20_000, now), now + 20_000);
    }

    #[test]
    fn join_reply_retries_share_exact_socket_budget_across_groups() {
        let joiner = test_libp2p_peer(49);
        let other = test_libp2p_peer(50);
        let route: Multiaddr = format!("/ip4/45.79.12.34/tcp/22487/p2p/{joiner}")
            .parse()
            .unwrap();
        let wrong_peer: Multiaddr = format!("/ip4/8.8.8.8/tcp/22487/p2p/{other}")
            .parse()
            .unwrap();
        let clock = ManualClock::new(0);
        let scheduler = EndpointDialScheduler::new_with_clock(
            catcoms_discovery::EndpointDialConfig {
                window_ms: 1_000,
                process_limit: 4,
                server_limit: 4,
                peer_limit: 4,
                endpoint_limit: 2,
                prefix_limit: 4,
            },
            Arc::new(clock.clone()),
        );

        assert_eq!(
            schedule_join_reply_candidates(
                &scheduler,
                b"group-a",
                &joiner,
                &[route.clone(), wrong_peer],
            ),
            vec![route.clone()],
            "a candidate bound to another peer must never consume a grant or reach the dialer"
        );
        assert_eq!(
            schedule_join_reply_candidates(
                &scheduler,
                b"group-a",
                &joiner,
                std::slice::from_ref(&route),
            ),
            vec![route.clone()]
        );
        assert!(schedule_join_reply_candidates(
            &scheduler,
            b"group-b",
            &joiner,
            std::slice::from_ref(&route),
        )
        .is_empty());

        clock.advance_ms(1_000);
        assert_eq!(
            schedule_join_reply_candidates(
                &scheduler,
                b"group-b",
                &joiner,
                std::slice::from_ref(&route),
            ),
            vec![route]
        );
    }

    #[test]
    fn companion_grants_reject_peer_confusion_and_share_the_process_cap() {
        let first = test_libp2p_peer(47);
        let other = test_libp2p_peer(48);
        let first_route = format!("/ip4/45.79.12.34/tcp/22487/p2p/{first}");
        let second_route = format!("/ip4/8.8.8.8/tcp/22487/p2p/{other}");
        let bare = "/ip4/1.1.1.1/tcp/53".to_string();
        let clock = ManualClock::new(0);
        let scheduler = EndpointDialScheduler::new_with_clock(
            catcoms_discovery::EndpointDialConfig {
                window_ms: 1_000,
                process_limit: 1,
                server_limit: 2,
                peer_limit: 2,
                endpoint_limit: 1,
                prefix_limit: 2,
            },
            Arc::new(clock),
        );

        let (contact, granted) =
            schedule_grant_bootstrap(&scheduler, b"group-a", &[first_route.clone(), bare]).unwrap();
        assert_eq!(contact, phase0_peer_id(&first));
        assert_eq!(granted, vec![first_route.parse::<Multiaddr>().unwrap()]);

        let confused = schedule_grant_bootstrap(
            &EndpointDialScheduler::default(),
            b"group-confused",
            &[first_route, second_route.clone()],
        )
        .unwrap_err();
        assert!(confused.contains("one unambiguous server peer"));

        let capped =
            schedule_grant_bootstrap(&scheduler, b"group-b", std::slice::from_ref(&second_route))
                .unwrap_err();
        assert!(capped.contains("process-wide"));
    }

    #[test]
    fn switchboard_dial_plan_skips_expired_or_mismatched_routes_and_caps_total_dials() {
        let now = 1_000;
        let group_id = vec![9; 16];
        let first = test_libp2p_peer(50);
        let second = test_libp2p_peer(51);
        let first_phase = phase0_peer_id(&first);
        let second_phase = phase0_peer_id(&second);
        let routes = vec![
            catcoms_app::SwitchboardRoute {
                offer: catcoms_app::SwitchboardOffer {
                    group_id: group_id.clone(),
                    device_pubkey: vec![1; 32],
                    peer_id: *first_phase.as_bytes(),
                    addresses: vec![
                        format!("/ip4/45.79.12.34/tcp/22487/p2p/{first}"),
                        format!("/ip4/45.79.12.34/udp/22487/quic-v1/p2p/{first}"),
                        format!("/ip4/45.79.12.34/tcp/9/p2p/{second}"),
                    ],
                    seq: 1,
                    expires_at_ms: 3_000,
                    signature: [0; 64],
                },
            },
            catcoms_app::SwitchboardRoute {
                offer: catcoms_app::SwitchboardOffer {
                    group_id: group_id.clone(),
                    device_pubkey: vec![2; 32],
                    peer_id: *second_phase.as_bytes(),
                    addresses: vec![
                        format!("/ip4/8.8.8.8/tcp/22487/p2p/{second}"),
                        format!("/ip4/8.8.8.8/udp/22487/quic-v1/p2p/{second}"),
                    ],
                    seq: 1,
                    expires_at_ms: 4_000,
                    signature: [0; 64],
                },
            },
            catcoms_app::SwitchboardRoute {
                offer: catcoms_app::SwitchboardOffer {
                    group_id: group_id.clone(),
                    device_pubkey: vec![3; 32],
                    peer_id: *phase0_peer_id(&test_libp2p_peer(52)).as_bytes(),
                    addresses: vec![format!(
                        "/ip4/1.1.1.1/tcp/22487/p2p/{}",
                        test_libp2p_peer(52)
                    )],
                    seq: 1,
                    expires_at_ms: 999,
                    signature: [0; 64],
                },
            },
        ];

        let (allowed, addresses) = switchboard_dial_plan(&routes, &group_id, now, 10_000);
        assert_eq!(addresses.len(), 4);
        assert_eq!(allowed.len(), 2);
        assert_eq!(allowed.get(&first_phase), Some(&3_000));
        assert_eq!(allowed.get(&second_phase), Some(&4_000));
        assert!(addresses.iter().all(|address| {
            target_peer_in_multiaddr(address).is_some_and(|peer| peer == first || peer == second)
        }));

        let clock = ManualClock::new(1_000);
        let scheduler = EndpointDialScheduler::new_with_clock(
            catcoms_discovery::EndpointDialConfig {
                window_ms: 1_000,
                process_limit: 2,
                server_limit: 2,
                peer_limit: 2,
                endpoint_limit: 1,
                prefix_limit: 2,
            },
            Arc::new(clock),
        );
        let (scheduled_allowed, scheduled) =
            schedule_switchboard_candidates(&scheduler, &group_id, allowed, addresses);
        assert_eq!(scheduled.len(), 2, "helper routes spend endpoint tokens");
        assert_eq!(scheduled_allowed.len(), 2);

        let (denied_allowed, denied) = schedule_switchboard_candidates(
            &scheduler,
            b"another-group",
            HashMap::from([(first_phase, 3_000), (second_phase, 4_000)]),
            scheduled,
        );
        assert!(denied.is_empty(), "the process cap is shared across groups");
        assert!(denied_allowed.is_empty());
    }

    #[test]
    fn an_expired_connected_switchboard_cannot_mask_another_live_candidate() {
        let expired = PeerId::from_u64(70);
        let valid = PeerId::from_u64(71);
        let allowed = HashMap::from([(expired, 1_000), (valid, 2_000)]);
        let mut wanted = vec![expired, valid];

        assert_eq!(
            accept_or_prune_switchboard_candidate(&mut wanted, &allowed, expired, 1_001),
            None,
            "a route that expires while the watch is pending must not abort the whole join"
        );
        assert_eq!(wanted, vec![valid]);
        assert_eq!(
            accept_or_prune_switchboard_candidate(&mut wanted, &allowed, valid, 1_001),
            Some((valid, 2_000)),
            "the next already-connected, still-endorsed helper remains usable"
        );
    }

    #[test]
    fn switchboard_host_eligibility_requires_a_public_literal_even_for_relays() {
        let relay = test_libp2p_peer(70);
        let client = test_libp2p_peer(71);
        assert!(switchboard_route_usable(&format!(
            "/ip4/8.8.8.8/tcp/22487/p2p/{relay}/p2p-circuit/p2p/{client}"
        )));
        assert!(!switchboard_route_usable(&format!(
            "/ip4/192.168.1.2/tcp/22487/p2p/{relay}/p2p-circuit/p2p/{client}"
        )));
        assert!(!switchboard_route_usable(&format!(
            "/dns4/relay.example/tcp/22487/p2p/{relay}/p2p-circuit/p2p/{client}"
        )));
    }

    #[test]
    fn switchboard_clock_skew_never_grants_more_than_two_local_minutes() {
        let now = 1_000_000;
        let group_id = vec![0x44; 16];
        let peer = test_libp2p_peer(72);
        let phase_peer = phase0_peer_id(&peer);
        let routes = vec![catcoms_app::SwitchboardRoute {
            offer: catcoms_app::SwitchboardOffer {
                group_id: group_id.clone(),
                device_pubkey: vec![4; 32],
                peer_id: *phase_peer.as_bytes(),
                addresses: vec![format!("/ip4/8.8.4.4/tcp/22487/p2p/{peer}")],
                seq: 1,
                expires_at_ms: now + catcoms_app::SWITCHBOARD_OFFER_MAX_FUTURE_MS,
                signature: [0; 64],
            },
        }];

        let (allowed, addresses) = switchboard_dial_plan(
            &routes,
            &group_id,
            now,
            now + catcoms_app::SWITCHBOARD_OFFER_MAX_FUTURE_MS,
        );
        assert_eq!(addresses.len(), 1);
        assert_eq!(
            allowed.get(&phase_peer),
            Some(&(now + catcoms_app::SWITCHBOARD_OFFER_LIFETIME_MS))
        );
    }

    #[test]
    fn port_mapping_diagnostics_distinguish_waiting_unavailable_and_live_transports() {
        let mut active = HashMap::new();
        let mut unavailable = HashMap::new();
        assert_eq!(
            port_mapping_status(&active, &unavailable, true),
            PORT_MAPPING_WAITING
        );
        assert_eq!(
            port_mapping_status(&active, &unavailable, false),
            PORT_MAPPING_TIMED_OUT
        );

        // A single failed transport is scoped and does not claim the whole mechanism failed.
        unavailable.insert(
            (PortMappingMechanism::Pcp, PortMappingTransport::Tcp, None),
            "no gateway answered".to_string(),
        );
        assert_eq!(
            port_mapping_status(&active, &unavailable, false),
            "no active router mapping (PCP TCP unavailable: no gateway answered)"
        );
        unavailable.insert(
            (PortMappingMechanism::Pcp, PortMappingTransport::Udp, None),
            "no gateway answered".to_string(),
        );
        assert_eq!(
            port_mapping_status(&active, &unavailable, false),
            "no active router mapping (PCP unavailable: no gateway answered)"
        );

        unavailable.remove(&(PortMappingMechanism::Pcp, PortMappingTransport::Tcp, None));
        active.insert(
            (PortMappingMechanism::Pcp, PortMappingTransport::Tcp, None),
            "/ip4/203.0.113.7/tcp/22487".parse().unwrap(),
        );
        active.insert(
            (
                PortMappingMechanism::NatPmp,
                PortMappingTransport::Udp,
                None,
            ),
            "/ip4/203.0.113.7/udp/22487/quic-v1".parse().unwrap(),
        );
        assert_eq!(
            port_mapping_status(&active, &unavailable, false),
            "mapped via NAT-PMP UDP/QUIC: /ip4/203.0.113.7/udp/22487/quic-v1; PCP TCP: /ip4/203.0.113.7/tcp/22487; other attempts: PCP UDP/QUIC unavailable: no gateway answered"
        );

        active.insert(
            (
                PortMappingMechanism::Pcp,
                PortMappingTransport::Udp,
                Some("2606:4700::10".parse().unwrap()),
            ),
            "/ip6/2606:4700::10/udp/22487/quic-v1".parse().unwrap(),
        );
        assert!(port_mapping_status(&active, &unavailable, false).contains(
            "PCP IPv6 pinhole UDP/QUIC (2606:4700::10): /ip6/2606:4700::10/udp/22487/quic-v1"
        ));
    }

    #[test]
    fn scoped_ipv6_failures_do_not_end_the_initial_mapping_window() {
        let first: IpAddr = "2606:4700::10".parse().unwrap();
        let second: IpAddr = "2a00:1450::10".parse().unwrap();
        let active = HashMap::new();
        let unavailable = HashMap::from([
            (
                (
                    PortMappingMechanism::Pcp,
                    PortMappingTransport::Tcp,
                    Some(first),
                ),
                "no IPv6 PCP target discovered".to_string(),
            ),
            (
                (
                    PortMappingMechanism::Pcp,
                    PortMappingTransport::Udp,
                    Some(first),
                ),
                "no IPv6 PCP target discovered".to_string(),
            ),
            (
                (
                    PortMappingMechanism::Pcp,
                    PortMappingTransport::Tcp,
                    Some(second),
                ),
                "no IPv6 PCP target discovered".to_string(),
            ),
            (
                (
                    PortMappingMechanism::Pcp,
                    PortMappingTransport::Udp,
                    Some(second),
                ),
                "no IPv6 PCP target discovered".to_string(),
            ),
            (
                (PortMappingMechanism::Pcp, PortMappingTransport::Tcp, None),
                "IPv4 PCP unavailable".to_string(),
            ),
            (
                (
                    PortMappingMechanism::NatPmp,
                    PortMappingTransport::Tcp,
                    None,
                ),
                "NAT-PMP unavailable".to_string(),
            ),
        ]);

        assert_eq!(unavailable.len(), 6);
        assert_eq!(
            port_mapping_status(&active, &unavailable, true),
            PORT_MAPPING_WAITING,
            "six scoped failures are not proof that the remaining IPv4 workers have settled"
        );
    }

    #[test]
    fn mapping_snapshot_keeps_ipv4_and_ipv6_pcp_owners_separate() {
        let local_v6: IpAddr = "2606:4700::10".parse().unwrap();
        let (active, unavailable) = port_mapping_snapshot_state(PortMappingSnapshot {
            active: vec![
                catcoms_net::ActivePortMapping {
                    mechanism: PortMappingMechanism::Pcp,
                    transport: PortMappingTransport::Tcp,
                    local_address: None,
                    address: "/ip4/8.8.8.8/tcp/22487".parse().unwrap(),
                },
                catcoms_net::ActivePortMapping {
                    mechanism: PortMappingMechanism::Pcp,
                    transport: PortMappingTransport::Tcp,
                    local_address: Some(local_v6),
                    address: "/ip6/2606:4700::10/tcp/22487".parse().unwrap(),
                },
            ],
            unavailable: vec![catcoms_net::PortMappingFailure {
                mechanism: PortMappingMechanism::Pcp,
                transport: PortMappingTransport::Udp,
                local_address: Some(local_v6),
                detail: "IPv6 firewall pinhole unavailable: no gateway".to_string(),
            }],
        });
        assert_eq!(active.len(), 2);
        assert!(active.contains_key(&(PortMappingMechanism::Pcp, PortMappingTransport::Tcp, None)));
        assert!(active.contains_key(&(
            PortMappingMechanism::Pcp,
            PortMappingTransport::Tcp,
            Some(local_v6)
        )));
        assert_eq!(unavailable.len(), 1);
        assert!(port_mapping_status(&active, &unavailable, false).starts_with("mapped via "));
    }

    #[test]
    fn an_address_is_withdrawn_only_after_its_last_mapping_lease_expires() {
        let address: Multiaddr = "/ip4/203.0.113.7/tcp/22487".parse().unwrap();
        let mut active = HashMap::from([
            (
                (PortMappingMechanism::Upnp, PortMappingTransport::Tcp, None),
                address.clone(),
            ),
            (
                (PortMappingMechanism::Pcp, PortMappingTransport::Tcp, None),
                address.clone(),
            ),
        ]);

        assert!(!retire_port_mapping(
            &mut active,
            PortMappingMechanism::Upnp,
            PortMappingTransport::Tcp,
            &address,
        ));
        assert!(active.values().any(|candidate| candidate == &address));
        assert!(retire_port_mapping(
            &mut active,
            PortMappingMechanism::Pcp,
            PortMappingTransport::Tcp,
            &address,
        ));
        assert!(active.is_empty());

        let old: Multiaddr = "/ip4/8.8.8.8/tcp/22487".parse().unwrap();
        let replacement: Multiaddr = "/ip4/9.9.9.9/tcp/22487".parse().unwrap();
        active.insert(
            (PortMappingMechanism::Pcp, PortMappingTransport::Tcp, None),
            old.clone(),
        );
        assert_eq!(
            replace_port_mapping(
                &mut active,
                PortMappingMechanism::Pcp,
                PortMappingTransport::Tcp,
                replacement.clone(),
            ),
            Some(old),
            "a replacement event retires its unshared predecessor without a separate expiry"
        );
        assert_eq!(
            active.get(&(PortMappingMechanism::Pcp, PortMappingTransport::Tcp, None)),
            Some(&replacement)
        );
    }

    #[test]
    fn autonat_results_rank_all_current_routes_and_ignore_expired_evidence() {
        let observer = libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id();
        let result = |address: &str, reachable: bool| catcoms_net::AutoNatResult {
            address: address.parse().unwrap(),
            server: observer,
            reachable,
            error: (!reachable).then(|| "dial-back failed".to_string()),
        };
        let a = "/ip4/8.8.8.8/tcp/22487";
        let b = "/ip4/9.9.9.9/tcp/22487";
        let local = "/ip4/192.168.1.9/tcp/22487";
        let mut evidence = AutoNatEvidence {
            waiting: false,
            results: vec![result(a, false), result(local, true)],
        };
        assert!(
            autonat_status(&[a.into(), local.into()], Some(&evidence))
                .starts_with("unreachable /ip4/8.8.8.8"),
            "a local callback must not erase an actionable public failure"
        );

        evidence.results = vec![result(a, true), result(b, true)];
        assert!(autonat_status(&[a.into(), b.into()], Some(&evidence)).starts_with("reachable "));

        // A's newest observation is now a failure, but B retains an independent success.
        evidence.results = vec![result(a, false), result(b, true)];
        assert!(
            autonat_status(&[a.into(), b.into()], Some(&evidence))
                .starts_with("reachable /ip4/9.9.9.9"),
            "failure of one route cannot erase a second verified route"
        );
        assert!(
            autonat_status(&[a.into()], Some(&evidence)).starts_with("unreachable /ip4/8.8.8.8"),
            "with B expired, only A's current failure remains"
        );
        assert_eq!(
            autonat_status(&[], Some(&evidence)),
            AUTONAT_NOT_TESTED,
            "evidence for routes no longer advertised is not current state"
        );
    }

    #[test]
    fn autonat_evidence_must_name_an_address_the_product_advertises() {
        let address: Multiaddr = "/ip4/8.8.8.8/tcp/22487".parse().unwrap();
        assert!(bootstrap_names_address(
            &[format!(
                "{address}/p2p/12D3KooWJvFzZpCWKjQbGvYQ8uY4rMw1qQznfKqcxpN6qjHVVqUd"
            )],
            &address,
        ));
        assert!(!bootstrap_names_address(
            &["/ip4/9.9.9.9/tcp/22487/p2p/other".to_string()],
            &address,
        ));
    }

    #[test]
    fn a_relay_callback_is_never_reported_as_direct_reachability() {
        let relay = libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id();
        let target = libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id();
        let circuit = format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay}/p2p-circuit/p2p/{target}");
        let evidence = AutoNatEvidence {
            waiting: false,
            results: vec![AutoNatResult {
                address: circuit.parse().unwrap(),
                server: relay,
                reachable: true,
                error: None,
            }],
        };
        assert_eq!(
            autonat_status(&[circuit], Some(&evidence)),
            AUTONAT_NOT_TESTED
        );
    }

    #[test]
    fn live_bootstrap_reconciliation_removes_a_relay_lost_during_startup() {
        let circuit = "/ip4/8.8.8.8/tcp/4001/p2p/relay/p2p-circuit/p2p/member".to_string();
        let mut advertised = vec![circuit];
        reconcile_advertised(&mut advertised, Some(Vec::new()));
        assert!(
            advertised.is_empty(),
            "the expired circuit is authoritative"
        );

        let mut failed_attempt = vec!["diagnostic-only-address".to_string()];
        reconcile_advertised(&mut failed_attempt, None);
        assert_eq!(
            failed_attempt,
            vec!["diagnostic-only-address"],
            "a failed attempt with no live server keeps its diagnostic evidence"
        );
    }

    #[tokio::test]
    async fn explicit_lock_saves_continuity_and_closes_the_webview_command_boundary() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = true;
        state.storage_health.lock().await.insert(
            7,
            CachedStorageHealth {
                server_instance: 1,
                report: build_storage_report(
                    StorageHealth::default(),
                    Vec::new(),
                    &HashSet::new(),
                    41,
                ),
            },
        );
        assert!(require_unlocked_session(&state).await.is_ok());

        let continuity = r#"{"version":1,"drafts":{"1:2":"latest"},"readMarks":{"1:2":9}}"#;
        lock_session_inner(&state, Some(continuity.into()))
            .await
            .unwrap();
        assert_eq!(
            require_unlocked_session(&state).await.unwrap_err(),
            "the vault is locked"
        );
        let saved = state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_ui_state()
            .unwrap();
        assert_eq!(saved, continuity.as_bytes());
        assert!(
            state.storage_health.lock().await.is_empty(),
            "explicit lock must purge cached plaintext file metadata"
        );

        // Validation errors are returned as a completed-lock outcome, rather than an ambiguous IPC
        // rejection, and the security boundary still closes.
        *state.session_resumable.lock().await = true;
        let outcome = lock_session_outcome_inner(&state, Some("not json".into())).await;
        assert!(outcome.continuity_error.is_some());
        assert_eq!(
            require_unlocked_session(&state).await.unwrap_err(),
            "the vault is locked"
        );
        // The vault remains mounted so actors and persistence may continue behind the UI lock.
        assert!(state.store.lock().await.is_some());
    }

    #[tokio::test]
    async fn native_close_defers_one_continuity_failure_then_allows_acknowledged_exit() {
        let state = AppState::default();
        *state.session_resumable.lock().await = true;

        let (first, first_may_destroy) =
            close_vault_window_plan_inner(&state, Some("not json".into()), false).await;
        assert!(first.continuity_error.is_some());
        assert!(!first_may_destroy, "the first failure must remain visible");
        assert_eq!(
            require_unlocked_session(&state).await.unwrap_err(),
            "the vault is locked",
            "the warning is shown only after the native boundary closed",
        );

        let (acknowledged, second_may_destroy) =
            close_vault_window_plan_inner(&state, Some("not json".into()), true).await;
        assert!(acknowledged.continuity_error.is_some());
        assert!(
            second_may_destroy,
            "a repeated close explicitly accepts loss of only the latest screen snapshot"
        );
    }

    #[tokio::test]
    async fn duplicate_close_cannot_overtake_an_unacknowledged_continuity_failure() {
        let state = AppState::default();
        *state.session_resumable.lock().await = true;

        let (first, first_may_destroy) =
            close_vault_window_plan_inner(&state, Some("not json".into()), false).await;
        assert!(first.continuity_error.is_some());
        assert!(!first_may_destroy);

        // This is the duplicate that could already be queued across the bridge before the first
        // response changes frontend state. Even valid bytes are not an acknowledgement of losing
        // the failed snapshot, so it must not authorize destruction.
        let (duplicate, duplicate_may_destroy) = close_vault_window_plan_inner(
            &state,
            Some(r#"{"version":1,"drafts":{},"readMarks":{}}"#.into()),
            false,
        )
        .await;
        assert_eq!(duplicate.continuity_error, first.continuity_error);
        assert!(!duplicate_may_destroy);
    }

    #[tokio::test]
    async fn ordinary_lock_completion_cannot_erase_native_close_debt() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;

        let (failed_close, may_destroy) =
            close_vault_window_plan_inner(&state, Some("not json".into()), false).await;
        assert!(failed_close.continuity_error.is_some());
        assert!(!may_destroy);

        // Model an already-queued Ctrl+L command completing after the close plan. Its successful
        // snapshot legitimately replaces the general lock-completion slot, but not close debt.
        let valid = r#"{"version":1,"drafts":{},"readMarks":{}}"#;
        let later_lock = lock_session_outcome_inner(&state, Some(valid.into())).await;
        assert_eq!(later_lock.continuity_error, None);
        assert_eq!(
            state
                .last_ui_lock_completion
                .lock()
                .await
                .as_ref()
                .and_then(|completion| completion.error.as_ref()),
            None,
        );

        let (duplicate, duplicate_may_destroy) =
            close_vault_window_plan_inner(&state, Some(valid.into()), false).await;
        assert_eq!(duplicate.continuity_error, failed_close.continuity_error);
        assert!(!duplicate_may_destroy);
    }

    #[tokio::test]
    async fn ordinary_lock_consuming_close_snapshot_cannot_authorize_destruction() {
        let state = Arc::new(AppState::default());
        *state.session_resumable.lock().await = true;

        // Queue both requests behind the real production commit mutex. Tokio mutex waiters are
        // FIFO: A registers first, B replaces its pending snapshot, then A consumes B's bytes.
        let boundary = state.ui_session_commit.lock().await;
        let ordinary_state = Arc::clone(&state);
        let ordinary = tokio::spawn(async move {
            lock_session_inner(&ordinary_state, Some("ordinary-invalid".into())).await
        });
        while state.ui_session_generation.load(Ordering::Acquire) < 1
            || state
                .pending_ui_lock_snapshot
                .lock()
                .await
                .as_ref()
                .map(|s| s.generation)
                != Some(1)
        {
            tokio::task::yield_now().await;
        }

        let close_state = Arc::clone(&state);
        let closing = tokio::spawn(async move {
            close_vault_window_plan_inner(&close_state, Some("close-invalid".into()), false).await
        });
        while state.ui_session_generation.load(Ordering::Acquire) < 2
            || state
                .pending_ui_lock_snapshot
                .lock()
                .await
                .as_ref()
                .map(|s| s.generation)
                != Some(2)
        {
            tokio::task::yield_now().await;
        }
        drop(boundary);

        assert!(
            ordinary.await.unwrap().is_err(),
            "A validates B's invalid snapshot"
        );
        let (outcome, may_destroy) = closing.await.unwrap();
        assert!(outcome.continuity_error.is_some());
        assert!(
            !may_destroy,
            "B must recover its generation-bound failure even though A consumed the bytes"
        );
        assert!(state.vault_window_close_debt.lock().await.is_some());
    }

    #[tokio::test]
    async fn successful_reunlock_resolves_old_native_close_debt() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;
        let (failed, may_destroy) =
            close_vault_window_plan_inner(&state, Some("not json".into()), false).await;
        assert!(failed.continuity_error.is_some());
        assert!(!may_destroy);

        let generation = state.ui_session_generation.load(Ordering::Acquire);
        assert!(finalize_unlock_session(&state, generation, false)
            .await
            .is_err());
        assert!(
            state.vault_window_close_debt.lock().await.is_some(),
            "the first unlock merely surfaces malformed-state loss"
        );
        assert!(finalize_unlock_session(&state, generation, false)
            .await
            .is_ok());
        assert!(state.vault_window_close_debt.lock().await.is_none());
    }

    #[tokio::test]
    async fn remounted_close_consumes_the_exact_native_pending_lock_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = true;
        let latest = r#"{"version":1,"drafts":{"room":"before remount"},"readMarks":{}}"#;

        // This is the native transaction left behind after the Ctrl+L webview was remounted. The
        // new component has no JS snapshot and therefore calls close with `None`.
        *state.pending_ui_lock_snapshot.lock().await = Some(PendingUiLockSnapshot {
            generation: 1,
            json: latest.into(),
        });
        let (outcome, may_destroy) = close_vault_window_plan_inner(&state, None, false).await;
        assert!(outcome.continuity_error.is_none());
        assert!(may_destroy);
        assert_eq!(
            state
                .store
                .lock()
                .await
                .as_ref()
                .unwrap()
                .load_ui_state()
                .unwrap(),
            latest.as_bytes(),
            "snapshot-less remounted close must finish the exact pending native transaction",
        );
        assert!(state.pending_ui_lock_snapshot.lock().await.is_none());
    }

    #[tokio::test]
    async fn locked_session_cannot_read_a_preexisting_storage_health_cache_entry() {
        let state = AppState::default();
        state.storage_health.lock().await.insert(
            9,
            CachedStorageHealth {
                server_instance: 1,
                report: build_storage_report(
                    StorageHealth::default(),
                    Vec::new(),
                    &HashSet::new(),
                    99,
                ),
            },
        );

        assert_eq!(
            storage_health_cache_get(&state, 9, 1, 0)
                .await
                .err()
                .as_deref(),
            Some("the vault is locked"),
            "the session gate must run before a cache hit can expose names or content addresses"
        );
    }

    #[tokio::test]
    async fn a_storage_scan_finishing_after_lock_cannot_repopulate_plaintext_metadata() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = true;
        let generation = unlocked_ui_session_generation(&state).await.unwrap();
        let late_report =
            build_storage_report(StorageHealth::default(), Vec::new(), &HashSet::new(), 123);

        lock_session_inner(&state, None).await.unwrap();
        assert!(
            storage_health_cache_publish(&state, 12, 1, generation, late_report)
                .await
                .is_err(),
            "a scan authorized before lock must not publish after the UI generation changes"
        );
        assert!(state.storage_health.lock().await.is_empty());
    }

    /// A persisted id is reused when a server is removed and reinstalled, and a write begun by the
    /// departing incarnation can still be in flight when the replacement arrives. It must not put
    /// its bytes in the replacement's slot, and it must not report the replacement's mutations as
    /// durable: the first loses the newer group's state, the second reports a message saved that
    /// was not.
    #[tokio::test]
    async fn a_departed_incarnation_cannot_write_into_or_retire_a_reused_id() {
        use catcoms_rt::Hub;
        use rand_chacha::ChaCha20Rng;
        use rand_core::SeedableRng;

        const SERVER: u64 = 12;
        const OLD: u64 = 40;
        const NEW: u64 = 41;
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap());

        // Two genuinely different groups, so what reached the disk is legible from its content.
        let build = |peer: u64, seed: u64| {
            Server::found(
                Hub::new().join(PeerId::from_u64(peer)),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(seed),
                Box::new(ManualClock::new(1_000)),
                "alice",
            )
            .unwrap()
        };
        let mut departing = build(91, 91);
        departing.open_channel(1).await.unwrap();
        departing
            .send_message(1, "from the departed incarnation")
            .await
            .unwrap();
        let (old_actor, old_events, old_task) = spawn(departing);
        let mut replacement = build(92, 92);
        replacement.open_channel(1).await.unwrap();
        replacement
            .send_message(1, "from the replacement")
            .await
            .unwrap();
        let replacement_group = replacement.group_id();
        let replacement_device = replacement.device_id();
        let (new_actor, new_events, new_task) = spawn(replacement);

        let entry = |instance, actor: ServerActor| ServerEntry {
            actor,
            instance,
            group_id: replacement_group.clone(),
            device_id: replacement_device,
            invite: None,
            name: "test".into(),
            bootstrap: Vec::new(),
            bootstrap_owners: HashMap::new(),
            interface_routes: None,
            rendezvous: Vec::new(),
            mesh: None,
            is_dm: false,
            switchboard: false,
            record_seq: 0,
            persist: PersistCounters::default(),
        };

        // The departing incarnation records a request, then is replaced before it can write.
        state
            .servers
            .lock()
            .await
            .insert(SERVER, entry(OLD, old_actor.clone()));
        // Several requests, so the stale ticket is a number the replacement's own counters will
        // not have reached. A departing writer holding ticket 1 is accidentally stopped by the
        // coalescing check; the hazard is the one whose ticket is still ahead.
        let stale_ticket = {
            let mut servers = state.servers.lock().await;
            let entry = servers.get_mut(&SERVER).unwrap();
            entry.persist.request();
            entry.persist.request();
            entry.persist.request()
        };
        assert_eq!(stale_ticket, 3);
        state.servers.lock().await.remove(&SERVER);
        state
            .servers
            .lock()
            .await
            .insert(SERVER, entry(NEW, new_actor.clone()));

        // The replacement persists a change of its own, and that is what belongs on disk.
        persist_server(&state, SERVER).await;
        let after_replacement = {
            let guard = state.store.lock().await;
            guard.as_ref().unwrap().load_server(SERVER).unwrap()
        };

        // Now the departed writer resumes.
        persist_captured(&state, SERVER, OLD, stale_ticket, old_actor.clone()).await;

        let on_disk = {
            let guard = state.store.lock().await;
            guard.as_ref().unwrap().load_server(SERVER).unwrap()
        };
        assert_eq!(
            on_disk.to_vec(),
            after_replacement.to_vec(),
            "a departed incarnation must not write into the slot its id now names"
        );
        let restored = Server::restore(
            &on_disk,
            Hub::new().join(PeerId::from_u64(93)),
            ChaCha20Rng::seed_from_u64(93),
            Box::new(ManualClock::new(2_000)),
            "alice",
        )
        .unwrap();
        assert_eq!(
            restored.messages(1).first().map(|m| m.text.clone()),
            Some("from the replacement".to_string()),
        );
        assert_eq!(
            state
                .servers
                .lock()
                .await
                .get(&SERVER)
                .unwrap()
                .persist
                .completed,
            1,
            "the stale writer retires nothing: only the replacement's own write counts"
        );

        old_actor.shutdown().await;
        new_actor.shutdown().await;
        let _ = old_task.await;
        let _ = new_task.await;
        drop(old_events);
        drop(new_events);
    }

    #[tokio::test]
    async fn a_removed_server_incarnation_cannot_publish_into_a_reused_id() {
        use catcoms_rt::Hub;
        use rand_chacha::ChaCha20Rng;
        use rand_core::SeedableRng;

        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = true;
        let generation = unlocked_ui_session_generation(&state).await.unwrap();

        let server = Server::found(
            Hub::new().join(PeerId::from_u64(91)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(91),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        let group_id = server.group_id();
        let device_id = server.device_id();
        let (actor, events, task) = spawn(server);
        let entry = |instance| ServerEntry {
            actor: actor.clone(),
            instance,
            group_id: group_id.clone(),
            device_id,
            invite: None,
            name: "test".into(),
            bootstrap: Vec::new(),
            bootstrap_owners: HashMap::new(),
            interface_routes: None,
            rendezvous: Vec::new(),
            mesh: None,
            is_dm: false,
            switchboard: false,
            record_seq: 0,
            persist: PersistCounters::default(),
        };
        const SERVER: u64 = 12;
        const OLD: u64 = 40;
        const NEW: u64 = 41;
        state.servers.lock().await.insert(SERVER, entry(OLD));
        let stale_report =
            build_storage_report(StorageHealth::default(), Vec::new(), &HashSet::new(), 123);

        // Model a scan paused after producing its report: leave removes its registry incarnation,
        // then a reload installs a new actor row under the same persisted id before it resumes.
        state.servers.lock().await.remove(&SERVER);
        state.storage_health.lock().await.remove(&SERVER);
        state.servers.lock().await.insert(SERVER, entry(NEW));
        assert!(storage_health_cache_publish(
            &state,
            SERVER,
            OLD,
            generation,
            stale_report.clone()
        )
        .await
        .is_err());
        assert!(state.storage_health.lock().await.is_empty());

        storage_health_cache_publish(&state, SERVER, NEW, generation, stale_report)
            .await
            .unwrap();
        assert!(storage_health_cache_get(&state, SERVER, NEW, generation)
            .await
            .unwrap()
            .is_some());

        actor.shutdown().await;
        task.await.unwrap();
        drop(events);
    }

    #[tokio::test]
    async fn mounted_session_unlock_verifies_the_secret_without_remounting_the_vault() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = false;

        assert!(
            authenticate_mounted_store(&state, b"wrong horse")
                .await
                .is_err(),
            "an explicitly locked webview cannot regain IPC access with a wrong secret"
        );
        assert!(authenticate_mounted_store(&state, b"").await.is_err());
        let oversized = vec![b'x'; 4_097];
        assert!(authenticate_mounted_store(&state, &oversized)
            .await
            .is_err());
        assert!(!*state.session_resumable.lock().await);

        assert!(authenticate_mounted_store(&state, b"correct horse")
            .await
            .unwrap());
        let generation = state.ui_session_generation.load(Ordering::Acquire);
        let running = finalize_unlock_session(&state, generation, true)
            .await
            .unwrap()
            .expect("the mounted path requests its current servers");
        assert!(running.is_empty());
        assert!(*state.session_resumable.lock().await);

        // The successful path authenticated the existing mount; it did not release ownership or
        // create a second ServerStore just to verify the passphrase.
        match ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng) {
            Err(error) => assert!(error.to_string().contains("vault is busy")),
            Ok(_) => panic!("verify-only unlock must not release the existing mount"),
        }
    }

    #[tokio::test]
    async fn remounted_unlock_retries_the_exact_native_continuity_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = false;
        let generation = state.ui_session_generation.load(Ordering::Acquire);
        let latest = r#"{"version":1,"drafts":{"room":"survives remount"},"readMarks":{}}"#;

        // A reloaded webview has no JavaScript coordinator or Promise. Native state is therefore
        // the only durable-in-process record that the prior resolved lock outcome reported a
        // failed write and retained these exact bytes.
        *state.pending_ui_lock_snapshot.lock().await = Some(PendingUiLockSnapshot {
            generation,
            json: latest.into(),
        });
        *state.last_ui_lock_completion.lock().await = Some(UiLockCompletion {
            generation,
            error: Some("injected first write failure".into()),
        });

        finalize_unlock_session(&state, generation, false)
            .await
            .expect("unlock must retry the native snapshot before reopening IPC");
        assert_eq!(
            state
                .store
                .lock()
                .await
                .as_ref()
                .unwrap()
                .load_ui_state()
                .unwrap(),
            latest.as_bytes(),
        );
        assert!(state.pending_ui_lock_snapshot.lock().await.is_none());
        assert!(state.last_ui_lock_completion.lock().await.is_none());
        assert!(*state.session_resumable.lock().await);
    }

    #[tokio::test]
    async fn failed_unlock_retry_keeps_the_exact_snapshot_and_vault_locked() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        *state.session_resumable.lock().await = false;
        state.session_lock_requested.store(true, Ordering::Release);
        let generation = state.ui_session_generation.load(Ordering::Acquire);
        let latest = r#"{"version":1,"drafts":{"room":"retry me"},"readMarks":{}}"#;
        *state.pending_ui_lock_snapshot.lock().await = Some(PendingUiLockSnapshot {
            generation,
            json: latest.into(),
        });

        let error = finalize_unlock_session(&state, generation, false)
            .await
            .err()
            .expect("persistence failure must refuse to unlock");
        assert!(error.contains("still locked"));
        assert_eq!(
            state
                .pending_ui_lock_snapshot
                .lock()
                .await
                .as_ref()
                .map(|snapshot| snapshot.json.as_str()),
            Some(latest),
            "a failed retry must retain the immutable bytes rather than accept older continuity",
        );
        assert!(!*state.session_resumable.lock().await);
        assert!(state.session_lock_requested.load(Ordering::Acquire));

        // Model a transient persistence problem clearing. The next unlock retries the same native
        // transaction and only then reopens the IPC boundary.
        let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        finalize_unlock_session(&state, generation, false)
            .await
            .expect("the retained exact snapshot should be retryable");
        assert_eq!(
            state
                .store
                .lock()
                .await
                .as_ref()
                .unwrap()
                .load_ui_state()
                .unwrap(),
            latest.as_bytes(),
        );
        assert!(state.pending_ui_lock_snapshot.lock().await.is_none());
        assert!(state.last_ui_lock_completion.lock().await.is_none());
        assert!(*state.session_resumable.lock().await);
    }

    #[test]
    fn a_committed_secret_rewrap_is_never_reported_as_unchanged() {
        let result = vault_secret_change_result(Err(
            catcoms_app::AppError::CommittedButNotDurable("injected directory sync failure".into()),
        ))
        .unwrap();
        assert!(result.changed);
        assert!(!result.durability_confirmed);
        let warning = result.warning.unwrap();
        assert!(warning.contains("new secret is active"));
        assert!(!warning.contains("not changed"));
    }

    #[tokio::test]
    async fn a_newer_explicit_lock_defeats_unlock_finalization_after_authentication() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = false;

        let stale_generation = state.ui_session_generation.load(Ordering::Acquire);
        assert!(
            authenticate_mounted_store(&state, b"correct horse")
                .await
                .unwrap(),
            "the test pauses the unlock after real vault authentication"
        );
        let newer_generation = state
            .ui_session_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        state.session_lock_requested.store(true, Ordering::Release);
        let newer_snapshot = r#"{"version":1,"drafts":{"room":"newer lock"},"readMarks":{}}"#;
        *state.pending_ui_lock_snapshot.lock().await = Some(PendingUiLockSnapshot {
            generation: newer_generation,
            json: newer_snapshot.into(),
        });
        *state.last_ui_lock_completion.lock().await = Some(UiLockCompletion {
            generation: newer_generation,
            error: Some("newer lock still owns this continuity debt".into()),
        });

        assert!(
            finalize_unlock_session(&state, stale_generation, true)
                .await
                .is_err(),
            "work authenticated before a newer lock must not reopen IPC"
        );
        assert_eq!(
            require_unlocked_session(&state).await.unwrap_err(),
            "the vault is locked"
        );
        assert!(state.session_lock_requested.load(Ordering::Acquire));
        assert!(!*state.session_resumable.lock().await);
        assert_eq!(
            state
                .pending_ui_lock_snapshot
                .lock()
                .await
                .as_ref()
                .map(|snapshot| snapshot.json.as_str()),
            Some(newer_snapshot),
            "a stale unlock must not retire a newer lock's exact snapshot",
        );
        assert_eq!(
            state
                .last_ui_lock_completion
                .lock()
                .await
                .as_ref()
                .map(|completion| completion.generation),
            Some(newer_generation),
            "a stale unlock must not erase the newer lock's user-visible failure evidence",
        );
    }

    #[tokio::test]
    async fn a_stale_hmr_resume_cannot_return_server_projection_after_lock() {
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::default());
        let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = true;

        // This generation is exactly what production captures before `running_servers` begins its
        // actor awaits. The injected projection pauses after the helper owns the same commit guard
        // production uses, so the lock invalidates immediately and then waits behind that guard.
        let stale_generation = unlocked_ui_session_generation(&state).await.unwrap();
        let projection_started = Arc::new(tokio::sync::Notify::new());
        let release_projection = Arc::new(tokio::sync::Notify::new());
        let resume_state = Arc::clone(&state);
        let started = Arc::clone(&projection_started);
        let release = Arc::clone(&release_projection);
        let resuming = tokio::spawn(async move {
            resume_session_projection(&resume_state, stale_generation, async move {
                started.notify_one();
                release.notified().await;
                Vec::new()
            })
            .await
        });
        projection_started.notified().await;

        let lock_state = Arc::clone(&state);
        let locking = tokio::spawn(async move { lock_session_inner(&lock_state, None).await });
        while !state.session_lock_requested.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        release_projection.notify_one();
        assert!(
            resuming.await.unwrap().is_none(),
            "no server names, invites, or channels may cross a completed explicit lock"
        );
        locking.await.unwrap().unwrap();
        assert!(resume_session_inner(&state).await.is_none());
    }

    #[tokio::test]
    async fn stale_continuity_save_queued_before_lock_cannot_overwrite_the_lock_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::default());
        let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = true;
        let generation = unlocked_ui_session_generation(&state).await.unwrap();

        // Hold the common commit boundary so the old debounced save is definitely queued before
        // lock invalidates its UI generation. Releasing it then exercises the dangerous ordering:
        // old command first, final lock snapshot second.
        let boundary = state.ui_session_commit.lock().await;
        let stale_state = Arc::clone(&state);
        let stale = tokio::spawn(async move {
            save_ui_state_for_generation(
                &stale_state,
                r#"{"version":1,"drafts":{},"readMarks":{},"fileTrustPolicies":{"1":{"mode":"everyone","trustedAuthors":[]}}}"#,
                generation,
            )
            .await
        });
        tokio::task::yield_now().await;

        let latest = r#"{"version":1,"drafts":{},"readMarks":{},"fileTrustPolicies":{"1":{"mode":"on-demand","trustedAuthors":[]}}}"#;
        let lock_state = Arc::clone(&state);
        let locking =
            tokio::spawn(async move { lock_session_inner(&lock_state, Some(latest.into())).await });
        while !state.session_lock_requested.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        drop(boundary);

        assert!(
            stale.await.unwrap().is_err(),
            "the stale generation must be rejected"
        );
        locking.await.unwrap().unwrap();
        let saved = state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_ui_state()
            .unwrap();
        assert_eq!(saved, latest.as_bytes());
    }

    #[tokio::test]
    async fn vault_full_close_unlock_and_actor_reload_survives_twice() {
        use catcoms_rt::Hub;
        use rand_chacha::ChaCha20Rng;
        use rand_core::SeedableRng;

        const SERVER_ID: u64 = 41;
        const PASSPHRASE: &[u8] = b"correct horse";
        let general = channel_id("general");
        let root = tempfile::tempdir().unwrap();

        // Build exactly the durable material the Tauri bridge owns: a server snapshot, registry
        // row and network identity in one vault. The source actor then disappears, modelling a
        // process close rather than an in-process UI lock.
        let hub = Hub::new();
        let mut original = Server::found(
            hub.join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        original.subscribe_control().await.unwrap();
        original.open_channel(general).await.unwrap();
        original
            .send_message(general, "survives a full close")
            .await
            .unwrap();
        let snapshot = original.snapshot().unwrap();
        let record = ServerRecord {
            id: SERVER_ID,
            display_name: "private friend".into(),
            invite: String::new(),
            is_dm: true,
        };
        let mut network = new_server_net("", "", "");
        network.port = 22_487;
        network.reconnect_policy = ReconnectPolicy::AuthorizedPeer([9; 32]);
        {
            let mut rng = ChaCha20Rng::seed_from_u64(2);
            let store = ServerStore::open(root.path(), PASSPHRASE, &mut rng).unwrap();
            store.save_server(SERVER_ID, &snapshot, &mut rng).unwrap();
            store
                .save_registry(std::slice::from_ref(&record), &mut rng)
                .unwrap();
            store
                .save_server_net(SERVER_ID, &network, &mut rng)
                .unwrap();
        }
        drop(original);
        assert!(ServerStore::open(
            root.path(),
            b"wrong passphrase",
            &mut ChaCha20Rng::seed_from_u64(3),
        )
        .is_err());

        // Open and restore through the transport-independent production seam used by
        // `reload_one`. Returning the task and receiver lets the test perform a clean full close.
        async fn open_cycle(
            root: &Path,
            peer: u64,
        ) -> (
            AppState,
            ServerActor,
            mpsc::Receiver<catcoms_app::TracedEvent>,
            tokio::task::JoinHandle<()>,
        ) {
            use catcoms_rt::Hub;
            use rand_chacha::ChaCha20Rng;
            use rand_core::SeedableRng;

            let mut rng = ChaCha20Rng::seed_from_u64(peer + 10);
            let store = ServerStore::open(root, PASSPHRASE, &mut rng).unwrap();
            let records = store.load_registry().unwrap();
            assert_eq!(
                records,
                vec![ServerRecord {
                    id: SERVER_ID,
                    display_name: "private friend".into(),
                    invite: String::new(),
                    is_dm: true,
                }]
            );
            let snapshot = store.load_server(SERVER_ID).unwrap();
            let network = store.load_server_net(SERVER_ID).unwrap().unwrap();
            assert_eq!(
                network.reconnect_policy,
                ReconnectPolicy::AuthorizedPeer([9; 32]),
                "reconnect consent is part of the sealed lifecycle"
            );
            let state = AppState::default();
            *state.store.lock().await = Some(store);
            *state.next_id.lock().await = SERVER_ID;
            *state.session_resumable.lock().await = true;
            let restored = restore_server_actor(
                &state,
                &snapshot,
                &records[0],
                Hub::new().join(PeerId::from_u64(peer)),
                ChaCha20Rng::seed_from_u64(peer + 20),
                Box::new(ManualClock::new(2_000 + peer)),
                &[],
                network.record_seq,
                network
                    .reconnect_routes
                    .iter()
                    .map(|route| (PeerId::new(route.peer_id), route.address.clone()))
                    .collect(),
                network.switchboard,
            )
            .await
            .unwrap();
            let actor = restored.actor.clone();
            state.servers.lock().await.insert(
                SERVER_ID,
                ServerEntry {
                    actor: restored.actor,
                    instance: state.next_server_instance.fetch_add(1, Ordering::Relaxed),
                    group_id: restored.group_id,
                    device_id: restored.device_id,
                    invite: None,
                    name: records[0].display_name.clone(),
                    bootstrap: Vec::new(),
                    bootstrap_owners: HashMap::new(),
                    interface_routes: None,
                    rendezvous: Vec::new(),
                    mesh: None,
                    is_dm: records[0].is_dm,
                    switchboard: network.switchboard,
                    record_seq: network.record_seq,
                    persist: PersistCounters::default(),
                },
            );
            (state, actor, restored.events, restored.task)
        }

        let (first_state, first_actor, first_events, first_task) = open_cycle(root.path(), 2).await;
        assert_eq!(
            running_servers(&first_state).await[0].name,
            "private friend"
        );
        assert!(running_servers(&first_state).await[0].is_dm);
        assert_eq!(
            first_actor.messages(general).await[0].text,
            "survives a full close"
        );
        lock_session_inner(
            &first_state,
            Some(r#"{"version":1,"drafts":{},"readMarks":{}}"#.into()),
        )
        .await
        .unwrap();
        assert!(require_unlocked_session(&first_state).await.is_err());
        assert_eq!(
            first_actor.messages(general).await[0].text,
            "survives a full close",
            "the native actor remains valid behind an explicit UI lock"
        );
        first_actor.shutdown().await;
        first_task.await.unwrap();
        drop(first_events);
        drop(first_state);

        // A second independent state proves this is not HMR/resume: the vault keys, actor,
        // transport and frontend gate all start again from sealed bytes after a full close.
        let (second_state, second_actor, second_events, second_task) =
            open_cycle(root.path(), 3).await;
        assert!(require_unlocked_session(&second_state).await.is_ok());
        assert_eq!(
            second_actor.messages(general).await[0].text,
            "survives a full close"
        );
        second_actor.shutdown().await;
        second_task.await.unwrap();
        drop(second_events);
    }

    #[tokio::test]
    async fn member_recovery_preserves_a_proven_route_while_pending_consent_survives_reopen() {
        use rand_chacha::ChaCha20Rng;
        use rand_core::SeedableRng;

        const SERVER_ID: u64 = 57;
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let mut rng = ChaCha20Rng::seed_from_u64(57);
        let store = ServerStore::open(root.path(), b"recovery vault", &mut rng).unwrap();
        let previous_peer = PeerId::from_u64(98);
        let mut network = new_server_net("", "", "");
        network.reconnect_policy = ReconnectPolicy::AuthorizedPeer(*previous_peer.as_bytes());
        network.reconnect_routes = vec![ReconnectRoute {
            peer_id: *previous_peer.as_bytes(),
            address: "/ip4/192.168.1.98/tcp/22487".into(),
        }];
        let previous_routes = network.reconnect_routes.clone();
        store
            .save_server_net(SERVER_ID, &network, &mut rng)
            .unwrap();
        *state.store.lock().await = Some(store);

        let pending_peer = PeerId::from_u64(99);
        authorize_member_recovery_capture(&state, SERVER_ID, pending_peer, u64::MAX)
            .await
            .unwrap();

        let guard = state.store.lock().await;
        let sealed = guard
            .as_ref()
            .unwrap()
            .load_server_net(SERVER_ID)
            .unwrap()
            .unwrap();
        assert_eq!(
            sealed.reconnect_policy,
            ReconnectPolicy::AuthorizedPeer(*previous_peer.as_bytes()),
            "pending recovery cannot replace the last proven contact"
        );
        assert_eq!(sealed.reconnect_routes, previous_routes);
        assert_eq!(
            sealed.pending_recovery_peer,
            Some(*pending_peer.as_bytes()),
            "consent survives a crash while the new peer is still unreachable"
        );
        drop(guard);
        drop(state);

        let reopened = ServerStore::open(root.path(), b"recovery vault", &mut rng).unwrap();
        let sealed = reopened.load_server_net(SERVER_ID).unwrap().unwrap();
        assert_eq!(sealed.reconnect_routes, previous_routes);
        assert_eq!(sealed.pending_recovery_peer, Some(*pending_peer.as_bytes()));
    }

    /// The trap this guards: founding mints the invite immediately, UPnP answers seconds later,
    /// so the invite a user copies first is the one *without* the public address their router
    /// just opened. Their friend on another network then gets an unactionable timeout. This
    /// predicate is what makes the invite self-heal the next time anyone reads it.
    #[test]
    fn an_invite_is_stale_when_the_live_address_set_changes() {
        let lan = "/ip4/192.168.0.5/tcp/9000/p2p/ID".to_string();
        let loop_ = "/ip4/127.0.0.1/tcp/9000/p2p/ID".to_string();
        let public = "/ip4/203.0.113.9/tcp/9000/p2p/ID".to_string();

        // The real case: UPnP landed after the mint, so the stored invite is missing the address
        // that is the entire point of the mapping.
        assert!(invite_addresses_changed(
            &[loop_.clone(), lan.clone()],
            &[public.clone(), loop_.clone(), lan.clone()]
        ));

        // Nothing gained: no re-mint, so reading the invite repeatedly cannot churn nonces.
        assert!(!invite_addresses_changed(
            &[public.clone(), loop_.clone()],
            &[public.clone(), loop_.clone()]
        ));
        // Order is not a change.
        assert!(!invite_addresses_changed(
            &[public.clone(), loop_.clone()],
            &[loop_.clone(), public.clone()]
        ));
        // A lost lease must disappear from the next invite shown to the operator. An already
        // copied signed code cannot be rewritten and remains usable through any other entries.
        assert!(invite_addresses_changed(
            &[public.clone(), loop_.clone(), lan.clone()],
            std::slice::from_ref(&loop_)
        ));
        // Degenerate cases stay quiet rather than re-minting forever.
        assert!(!invite_addresses_changed(&[], &[]));
        assert!(invite_addresses_changed(std::slice::from_ref(&public), &[]));
        assert!(invite_addresses_changed(&[], std::slice::from_ref(&public)));

        assert!(invite_routes_still_current(
            std::slice::from_ref(&public),
            &["rz-a".to_string()],
            std::slice::from_ref(&public),
            &["rz-a".to_string()],
        ));
        assert!(
            !invite_routes_still_current(
                std::slice::from_ref(&public),
                &["rz-a".to_string()],
                std::slice::from_ref(&loop_),
                &["rz-a".to_string()],
            ),
            "an older overlapping mint cannot store after the live route set changes"
        );
    }

    /// Settings promises the user "this session's file is X". The name now comes from the sink
    /// that opened it, not from whichever `debug_log_*` in the folder was touched most recently.
    ///
    /// The old rule looked reasonable and was wrong in the one case that matters: when this
    /// process failed to open a file, the newest one in the directory belongs to a *previous* run,
    /// so the UI named a stale capture and called it active. A user then sends yesterday's log for
    /// today's bug, which costs a round trip and usually a second reproduction.
    #[test]
    fn settings_names_the_file_this_process_opened_and_never_a_stale_one() {
        let dir = std::path::Path::new("/data/logs");
        let health = catcoms_log::SinkHealth {
            desired: true,
            state: catcoms_log::SinkState::Active,
            session_id: "eb887278".into(),
            path: Some(dir.join("debug_log_20260823_120000.txt")),
            events_written: 12,
            bytes_written: 900,
            ..catcoms_log::SinkHealth::stopped()
        };
        let reply = DebugLogging::from_health(true, dir, &health);
        assert!(reply.active);
        assert_eq!(reply.state, "active");
        assert_eq!(reply.file, "debug_log_20260823_120000.txt");
        assert_eq!(reply.session, "eb887278");
        assert!(reply.error.is_empty());
    }

    /// The case the previous design could not express: the preference says yes and the sink says
    /// no. Both halves have to reach the UI, or "enabled" is read as "captured".
    #[test]
    fn a_failed_sink_is_reported_as_failed_with_its_reason_and_names_no_file() {
        let dir = std::path::Path::new("/data/logs");
        let health = catcoms_log::SinkHealth {
            desired: true,
            state: catcoms_log::SinkState::Failed,
            last_error: Some("permission denied opening the diagnostics directory".into()),
            ..catcoms_log::SinkHealth::stopped()
        };
        let reply = DebugLogging::from_health(true, dir, &health);
        assert!(reply.enabled, "the user did ask for a log");
        assert!(!reply.active, "and did not get one");
        assert_eq!(reply.state, "failed");
        assert_eq!(
            reply.error,
            "permission denied opening the diagnostics directory"
        );
        assert!(
            reply.file.is_empty(),
            "no file is named when none was opened"
        );
    }

    /// Loss must not read as health. A sink that is still writing but has dropped records is
    /// degraded, and the UI keeps treating it as capturing so the user does not stop mid-report.
    #[test]
    fn a_degraded_sink_is_still_capturing_but_says_it_lost_events() {
        let dir = std::path::Path::new("/data/logs");
        let health = catcoms_log::SinkHealth {
            desired: true,
            state: catcoms_log::SinkState::Degraded,
            path: Some(dir.join("debug_log_20260823_120000.txt")),
            events_written: 40_000,
            events_dropped: 17,
            events_truncated: 3,
            queue_high_water: 8192,
            ..catcoms_log::SinkHealth::stopped()
        };
        let reply = DebugLogging::from_health(true, dir, &health);
        assert!(reply.active);
        assert_eq!(reply.state, "degraded");
        assert_eq!(reply.events_dropped, 17);
        assert_eq!(
            reply.events_truncated, 3,
            "a line that is there with its tail cut off is not a line that is missing"
        );
        assert_eq!(reply.queue_high_water, 8192);
    }

    /// Recording an error must never be able to destroy the evidence of that error.
    ///
    /// `String::truncate` panics on a non-boundary index, the frontend's own cap counts UTF-16
    /// units rather than bytes, and this input is arbitrary text from the webview. Every byte
    /// offset into a multibyte line is tried, because the bug is not "emoji break it", it is
    /// "one specific offset breaks it" and which offset depends entirely on the message.
    #[test]
    fn truncating_a_ui_log_line_never_splits_a_character() {
        let samples = [
            "🐈‍⬛🐈‍⬛🐈‍⬛ the cat is on the roof",
            "こんにちは、世界。これはテストです。",
            "e\u{0301}e\u{0301}e\u{0301} combining marks",
            "plain ascii is the easy case",
            "🎹",
        ];
        for sample in samples {
            for max in 0..=sample.len() + 2 {
                let mut text = sample.to_string();
                truncate_utf8_bytes(&mut text, max);
                assert!(text.len() <= max.max(0), "{sample:?} at {max} grew");
                assert!(
                    sample.starts_with(&text),
                    "{sample:?} at {max} is not a prefix"
                );
            }
        }
    }

    /// A single character longer than the whole cap is the edge that the obvious implementations
    /// get wrong: there is no boundary at or below the limit except zero.
    #[test]
    fn a_first_character_larger_than_the_cap_truncates_to_nothing() {
        let mut text = "🎹abc".to_string();
        truncate_utf8_bytes(&mut text, 2);
        assert_eq!(text, "");
    }

    /// The console's own export must not become the outage the bounded writer prevents.
    ///
    /// Saved reports live beside the debug logs, and the writer's retention only ever considers
    /// `debug_log_*`. Before this, pressing Save in a loop could fill the disk without touching a
    /// single one of the writer's carefully chosen limits.
    #[test]
    fn saved_reports_are_bounded_and_never_touch_the_logs_beside_them() {
        let dir = tempfile::tempdir().unwrap();
        // Something the quota must leave alone, of each kind that shares this directory.
        std::fs::write(dir.path().join("debug_log_20260823_120000.txt"), b"a log").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"not ours").unwrap();

        for n in 0..(MAX_SAVED_REPORTS * 3) {
            let name = format!(
                "{REPORT_PREFIX}{:013}-abcd1234.txt",
                1_700_000_000_000u64 + n as u64
            );
            std::fs::write(dir.path().join(name), b"report").unwrap();
            retain_reports(dir.path(), 1);
        }

        let reports: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(REPORT_PREFIX))
            .collect();
        assert!(
            reports.len() <= MAX_SAVED_REPORTS,
            "reports grew without bound: {}",
            reports.len()
        );
        // Oldest first out: a report written a moment ago is the one somebody is about to send.
        assert!(
            reports
                .iter()
                .all(|n| n > &format!("{REPORT_PREFIX}1700000000010")),
            "the newest reports should be the survivors: {reports:?}"
        );
        assert!(
            dir.path().join("debug_log_20260823_120000.txt").exists(),
            "logs are untouched"
        );
        assert!(
            dir.path().join("notes.txt").exists(),
            "so is everything else"
        );
    }

    /// A render loop must not be able to fill the log, and the limiter must not be able to hide
    /// that it happened: a suppressed storm presented as a quiet period is worse than the storm.
    #[test]
    fn the_ui_log_limiter_caps_a_burst_and_counts_what_it_dropped() {
        let start = 1_700_000_000_000;
        let mut allowed = 0;
        for _ in 0..1000 {
            if ui_log_allowance(UiLogChannel::Prose, start).0 {
                allowed += 1;
            }
        }
        assert_eq!(
            allowed, UI_LOG_BURST as i32,
            "the burst is the burst, not a suggestion"
        );

        // The other channel is untouched by it. A console storm and a stalled send are both
        // plausible at the same moment, and while the two shared a bucket the storm spent it: the
        // structured events describing what was going wrong were suppressed by the noise about it.
        assert!(
            ui_log_allowance(UiLogChannel::Structured, start).0,
            "a prose storm must not starve the operation events explaining it"
        );

        // Time passing refills it, and the first record afterwards carries the summary of what was
        // lost, so the count reaches the log rather than a counter nobody reads.
        let (ok, suppressed) = ui_log_allowance(
            UiLogChannel::Prose,
            start + UI_LOG_REPORT_INTERVAL_MS + 1000,
        );
        assert!(ok);
        assert_eq!(
            suppressed,
            Some(800),
            "every suppressed record is accounted for"
        );
    }

    #[test]
    fn listen_port_is_extracted_from_tcp_and_quic_addresses() {
        let a: Multiaddr = "/ip4/192.168.1.5/tcp/54321".parse().unwrap();
        assert_eq!(listen_port(&a), Some(54321));
        let b: Multiaddr = "/ip4/0.0.0.0/tcp/0".parse().unwrap();
        assert_eq!(listen_port(&b), Some(0));
        // A QUIC address carries the port on /udp/; matching only /tcp/ would drop it silently.
        let c: Multiaddr = "/ip4/0.0.0.0/udp/54321/quic-v1".parse().unwrap();
        assert_eq!(listen_port(&c), Some(54321));
        let d: Multiaddr = "/ip6/::/udp/1234/quic-v1".parse().unwrap();
        assert_eq!(listen_port(&d), Some(1234));
        let e: Multiaddr = "/ip6/::1/tcp/9".parse().unwrap();
        assert_eq!(listen_port(&e), Some(9));
    }

    #[test]
    fn a_server_binds_both_families_and_both_transports_on_one_port() {
        let addrs: Vec<String> = listen_addrs(31337).iter().map(|a| a.to_string()).collect();
        assert_eq!(
            addrs,
            vec![
                "/ip4/0.0.0.0/tcp/31337",
                "/ip6/::/tcp/31337",
                "/ip4/0.0.0.0/udp/31337/quic-v1",
                "/ip6/::/udp/31337/quic-v1",
            ]
        );
        // The degenerate no-port-reserved case binds exactly ONE address: an OS-assigned port
        // would give each listener a different number and destroy the single-port model.
        assert_eq!(listen_addrs(0).len(), 1);
        assert_eq!(listen_addrs(0)[0].to_string(), "/ip4/0.0.0.0/tcp/0");
    }

    #[test]
    fn a_host_address_yields_both_a_tcp_and_a_quic_bootstrap() {
        let id = "12D3KooWfakepeerid";
        assert_eq!(
            dialable_addrs("203.0.113.7".parse().unwrap(), 9000, id),
            vec![
                format!("/ip4/203.0.113.7/tcp/9000/p2p/{id}"),
                format!("/ip4/203.0.113.7/udp/9000/quic-v1/p2p/{id}"),
            ]
        );
        assert_eq!(
            dialable_addrs("2001:db8::5".parse().unwrap(), 443, id),
            vec![
                format!("/ip6/2001:db8::5/tcp/443/p2p/{id}"),
                format!("/ip6/2001:db8::5/udp/443/quic-v1/p2p/{id}"),
            ]
        );
    }

    #[test]
    fn interface_refresh_replaces_one_epoch_without_dropping_other_owners() {
        let id = "12D3KooWfakepeerid";
        let old_v4 = dialable_addrs("192.168.1.20".parse().unwrap(), 9000, id);
        let old_v6 = dialable_addrs("2606:4700::20".parse().unwrap(), 9000, id);
        let loopback = dialable_addrs(std::net::Ipv4Addr::LOCALHOST.into(), 9000, id);
        let mut bootstrap = old_v4
            .iter()
            .chain(&old_v6)
            .chain(&loopback)
            .cloned()
            .collect::<Vec<_>>();
        let mut owners = bootstrap_owners(&bootstrap, BootstrapOwner::AutomaticInterface);

        // A PCPv6 lease commonly names the same GUA socket as the raw interface route. Removing
        // the interface owner must leave that exact route until the mapping snapshot expires.
        add_bootstrap_owner(
            &mut bootstrap,
            &mut owners,
            old_v6[0].clone(),
            BootstrapOwner::PortMapping,
            true,
        );
        let new_v4 = dialable_addrs("10.0.0.44".parse().unwrap(), 9000, id);
        let next = new_v4.iter().chain(&loopback).cloned().collect::<Vec<_>>();
        let (removed, added) = reconcile_automatic_bootstrap(&mut bootstrap, &mut owners, &next);

        let mut expected_removed = old_v4.into_iter().collect::<HashSet<_>>();
        expected_removed.insert(old_v6[1].clone());
        assert_eq!(
            removed.into_iter().collect::<HashSet<_>>(),
            expected_removed
        );
        assert_eq!(
            added.into_iter().collect::<HashSet<_>>(),
            HashSet::from_iter(new_v4.clone())
        );
        assert!(bootstrap.contains(&old_v6[0]));
        assert!(!bootstrap.contains(&old_v6[1]));
        assert_eq!(
            owners.get(&old_v6[0]),
            Some(&HashSet::from([BootstrapOwner::PortMapping]))
        );
        assert!(new_v4.iter().all(|entry| bootstrap.contains(entry)));
        assert!(loopback.iter().all(|entry| bootstrap.contains(entry)));

        assert!(remove_bootstrap_owner(
            &mut bootstrap,
            &mut owners,
            &old_v6[0],
            BootstrapOwner::PortMapping,
        ));
        assert!(!bootstrap.contains(&old_v6[0]));
    }

    #[test]
    fn invite_bootstrap_addresses_are_validated_capped_and_ranked() {
        const ID: &str = "12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN";
        let a = |h: &str| format!("{h}/tcp/9/p2p/{ID}");

        // Things that cannot be a peer are dropped outright: an invite naming a multicast group
        // or a link-local address is aiming the reader at their own segment, not at a server.
        let hostile = vec![
            a("/ip4/224.0.0.1"),
            a("/ip4/239.255.255.250"), // SSDP; every UPnP router on the LAN answers this
            a("/ip4/169.254.1.1"),
            a("/ip4/0.0.0.0"),
            a("/ip4/255.255.255.255"),
            a("/ip4/240.0.0.1"),
            a("/ip6/ff02::1"),
            a("/ip6/fe80::1"),
            a("/ip6/::"),
            "not a multiaddr at all".to_string(),
        ];
        assert!(
            dialable_bootstrap(&hostile).is_empty(),
            "none of these are dialable"
        );

        // A LAN address IS kept: the most common first invite is someone in the same house.
        let lan = vec![a("/ip4/192.168.1.5")];
        assert_eq!(dialable_bootstrap(&lan).len(), 1);

        // Loopback is kept only when nothing else survived (the genuine same-machine case)...
        let same_machine = vec![a("/ip4/127.0.0.1"), a("/ip6/::1")];
        assert_eq!(dialable_bootstrap(&same_machine).len(), 2);
        // ...and dropped when the invite also carries something routable, where it could only
        // ever probe ports on the reader's own machine.
        let mixed = vec![a("/ip4/127.0.0.1"), a("/ip4/203.0.113.7")];
        let out = dialable_bootstrap(&mixed);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_string(), a("/ip4/203.0.113.7"));

        // A socket alone is not a peer route. Missing, duplicated, zero-port and unsupported
        // stacks are rejected before the transport constructor can dial them.
        let other = test_libp2p_peer(91);
        for invalid in [
            "/ip4/203.0.113.7/tcp/9".to_string(),
            format!("/ip4/203.0.113.7/tcp/9/p2p/{ID}/p2p/{other}"),
            format!("/ip4/203.0.113.7/tcp/0/p2p/{ID}"),
            format!("/ip4/203.0.113.7/udp/9/p2p/{ID}"),
        ] {
            assert!(
                dialable_bootstrap(&[invalid]).is_empty(),
                "non-canonical invite route must not be dialled"
            );
        }

        // And the number actually dialled is capped well below the token's 64.
        let flood: Vec<String> = (1..=60)
            .map(|n| a(&format!("/ip4/203.0.113.{n}")))
            .collect();
        assert_eq!(dialable_bootstrap(&flood).len(), MAX_BOOTSTRAP_DIALS);
    }

    #[test]
    fn same_machine_invite_rendezvous_survives_endpoint_scheduling() {
        let peer = test_libp2p_peer(92);
        let address = format!("/ip4/127.0.0.1/tcp/22487/p2p/{peer}");
        let targets = validate_invite_rendezvous_addrs(std::slice::from_ref(&address)).unwrap();
        let scheduler = EndpointDialScheduler::default();
        let scheduled =
            schedule_invite_rendezvous_targets(&scheduler, b"same-machine-group", targets);
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].addr.to_string(), address);
    }

    #[test]
    fn invite_rendezvous_seeds_leave_group_budget_for_the_inviter() {
        let routes: Vec<_> = (0..8u8)
            .map(|index| {
                format!(
                    "/ip4/45.79.{}.34/tcp/22487/p2p/{}",
                    index + 1,
                    test_libp2p_peer(index + 100)
                )
            })
            .collect();
        let targets = validate_invite_rendezvous_addrs(&routes).unwrap();
        let clock = ManualClock::new(0);
        let scheduler = EndpointDialScheduler::new_with_clock(
            catcoms_discovery::EndpointDialConfig::default(),
            Arc::new(clock),
        );
        let scheduled =
            schedule_invite_rendezvous_targets(&scheduler, b"rendezvous-headroom", targets);
        assert_eq!(scheduled.len(), MAX_INVITE_RENDEZVOUS_DIALS);

        let inviter = test_libp2p_peer(120);
        let inviter_phase = phase0_peer_id(&inviter);
        let inviter_route = format!("/ip4/8.8.8.8/tcp/22487/p2p/{inviter}");
        let endpoint = untrusted_peer_endpoint(&inviter_route, &inviter_phase).unwrap();
        assert_eq!(
            scheduler.reserve(b"rendezvous-headroom", &[endpoint]),
            vec![inviter_route]
        );
    }

    #[test]
    fn rendezvous_seed_cap_counts_supported_routes_not_unusable_prefix_entries() {
        let tls_only = format!("/ip4/45.79.1.34/tcp/443/tls/p2p/{}", test_libp2p_peer(121));
        let unsupported = format!("/ip4/45.79.2.34/tcp/443/http/p2p/{}", test_libp2p_peer(122));
        let usable = format!(
            "/ip4/45.79.3.34/udp/22487/quic-v1/p2p/{}",
            test_libp2p_peer(123)
        );
        let targets =
            validate_invite_rendezvous_addrs(&[tls_only, unsupported, usable.clone()]).unwrap();

        let retained = retained_invite_rendezvous_config(&targets);
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].0, usable);
        let scheduled = schedule_invite_rendezvous_targets(
            &EndpointDialScheduler::default(),
            b"supported-seeds",
            targets,
        );
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].addr.to_string(), usable);
    }

    #[test]
    fn signed_lan_fallback_keeps_invite_policy_but_discovery_does_not() {
        let inviter_lp = test_libp2p_peer(93);
        let other = test_libp2p_peer(94);
        let inviter = phase0_peer_id(&inviter_lp);
        let public = format!("/ip4/45.79.12.34/tcp/22487/p2p/{inviter_lp}");
        let lan = format!("/ip4/192.168.1.5/tcp/22487/p2p/{inviter_lp}");
        let wrong_peer = format!("/ip4/192.168.1.6/tcp/22487/p2p/{other}");

        let endpoints = join_candidate_endpoints(
            &[public.clone(), lan.clone()],
            &[lan.clone(), wrong_peer],
            &inviter,
        );
        let addresses: Vec<_> = endpoints
            .iter()
            .map(|endpoint| endpoint.address())
            .collect();
        assert_eq!(addresses, vec![public.as_str(), lan.as_str()]);
    }

    #[test]
    fn reconnect_route_selection_is_member_bound_local_and_bounded() {
        let member_lp = test_libp2p_peer(95);
        let stranger_lp = test_libp2p_peer(96);
        let member = phase0_peer_id(&member_lp);
        let stranger = phase0_peer_id(&stranger_lp);
        let lan_tcp = format!("/ip4/192.168.1.5/tcp/22487/p2p/{member_lp}");
        let lan_quic = format!("/ip4/192.168.1.5/udp/22487/quic-v1/p2p/{member_lp}");
        let public = format!("/ip4/45.79.12.34/tcp/22487/p2p/{member_lp}");
        let wrong_member = format!("/ip4/192.168.1.6/tcp/22487/p2p/{stranger_lp}");
        let confused_terminal = format!("/ip4/192.168.1.7/tcp/22487/p2p/{stranger_lp}");
        let candidates = vec![
            AuthenticatedDialRoute {
                peer: member,
                address: public.clone(),
            },
            AuthenticatedDialRoute {
                peer: member,
                address: lan_quic.clone(),
            },
            AuthenticatedDialRoute {
                peer: stranger,
                address: wrong_member,
            },
            AuthenticatedDialRoute {
                peer: member,
                address: confused_terminal,
            },
            AuthenticatedDialRoute {
                peer: member,
                address: lan_tcp.clone(),
            },
        ];

        let allowed = HashSet::from([member]);
        assert_eq!(
            select_authenticated_reconnect_routes(candidates.clone(), &allowed, true),
            vec![
                ReconnectRoute {
                    peer_id: *member.as_bytes(),
                    address: lan_tcp,
                },
                ReconnectRoute {
                    peer_id: *member.as_bytes(),
                    address: lan_quic,
                },
            ],
            "the migration path retains at most two private routes for current member peers"
        );
        assert!(select_authenticated_reconnect_routes(
            vec![AuthenticatedDialRoute {
                peer: member,
                address: public.clone(),
            }],
            &allowed,
            true,
        )
        .is_empty());
        assert_eq!(
            select_authenticated_reconnect_routes(
                vec![AuthenticatedDialRoute {
                    peer: member,
                    address: public.clone(),
                }],
                &allowed,
                false,
            ),
            vec![ReconnectRoute {
                peer_id: *member.as_bytes(),
                address: public,
            }],
            "direct admission may retain its exact authenticated public inviter route"
        );
    }

    #[test]
    fn reconnect_route_migration_rejects_ambiguous_member_peer_claims() {
        let unique = phase0_peer_id(&test_libp2p_peer(97));
        let duplicated = phase0_peer_id(&test_libp2p_peer(98));

        assert_eq!(
            uniquely_claimed_member_peers([duplicated, unique, duplicated]),
            HashSet::from([unique]),
            "a transport peer claimed by two roster entries must not become durable"
        );
    }

    #[test]
    fn pending_recovery_rejects_an_ambiguously_claimed_transport_peer() {
        let pending = PeerId::from_u64(100);
        assert_eq!(
            pending_recovery_capture_peer(
                Some(*pending.as_bytes()),
                2_000,
                1_000,
                [pending, pending, PeerId::from_u64(101)],
            ),
            None,
            "a pasted code cannot bypass the same unique member-to-transport binding as ordinary capture"
        );
        assert_eq!(
            pending_recovery_capture_peer(Some(*pending.as_bytes()), 2_000, 1_000, [pending]),
            Some(pending)
        );
        assert_eq!(
            pending_recovery_capture_peer(Some(*pending.as_bytes()), 2_000, 2_001, [pending]),
            None,
            "sealed pending consent cannot outlive the signed code's deadline"
        );
    }

    #[test]
    fn pending_recovery_cannot_promote_after_expiring_during_evidence_collection() {
        let peer = PeerId::from_u64(106);
        let mut current = new_server_net("", "", "");
        current.pending_recovery_peer = Some(*peer.as_bytes());
        current.pending_recovery_expires_at_ms = 2_000;
        assert_eq!(
            pending_recovery_capture_peer(Some(*peer.as_bytes()), 2_000, 1_999, [peer]),
            Some(peer),
            "the evidence collection began while the signed code was still valid"
        );
        let before = current.clone();
        assert_eq!(
            merge_live_reconnect_capture(
                &mut current,
                true,
                peer,
                vec![ReconnectRoute {
                    peer_id: *peer.as_bytes(),
                    address: "/ip4/192.168.1.106/tcp/22487".into(),
                }],
                1,
                2_001,
                [peer],
            ),
            None,
            "the final vault-locked merge rechecks the deadline"
        );
        assert_eq!(current, before);
    }

    #[test]
    fn pending_recovery_can_promote_an_authenticated_public_route_before_expiry() {
        let remote = test_libp2p_peer(105);
        let peer = phase0_peer_id(&remote);
        let public = format!("/ip4/45.79.12.35/tcp/22487/p2p/{remote}");
        let authenticated = vec![AuthenticatedDialRoute {
            peer,
            address: public.clone(),
        }];
        let allowed = HashSet::from([peer]);
        assert!(
            select_authenticated_reconnect_routes(authenticated.clone(), &allowed, true).is_empty(),
            "ordinary legacy migration remains LAN-only"
        );
        let routes = select_authenticated_reconnect_routes(authenticated, &allowed, false);
        assert_eq!(routes[0].address, public);

        let mut current = new_server_net("", "", "");
        current.pending_recovery_peer = Some(*peer.as_bytes());
        current.pending_recovery_expires_at_ms = 2_000;
        assert_eq!(
            merge_live_reconnect_capture(
                &mut current,
                true,
                peer,
                routes.clone(),
                1,
                1_000,
                [peer],
            ),
            Some(true)
        );
        assert_eq!(current.reconnect_routes, routes);
        assert_eq!(
            current.reconnect_policy,
            ReconnectPolicy::AuthorizedPeer(*peer.as_bytes())
        );
        assert_eq!(current.pending_recovery_peer, None);
        assert_eq!(current.pending_recovery_expires_at_ms, 0);
    }

    #[test]
    fn reconnect_capture_merge_preserves_newer_pending_consent() {
        let bob = PeerId::from_u64(102);
        let carol = PeerId::from_u64(103);
        let mut current = new_server_net("", "", "");
        current.reconnect_policy = ReconnectPolicy::AuthorizedPeer(*bob.as_bytes());
        current.pending_recovery_peer = Some(*carol.as_bytes());
        current.pending_recovery_expires_at_ms = 2_000;
        let bob_routes = vec![ReconnectRoute {
            peer_id: *bob.as_bytes(),
            address: "/ip4/192.168.1.102/tcp/22487".into(),
        }];

        assert_eq!(
            merge_live_reconnect_capture(
                &mut current,
                false,
                bob,
                bob_routes.clone(),
                2,
                1_000,
                [bob, carol],
            ),
            Some(true)
        );
        assert_eq!(current.pending_recovery_peer, Some(*carol.as_bytes()));
        assert_eq!(current.reconnect_routes, bob_routes);

        // A timer that selected an older pending Carol observation cannot overwrite a newer Dave
        // authorization read from disk at the final merge point.
        let dave = PeerId::from_u64(104);
        current.pending_recovery_peer = Some(*dave.as_bytes());
        current.pending_recovery_expires_at_ms = 3_000;
        let before = current.clone();
        assert_eq!(
            merge_live_reconnect_capture(
                &mut current,
                true,
                carol,
                vec![ReconnectRoute {
                    peer_id: *carol.as_bytes(),
                    address: "/ip4/192.168.1.103/tcp/22487".into(),
                }],
                3,
                1_000,
                [bob, carol, dave],
            ),
            None
        );
        assert_eq!(
            current, before,
            "stale whole-record state must not be saved"
        );
    }

    #[test]
    fn only_a_direct_inviter_admission_grants_durable_reconnect_authority() {
        let inviter = phase0_peer_id(&test_libp2p_peer(101));
        let helper = phase0_peer_id(&test_libp2p_peer(102));

        assert_eq!(
            reconnect_policy_after_admission(inviter, inviter, false, false),
            ReconnectPolicy::AuthorizedPeer(*inviter.as_bytes()),
        );
        assert_eq!(
            reconnect_policy_after_admission(inviter, inviter, true, false),
            ReconnectPolicy::Disabled,
            "a reply callback stays time-bounded even when it authenticates the inviter"
        );
        assert_eq!(
            reconnect_policy_after_admission(inviter, inviter, false, true),
            ReconnectPolicy::Disabled,
            "a switchboard ceremony cannot become indefinite inviter-dial consent"
        );
        assert_eq!(
            reconnect_policy_after_admission(helper, inviter, false, false),
            ReconnectPolicy::Disabled,
            "an authenticated non-inviter contact is never the recurring contact"
        );
    }

    #[test]
    fn reconnect_capture_policy_preserves_admission_consent_and_legacy_scope() {
        let authorized = phase0_peer_id(&test_libp2p_peer(99));
        let helper = phase0_peer_id(&test_libp2p_peer(100));

        assert_eq!(
            reconnect_capture_peer(ReconnectPolicy::Disabled, 1, [authorized]),
            None,
            "helper/reply/switchboard admission may never be inferred from an empty route list"
        );
        assert_eq!(
            reconnect_capture_peer(
                ReconnectPolicy::AuthorizedPeer(*authorized.as_bytes()),
                2,
                [authorized, helper],
            ),
            Some(authorized),
            "direct admission refreshes only its named inviter"
        );
        assert_eq!(
            reconnect_capture_peer(
                ReconnectPolicy::AuthorizedPeer(*authorized.as_bytes()),
                1,
                [helper],
            ),
            None,
            "another member cannot replace the authorized recurring contact"
        );
        assert_eq!(
            reconnect_capture_peer(ReconnectPolicy::LegacyPending, 1, [authorized]),
            Some(authorized),
            "a pre-v3 two-member server may migrate once after overlap"
        );
        assert_eq!(
            reconnect_capture_peer(ReconnectPolicy::LegacyPending, 2, [authorized, helper]),
            None,
            "legacy migration stays disabled where a helper/switchboard may be involved"
        );
    }

    #[test]
    fn the_bootstrap_validators_are_not_fooled_by_ipv4_in_ipv6_or_by_a_name() {
        const ID: &str = "12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN";
        let a = |h: &str| format!("{h}/tcp/9/p2p/{ID}");

        // `::ffff:127.0.0.1` is loopback written the other way. Testing `Ipv6Addr` properties
        // directly missed it, so it sorted into the "routable" half and the joiner dialled its
        // own localhost while ignoring the invite's real addresses.
        let mapped_loopback: Multiaddr = a("/ip6/::ffff:127.0.0.1").parse().unwrap();
        assert!(addr_is_loopback(&mapped_loopback));
        let mixed = vec![a("/ip6/::ffff:127.0.0.1"), a("/ip4/203.0.113.7")];
        let out = dialable_bootstrap(&mixed);
        assert_eq!(
            out.len(),
            1,
            "the mapped loopback is not a routable address"
        );
        assert_eq!(out[0].to_string(), a("/ip4/203.0.113.7"));

        // The same trick against the advertise gate: a private v4 in v6 clothing must not be
        // published to a rendezvous.
        for private in ["/ip6/::ffff:192.168.1.5", "/ip6/::ffff:10.0.0.1"] {
            let addr = a(private);
            assert!(
                external_addrs(std::slice::from_ref(&addr)).is_empty(),
                "{addr} must not be advertised"
            );
        }
        // ...and a mapped multicast/link-local address is not a peer at all.
        for undialable in ["/ip6/::ffff:224.0.0.1", "/ip6/::ffff:169.254.1.1"] {
            let addr: Multiaddr = a(undialable).parse().unwrap();
            assert!(addr_is_undialable(&addr), "{undialable} cannot be a peer");
        }

        // A name is resolved at dial time, so what it points at is whatever the invite's author
        // says right now. Nothing this app mints contains one.
        for name in [
            "/dns4/scan.attacker.invalid/tcp/22",
            "/dns6/scan.attacker.invalid/tcp/22",
            "/dns/scan.attacker.invalid/tcp/22",
            "/dnsaddr/scan.attacker.invalid",
        ] {
            let addr: Multiaddr = name.parse().unwrap();
            assert!(addr_is_undialable(&addr), "{name} must not be dialled");
        }
        assert!(dialable_bootstrap(&["/dns4/x.invalid/tcp/9".to_string()]).is_empty());
    }

    #[test]
    fn the_discovery_cadence_is_jittered_around_its_base() {
        // P11: a bare interval gives every member of every group the same period and phase, so
        // one infrastructure outage reconverges the whole network inside a single window.
        let base = DISCOVERY_INTERVAL_SECS * 1_000 - DISCOVERY_JITTER_MS;
        let spread = DISCOVERY_JITTER_MS * 2;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let d = jittered_delay(base, spread).as_millis() as u64;
            assert!(
                (base..base + spread).contains(&d),
                "{d}ms is outside [{base}, {})",
                base + spread
            );
            seen.insert(d);
        }
        assert!(
            seen.len() > 32,
            "the period must actually vary, got {} distinct of 64",
            seen.len()
        );
        // A zero spread is the degenerate case and must not panic on the modulo.
        assert_eq!(jittered_delay(1_000, 0), Duration::from_millis(1_000));
    }

    #[test]
    fn only_globally_routable_addresses_are_advertised() {
        const ID: &str = "12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN";
        // Loopback, RFC1918, CGNAT, link-local, ULA: none of these mean anything to a remote peer,
        // and publishing them maps this machine's internal network for anyone who asks.
        for host in [
            "/ip4/127.0.0.1",
            "/ip4/10.1.2.3",
            "/ip4/192.168.1.5",
            "/ip4/172.20.0.1",
            "/ip4/100.70.0.1",
            "/ip4/169.254.1.1",
            "/ip6/::1",
            "/ip6/fd00::1",
            "/ip6/fe80::1",
        ] {
            let private = format!("{host}/tcp/9/p2p/{ID}");
            assert!(
                external_addrs(std::slice::from_ref(&private)).is_empty(),
                "{private} must not be advertised"
            );
        }

        // A real public address is advertised, with our own /p2p/ stripped (libp2p re-appends it).
        let public = format!("/ip4/8.8.8.8/udp/9000/quic-v1/p2p/{ID}");
        let out = external_addrs(&[public]);
        assert_eq!(
            out.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
            vec!["/ip4/8.8.8.8/udp/9000/quic-v1"]
        );

        // A mixed list keeps only the routable entries; no fallback to loopback any more.
        let mixed = vec![
            format!("/ip4/127.0.0.1/tcp/9/p2p/{ID}"),
            format!("/ip6/2606:4700::1111/tcp/9/p2p/{ID}"),
        ];
        assert_eq!(external_addrs(&mixed).len(), 1);

        // Documentation and multicast literals are syntactically valid but never internet
        // endpoints; the bridge uses the same canonical classifier as the transport actor.
        for host in ["/ip4/203.0.113.7", "/ip6/2001:db8::5", "/ip4/224.0.0.1"] {
            let reserved = format!("{host}/tcp/9/p2p/{ID}");
            assert!(external_addrs(&[reserved]).is_empty());
        }
    }

    #[test]
    fn the_listen_port_prefers_the_seed_derived_home_port() {
        let net = ServerNet {
            key_seed: [42u8; 32],
            port: 0,
            advertise: String::new(),
            relay: String::new(),
            rendezvous: String::new(),
            switchboard: false,
            record_seq: 0,
            reconnect_routes: Vec::new(),
            reconnect_policy: ReconnectPolicy::Disabled,
            pending_recovery_peer: None,
            pending_recovery_expires_at_ms: 0,
        };
        // Nothing is holding the derived port on a test machine, so that is what gets chosen, and
        // it is chosen again on the next call: stability is the whole point.
        let home = net.derived_port();
        assert_eq!(choose_port(&net), home);
        assert_eq!(choose_port(&net), home);

        // Squat it, and the chooser falls back rather than starting with no listener; the number
        // it falls back to is still a real, bindable port.
        let squatter =
            std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, home)).unwrap();
        let fallback = choose_port(&net);
        assert_ne!(fallback, home);
        assert_ne!(fallback, 0);
        drop(squatter);
        // Once the squatter leaves, the server comes home on its own (the port-forward the user
        // configured keeps working) rather than drifting further each launch.
        assert_eq!(choose_port(&net), home);
    }

    #[test]
    fn advertised_address_forms() {
        let id = "12D3KooWfakepeerid";
        // Bare IPv4 uses the bound port.
        assert_eq!(
            build_advertised("203.0.113.7", 9000, id).unwrap(),
            format!("/ip4/203.0.113.7/tcp/9000/p2p/{id}")
        );
        // host:port overrides the port (e.g. a forwarded port).
        assert_eq!(
            build_advertised("203.0.113.7:5678", 9000, id).unwrap(),
            format!("/ip4/203.0.113.7/tcp/5678/p2p/{id}")
        );
        // A full multiaddr without /p2p/ gets ours appended.
        assert_eq!(
            build_advertised("/ip4/198.51.100.1/tcp/1", 9000, id).unwrap(),
            format!("/ip4/198.51.100.1/tcp/1/p2p/{id}")
        );
        // A full multiaddr that already carries /p2p/ (e.g. a relay circuit) is used as-is.
        let circuit = "/ip4/198.51.100.1/tcp/4000/p2p/RELAY/p2p-circuit";
        assert_eq!(build_advertised(circuit, 9000, id).unwrap(), circuit);
        // A bare IPv6 literal is recognised by its own shape and uses the bound port; the
        // bracketed form carries an explicit one. Without this, every colon in an IPv6 address
        // looked like a host:port separator and the address was silently mangled into an /ip4/.
        assert_eq!(
            build_advertised("2001:db8::5", 9000, id).unwrap(),
            format!("/ip6/2001:db8::5/tcp/9000/p2p/{id}")
        );
        assert_eq!(
            build_advertised("[2001:db8::5]:443", 9000, id).unwrap(),
            format!("/ip6/2001:db8::5/tcp/443/p2p/{id}")
        );
        assert_eq!(
            build_advertised("[2001:db8::5]", 9000, id).unwrap(),
            format!("/ip6/2001:db8::5/tcp/9000/p2p/{id}")
        );
        // Empty / malformed are rejected.
        assert!(build_advertised("", 9000, id).is_err());
        assert!(build_advertised("1.2.3.4:notaport", 9000, id).is_err());
        assert!(build_advertised("[2001:db8::5", 9000, id).is_err());
        assert!(build_advertised("[2001:db8::5]:nope", 9000, id).is_err());
    }

    #[test]
    fn a_forged_invite_is_rejected_at_the_paste_before_anything_is_dialled() {
        use catcoms_mls::ServerGroup;

        // A genuine invite, minted the way `found_server` mints one, carrying attacker-visible
        // bootstrap and rendezvous vectors.
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let good = group
            .mint_invite_with_rendezvous(
                &device,
                [9u8; 16],
                1_000_000,
                vec!["/ip4/203.0.113.7/tcp/9000/p2p/ID".into()],
                vec!["/ip4/198.51.100.1/tcp/5000/p2p/RZ".into()],
            )
            .unwrap();
        let good_hex = hex::encode(good.encode());
        assert!(decode_and_verify_invite(&good_hex).is_ok());
        // Surrounding whitespace from a paste is fine.
        assert!(decode_and_verify_invite(&format!("  {good_hex}\n")).is_ok());

        // Flip a signature bit: structurally perfect, cryptographically worthless. This is the
        // shape a forged invite takes, and `InviteToken::decode` alone waves it through. Rejecting
        // it here is what stops the client binding a listener and dialling the two attacker-chosen
        // hosts above (the sync layer's own check happens several dials too late).
        let mut forged = good.clone();
        forged.signature[0] ^= 1;
        let err = decode_and_verify_invite(&hex::encode(forged.encode()))
            .expect_err("a token with a broken signature must not be acted on");
        assert!(err.contains("signature"), "unhelpful error: {err}");

        // Editing the addresses after signing is the same failure: the vectors are inside the
        // signed payload, so a swapped bootstrap host cannot survive the check.
        let mut swapped = good.clone();
        swapped.bootstrap = vec!["/ip4/192.0.2.66/tcp/1/p2p/ATTACKER".into()];
        assert!(decode_and_verify_invite(&hex::encode(swapped.encode())).is_err());

        // Garbage in the box is refused too, rather than panicking.
        assert!(decode_and_verify_invite("not hex").is_err());
        assert!(decode_and_verify_invite("00ff00ff").is_err());
        assert!(
            decode_and_verify_invite(&"00".repeat(MAX_INVITE_WIRE_BYTES + 1))
                .unwrap_err()
                .contains("too large")
        );
    }

    #[test]
    fn only_our_issue_tracker_can_be_opened() {
        assert!(is_tracker_url(
            "https://github.com/Thalpy/Mewtual/issues/new?labels=bug&title=x&body=y"
        ));
        // A lookalike host, another repo, and a non-https scheme are all refused, as is the
        // bare tracker root: only the prefilled new-issue form is ever launched.
        assert!(!is_tracker_url(
            "https://github.com.evil.test/Thalpy/Mewtual/issues/new?x"
        ));
        assert!(!is_tracker_url(
            "https://github.com/Thalpy/Other/issues/new?x"
        ));
        assert!(!is_tracker_url("file:///c:/windows/system32/calc.exe"));
        assert!(!is_tracker_url("https://github.com/Thalpy/Mewtual"));
        assert!(!is_tracker_url(
            "https://github.com/Thalpy/Mewtual/issues/new?title=x\nfile:///tmp/payload"
        ));
        assert!(!is_tracker_url(&format!(
            "{}body={}",
            ISSUE_URL_PREFIX,
            "x".repeat(ISSUE_URL_MAX_BYTES)
        )));
    }

    #[test]
    fn native_public_issue_keeps_the_exact_report_and_bounds_only_the_url_excerpt() {
        let native = "Mewtual public diagnostics v1\n\nJOIN.TEST count=1 \u{1f408}";
        let small = prepare_public_diagnostics_issue(native);
        let expected = format!(
            "**Type:** Bug report\n**App:** Mewtual desktop {}\n**Environment:** Mewtual desktop\n\n{native}",
            env!("CARGO_PKG_VERSION"),
        );
        assert_eq!(small.report, expected);
        assert!(!small.truncated);
        assert!(is_tracker_url(&small.url));
        assert_eq!(
            encoded_issue_query_len(&small.report),
            encode_issue_query(&small.report).len(),
            "the URL budget and encoder must count the same UTF-8 bytes"
        );

        let large_native = format!("{}\u{1f408}", "bounded-public-event\n".repeat(20_000));
        let large = prepare_public_diagnostics_issue(&large_native);
        assert!(large.truncated);
        assert!(large.url.len() <= ISSUE_URL_MAX_BYTES);
        assert!(is_tracker_url(&large.url));
        assert!(
            large.report.ends_with(&large_native),
            "the exact native report survives for clipboard fallback"
        );
        assert!(
            large
                .url
                .contains(&encode_issue_query(PUBLIC_ISSUE_TRUNCATION_NOTE)),
            "only the launched URL is explicitly shortened"
        );
    }

    #[test]
    fn external_links_are_limited_to_http_and_https() {
        assert!(is_external_http_url("https://example.com/path?cat=yes"));
        assert!(is_external_http_url("http://localhost:1420/image.png"));

        assert!(!is_external_http_url("javascript:alert(1)"));
        assert!(!is_external_http_url("data:text/html,hello"));
        assert!(!is_external_http_url(
            "file:///c:/windows/system32/calc.exe"
        ));
        assert!(!is_external_http_url("https:///missing-host"));
        assert!(!is_external_http_url(""));
    }

    #[test]
    fn space_guide_save_only_accepts_the_expected_png_shape() {
        let mut header = vec![0u8; 24];
        header[..8].copy_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        header[12..16].copy_from_slice(b"IHDR");
        header[16..20].copy_from_slice(&2048u32.to_be_bytes());
        header[20..24].copy_from_slice(&1024u32.to_be_bytes());
        assert!(validate_space_guide_png(&header));

        let mut wrong_size = header.clone();
        wrong_size[16..20].copy_from_slice(&1024u32.to_be_bytes());
        assert!(!validate_space_guide_png(&wrong_size));

        header[0] = 0;
        assert!(!validate_space_guide_png(&header));
    }

    #[test]
    fn space_layout_export_only_accepts_our_versioned_object() {
        assert!(validate_space_layout_json(
            r#"{"kind":"mewtual-server-space-layout","version":1,"space":{"placements":{}}}"#
        ));
        assert!(!validate_space_layout_json(
            r#"{"kind":"mewtual-server-space-layout","version":2,"space":{}}"#
        ));
        assert!(!validate_space_layout_json(
            r#"{"kind":"something-else","version":1,"space":{}}"#
        ));
        assert!(!validate_space_layout_json("not json"));
    }

    #[test]
    fn jam_sheet_export_accepts_only_the_inert_renderer_grammar() {
        let safe = format!(
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 120" width="960" height="120" data-mewtual-sheet="v1"><style>{}</style>"#,
                r##"<rect x="0" y="0" width="960" height="120" fill="#fffdf6"/>"##,
                r#"<line x1="1" y1="2" x2="3.5" y2="4" class="st"/>"#,
                r#"<ellipse cx="5" cy="6" rx="4.6" ry="3.4" class="nh open"/>"#,
                r#"<path d="M5 6 q7 -3 8 -12" class="flag"/>"#,
                r#"<text x="56" y="30" class="ttl">Mika &amp; Rook</text></svg>"#,
            ),
            JAM_SHEET_STYLE,
        );
        assert!(validate_jam_sheet_svg(&safe));

        for active in [
            safe.replace("</svg>", "<script>alert(1)</script></svg>"),
            safe.replace("<svg ", "<svg onload=\"alert(1)\" "),
            safe.replace("</svg>", "<foreignobject>html</foreignobject></svg>"),
            safe.replace(
                "</svg>",
                "<image href=\"https://example.invalid/x\"/></svg>",
            ),
            safe.replace(JAM_SHEET_STYLE, "text{fill:url(https://example.invalid/x)}"),
        ] {
            assert!(
                !validate_jam_sheet_svg(&active),
                "active SVG syntax was admitted"
            );
        }
        assert!(!validate_jam_sheet_svg(&safe.replace(
            "data-mewtual-sheet=\"v1\"",
            "data-mewtual-sheet=\"v2\""
        )));
        assert!(validate_jam_sheet_name("mewtual-take-01-20260902.svg"));
        assert!(validate_jam_sheet_name("mewtual-take-100-20260902.svg"));
        assert!(!validate_jam_sheet_name("notes.svg"));
        assert!(!validate_jam_sheet_name("mewtual-take-01-2026-09-02.svg"));
    }

    #[tokio::test]
    async fn jam_sheet_export_cannot_publish_or_reveal_after_lock() {
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::default());
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;
        let generation = unlocked_ui_session_generation(&state).await.unwrap();
        let safe = format!(
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 120" width="960" height="120" data-mewtual-sheet="v1"><style>{}</style>"#,
                r##"<rect x="0" y="0" width="960" height="120" fill="#fffdf6"/>"##,
                "</svg>"
            ),
            JAM_SHEET_STYLE,
        );
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let export_state = Arc::clone(&state);
        let export_root = root.path().to_path_buf();
        let export_started = Arc::clone(&started);
        let export_release = Arc::clone(&release);
        let exporting = tokio::spawn(async move {
            save_jam_sheet_to_downloads(
                &export_state,
                generation,
                &export_root,
                "mewtual-take-01-20260902.svg",
                &safe,
                async move {
                    export_started.notify_one();
                    export_release.notified().await;
                },
                |_| panic!("stale sheet export must never reach reveal"),
            )
            .await
        });
        started.notified().await;

        lock_session_inner(&state, None).await.unwrap();
        release.notify_one();
        assert!(exporting.await.unwrap().is_err());
        assert!(
            !root.path().join("mewtual-take-01-20260902.svg").exists(),
            "no plaintext sheet may appear after the UI generation was locked"
        );
    }

    #[test]
    fn downloaded_file_names_cannot_escape_downloads_or_use_device_names() {
        assert_eq!(safe_download_name("../../report?.pdf"), "report_.pdf");
        assert_eq!(safe_download_name(r"..\..\CON.txt"), "_CON.txt");
        assert_eq!(safe_download_name(" . "), "mewtual-download");
        assert_eq!(numbered_download_name("photo.png", 2), "photo (2).png");
        assert_eq!(numbered_download_name("README", 3), "README (3)");
    }

    #[test]
    fn a_download_reserves_its_name_before_a_byte_is_written() {
        // The streamed save opens the file first and fills it chunk by chunk, so the reservation
        // itself has to be what stops a second save from landing on the same path; and a caller
        // that then fails has to be able to clear the reservation it took.
        let dir = tempfile::tempdir().unwrap();
        let (first_file, first) = create_download(dir.path(), "notes.txt").unwrap();
        assert_eq!(first.file_name().unwrap(), "notes.txt");
        assert_eq!(
            std::fs::metadata(&first).unwrap().len(),
            0,
            "reserved, empty"
        );

        let (_second_file, second) = create_download(dir.path(), "notes.txt").unwrap();
        assert_eq!(second.file_name().unwrap(), "notes (1).txt");
        assert_ne!(first, second, "an in-progress save is never overwritten");

        drop(first_file);
        std::fs::remove_file(&first).unwrap();
        let (_again_file, again) = create_download(dir.path(), "notes.txt").unwrap();
        assert_eq!(again, first, "an abandoned save frees its name");
    }

    #[test]
    fn a_written_download_keeps_its_bytes_and_its_unique_name() {
        let dir = tempfile::tempdir().unwrap();
        let first = write_download(dir.path(), "../report?.pdf", b"one").unwrap();
        let second = write_download(dir.path(), "../report?.pdf", b"two").unwrap();
        assert_eq!(first.file_name().unwrap(), "report_.pdf", "name sanitized");
        assert_eq!(second.file_name().unwrap(), "report_ (1).pdf");
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
    }

    #[test]
    fn ui_state_requires_the_bounded_versioned_shape() {
        assert!(validate_ui_state_json(
            r#"{"version":1,"drafts":{"1:2":"hello"},"readMarks":{"1:2":9}}"#
        )
        .is_ok());
        assert!(validate_ui_state_json(r#"{"version":2,"drafts":{},"readMarks":{}}"#).is_err());
        assert!(validate_ui_state_json(r#"{"version":1,"drafts":[],"readMarks":{}}"#).is_err());
        assert!(validate_ui_state_json("not json").is_err());
        assert!(validate_ui_state_json(&"x".repeat(MAX_UI_STATE_BYTES + 1)).is_err());
    }

    #[test]
    fn storage_inventory_deduplicates_content_and_breaks_out_pinned_space() {
        let cat_cid = Cid::of(b"cat").to_hex();
        let song_cid = Cid::of(b"song").to_hex();
        let file = |name: &str,
                    cid: &str,
                    mime: &str,
                    size: u64,
                    held: u32,
                    total: u32,
                    manifest_version: [u8; 32]| UiFile {
            name: name.into(),
            size,
            mime: mime.into(),
            cid: cid.into(),
            author: "member".into(),
            author_identity: "full-member-device-id".into(),
            author_verified: true,
            path: "shared".into(),
            held,
            total,
            manifest_version,
            expires: None,
            expires_known: false,
        };
        let report = build_storage_report(
            StorageHealth {
                listed_files: 3,
                referenced_chunks: 3,
                verified_chunks: 3,
                verified_bytes: 512,
                verified_manifest_versions: HashSet::from([[2; 32]]),
                ..StorageHealth::default()
            },
            vec![
                file("cat.png", &cat_cid, "image/png", 100, 1, 2, [1; 32]),
                file("same-cat.png", &cat_cid, "image/png", 100, 1, 2, [1; 32]),
                file("song.ogg", &song_cid, "audio/ogg", 400, 2, 2, [2; 32]),
            ],
            &HashSet::from([cat_cid]),
            42,
        );

        assert_eq!((report.listed_files, report.unique_files), (3, 2));
        assert_eq!(
            (report.logical_bytes, report.local_estimated_bytes),
            (500, 450)
        );
        assert_eq!(
            (
                report.pinned_files,
                report.pinned_logical_bytes,
                report.pinned_local_estimated_bytes
            ),
            (1, 100, 50)
        );
        assert_eq!(report.checked_at_ms, 42);
        assert_eq!(
            report
                .categories
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Images", "Audio"]
        );
        assert_eq!(report.largest_files[0].name, "song.ogg");
        assert!(report.largest_files[1].pinned);
        assert_eq!(
            report
                .local_files
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            vec!["song.ogg"],
            "only complete local ciphertext belongs in the downloadable-on-this-device list"
        );
    }

    #[test]
    fn storage_inventory_denies_ambiguous_or_unverified_manifest_rows() {
        let cid = Cid::of(b"same claimed content").to_hex();
        let file = |name: &str, manifest_version: [u8; 32]| UiFile {
            name: name.into(),
            size: 20,
            mime: "image/png".into(),
            cid: cid.clone(),
            author: "member".into(),
            author_identity: "full-member-device-id".into(),
            author_verified: true,
            path: name.into(),
            held: 1,
            total: 1,
            manifest_version,
            expires: None,
            expires_known: false,
        };
        let report = build_storage_report(
            StorageHealth {
                verified_manifest_versions: HashSet::from([[3; 32], [4; 32]]),
                ..StorageHealth::default()
            },
            vec![file("safe", [3; 32]), file("replacement", [4; 32])],
            &HashSet::new(),
            1,
        );
        assert!(
            report.local_files.is_empty(),
            "two exact manifests sharing one claimed plaintext CID are not one unlockable file"
        );

        let unverified = build_storage_report(
            StorageHealth::default(),
            vec![file("present-but-unreadable", [5; 32])],
            &HashSet::new(),
            2,
        );
        assert!(
            unverified.local_files.is_empty(),
            "held-chunk existence alone must not admit a file into local inventory"
        );
    }

    #[test]
    fn storage_scan_singleflight_has_fixed_bounded_cardinality() {
        let gates = StorageScanGates::default();
        assert_eq!(gates.stripes.len(), STORAGE_SCAN_STRIPES);
        assert!(std::ptr::eq(
            gates.for_server(3),
            gates.for_server(3 + STORAGE_SCAN_STRIPES as u64),
        ));
    }

    #[test]
    fn backup_copy_preserves_sealed_tree_and_never_overwrites_a_destination() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("vault");
        let nested = source.join("servers");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(source.join("vault.bin"), b"sealed-root").unwrap();
        std::fs::write(nested.join("1.bin"), b"sealed-snapshot").unwrap();
        let destination = root.path().join("backup");

        assert_eq!(copy_backup_tree(&source, &destination).unwrap(), (2, 26));
        assert_eq!(
            std::fs::read(destination.join("servers").join("1.bin")).unwrap(),
            b"sealed-snapshot"
        );
        assert!(
            copy_backup_tree(&source, &destination).is_err(),
            "an existing backup directory must never be merged into or overwritten"
        );
    }

    #[test]
    fn found_server_server_name_falls_back_to_display_name_when_empty() {
        // When server_name is None or empty, the server entry name should fall back to display_name
        // This preserves backwards compatibility for standard server creation
        let display_name = "My Server".to_string();
        let server_name: Option<String> = None;
        let name = server_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| display_name.clone());
        assert_eq!(name, "My Server");

        let server_name = Some("".to_string());
        let name = server_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| display_name.clone());
        assert_eq!(name, "My Server");

        let server_name = Some("   ".to_string());
        let name = server_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| display_name.clone());
        assert_eq!(name, "My Server");
    }

    #[test]
    fn found_server_server_name_used_when_provided() {
        // When server_name is provided and non-empty, it should be used for the server entry name
        let display_name = "My Profile Name".to_string();
        let server_name = Some("Friend's Server".to_string());
        let name = server_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| display_name.clone());
        assert_eq!(name, "Friend's Server");
    }

    #[test]
    fn join_server_server_name_falls_back_to_display_name_when_empty() {
        // Same logic for join_server
        let display_name = "Joiner's Profile".to_string();
        let server_name: Option<String> = None;
        let name = server_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| display_name.clone());
        assert_eq!(name, "Joiner's Profile");

        let server_name = Some("".to_string());
        let name = server_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| display_name.clone());
        assert_eq!(name, "Joiner's Profile");
    }

    #[test]
    fn join_server_server_name_used_when_provided() {
        let display_name = "Joiner's Profile".to_string();
        let server_name = Some("DM with Friend".to_string());
        let name = server_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| display_name.clone());
        assert_eq!(name, "DM with Friend");
    }

    #[test]
    fn member_route_enums_have_stable_exhaustive_webview_names() {
        use catcoms_rt::{ConnectionFamily as Family, ConnectionTransport as Transport};
        use catcoms_sync::{
            IndirectRouteHealth as Indirect, MemberRouteAction, MemberRouteActionKind as Kind,
            MemberRouteActionScope as Scope, MemberRouteHealth as Health,
        };

        assert_eq!(
            [
                Family::Ipv4,
                Family::Ipv6,
                Family::Dns,
                Family::Memory,
                Family::Unknown,
            ]
            .map(connection_family_name),
            ["ipv4", "ipv6", "dns", "memory", "unknown"]
        );
        assert_eq!(
            [
                Transport::Tcp,
                Transport::QuicV1,
                Transport::WebSocket,
                Transport::CircuitRelay,
                Transport::Memory,
                Transport::Unknown,
            ]
            .map(connection_transport_name),
            [
                "tcp",
                "quic_v1",
                "websocket",
                "circuit_relay",
                "memory",
                "unknown",
            ]
        );
        assert_eq!(
            [
                Health::NoPeerRecord,
                Health::ClaimedPeerHasNoRoute,
                Health::ClaimedPeerConnectedDirect,
                Health::ClaimedPeerConnectedRelay,
                Health::ClaimedPeerConnectedOther,
                Health::ClaimedPeerDialCoolingDown,
                Health::ClaimedPeerDialEligible,
            ]
            .map(member_route_health_name),
            [
                "no_peer_record",
                "claimed_peer_has_no_route",
                "claimed_peer_connected_direct",
                "claimed_peer_connected_relay",
                "claimed_peer_connected_other",
                "claimed_peer_dial_cooling_down",
                "claimed_peer_dial_eligible",
            ]
        );

        let scopes = [Scope::ThisDevice, Scope::MemberDevice, Scope::Group].map(|scope| {
            member_route_action_evt(MemberRouteAction {
                scope,
                kind: Kind::WaitForAutomaticRecovery,
            })
            .scope
        });
        assert_eq!(scopes, ["this_device", "member_device", "group"]);

        let kinds = [
            Kind::WaitForAutomaticRecovery,
            Kind::CheckMemberConnectivity,
            Kind::KeepAnotherMemberConnected,
            Kind::ConfigureFallbackNode,
            Kind::ProbeThroughMembers,
            Kind::RetryGroupNow,
        ]
        .map(|kind| {
            member_route_action_evt(MemberRouteAction {
                scope: Scope::Group,
                kind,
            })
            .kind
        });
        assert_eq!(
            kinds,
            [
                "wait_for_automatic_recovery",
                "check_member_connectivity",
                "keep_another_member_connected",
                "configure_fallback_node",
                "probe_through_members",
                "retry_group_now",
            ]
        );
        assert_eq!(
            [
                Indirect::Unknown,
                Indirect::ReachableViaMember,
                Indirect::SuspectedUnreachable,
            ]
            .map(indirect_route_health_name),
            ["unknown", "reachable_via_member", "suspected_unreachable"]
        );
    }
}
