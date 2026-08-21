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
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use catcoms_app::store::MAX_UI_STATE_BYTES;
use catcoms_app::{
    channel_id, spawn, AppEvent, Cid, DeviceId, FileListing, InviteJoinPlan, Livery, PairingLedger,
    PairingSecrets, PerServerGrant, Profile, Server, ServerActor, ServerNet, ServerRecord,
    ServerStore, StorageHealth, MAX_AVATAR_BYTES, MAX_BANNER_BYTES, MAX_FILE_BYTES,
    MAX_SERVER_CURSOR_BYTES, MAX_SERVER_ICON_BYTES,
};
use catcoms_discovery::{Candidate, DiscoveryPolicy, PolicyConfig, Source};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_net::{
    addr_is_globally_routable, addr_is_loopback, addr_is_undialable, keypair_from_seed,
    phase0_peer_id, target_peer_in_multiaddr, validate_invite_rendezvous_addrs,
    validate_operator_rendezvous_addrs, AutoNatResult, AutoNatSnapshot, JoinReply, MeshHandle,
    MeshObservationSnapshot, MeshService, PortMappingMechanism, PortMappingSnapshot,
    PortMappingTransport, RelayAddressSnapshot, RendezvousTarget,
};
use catcoms_rt::{Clock, MeshTransport, OsCryptoRng, PeerId, RngCore, SystemClock, TransportEvent};
use catcoms_sync::{join_namespace, JOIN_REPLY_PROOF_KIND};
use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, watch, Mutex};
use tokio::time::timeout;
use zeroize::Zeroizing;

/// One running server: its actor handle, the single-use invite to share (founder only), and
/// its display name (kept here too so the registry can be re-sealed on disk, Phase 9f).
struct ServerEntry {
    actor: ServerActor,
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
}

/// App state managed by Tauri: every running server keyed by a bridge-assigned id, plus the
/// on-disk store once the user has unlocked it with a passphrase (`None` = in-memory only).
#[derive(Default)]
struct AppState {
    servers: Mutex<HashMap<u64, ServerEntry>>,
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
    next_id: Mutex<u64>,
    store: Mutex<Option<ServerStore>>,
    /// Whether a freshly-mounted frontend may restore the already-unlocked UI session. This stays
    /// true across F5/HMR, but an explicit Ctrl+L clears it so a reload cannot bypass the lock.
    session_resumable: Mutex<bool>,
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
    /// One integrity/inventory scan per server per process session. Health is a point-in-time
    /// observation, so file events deliberately do not invalidate it behind the user's back;
    /// explicit authenticated repair is the only operation that replaces a cached report.
    storage_health: Mutex<HashMap<u64, UiStorageHealth>>,
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
    if *state.session_resumable.lock().await && state.store.lock().await.is_some() {
        Ok(())
    } else {
        Err("the vault is locked".into())
    }
}

/// Internal actor lookup used by persistence/reload paths which must keep operating while the UI
/// is locked. Native command handlers use [`actor_of`] so the webview cannot cross that boundary.
async fn actor_of_unchecked(state: &AppState, server: u64) -> Result<ServerActor, String> {
    state
        .servers
        .lock()
        .await
        .get(&server)
        .map(|e| e.actor.clone())
        .ok_or_else(|| "unknown server".to_string())
}

/// Clone out the actor for an unlocked webview command (never holding either lock across actor I/O).
async fn actor_of(state: &AppState, server: u64) -> Result<ServerActor, String> {
    require_unlocked_session(state).await?;
    actor_of_unchecked(state, server).await
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
    path: String,
    /// Chunks of this file already held locally (availability indicator).
    held: u32,
    /// Total chunks the file is split into.
    total: u32,
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

/// The shared file list plus whether any peer is currently reachable to fetch from; the payload
/// of `get_files`, so the UI can color each file by availability in one round-trip.
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
        path: listing.entry.path,
        held: listing.held_chunks,
        total: listing.total_chunks,
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
    for file in files {
        unique.entry(file.cid.clone()).or_insert(file);
    }
    let mut category_map = HashMap::<String, UiStorageCategory>::new();
    let mut largest_files = Vec::with_capacity(unique.len());
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
        largest_files.push(UiStorageFile {
            name: file.name.clone(),
            path: file.path.clone(),
            cid: file.cid.clone(),
            mime: file.mime.clone(),
            logical_bytes: file.size,
            local_estimated_bytes: local,
            pinned: is_pinned,
            held: file.held,
            total: file.total,
        });
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
    }
}

async fn storage_report(
    actor: &ServerActor,
    health: StorageHealth,
    checked_at_ms: u64,
) -> UiStorageHealth {
    let view = actor.files_view().await;
    let pins = actor.wiki_pinned_cids().await.into_iter().collect();
    build_storage_report(
        health,
        view.files.into_iter().map(ui_file).collect(),
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
}
#[derive(Serialize, Clone)]
struct DeliveryEvt {
    server: u64,
    channel: String,
    states: Vec<DeliveryStateEvt>,
}

fn delivery_payload(states: Vec<catcoms_app::DeliveryState>) -> Vec<DeliveryStateEvt> {
    states
        .into_iter()
        .map(|s| DeliveryStateEvt {
            id: s.id,
            delivered: s.delivered,
            reachable: s.reachable,
        })
        .collect()
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

/// Spawn a per-server timer that periodically drives steady-state discovery, so the group
/// re-finds itself after a restart and members keep exchanging peer records. Exits once the actor
/// stops (`drive_discovery` errors).
fn spawn_discovery_timer(app: AppHandle, server: u64, actor: ServerActor) {
    tokio::spawn(async move {
        // A short randomised start offset, then an independently randomised period each round.
        let mut delay = jittered_delay(0, DISCOVERY_START_SPREAD_MS);
        loop {
            SystemClock.sleep(delay).await;
            if actor.drive_discovery().await.is_err() {
                break; // the actor stopped
            }
            // The pass just refreshed the member records; seal the cache on the same cadence, so
            // the next launch starts from the members this one actually proved.
            persist_address_cache(&app, server).await;
            delay = jittered_delay(
                DISCOVERY_INTERVAL_SECS * 1_000 - DISCOVERY_JITTER_MS,
                DISCOVERY_JITTER_MS * 2,
            );
        }
    });
}

fn forward_events(app: AppHandle, server: u64, mut events: mpsc::Receiver<AppEvent>) {
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            // Actor networking stays live behind the explicit lock, but its event stream can
            // contain member fingerprints, channel ids, delivery state and call signalling.
            // Drop those notifications at the native boundary; unlock reloads fresh projections.
            if require_unlocked_session(&app.state::<AppState>())
                .await
                .is_err()
            {
                if matches!(&ev, AppEvent::Closed) {
                    break;
                }
                continue;
            }
            match ev {
                AppEvent::ChannelsUpdated => {
                    let _ = app.emit("channels-changed", ServerEvt { server });
                }
                AppEvent::ChannelUpdated { channel } => {
                    // Channel ids are u128; send as a string (JS numbers lose precision).
                    let _ = app.emit(
                        "channel-updated",
                        ChannelEvt {
                            server,
                            channel: channel.to_string(),
                        },
                    );
                }
                AppEvent::MembersChanged { count } => {
                    let _ = app.emit("members-changed", CountEvt { server, count });
                }
                AppEvent::ProfilesUpdated => {
                    let _ = app.emit("profiles-updated", ServerEvt { server });
                }
                AppEvent::LiveryUpdated => {
                    let _ = app.emit("livery-changed", ServerEvt { server });
                }
                AppEvent::BadgesUpdated => {
                    let _ = app.emit("badges-changed", ServerEvt { server });
                }
                AppEvent::DevicesUpdated => {
                    let _ = app.emit("devices-changed", ServerEvt { server });
                }
                AppEvent::FilesUpdated => {
                    let _ = app.emit("files-updated", ServerEvt { server });
                }
                AppEvent::StatusUpdated => {
                    let _ = app.emit("status-updated", ServerEvt { server });
                }
                AppEvent::EventsUpdated => {
                    let _ = app.emit("events-changed", ServerEvt { server });
                }
                AppEvent::WikiUpdated => {
                    let _ = app.emit("wiki-updated", ServerEvt { server });
                }
                AppEvent::RolesUpdated => {
                    let _ = app.emit("roles-updated", ServerEvt { server });
                }
                AppEvent::ModerationUpdated => {
                    let _ = app.emit("moderation-updated", ServerEvt { server });
                }
                AppEvent::EclipseChanged { caution } => {
                    let _ = app.emit("eclipse-changed", EclipseEvt { server, caution });
                }
                AppEvent::ConnectivityChanged { online } => {
                    let _ = app.emit("connectivity-changed", OnlineEvt { server, online });
                }
                AppEvent::SwitchboardsChanged => {
                    let _ = app.emit("switchboard-changed", server);
                }
                AppEvent::DeliveryChanged { channel, states } => {
                    let _ = app.emit(
                        "delivery-changed",
                        DeliveryEvt {
                            server,
                            channel: channel.to_string(),
                            states: delivery_payload(states),
                        },
                    );
                }
                AppEvent::DmRequestsChanged => {
                    let _ = app.emit("dm-requests-changed", ServerEvt { server });
                }
                AppEvent::CallSignal { from_fp, payload } => {
                    let _ = app.emit(
                        "call-signal",
                        CallSignalEvt {
                            server,
                            from_fp,
                            payload: B64.encode(payload),
                        },
                    );
                }
                AppEvent::Closed => {
                    let _ = app.emit("server-closed", ServerEvt { server });
                    break;
                }
            }
        }
    });
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
/// share one number. The OS only hands out a free *TCP* port, so the UDP half is re-probed and the
/// draw retried; a handful of attempts is plenty in practice.
fn os_chosen_port() -> u16 {
    for _ in 0..16 {
        let Ok(probe) = std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)) else {
            return 0;
        };
        let Ok(local) = probe.local_addr() else {
            return 0;
        };
        let port = local.port();
        drop(probe);
        if port_is_bindable(port) {
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

/// Insert a freshly-spawned server into the registry, forward its events, and return the
/// new server id.
#[allow(clippy::too_many_arguments)]
async fn register_server(
    app: &AppHandle,
    state: &AppState,
    actor: ServerActor,
    events: mpsc::Receiver<AppEvent>,
    group_id: Vec<u8>,
    device_id: DeviceId,
    invite: Option<String>,
    name: String,
    bootstrap: Vec<String>,
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
    forward_events(app.clone(), id, events);
    spawn_discovery_timer(app.clone(), id, actor.clone());
    state.servers.lock().await.insert(
        id,
        ServerEntry {
            actor,
            group_id,
            device_id,
            invite,
            name,
            bootstrap,
            rendezvous,
            mesh,
            is_dm,
            switchboard,
            record_seq,
        },
    );
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
async fn persist_server(state: &AppState, server: u64) {
    let actor = match actor_of_unchecked(state, server).await {
        Ok(a) => a,
        Err(_) => return,
    };
    let bytes = match actor.snapshot().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("persist: snapshot of server {server} failed: {e}");
            return;
        }
    };
    let guard = state.store.lock().await;
    if let Some(store) = guard.as_ref() {
        let mut rng = OsCryptoRng;
        if let Err(e) = store.save_server(server, &bytes, &mut rng) {
            eprintln!("persist: sealing server {server} failed: {e}");
        }
    }
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
                    eprintln!("persist: no address-cache key for server {server}: {e}");
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
            eprintln!("persist: sealing the address cache of server {server} failed: {e}");
        }
    }
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
            eprintln!("persist: sealing registry failed: {e}");
        }
    }
}

/// Attach the per-server sealing blob store (Phase 9h) if the vault is unlocked, so files +
/// avatars persist encrypted at rest (keyed by the stable group id, so a reloaded server
/// finds its blobs). Best-effort: a locked store or an error leaves the in-memory default.
/// Must run before any blob is added (i.e. before `spawn`).
async fn attach_blob_store(state: &AppState, server: &mut Server<MeshService, OsCryptoRng>) {
    let guard = state.store.lock().await;
    if let Some(store) = guard.as_ref() {
        let key = hex::encode(server.group_id());
        match store.blob_store(&key) {
            Ok(blobs) => server.set_blob_store(blobs),
            Err(e) => eprintln!("attach blob store failed: {e}"),
        }
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
/// * anything unparseable, or naming something that cannot be a peer ([`addr_is_undialable`]),
///   is dropped;
/// * loopback is kept only when **nothing else survived**. A loopback entry is by construction
///   the same-machine case (two instances on one dev box, the DM/self-pairing flows), and that
///   case is real and must keep working. But when the invite also carries routable addresses,
///   a loopback entry is not a fallback for anything: it can only ever probe ports on the
///   reader's own machine, so it is dropped rather than dialled;
/// * the survivors are capped at [`MAX_BOOTSTRAP_DIALS`], routable first.
///
/// Private (LAN) addresses are deliberately **kept**: a group on one home network is the single
/// most common first invite, and dropping them would break it. The exposure is bounded by the
/// cap and by the fact that a LAN address the author chose can only aim at the reader's own
/// segment, which the reader can already scan.
fn dialable_bootstrap(bootstrap: &[String]) -> Vec<Multiaddr> {
    let parsed: Vec<Multiaddr> = bootstrap
        .iter()
        .filter_map(|s| s.parse::<Multiaddr>().ok())
        .filter(|a| !addr_is_undialable(a))
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
                bootstrap.insert(0, a)
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
                bootstrap.insert(0, addr)
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
            eprintln!("reachability: could not offer {addr} to AutoNAT: {e}");
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
    timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(p)) = mesh.next_event().await {
                if p == relay_peer {
                    break;
                }
            }
        }
    })
    .await
    .map_err(|_| "could not connect to the relay".to_string())?;
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
    timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(p)) = mesh.next_event().await {
                if p == rz_peer {
                    break;
                }
            }
        }
    })
    .await
    .map_err(|_| "could not connect to the rendezvous".to_string())?;
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
            eprintln!("persist: sealing the network identity of server {server} failed: {e}");
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
                    eprintln!("reload: the network identity of server {server} did not load: {e}");
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
    fold_bootstrap_entry(app, server, &entry, add).await
}

/// Add or remove one exact dial address from this member's live peer record. Router mappings use a
/// bare transport address plus our peer id; relay circuit addresses already contain both relay and
/// local peer ids and therefore call this helper directly.
async fn fold_bootstrap_entry(app: &AppHandle, server: u64, entry: &str, add: bool) -> bool {
    let entry = entry.to_string();
    let state = app.state::<AppState>();
    let changed_and_actor = {
        let mut servers = state.inner().servers.lock().await;
        let Some(server_entry) = servers.get_mut(&server) else {
            return false;
        };
        let changed = if add {
            if server_entry.bootstrap.contains(&entry) {
                false
            } else {
                // Public router mappings beat LAN and loopback candidates during address racing.
                server_entry.bootstrap.insert(0, entry.clone());
                true
            }
        } else if let Some(index) = server_entry
            .bootstrap
            .iter()
            .position(|item| item == &entry)
        {
            server_entry.bootstrap.remove(index);
            true
        } else {
            false
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
        let _ = app.emit("reachability-changed", server);
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
    mapping_owned: &mut HashSet<Multiaddr>,
) {
    let (next, next_unavailable) = port_mapping_snapshot_state(snapshot);

    // Remove a replaced/expired address only if no new lease still names it. Multiple protocols
    // can own the same socket, and a baseline manual address is absent from `mapping_owned`.
    for (key, old) in active.iter() {
        if next.get(key) != Some(old)
            && !next.values().any(|candidate| candidate == old)
            && mapping_owned.remove(old)
        {
            fold_mapped_bootstrap(app, server, peer_id, old, false).await;
        }
    }
    for (key, address) in &next {
        if active.get(key) != Some(address)
            && fold_mapped_bootstrap(app, server, peer_id, address, true).await
        {
            mapping_owned.insert(address.clone());
        }
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
    tokio::spawn(async move {
        let deadline = SystemClock.sleep(Duration::from_secs(PORT_MAPPING_WINDOW_SECS));
        tokio::pin!(deadline);
        let mut waiting = true;
        let mut active = HashMap::new();
        let mut unavailable = HashMap::new();
        // Only remove addresses this collector inserted. A manual forward can intentionally be
        // identical to a router mapping and remains valid after that mapping lease expires.
        let mut mapping_owned = HashSet::new();
        let initial = rx.borrow_and_update().clone();
        apply_port_mapping_snapshot(
            &app,
            server,
            &peer_id,
            initial,
            &mut active,
            &mut unavailable,
            &mut mapping_owned,
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
                    &mut mapping_owned,
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
        fold_bootstrap_entry(app, server, expired, false).await;
    }
    for available in next.difference(previous) {
        fold_bootstrap_entry(app, server, available, true).await;
    }
    *previous = next;
}

/// Keep relay reservation addresses honest after the synchronous initial reservation. A relay
/// listener can expire or be re-created later; the watch snapshot ensures Settings, invites and
/// peer records all withdraw/add the same exact circuit address.
fn spawn_relay_fold(app: AppHandle, server: u64, mut rx: watch::Receiver<RelayAddressSnapshot>) {
    tokio::spawn(async move {
        let mut previous = HashSet::new();
        let initial = rx.borrow_and_update().clone();
        apply_relay_snapshot(&app, server, initial, &mut previous).await;
        while rx.changed().await.is_ok() {
            let snapshot = rx.borrow_and_update().clone();
            apply_relay_snapshot(&app, server, snapshot, &mut previous).await;
            let _ = app.emit("reachability-changed", server);
        }
    });
}

fn spawn_mesh_observation_fold(
    app: AppHandle,
    server: u64,
    mut rx: watch::Receiver<MeshObservationSnapshot>,
) {
    tokio::spawn(async move {
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
                let _ = app.emit("reachability-changed", server);
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
    });
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
        let _ = app.emit("reachability-changed", server);
    }
}

/// Collect coalesced per-address AutoNAT v2 evidence. The product filters it against live routes
/// only when read, closing startup-order and expiry races without an unbounded output queue.
fn spawn_autonat_fold(app: AppHandle, server: u64, mut rx: watch::Receiver<AutoNatSnapshot>) {
    tokio::spawn(async move {
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
async fn discover_and_connect(
    invite: &InviteToken,
    net: &ServerNet,
    expected_inviter: Option<PeerId>,
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
    timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(p)) = mesh.next_event().await {
                if rz_peers.contains(&p) {
                    break;
                }
            }
        }
    })
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
    })?;
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
            candidates.push(Candidate {
                peer: d.peer.to_bytes(),
                addresses: d.addresses.iter().map(|a| a.to_string()).collect(),
                source: Source::Rendezvous(root.clone()),
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
    for a in dialed.addresses.iter().chain(fallbacks.iter()) {
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
    timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(p)) = mesh.next_event().await {
                if p == inviter {
                    break;
                }
            }
        }
    })
    .await
    .map_err(|_| {
        steps.push(DiagStep::failed(
            "connect",
            "",
            "none of the dialled addresses answered within 20s",
        ));
        "timed out connecting to the discovered server".to_string()
    })?;
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
#[tauri::command]
async fn found_server(
    app: AppHandle,
    state: State<'_, AppState>,
    display_name: String,
    advertise: String,
    relay: String,
    rendezvous: String,
    is_dm: bool,
) -> Result<FoundResult, String> {
    require_unlocked_session(&state).await?;
    let mut diag = Connectivity {
        action: "found".into(),
        subject: display_name.clone(),
        at: SystemClock.now_ms(),
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
        &mut diag,
    )
    .await;
    // The verbatim error is the point: the connectivity panel shows exactly what the code said,
    // so a user can paste it rather than paraphrase it.
    if let Err(e) = &out {
        diag.last_error.clone_from(e);
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
    let name = display_name.clone();
    let mut server = Server::found(
        mesh,
        device,
        OsCryptoRng,
        Box::new(SystemClock),
        display_name,
    )
    .map_err(|e| e.to_string())?;
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
        eprintln!("found: publishing the peer record failed: {e}");
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
    let (actor, events, _task) = spawn(server);
    actor.open_channel(general).await;
    let channels = ui_channels(actor.channels().await);
    let server_id = register_server(
        app,
        state,
        actor,
        events,
        group_id,
        device_id,
        Some(invite_hex),
        name,
        bootstrap,
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
    timeout(within, async {
        loop {
            match mesh.next_event().await {
                Some(TransportEvent::PeerConnected(peer)) if peer == wanted => return,
                Some(_) => {}
                None => std::future::pending::<()>().await,
            }
        }
    })
    .await
    .map_err(|_| ())
}

/// Wait for whichever peer answers a reply-code dial-back.  The reply candidate is public and
/// may be applied by either the named inviter or an existing member helper; the subsequent MLS
/// path distinguishes them and never grants the helper admission authority.
async fn wait_for_reply_peer(
    mesh: &MeshService,
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
    timeout(within, async {
        loop {
            match mesh.next_event().await {
                Some(TransportEvent::PeerConnected(peer)) => {
                    if let Some(expires_at_ms) = allowed.get(&peer).copied() {
                        if SystemClock.now_ms() <= expires_at_ms {
                            return (peer, expires_at_ms);
                        }
                    }
                }
                Some(_) => {}
                None => std::future::pending::<()>().await,
            }
        }
    })
    .await
    .map_err(|_| ())
}

#[tauri::command]
async fn join_server(
    app: AppHandle,
    state: State<'_, AppState>,
    invite_hex: String,
    display_name: String,
    is_dm: bool,
    allow_switchboards: bool,
) -> Result<FoundResult, String> {
    require_unlocked_session(&state).await?;
    let mut diag = Connectivity {
        action: "join".into(),
        at: SystemClock.now_ms(),
        ..Default::default()
    };
    let out = join_server_inner(
        &app,
        &state,
        invite_hex,
        display_name,
        is_dm,
        allow_switchboards,
        &mut diag,
    )
    .await;
    if let Err(e) = &out {
        diag.last_error.clone_from(e);
    }
    *state.diag.lock().await = diag;
    out
}

/// The body of [`join_server`], split out so every exit records the attempt for the connectivity
/// panel (the exits that used to say nothing are the whole reason that panel exists).
async fn join_server_inner(
    app: &AppHandle,
    state: &AppState,
    invite_hex: String,
    display_name: String,
    is_dm: bool,
    allow_switchboards: bool,
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
        let fallback_rz_config: Vec<_> = targets
            .iter()
            .map(|target| (target.addr.to_string(), target.peer.to_bytes()))
            .collect();
        match discover_and_connect(&invite, &net, plan_inviter, &mut diag.steps).await {
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
        let addrs = dialable_bootstrap(&invite.bootstrap);
        // The dropped entries are worth naming: "the invite listed three addresses and every one
        // was loopback or otherwise undialable" is a diagnosis; a bare empty list is not.
        let dropped = invite.bootstrap.len().saturating_sub(addrs.len());
        if dropped > 0 {
            diag.steps.push(DiagStep::failed(
                "dial",
                "",
                format!("{dropped} address(es) in the invite were unusable and were not dialled"),
            ));
        }
        if addrs.is_empty() && !use_switchboards {
            return Err("invite carries no usable bootstrap address".to_string());
        }
        let inviter = match addrs.iter().find_map(target_peer_in_multiaddr) {
            Some(peer) => phase0_peer_id(&peer),
            None => plan_inviter.ok_or_else(|| {
                "invite has neither a direct inviter route nor a pinned assisted inviter"
                    .to_string()
            })?,
        };
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
                app.emit("join-reply-ready", &ready)
                    .map_err(|e| e.to_string())?;
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
    let name = display_name.clone();
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
        Server::join_from_reply(
            mesh,
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
        eprintln!("join: publishing the peer record failed: {e}");
    }

    let general = channel_id("general");
    let group_id = server.group_id();
    let device_id = server.device_id();
    let (actor, events, _task) = spawn(server);
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
    let server_id = register_server(
        app,
        state,
        actor,
        events,
        group_id,
        device_id,
        None,
        name,
        joiner_addrs,
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
                if !current || mesh.dial_join_candidates(&targets).await.is_err() {
                    return;
                }
                // A socket alone proves nothing: send the code-holder proof over the Noise
                // connection before the retained joiner reveals its bearer invite/KeyPackage.
                let mut proof_request = Vec::with_capacity(33);
                proof_request.push(JOIN_REPLY_PROOF_KIND);
                proof_request.extend_from_slice(&proof);
                let _ = mesh
                    .request_control(phase0_peer_id(&joiner), bytes::Bytes::from(proof_request))
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
    state.storage_health.lock().await.remove(&server);
    state.upnp.lock().await.remove(&server);
    state.autonat.lock().await.remove(&server);
    state.mesh_observations.lock().await.remove(&server);
    state
        .join_replies
        .lock()
        .await
        .retain(|(candidate_server, _), _| *candidate_server != server);
    if let Some(entry) = state.servers.lock().await.remove(&server) {
        entry.actor.shutdown().await;
    }
    // Drop the sealed snapshot + re-seal the (now smaller) registry.
    {
        let guard = state.store.lock().await;
        if let Some(store) = guard.as_ref() {
            if let Err(e) = store.remove_server(server) {
                eprintln!("leave: removing sealed server {server} failed: {e}");
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
            eprintln!("get_invite: re-mint after a reachability change failed: {e}");
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
                eprintln!("mint_invite_fresh: lifting outstanding evictions failed: {e}");
            }
        }
        Err(e) => eprintln!("mint_invite_fresh: no actor to lift evictions on: {e}"),
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
            icon: String::new(),
            cursor: String::new(),
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
    })
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

/// Share a file (base64-encoded bytes); returns its content-address hex.
#[allow(clippy::too_many_arguments)] // Tauri injects app/state; the remaining fields are one file form.
#[tauri::command]
async fn add_file(
    app: AppHandle,
    state: State<'_, AppState>,
    server: u64,
    name: String,
    mime: String,
    path: String,
    data: String,
    upload_id: Option<String>,
) -> Result<String, String> {
    let bytes = B64
        .decode(data.as_bytes())
        .map_err(|e| format!("bad file data: {e}"))?;
    let actor = actor_of(&state, server).await?;
    let (progress, progress_task) = match upload_id {
        Some(upload_id) => {
            let (tx, mut rx) = mpsc::channel(64);
            let task = tokio::spawn(async move {
                while let Some((done, total)) = rx.recv().await {
                    if require_unlocked_session(&app.state::<AppState>())
                        .await
                        .is_err()
                    {
                        continue;
                    }
                    let _ = app.emit(
                        "upload-progress",
                        UploadProgressEvt {
                            server,
                            upload_id: upload_id.clone(),
                            done,
                            total,
                        },
                    );
                }
            });
            (Some(tx), Some(task))
        }
        None => (None, None),
    };
    let result = actor
        .add_file_with_progress(name, mime, path, bytes, progress)
        .await;
    // The actor drops its progress sender when the command completes. Drain every queued event
    // before resolving the invoke so the frontend sees the bar advance before it paints Done.
    if let Some(task) = progress_task {
        let _ = task.await;
    }
    let cid = result?;
    persist_server(&state, server).await;
    Ok(cid)
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
    // Hold the dedicated cache lock across the actor round-trip. Simultaneous callers therefore
    // coalesce into one scan instead of both missing the cache and hammering the blob store.
    let mut cache = state.storage_health.lock().await;
    if let Some(report) = cache.get(&server) {
        return Ok(report.clone());
    }
    let actor = actor_of(&state, server).await?;
    let health = actor.storage_health().await;
    let report = storage_report(&actor, health, SystemClock.now_ms()).await;
    cache.insert(server, report.clone());
    Ok(report)
}

/// Ask authenticated peers for every missing/unreadable chunk, then verify the complete set.
#[tauri::command]
async fn repair_storage(
    state: State<'_, AppState>,
    server: u64,
) -> Result<UiStorageRepair, String> {
    let mut cache = state.storage_health.lock().await;
    let actor = actor_of(&state, server).await?;
    let repaired = actor.repair_storage().await?;
    let health = storage_report(&actor, repaired.health, SystemClock.now_ms()).await;
    cache.insert(server, health.clone());
    Ok(UiStorageRepair {
        attempted_chunks: repaired.attempted_chunks,
        recovered_chunks: repaired.recovered_chunks,
        health,
    })
}

/// The fingerprints of members reachable right now (presence indicators in the roster).
#[tauri::command]
async fn get_online_members(
    state: State<'_, AppState>,
    server: u64,
) -> Result<Vec<String>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor.online_members().await)
}

/// Delivery state for this device's recent messages in a channel; the seed a UI paints on open,
/// before the throttled `delivery-changed` event next fires. Empty until this session sends a
/// message (the message-id → change mapping is not persisted across a restart).
#[tauri::command]
async fn get_delivery(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
) -> Result<Vec<DeliveryStateEvt>, String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    Ok(delivery_payload(actor.delivery_snapshot(id).await))
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
#[tauri::command]
async fn send_call_signal(
    state: State<'_, AppState>,
    server: u64,
    target_fp: String,
    payload: String,
) -> Result<bool, String> {
    let bytes = B64
        .decode(payload.as_bytes())
        .map_err(|e| format!("bad payload: {e}"))?;
    let actor = actor_of(&state, server).await?;
    actor.send_call_signal(target_fp, bytes).await
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

/// Download a shared file by content-address hex; returns base64-encoded bytes. Fetches the file
/// ONE chunk per actor command (emitting `download-progress` after each), so a large download no
/// longer freezes the server actor; the actor returns to its loop between chunks and interleaves
/// other commands + network sync. The whole reassembled file is verified against the requested
/// content address (defends against a malicious manifest whose chunks individually verify).
#[tauri::command]
async fn download_file(
    app: AppHandle,
    state: State<'_, AppState>,
    server: u64,
    cid: String,
) -> Result<String, String> {
    let raw = hex::decode(cid.trim()).map_err(|e| format!("bad cid: {e}"))?;
    let target: [u8; 32] = raw
        .clone()
        .try_into()
        .map_err(|_| "bad cid length".to_string())?;
    let actor = actor_of(&state, server).await?;
    let (total, size) = actor.file_download_plan(raw.clone()).await.ok_or_else(|| {
        "this file can't be downloaded; it isn't listed, or its reference is invalid".to_string()
    })?;
    require_unlocked_session(&state).await?;
    let _ = app.emit(
        "download-progress",
        DownloadProgressEvt {
            server,
            cid: cid.clone(),
            done: 0,
            total,
            bytes_done: 0,
            bytes_total: size,
            network_bytes_done: 0,
            provider: None,
        },
    );
    let mut out = Vec::with_capacity(size as usize);
    let mut network_bytes_done = 0u64;
    for i in 0..total {
        // A transfer can outlive the click that started it. Do not return plaintext or continue
        // emitting file metadata after an explicit lock closes the webview session.
        require_unlocked_session(&state).await?;
        let (chunk, provider) = actor.fetch_file_chunk(raw.clone(), i).await?;
        if provider.is_some() {
            network_bytes_done = network_bytes_done.saturating_add(chunk.len() as u64);
        }
        out.extend_from_slice(&chunk);
        let _ = app.emit(
            "download-progress",
            DownloadProgressEvt {
                server,
                cid: cid.clone(),
                done: i + 1,
                total,
                bytes_done: out.len() as u64,
                bytes_total: size,
                network_bytes_done,
                provider,
            },
        );
    }
    if Cid::of(&out).as_bytes() != &target {
        return Err("the reassembled file failed its integrity check".into());
    }
    require_unlocked_session(&state).await?;
    Ok(B64.encode(&out))
}

/// Post to the server status feed.
#[tauri::command]
async fn post_status(state: State<'_, AppState>, server: u64, text: String) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.post_status(text).await;
    persist_server(&state, server).await;
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

/// Create a server event; re-seals the server. **Any member may**; an event is server content,
/// like a channel or a status post. Rejected with a message when the title is blank or over 120
/// UTF-8 bytes, the body is over 1024, or the end time precedes the start (`endTs: 0` = no end).
/// `image` is the hex content address of an already-shared file (empty for none), checked for
/// shape only: the blob is fetched over the file path like any other embed.
/// An `events-changed` event follows, so the UI re-reads the calendar.
#[tauri::command]
async fn create_event(
    state: State<'_, AppState>,
    server: u64,
    title: String,
    body: String,
    start_ts: u64,
    end_ts: u64,
    image: String,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor
        .create_event(title, body, start_ts, end_ts, image)
        .await?;
    persist_server(&state, server).await;
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
    let _ = app.emit("switchboard-changed", server);
    get_switchboard_status(state, server).await
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
    require_unlocked_session(&state).await?;
    validate_ui_state_json(&json)?;
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
) -> Result<bool, String> {
    let actor = actor_of(&state, server).await?;
    let queued = actor.write_wiki_page(name, body).await?;
    persist_server(&state, server).await;
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

/// Send a chat message to a channel (by id).
#[tauri::command]
async fn send_message(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    text: String,
    reply_to: Option<String>,
) -> Result<(), String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    actor
        .send_reply(id, text, reply_to.unwrap_or_default())
        .await?;
    persist_server(&state, server).await;
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
) -> Result<(), String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    actor.edit_message(id, msg_id, text).await?;
    persist_server(&state, server).await;
    Ok(())
}

/// Delete one of your own messages (by message id) from a channel.
#[tauri::command]
async fn delete_message(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    msg_id: String,
) -> Result<(), String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    actor.delete_message(id, msg_id).await?;
    persist_server(&state, server).await;
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
) -> Result<(), String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    actor.toggle_reaction(id, msg_id, emoji).await?;
    persist_server(&state, server).await;
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
) -> Result<(), String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    actor.set_pin(id, msg_id, pinned).await?;
    persist_server(&state, server).await;
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
) -> Result<(), String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    actor.set_channel_topic(id, topic).await?;
    persist_server(&state, server).await;
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
) -> Result<String, String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    let entry = actor.jukebox_add(id, cid, name).await?;
    persist_server(&state, server).await;
    Ok(entry)
}

/// Remove a jukebox entry (by entry id) from a channel; any member, and idempotent.
#[tauri::command]
async fn jukebox_remove(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    entry: String,
) -> Result<(), String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    actor.jukebox_remove(id, entry).await?;
    persist_server(&state, server).await;
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
        eprintln!("reload: server {} reachability: {p}", record.id);
    }
    let Reachability {
        bootstrap,
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

    let mut server = Server::restore(
        snapshot,
        mesh,
        OsCryptoRng,
        Box::new(SystemClock),
        &record.display_name,
    )
    .map_err(|e| e.to_string())?;
    server
        .subscribe_control()
        .await
        .map_err(|e| e.to_string())?;
    attach_blob_store(state, &mut server).await;
    // Republish this device's peer record on THIS launch's reserved sequence block (defect P1 and
    // the 1a-7 seq bug together). The identity and port are the same as last launch, but the
    // reachable address may not be (a new relay circuit, a different UPnP mapping), and a record
    // published from a number the peers have already seen is discarded by every one of them.
    if let Err(e) = server.publish_self_record(bootstrap.clone(), net.record_seq) {
        eprintln!("reload: publishing the peer record failed: {e}");
    }
    server.set_switchboard_offered(net.switchboard);
    // Restore the cross-session address cache: the previously-proven members this node can offer
    // the dial policy immediately, before any rendezvous has had a chance to answer with Sybils.
    // Best-effort; a missing, unreadable or tamper-detected cache just means no cached candidates.
    {
        let guard = state.store.lock().await;
        if let Some(store) = guard.as_ref() {
            match (
                store.address_cache_key(),
                store.load_address_cache(record.id),
            ) {
                (Ok(key), Ok(bytes)) if !bytes.is_empty() => {
                    if !server.load_address_cache(&bytes, &key) {
                        eprintln!(
                            "reload: the address cache of server {} was rejected",
                            record.id
                        );
                    }
                }
                (Err(e), _) | (_, Err(e)) => {
                    eprintln!(
                        "reload: the address cache of server {} did not load: {e}",
                        record.id
                    )
                }
                _ => {}
            }
        }
    }
    // Re-dial the last-known members now that the roster is loaded (the Phase 9g healing path).
    // `cache_known_records` folds the snapshot's restored peer records into the cache, pruning
    // anyone no longer on the roster, and `dial_cached_peers` runs the survivors through the
    // DiscoveryPolicy: routability-checked, ranked, and capped by the dial budget. The discovery
    // tick repeats this every minute or so; doing it eagerly here is only about reconnect latency.
    server.cache_known_records();
    let redialled = server.dial_cached_peers().await;
    if redialled > 0 {
        eprintln!(
            "reload: server {} re-dialled {redialled} known member(s)",
            record.id
        );
    }

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

    let general = channel_id("general");
    let group_id = server.group_id();
    let device_id = server.device_id();
    let (actor, events, _task) = spawn(server);
    actor.open_channel(general).await;
    // Register under the SAME id as on disk (don't allocate a new one).
    forward_events(app.clone(), record.id, events);
    spawn_discovery_timer(app.clone(), record.id, actor.clone());
    state.servers.lock().await.insert(
        record.id,
        ServerEntry {
            actor,
            group_id,
            device_id,
            invite: presented_invite,
            name: record.display_name.clone(),
            bootstrap,
            rendezvous: rz_vec,
            mesh: Some(mesh_handle),
            is_dm: record.is_dm,
            switchboard: net.switchboard,
            record_seq: net.record_seq,
        },
    );
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

/// Is this a new-issue URL on our own tracker? Split out from the command so the allowlist
/// itself is testable without launching a browser.
fn is_tracker_url(url: &str) -> bool {
    url.len() <= ISSUE_URL_MAX_BYTES
        && !url
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        && url.starts_with(ISSUE_URL_PREFIX)
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

/// Write without replacing an earlier download. `create_new` also closes the exists/write race.
fn write_download(downloads: &Path, requested_name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
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
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes) {
                    let _ = std::fs::remove_file(&path);
                    return Err(error.to_string());
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("too many copies of the guide already exist in Downloads".into())
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
async fn create_backup(app: AppHandle, state: State<'_, AppState>) -> Result<BackupResult, String> {
    require_unlocked_session(&state).await?;
    // Capture every actor first, without holding either state lock across its round trip.
    let servers: Vec<(u64, ServerActor, ServerRecord)> = {
        let servers = state.servers.lock().await;
        servers
            .iter()
            .map(|(id, entry)| {
                (
                    *id,
                    entry.actor.clone(),
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
        snapshots.push((*id, actor.snapshot().await?));
    }
    let records: Vec<ServerRecord> = servers.into_iter().map(|(_, _, record)| record).collect();

    let downloads = app
        .path()
        .download_dir()
        .map_err(|error| error.to_string())?;
    let destination = backup_destination(&downloads, SystemClock.now_ms())?;
    let (files, bytes) = {
        // Serialize persistence and the filesystem copy with every other vault write so the
        // exported registry and snapshots form one coherent point-in-time image.
        let guard = state.store.lock().await;
        let store = guard
            .as_ref()
            .ok_or_else(|| "unlock the vault before creating a backup".to_string())?;
        let mut rng = OsCryptoRng;
        for (id, snapshot) in snapshots {
            store
                .save_server(id, &snapshot, &mut rng)
                .map_err(|error| error.to_string())?;
        }
        store
            .save_registry(&records, &mut rng)
            .map_err(|error| error.to_string())?;
        copy_backup_tree(store.backup_source_dir(), &destination)?
    };
    let warning = reveal_path(&destination)
        .err()
        .map(|error| format!("The backup was created, but Downloads could not be opened: {error}"));
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
) -> Result<(), String> {
    require_unlocked_session(&state).await?;
    let current_secret = Zeroizing::new(current_secret);
    let new_secret = Zeroizing::new(new_secret);
    let guard = state.store.lock().await;
    let store = guard
        .as_ref()
        .ok_or_else(|| "unlock the vault before changing its secret".to_string())?;
    store
        .change_passphrase(
            current_secret.as_bytes(),
            new_secret.as_bytes(),
            &mut OsCryptoRng,
        )
        .map_err(|error| error.to_string())
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
    })
}

/// Persist a completed group-file transfer through the native shell. Unlike `<a download>`, this
/// has an observable result in a Tauri webview. The file is revealed, never executed automatically.
#[tauri::command]
async fn save_download(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    data_base64: String,
) -> Result<SavedFileResult, String> {
    require_unlocked_session(&state).await?;
    if data_base64.len() > MAX_FILE_BYTES * 2 {
        return Err("the downloaded file is unexpectedly large".into());
    }
    let bytes = B64
        .decode(data_base64)
        .map_err(|_| "the downloaded file could not be decoded".to_string())?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!(
            "file is larger than the {MAX_FILE_BYTES}-byte limit"
        ));
    }
    let downloads = app.path().download_dir().map_err(|e| e.to_string())?;
    let path = write_download(&downloads, &name, &bytes)?;
    let warning = reveal_path(&path)
        .err()
        .map(|error| format!("The file was saved, but Downloads could not be opened: {error}"));
    Ok(SavedFileResult {
        path: path.to_string_lossy().into_owned(),
        displayed: warning.is_none(),
        warning,
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

/// Unlock the on-disk store with `passphrase` and reload every persisted server. Called once
/// at launch. A wrong passphrase fails (the vault won't open); a first-ever launch just
/// creates the vault and returns no servers. Returns the reloaded servers for the rail.
#[tauri::command]
async fn unlock(
    app: AppHandle,
    state: State<'_, AppState>,
    passphrase: String,
) -> Result<Vec<ReloadedServer>, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("vault");
    let mut rng = OsCryptoRng;
    // Opening the vault verifies the passphrase (the DEK won't decrypt otherwise).
    let store =
        ServerStore::open(&dir, passphrase.as_bytes(), &mut rng).map_err(|e| e.to_string())?;

    // If the vault is already unlocked (e.g. a dev HMR re-mounted the frontend while the Rust
    // process kept running), don't reload from disk; that would spawn a duplicate actor +
    // transport per server. Return the servers already registered so the rail repopulates.
    if state.store.lock().await.is_some() {
        *state.session_resumable.lock().await = true;
        return Ok(running_servers(&state).await);
    }

    let records = store.load_registry().map_err(|e| e.to_string())?;

    // Restore the grant-ceremony ledger: a pairing request must stay single-use across a restart,
    // or re-pasting one would mint a second bundle. A corrupt/missing blob leaves an empty ledger
    // (the pre-persistence behaviour) rather than blocking unlock.
    match store.load_pairing_ledger() {
        Ok(bytes) if !bytes.is_empty() => match PairingLedger::restore(&bytes) {
            Ok(led) => *state.pairing_ledger.lock().await = led,
            Err(e) => eprintln!("unlock: the pairing ledger did not restore: {e}"),
        },
        Ok(_) => {}
        Err(e) => eprintln!("unlock: reading the pairing ledger failed: {e}"),
    }

    // Load every server's sealed snapshot up front, while we still own `store` locally.
    let snapshots: Vec<_> = records
        .iter()
        .map(|r| match store.load_server(r.id) {
            Ok(b) => Some(b),
            Err(e) => {
                eprintln!("unlock: loading server {} failed: {e}", r.id);
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
    for (record, snap) in records.iter().zip(snapshots.iter()) {
        let Some(bytes) = snap else { continue };
        if let Err(e) = reload_one(&app, &state, bytes, record).await {
            eprintln!("unlock: restoring server {} failed: {e}", record.id);
            continue;
        }
        reloaded.push(ReloadedServer {
            server: record.id,
            name: record.display_name.clone(),
            invite: record.invite.clone(),
            channel: channel_id("general").to_string(),
            channels: ui_channels(
                actor_of_unchecked(&state, record.id)
                    .await?
                    .channels()
                    .await,
            ),
            is_dm: record.is_dm,
        });
    }
    *state.session_resumable.lock().await = true;
    Ok(reloaded)
}

/// Restore an already-unlocked frontend after F5/HMR without asking for the vault passphrase
/// again. An explicit UI lock disables this path until `unlock` verifies the passphrase.
#[tauri::command]
async fn resume_session(state: State<'_, AppState>) -> Result<Option<Vec<ReloadedServer>>, String> {
    if !*state.session_resumable.lock().await || state.store.lock().await.is_none() {
        return Ok(None);
    }
    Ok(Some(running_servers(&state).await))
}

#[tauri::command]
async fn lock_session(
    state: State<'_, AppState>,
    ui_state_json: Option<String>,
) -> Result<(), String> {
    lock_session_inner(&state, ui_state_json).await
}

/// Testable core of the explicit lock operation. Closing the command boundary is unconditional:
/// malformed continuity state or a vault write failure is reported, but must never leave the
/// sensitive webview session open as a side effect of that error.
async fn lock_session_inner(state: &AppState, ui_state_json: Option<String>) -> Result<(), String> {
    // Save the final draft/read snapshot and close IPC as one ordered native operation. Two
    // separate fire-and-forget commands could race, causing the save to arrive after the lock and
    // be correctly rejected by the new session gate.
    let save_result = if let Some(json) = ui_state_json {
        match validate_ui_state_json(&json) {
            Err(error) => Err(error),
            Ok(()) => {
                let guard = state.store.lock().await;
                match guard.as_ref() {
                    Some(store) => store
                        .save_ui_state(json.as_bytes(), &mut OsCryptoRng)
                        .map_err(|error| error.to_string()),
                    None => Err("unlock the vault before saving UI state".to_string()),
                }
            }
        }
    } else {
        Ok(())
    };
    *state.session_resumable.lock().await = false;
    save_result
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
            eprintln!("persist: sealing the pairing ledger failed: {e}");
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
    let addrs: Vec<Multiaddr> = grant
        .bootstrap
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    if addrs.is_empty() {
        return Err(if grant.rendezvous.is_empty() {
            "this grant carries no usable address for that server".to_string()
        } else {
            "this grant is rendezvous-only; pairing needs a directly-dialable server".to_string()
        });
    }
    let contact_lp = addrs
        .iter()
        .find_map(target_peer_in_multiaddr)
        .ok_or_else(|| "grant address has no peer id".to_string())?;
    let contact = phase0_peer_id(&contact_lp);
    let (mesh, _id) = MeshService::new_tcp(None, &addrs).map_err(|e| e.to_string())?;
    let mesh_handle = mesh.handle();
    timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(p)) = mesh.next_event().await {
                if p == contact {
                    break;
                }
            }
        }
    })
    .await
    .map_err(|_| "timed out connecting to the server".to_string())?;

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
            eprintln!("pair: the grant's rendezvous addresses were rejected ({e}); pairing without steady-state discovery");
            Vec::new()
        }
    };
    if !rz_config.is_empty() {
        server.set_rendezvous_nodes(rz_config);
    }

    let general = channel_id("general");
    let group_id = server.group_id();
    let device_id = server.device_id();
    let (actor, events, _task) = spawn(server);
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
        group_id,
        device_id,
        None,
        name,
        Vec::new(),
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

/// The preference file. Its **presence** is the setting: a flag with no parser cannot be
/// corrupted into an unreadable state that silently turns logging on.
fn debug_flag_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(log_dir(app)?.join("debug-logging-enabled"))
}

/// The state of the debug log, as shown in Settings.
#[derive(Serialize, Clone)]
struct DebugLogging {
    /// Whether the preference is on right now.
    enabled: bool,
    /// Whether *this running process* is actually writing a file. A subscriber can only be
    /// installed once per process, so a toggle applies at the next launch, and saying so is the
    /// difference between a working setting and a user who thinks they captured a log.
    active: bool,
    /// The folder the log is written to, always shown so the user can go and get it.
    dir: String,
    /// The current session's file, when there is one.
    file: String,
}

/// Read the debug-logging preference. Never fails the caller: an unreadable app data directory
/// means "off", which is the safe answer for a privacy tool.
fn debug_logging_enabled(app: &AppHandle) -> bool {
    debug_flag_path(app).map(|p| p.exists()).unwrap_or(false)
}

/// Whether this process installed a debug-log file layer, and which file it is writing. Set once
/// in `setup`; read by `get_debug_logging` so the UI can distinguish "on" from "on since the
/// last restart".
struct LogState {
    /// Dropping this flushes the file, so it is held for the life of the process.
    _guard: catcoms_log::LogGuard,
    active: bool,
    dir: std::path::PathBuf,
}

/// The newest `debug_log_*.txt` in `dir`, so Settings can name the file this session is writing
/// rather than making the user guess which timestamp is theirs.
fn newest_log_file(dir: &std::path::Path) -> String {
    let mut best: Option<(std::time::SystemTime, String)> = None;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return String::new();
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("debug_log_") {
            continue;
        }
        let Ok(modified) = e.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, name));
        }
    }
    best.map(|(_, n)| n).unwrap_or_default()
}

/// The debug log's current state, for Settings.
#[tauri::command]
async fn get_debug_logging(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DebugLogging, String> {
    require_unlocked_session(&state).await?;
    let log = app.try_state::<LogState>();
    let active = log.as_ref().is_some_and(|l| l.active);
    // Prefer the directory this process actually opened, so the path shown is the path being
    // written to even if the app data directory moved under us.
    let dir = match log.as_ref() {
        Some(l) => l.dir.clone(),
        None => log_dir(&app)?,
    };
    Ok(DebugLogging {
        enabled: debug_logging_enabled(&app),
        active,
        dir: dir.display().to_string(),
        file: if active {
            newest_log_file(&dir)
        } else {
            String::new()
        },
    })
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
    let flag = debug_flag_path(&app)?;
    if enabled {
        std::fs::write(&flag, b"on").map_err(|e| e.to_string())?;
    } else if flag.exists() {
        std::fs::remove_file(&flag).map_err(|e| e.to_string())?;
    }
    get_debug_logging(app, state).await
}

/// Build and run the Tauri application.
pub fn run() {
    tauri::Builder::default()
        // Update checking lives in Rust so the download and its minisign verification never
        // touch the webview: the frontend only asks "is there one?" and "install it".
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState::default())
        // Install the tracing subscriber before anything interesting happens. Until this landed
        // the desktop app had **no** subscriber at all, so every `tracing::warn!` in the whole
        // protocol stack (including the five distinct reasons a join can be refused) was
        // discarded and there was no log file anywhere. Off by default: see `DebugLogging`, and
        // `catcoms_log`'s module docs for what an enabled log may contain.
        .setup(|app| {
            let handle = app.handle().clone();
            let enabled = debug_logging_enabled(&handle);
            let dir = log_dir(&handle).unwrap_or_else(|_| std::path::PathBuf::from("logs"));
            let guard = catcoms_log::init_debug_with(enabled, &dir, catcoms_log::APP_FILE_FILTER);
            app.manage(LogState {
                _guard: guard,
                active: enabled,
                dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_exists,
            resume_session,
            lock_session,
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
            rename_server,
            get_members,
            set_profile,
            get_profiles,
            set_livery,
            set_server_icon,
            set_server_cursor,
            get_livery,
            set_member_badge,
            get_badges,
            get_devices,
            add_file,
            get_files,
            get_storage_health,
            repair_storage,
            get_online_members,
            get_delivery,
            dm_stats,
            send_dm_invite,
            get_dm_requests,
            send_call_signal,
            call_media_key,
            dismiss_dm_request,
            download_file,
            file_available,
            delete_file,
            set_file_expiry,
            get_file_usage,
            get_wiki_pinned_cids,
            post_status,
            get_statuses,
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
            get_switchboard_status,
            set_switchboard_offered,
            get_ui_state,
            save_ui_state,
            create_backup,
            change_vault_secret,
            get_debug_logging,
            set_debug_logging,
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
            save_download
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Mewtual desktop app");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_libp2p_peer(n: u8) -> libp2p::PeerId {
        let mut seed = [21; 32];
        seed[0] = n;
        keypair_from_seed(seed).unwrap().public().to_peer_id()
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

        // Validation errors are returned to the caller, but the security boundary still closes.
        *state.session_resumable.lock().await = true;
        assert!(lock_session_inner(&state, Some("not json".into()))
            .await
            .is_err());
        assert_eq!(
            require_unlocked_session(&state).await.unwrap_err(),
            "the vault is locked"
        );
        // The vault remains mounted so actors and persistence may continue behind the UI lock.
        assert!(state.store.lock().await.is_some());
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

    /// Settings promises the user "this session's file is X". If that name is wrong they open
    /// somebody else's capture (or yesterday's) and send the wrong thing, so the newest-wins rule
    /// and the "only debug_log_* counts" rule are both pinned.
    #[test]
    fn the_named_log_file_is_the_newest_debug_log_in_the_folder() {
        let dir = std::env::temp_dir().join(format!("mewtual-logtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Nothing yet: an empty answer, never a guessed filename.
        assert_eq!(newest_log_file(&dir), "");
        // A missing folder is the same "nothing to name", not a panic.
        assert_eq!(newest_log_file(&dir.join("absent")), "");

        std::fs::write(dir.join("debug_log_20260819_100000.txt"), b"old").unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a log").unwrap();
        assert_eq!(newest_log_file(&dir), "debug_log_20260819_100000.txt");

        // Written second, so it is the newer file whatever the timestamps in the names say; the
        // rule is modification time, because that is what "this session's" actually means.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("debug_log_20260101_000000.txt"), b"new").unwrap();
        assert_eq!(newest_log_file(&dir), "debug_log_20260101_000000.txt");

        let _ = std::fs::remove_dir_all(&dir);
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

        // And the number actually dialled is capped well below the token's 64.
        let flood: Vec<String> = (1..=60)
            .map(|n| a(&format!("/ip4/203.0.113.{n}")))
            .collect();
        assert_eq!(dialable_bootstrap(&flood).len(), MAX_BOOTSTRAP_DIALS);
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
    fn downloaded_file_names_cannot_escape_downloads_or_use_device_names() {
        assert_eq!(safe_download_name("../../report?.pdf"), "report_.pdf");
        assert_eq!(safe_download_name(r"..\..\CON.txt"), "_CON.txt");
        assert_eq!(safe_download_name(" . "), "mewtual-download");
        assert_eq!(numbered_download_name("photo.png", 2), "photo (2).png");
        assert_eq!(numbered_download_name("README", 3), "README (3)");
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
        let file = |name: &str, cid: &str, mime: &str, size: u64, held: u32, total: u32| UiFile {
            name: name.into(),
            size,
            mime: mime.into(),
            cid: cid.into(),
            author: "member".into(),
            path: "shared".into(),
            held,
            total,
            expires: None,
            expires_known: false,
        };
        let report = build_storage_report(
            StorageHealth {
                listed_files: 3,
                referenced_chunks: 3,
                verified_chunks: 3,
                verified_bytes: 512,
                ..StorageHealth::default()
            },
            vec![
                file("cat.png", "aa", "image/png", 100, 1, 2),
                file("same-cat.png", "aa", "image/png", 100, 1, 2),
                file("song.ogg", "bb", "audio/ogg", 400, 2, 2),
            ],
            &HashSet::from(["aa".to_string()]),
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
}
