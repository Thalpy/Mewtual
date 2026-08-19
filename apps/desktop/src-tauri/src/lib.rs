//! The Tauri command/event bridge; a thin shell over the `catcoms-app` event-stream
//! actors. The frontend `invoke`s these commands and `listen`s for the forwarded events;
//! all the real work lives in the tested `catcoms-app` actor, which itself wraps the
//! protocol stack. The GUI never touches MLS or automerge.
//!
//! Multi-server (8p): the app can run several servers at once. Each is a separate
//! `Server`/actor (its own MLS group + transport + event stream); the bridge keys them by
//! a `u64` server id. Every command takes a `server` id selecting which one to act on, and
//! every forwarded event is tagged with its server id so the UI routes it correctly.

use std::collections::HashMap;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use catcoms_app::{
    channel_id, spawn, AppEvent, Cid, DeviceId, Livery, PairingLedger, PairingSecrets,
    PerServerGrant, Profile, Server, ServerActor, ServerNet, ServerRecord, ServerStore,
    MAX_AVATAR_BYTES, MAX_BANNER_BYTES, MAX_SERVER_CURSOR_BYTES, MAX_SERVER_ICON_BYTES,
};
use catcoms_discovery::{Candidate, DiscoveryPolicy, PolicyConfig, Source};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_net::{
    addr_is_loopback, addr_is_private, addr_is_undialable, keypair_from_seed, phase0_peer_id,
    target_peer_in_multiaddr, validate_invite_rendezvous_addrs, validate_operator_rendezvous_addrs,
    MeshHandle, MeshService, RendezvousTarget,
};
use catcoms_rt::{Clock, MeshTransport, OsCryptoRng, PeerId, RngCore, SystemClock, TransportEvent};
use catcoms_sync::join_namespace;
use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;

/// One running server: its actor handle, the single-use invite to share (founder only), and
/// its display name (kept here too so the registry can be re-sealed on disk, Phase 9f).
struct ServerEntry {
    actor: ServerActor,
    invite: Option<String>,
    name: String,
    /// The current reachable bootstrap addresses for this device, captured when the server was
    /// founded/reloaded. Reused to mint a *fresh* invite on demand (so it carries the live
    /// address, not a stale one). Empty for a joiner (only the owner mints).
    bootstrap: Vec<String>,
    /// The rendezvous infra multiaddrs this server registered at (if any), so a fresh on-demand
    /// invite is also discovery-enabled. Empty when the server uses direct bootstrap only. Not
    /// separately persisted; on reload it is recovered from the persisted invite's `rendezvous`.
    rendezvous: Vec<String>,
    /// A clonable handle to this server's live transport, kept so the bridge can register a
    /// freshly-minted invite's namespace at the rendezvous *after* the `Server` was moved into its
    /// actor. `None` for a joiner (never registers) or a server without rendezvous.
    mesh: Option<MeshHandle>,
    /// Whether this group is a 1:1 DM (shown behind the DMs circle) rather than a server.
    is_dm: bool,
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
    /// Per-server UPnP result, kept out of [`Connectivity`] because it lands *after* the attempt
    /// that started it returns (and possibly after the user has started another one). Keyed by
    /// server id so a late answer can never be reported against a different server's attempt.
    upnp: Mutex<HashMap<u64, String>>,
}

/// UPnP has not been asked yet (no transport window was taken).
const UPNP_NOT_ATTEMPTED: &str = "not attempted";
/// The mapping request is out and the router has not answered yet.
const UPNP_WAITING: &str = "waiting for the router";
/// The transport reported no usable IGD gateway (no UPnP, or a CGNAT'd one).
const UPNP_NO_GATEWAY: &str = "no usable gateway (no UPnP, or your ISP uses CGNAT)";
/// The router never answered inside [`UPNP_WINDOW_SECS`].
const UPNP_TIMED_OUT: &str = "the router did not answer in time";

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
/// Deliberately says nothing about "am I reachable from the internet". AutoNAT is not
/// implemented (`docs/design-zeroconf-reachability.md` rung 0c), so nothing here can dial this
/// node back, and a public address obtained from UPnP or a relay circuit is evidence, not proof.
/// The UI states that rather than showing a green tick the code cannot support.
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
    /// The UPnP result for `server` (filled in by `get_connectivity` from the per-server map).
    upnp: String,
    /// What the attempt did, oldest first.
    steps: Vec<DiagStep>,
    /// The last error, verbatim, so the user can copy exactly what the code said.
    last_error: String,
}

/// The origin-side pending ceremony: what `pairing_read` decoded and anchored, held so
/// `pairing_mint`/`pairing_decline` act on the approved request and nothing else.
struct PendingGrant {
    view: catcoms_app::PairingRequestView,
    origin: DeviceId,
}

/// Clone out the actor for `server` (so we never hold the servers lock across an await).
async fn actor_of(state: &AppState, server: u64) -> Result<ServerActor, String> {
    state
        .servers
        .lock()
        .await
        .get(&server)
        .map(|e| e.actor.clone())
        .ok_or_else(|| "unknown server".to_string())
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
/// seam (`catcoms_rt::OsCryptoRng`) rather than an ambient `rand::random`, so this file keeps to
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
            tokio::time::sleep(delay).await;
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
                AppEvent::EclipseChanged { caution } => {
                    let _ = app.emit("eclipse-changed", EclipseEvt { server, caution });
                }
                AppEvent::ConnectivityChanged { online } => {
                    let _ = app.emit("connectivity-changed", OnlineEvt { server, online });
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
    invite: Option<String>,
    name: String,
    bootstrap: Vec<String>,
    rendezvous: Vec<String>,
    mesh: Option<MeshHandle>,
    is_dm: bool,
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
            invite,
            name,
            bootstrap,
            rendezvous,
            mesh,
            is_dm,
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
    let actor = match actor_of(state, server).await {
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
    let Ok(actor) = actor_of(state, server).await else {
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
        .filter(|a| !addr_is_private(a))
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

/// How long to let the router answer a UPnP/NAT-PMP mapping request, in the background. SSDP/IGD
/// discovery on a consumer router routinely needs ten seconds or more, so the old four-second
/// window (which additionally only ran when the user had supplied neither an advertise address nor
/// a relay) lost that race far more often than it won it, and the one genuinely zero-config path
/// to reachability was wasted. It now always runs, never blocks founding, and folds its answer
/// into the stored bootstrap so every later invite carries it.
const UPNP_WINDOW_SECS: u64 = 25;

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
    net: &ServerNet,
    steps: &mut Vec<DiagStep>,
) -> (Reachability, Vec<String>) {
    let mut problems = Vec::new();
    let mut bootstrap = auto_bootstrap(port, peer_id);
    steps.push(DiagStep::ok(
        "listen",
        format!("port {port}"),
        format!(
            "bound IPv4 + IPv6 over TCP + QUIC; {} address(es) auto-detected",
            bootstrap.len()
        ),
    ));

    let advertise = net.advertise.trim();
    if !advertise.is_empty() {
        match build_advertised(advertise, port, peer_id) {
            // The user's own address is the authoritative one.
            Ok(a) => {
                steps.push(DiagStep::unknown(
                    "advertise",
                    a.clone(),
                    "the address you supplied; nothing here can test it from outside",
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

    // Optional rendezvous: connect to it and advertise our reachable address(es) on the raw mesh
    // (so the deferred registration can flush), keeping a handle to register each invite's
    // namespace after the server is spawned. The node is then discoverable with no hard-coded
    // address; a joiner needs only the pasted invite.
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

/// Dial the rendezvous node, wait for the connection, and advertise our routable addresses so the
/// deferred registration can flush. A relay *circuit* address is intentionally not advertised
/// here; it auto-promotes to an external address on reservation (in the transport actor), so the
/// rendezvous still learns it.
async fn connect_rendezvous(
    mesh: &MeshService,
    rendezvous: &str,
    bootstrap: &[String],
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
    for addr in external_addrs(bootstrap) {
        mesh.add_external_address(addr)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(rz)
}

/// Build this server's transport on its **own** persisted libp2p identity and stable port.
///
/// The identity is per server, never per device: Mewtual deliberately gives each server a separate
/// network identity so two servers cannot be correlated to the same person. Reusing it across
/// launches is what keeps an already-issued invite (which embeds `/p2p/<id>`) redeemable.
///
/// Returns the transport, the peer id, and the port actually in use (which differs from
/// `net.port` only when the saved port had to be abandoned; the caller persists it back).
fn build_transport(
    net: &ServerNet,
    dial: &[Multiaddr],
) -> Result<(MeshService, libp2p::PeerId, u16), String> {
    let key = keypair_from_seed(net.key_seed).map_err(|e| e.to_string())?;
    let port = choose_port(net);
    let (mesh, libp2p_id, bound) =
        MeshService::new_tcp_with_key(key, &listen_addrs(port), dial).map_err(|e| e.to_string())?;
    // With `port == 0` the OS assigned the number; read it back off whatever bound.
    let port = if port != 0 {
        port
    } else {
        bound.iter().find_map(listen_port).unwrap_or(0)
    };
    Ok((mesh, libp2p_id, port))
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

/// Watch for the UPnP mapping this node asked its router for, in the background, and fold the
/// public address it reports into the server's stored bootstrap list so every invite minted from
/// then on carries a directly-dialable address: no relay, no port-forward. Founding must not wait
/// on it (see [`UPNP_WINDOW_SECS`]), so the answer is collected out here instead.
fn spawn_upnp_fold(
    app: AppHandle,
    server: u64,
    mut rx: mpsc::Receiver<Option<Multiaddr>>,
    peer_id: String,
) {
    tokio::spawn(async move {
        let found = timeout(Duration::from_secs(UPNP_WINDOW_SECS), rx.recv()).await;
        // Nested Options: the outer is the timeout, the middle the channel, the inner the actor's
        // "no usable gateway" signal (sent promptly so we are not just waiting out the window).
        // Each of the three means something different to a user staring at a failed invite, so
        // the connectivity panel gets the distinction rather than a shrug.
        let state = app.state::<AppState>();
        let outcome = match &found {
            Ok(Some(Some(addr))) => addr.to_string(),
            Ok(Some(None)) => UPNP_NO_GATEWAY.to_string(),
            // The channel closed with the transport (the server was dropped mid-window).
            Ok(None) => UPNP_NOT_ATTEMPTED.to_string(),
            Err(_) => UPNP_TIMED_OUT.to_string(),
        };
        state
            .inner()
            .upnp
            .lock()
            .await
            .insert(server, outcome.clone());
        let Ok(Some(Some(addr))) = found else { return };
        let entry = format!("{addr}/p2p/{peer_id}");
        let (actor, bootstrap) = {
            let mut servers = state.inner().servers.lock().await;
            let Some(e) = servers.get_mut(&server) else {
                return;
            };
            if !e.bootstrap.contains(&entry) {
                // Front of the list: a public address beats the LAN and loopback entries beside it.
                e.bootstrap.insert(0, entry);
            }
            (e.actor.clone(), e.bootstrap.clone())
        };
        // The whole point of the mapping is that members can now reach us directly, and they only
        // learn that from a *fresher* peer record. Republishing on the next number in this
        // launch's block is what turns the UPnP result into something other members can act on;
        // without it the router opened a port nobody was ever told about.
        if let Some(seq) = next_record_seq(state.inner(), server).await {
            actor.publish_self_record(bootstrap, seq).await;
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
fn decode_and_verify_invite(invite_hex: &str) -> Result<InviteToken, String> {
    let bytes = hex::decode(invite_hex.trim()).map_err(|e| e.to_string())?;
    let invite = InviteToken::decode(&bytes).map_err(|e| e.to_string())?;
    if !invite.verify_self() {
        return Err("this invite's signature is not valid; it may have been altered or forged, so nothing was contacted".into());
    }
    Ok(invite)
}

/// The discover-on-join path (no hard-coded inviter address): build a transport, dial the invite's
/// rendezvous node(s), discover the inviter's records under the pre-join namespace, rank them
/// through the [`DiscoveryPolicy`] (never auto-dial), then dial the chosen addresses; plus the
/// invite's `bootstrap` addrs as direct fallbacks; and return the connected transport + the
/// inviter's peer id. Mirrors `tcp_rendezvous_e2e.rs`.
async fn discover_and_connect(
    invite: &InviteToken,
    net: &ServerNet,
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
    let (mesh, libp2p_id, port) = build_transport(net, &rz_addrs)?;
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
    let (mesh, libp2p_id, port) = build_transport(&net, &relay_dial)?;
    net.port = port;
    let id = libp2p_id.to_string();

    // Everything that makes us reachable from off this machine. `reload_one` runs the very same
    // helper, so a restart reproduces this reachability instead of collapsing to loopback.
    let (reach, problems) = establish_reachability(&mesh, &id, port, &net, &mut diag.steps).await;
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
    // Take the UPnP channel before the transport disappears into the server, so a background task
    // can wait out a realistic router-discovery window without holding up the UI.
    let upnp_rx = mesh.take_external_addrs().await;
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
    let (actor, events, _task) = spawn(server);
    actor.open_channel(general).await;
    let channels = ui_channels(actor.channels().await);
    let server_id = register_server(
        app,
        state,
        actor,
        events,
        Some(invite_hex),
        name,
        bootstrap,
        rz_vec,
        rz_handle,
        is_dm,
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
    if let Some(rx) = upnp_rx {
        state
            .upnp
            .lock()
            .await
            .insert(server_id, UPNP_WAITING.to_string());
        spawn_upnp_fold(app.clone(), server_id, rx, id);
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
#[tauri::command]
async fn join_server(
    app: AppHandle,
    state: State<'_, AppState>,
    invite_hex: String,
    display_name: String,
    is_dm: bool,
) -> Result<FoundResult, String> {
    let mut diag = Connectivity {
        action: "join".into(),
        at: SystemClock.now_ms(),
        ..Default::default()
    };
    let out = join_server_inner(&app, &state, invite_hex, display_name, is_dm, &mut diag).await;
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
    diag: &mut Connectivity,
) -> Result<FoundResult, String> {
    let invite = decode_and_verify_invite(&invite_hex).inspect_err(|e| {
        diag.steps.push(DiagStep::failed("invite", "", e.clone()));
    })?;
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

    // A joiner gets its own per-server identity + stable port too: it is a full member afterwards,
    // so its peer record has to keep resolving across restarts exactly like the founder's.
    let mut net = new_server_net("", "", "");

    // If the invite points at a rendezvous, discover the inviter there (no hard-coded address);
    // otherwise dial the invite's bootstrap addresses directly (loopback / LAN / relayed).
    let (mesh, inviter, rz_config) = if !invite.rendezvous.is_empty() {
        let (mesh, inviter, rz_config, port) =
            discover_and_connect(&invite, &net, &mut diag.steps).await?;
        net.port = port;
        (mesh, inviter, rz_config)
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
        if addrs.is_empty() {
            return Err("invite carries no usable bootstrap address".to_string());
        }
        let inviter_lp = addrs
            .iter()
            .find_map(target_peer_in_multiaddr)
            .ok_or_else(|| "bootstrap has no peer id".to_string())?;
        let inviter = phase0_peer_id(&inviter_lp);
        let (mesh, _id, port) = build_transport(&net, &addrs)?;
        net.port = port;
        // The transport dials these itself, concurrently, so a per-address outcome is not
        // observable here; what IS observable is which were tried and whether any answered.
        for a in &addrs {
            diag.steps
                .push(DiagStep::unknown("dial", a.to_string(), "dialled"));
        }
        // Wait for the connection to the inviter before requesting the join.
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
            diag.steps.push(DiagStep::failed(
                "connect",
                "",
                "none of the dialled addresses answered within 20s",
            ));
            "timed out connecting to the server".to_string()
        })?;
        diag.steps
            .push(DiagStep::ok("connect", "", "connected to the server"));
        (mesh, inviter, Vec::new())
    };

    let device = MlsDevice::generate().map_err(|e| e.to_string())?;
    let name = display_name.clone();
    // Past this point the transport is up, so a failure here is the MLS admission itself: the
    // one the serving node's join log explains and this side cannot.
    let mut server = Server::join(
        mesh,
        device,
        OsCryptoRng,
        Box::new(SystemClock),
        display_name,
        inviter,
        &invite,
    )
    .await
    .map_err(|e| {
        let msg = e.to_string();
        diag.steps.push(DiagStep::failed(
            "join",
            "",
            format!("{msg}; the server refused the join, and only the serving node knows why: ask its operator to read Server settings / Join log"),
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
    // record exactly like the founder does (defect P1). It mints no invites and therefore keeps no
    // `bootstrap` list, so the addresses are the auto-detected ones; the non-routable entries are
    // stripped inside `publish_self_record`.
    let joiner_addrs = match peer_id_of(&net) {
        Ok(id) => auto_bootstrap(net.port, &id),
        Err(e) => {
            eprintln!("join: could not derive the local peer id: {e}");
            Vec::new()
        }
    };
    diag.advertised.clone_from(&joiner_addrs);
    if let Err(e) = server.publish_self_record(joiner_addrs, net.record_seq) {
        eprintln!("join: publishing the peer record failed: {e}");
    }

    let general = channel_id("general");
    let (actor, events, _task) = spawn(server);
    actor.catch_up_channel_index(inviter).await;
    actor.open_channel(general).await;
    actor.catch_up(inviter, general).await;
    actor.catch_up_profiles(inviter).await;
    actor.catch_up_livery(inviter).await;
    actor.catch_up_badges(inviter).await;
    actor.catch_up_files(inviter).await;
    actor.catch_up_status(inviter).await;
    actor.catch_up_calendar(inviter).await;
    actor.catch_up_wiki(inviter).await;
    actor.catch_up_roles(inviter).await;
    let channels = ui_channels(actor.channels().await);
    // A joiner mints no invites (owner-scoped), so it carries no bootstrap/rendezvous of its own.
    let server_id = register_server(
        app,
        state,
        actor,
        events,
        None,
        name,
        Vec::new(),
        Vec::new(),
        None,
        is_dm,
        net.record_seq,
    )
    .await;
    // Seal the joined server, its network identity and the registry to disk (if unlocked).
    persist_server(state, server_id).await;
    persist_server_net(state, server_id, &net).await;
    persist_registry(state).await;
    diag.server = server_id;
    Ok(FoundResult {
        server: server_id,
        channel: general.to_string(),
        channels,
        is_dm,
    })
}

/// Leave a server: shut down its actor and drop it from the registry.
#[tauri::command]
async fn leave_server(state: State<'_, AppState>, server: u64) -> Result<(), String> {
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
async fn get_channels(
    state: State<'_, AppState>,
    server: u64,
) -> Result<Vec<UiChannel>, String> {
    Ok(ui_channels(actor_of(&state, server).await?.channels().await))
}

/// Does `current` offer a way to reach us that `minted` does not?
///
/// An `InviteToken` signs its bootstrap list, so an address learned after minting cannot be
/// patched in: the invite has to be re-minted or it is simply wrong. This is the predicate that
/// decides when that is worth doing, and it is deliberately one-directional. Losing an address
/// (a relay circuit that dropped) does not make an invite stale, because the remaining addresses
/// still work and re-minting would invalidate nothing and churn a nonce for no one's benefit.
/// Gaining one does, because the invite is now strictly less useful than the node it points at.
fn invite_missing_addresses(minted: &[String], current: &[String]) -> bool {
    current.iter().any(|a| !minted.contains(a))
}

/// The single-use invite to share (founder only); `None` for a joiner.
///
/// Re-mints first if reachability improved since the stored invite was made. This exists because
/// of a real ordering trap: founding mints the invite immediately, but UPnP takes seconds to
/// answer, so the very invite a user naturally copies was the one that lacked the public address
/// the router had just opened. A friend on another network then got "timed out connecting to the
/// server" and every fix underneath looked broken.
///
/// Doing it here rather than in the UPnP path is what makes it general: every display path goes
/// through this command, so *any* later reachability gain (UPnP, a relay circuit, a rendezvous
/// registration) produces a correct invite the next time anyone looks, with no event to miss and
/// no frontend change to keep in step.
#[tauri::command]
async fn get_invite(state: State<'_, AppState>, server: u64) -> Result<Option<String>, String> {
    let (stored, current) = {
        let servers = state.servers.lock().await;
        match servers.get(&server) {
            Some(e) => (e.invite.clone(), e.bootstrap.clone()),
            None => return Ok(None),
        }
    };
    let Some(hex_invite) = stored else {
        return Ok(None); // a joiner mints nothing
    };
    let stale = hex::decode(&hex_invite)
        .ok()
        .and_then(|b| InviteToken::decode(&b).ok())
        .is_some_and(|t| invite_missing_addresses(&t.bootstrap, &current));
    if !stale {
        return Ok(Some(hex_invite));
    }
    // Best effort: a re-mint that fails (no longer owner, actor gone) must not lose the invite
    // the caller already has, so fall back to it rather than surfacing an error here.
    match mint_and_store_invite(&state, server).await {
        Ok(fresh) => Ok(Some(fresh)),
        Err(e) => {
            eprintln!("get_invite: re-mint after a reachability change failed: {e}");
            Ok(Some(hex_invite))
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
    let (bootstrap, rendezvous, handle) = {
        let servers = state.servers.lock().await;
        let e = servers
            .get(&server)
            .ok_or_else(|| "unknown server".to_string())?;
        (e.bootstrap.clone(), e.rendezvous.clone(), e.mesh.clone())
    };
    let actor = actor_of(&state, server).await?;
    let mut nonce = [0u8; 16];
    let mut rng = OsCryptoRng;
    rng.fill_bytes(&mut nonce);
    let expires = SystemClock.now_ms() + 3_600_000; // single-use, valid for 1 hour
    let encoded = if rendezvous.is_empty() {
        actor.mint_invite(nonce, expires, bootstrap).await?
    } else {
        let encoded = actor
            .mint_invite_with_rendezvous(nonce, expires, bootstrap, rendezvous.clone())
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
    };
    let invite_hex = hex::encode(encoded);
    if let Some(e) = state.servers.lock().await.get_mut(&server) {
        e.invite = Some(invite_hex.clone());
    }
    persist_registry(&state).await;
    Ok(invite_hex)
}

/// Rename a server; a **local** display label in this client's rail (server names are not
/// shared between members), persisted to the registry.
#[tauri::command]
async fn rename_server(
    state: State<'_, AppState>,
    server: u64,
    name: String,
) -> Result<(), String> {
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
    let files = view
        .files
        .into_iter()
        .map(|l| UiFile {
            name: l.entry.name,
            size: l.entry.size,
            mime: l.entry.mime,
            cid: hex::encode(&l.entry.cid),
            author: l.entry.author,
            path: l.entry.path,
            held: l.held_chunks,
            total: l.total_chunks,
            expires: l.entry.expires.deadline_ms(),
            expires_known: l.entry.expires.is_recorded(),
        })
        .collect();
    Ok(FilesPayload {
        files,
        has_peers: view.has_peers,
    })
}

/// The fingerprints of members reachable right now (presence indicators in the roster).
#[tauri::command]
async fn get_online_members(state: State<'_, AppState>, server: u64) -> Result<Vec<String>, String> {
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
    let (total, size) = actor
        .file_download_plan(raw.clone())
        .await
        .ok_or_else(|| {
            "this file can't be downloaded; it isn't listed, or its reference is invalid".to_string()
        })?;
    let _ = app.emit(
        "download-progress",
        DownloadProgressEvt {
            server,
            cid: cid.clone(),
            done: 0,
            total,
            provider: None,
        },
    );
    let mut out = Vec::with_capacity(size as usize);
    for i in 0..total {
        let (chunk, provider) = actor.fetch_file_chunk(raw.clone(), i).await?;
        out.extend_from_slice(&chunk);
        let _ = app.emit(
            "download-progress",
            DownloadProgressEvt {
                server,
                cid: cid.clone(),
                done: i + 1,
                total,
                provider,
            },
        );
    }
    if Cid::of(&out).as_bytes() != &target {
        return Err("the reassembled file failed its integrity check".into());
    }
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
#[tauri::command]
async fn get_connectivity(state: State<'_, AppState>) -> Result<Connectivity, String> {
    let mut diag = state.diag.lock().await.clone();
    diag.upnp = state
        .upnp
        .lock()
        .await
        .get(&diag.server)
        .cloned()
        .unwrap_or_else(|| UPNP_NOT_ATTEMPTED.to_string());
    Ok(diag)
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
            hex::decode(&record.invite)
                .ok()
                .map(|b| InviteToken::decode(&b).ok())
        })
        .flatten()
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
    let (mesh, libp2p_id, port) = build_transport(&net, &relay_dial)?;
    net.port = port;

    // Re-run the founder's reachability work verbatim: the advertise address, the UPnP probe, the
    // relay-circuit reservation and the rendezvous registration. Before this, a reload rebuilt a
    // loopback-only bootstrap and every invite minted afterwards was same-machine only.
    let id = libp2p_id.to_string();
    let (reach, problems) = establish_reachability(&mesh, &id, port, &net, &mut Vec::new()).await;
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
    // Advertise our reachable addresses unconditionally, so that a server whose steady-state
    // rendezvous config was restored from the snapshot can re-register a dialable record on the
    // actor's next discovery tick. Harmless for a server without rendezvous.
    for addr in external_addrs(&bootstrap) {
        let _ = mesh.add_external_address(addr).await;
    }
    let upnp_rx = mesh.take_external_addrs().await;

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
    let (actor, events, _task) = spawn(server);
    actor.open_channel(general).await;
    // Register under the SAME id as on disk (don't allocate a new one).
    forward_events(app.clone(), record.id, events);
    spawn_discovery_timer(app.clone(), record.id, actor.clone());
    state.servers.lock().await.insert(
        record.id,
        ServerEntry {
            actor,
            invite: presented_invite,
            name: record.display_name.clone(),
            bootstrap,
            rendezvous: rz_vec,
            mesh: rz_handle,
            is_dm: record.is_dm,
            record_seq: net.record_seq,
        },
    );
    // Re-seal if the port moved. (The reserved peer-record sequence block was already sealed by
    // `load_or_init_server_net`, before the transport came up.)
    if net.port != saved_port {
        persist_server_net(state, record.id, &net).await;
    }
    if let Some(rx) = upnp_rx {
        spawn_upnp_fold(app.clone(), record.id, rx, id);
    }
    Ok(())
}

/// The only external URL prefix the app will hand to the OS: our own issue tracker's
/// new-issue form. `open_issue_url` is a launcher, and a launcher that takes any URL is a way
/// to point the user's browser (or a registered `foo://` handler) anywhere, so the allowlist
/// is a constant here rather than anything the webview can influence.
const ISSUE_URL_PREFIX: &str = "https://github.com/Thalpy/Mewtual/issues/new?";

/// Is this a new-issue URL on our own tracker? Split out from the command so the allowlist
/// itself is testable without launching a browser.
fn is_tracker_url(url: &str) -> bool {
    url.starts_with(ISSUE_URL_PREFIX)
}

fn is_external_http_url(url: &str) -> bool {
    if url.is_empty() || url.len() > 4096 || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
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

/// Open a prefilled bug report / feature request on the tracker in the user's default browser.
/// Mewtual has no service of its own to receive feedback, so filing means handing GitHub a
/// filled-in form: the app carries no GitHub credentials and posts nothing itself, and the user
/// submits (and so authors) the issue from their own browser.
#[tauri::command]
async fn open_issue_url(url: String) -> Result<(), String> {
    if !is_tracker_url(&url) {
        return Err("refusing to open a URL outside the issue tracker".into());
    }
    launch_url(&url)
}

/// Open a chat/wiki link in the system browser, keeping the Mewtual webview on the conversation.
#[tauri::command]
async fn open_external_url(url: String) -> Result<(), String> {
    if !is_external_http_url(&url) {
        return Err("only http and https links can be opened".into());
    }
    launch_url(&url)
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
            channels: ui_channels(actor_of(&state, record.id).await?.channels().await),
            is_dm: record.is_dm,
        });
    }
    *state.session_resumable.lock().await = true;
    Ok(reloaded)
}

/// Restore an already-unlocked frontend after F5/HMR without asking for the vault passphrase
/// again. An explicit UI lock disables this path until `unlock` verifies the passphrase.
#[tauri::command]
async fn resume_session(
    state: State<'_, AppState>,
) -> Result<Option<Vec<ReloadedServer>>, String> {
    if !*state.session_resumable.lock().await || state.store.lock().await.is_none() {
        return Ok(None);
    }
    Ok(Some(running_servers(&state).await))
}

#[tauri::command]
async fn lock_session(state: State<'_, AppState>) -> Result<(), String> {
    *state.session_resumable.lock().await = false;
    Ok(())
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
    let origin = ceremony_origin(&state).await?;
    let view = catcoms_app::read_pairing_blob(&blob, &origin).map_err(|e| e.to_string())?;
    if state.pairing_ledger.lock().await.is_spent(&view.request.pairing_nonce) {
        return Err("that pairing request has already been used".to_string());
    }
    // The scope the popup must disclose: everything an accept would grant.
    let (servers, dm_count) = {
        let guard = state.servers.lock().await;
        let mut names: Vec<(u64, String, bool)> =
            guard.iter().map(|(id, e)| (*id, e.name.clone(), e.is_dm)).collect();
        names.sort_by_key(|(id, _, _)| *id);
        let dm_count = names.iter().filter(|(_, _, dm)| *dm).count();
        (names.into_iter().map(|(_, n, _)| n).collect::<Vec<_>>(), dm_count)
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
    if state.pairing_ledger.lock().await.is_spent(&view.request.pairing_nonce) {
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
        None,
        name,
        Vec::new(),
        Vec::new(),
        None,
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
async fn get_debug_logging(app: AppHandle) -> Result<DebugLogging, String> {
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
async fn set_debug_logging(app: AppHandle, enabled: bool) -> Result<DebugLogging, String> {
    let dir = log_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let flag = debug_flag_path(&app)?;
    if enabled {
        std::fs::write(&flag, b"on").map_err(|e| e.to_string())?;
    } else if flag.exists() {
        std::fs::remove_file(&flag).map_err(|e| e.to_string())?;
    }
    get_debug_logging(app).await
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
            join_server,
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
            get_join_attempts,
            get_connectivity,
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
            open_external_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Mewtual desktop app");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trap this guards: founding mints the invite immediately, UPnP answers seconds later,
    /// so the invite a user copies first is the one *without* the public address their router
    /// just opened. Their friend on another network then gets an unactionable timeout. This
    /// predicate is what makes the invite self-heal the next time anyone reads it.
    #[test]
    fn an_invite_is_stale_only_when_reachability_was_gained() {
        let lan = "/ip4/192.168.0.5/tcp/9000/p2p/ID".to_string();
        let loop_ = "/ip4/127.0.0.1/tcp/9000/p2p/ID".to_string();
        let public = "/ip4/203.0.113.9/tcp/9000/p2p/ID".to_string();

        // The real case: UPnP landed after the mint, so the stored invite is missing the address
        // that is the entire point of the mapping.
        assert!(invite_missing_addresses(
            &[loop_.clone(), lan.clone()],
            &[public.clone(), loop_.clone(), lan.clone()]
        ));

        // Nothing gained: no re-mint, so reading the invite repeatedly cannot churn nonces.
        assert!(!invite_missing_addresses(
            &[public.clone(), loop_.clone()],
            &[public.clone(), loop_.clone()]
        ));
        // Order is not a change.
        assert!(!invite_missing_addresses(
            &[public.clone(), loop_.clone()],
            &[loop_.clone(), public.clone()]
        ));
        // Deliberately one-directional: an address *lost* (a relay circuit that dropped) leaves
        // the rest working, so it is not worth invalidating a nonce somebody may be about to use.
        assert!(!invite_missing_addresses(
            &[public.clone(), loop_.clone(), lan.clone()],
            &[loop_.clone()]
        ));
        // Degenerate cases stay quiet rather than re-minting forever.
        assert!(!invite_missing_addresses(&[], &[]));
        assert!(!invite_missing_addresses(&[public.clone()], &[]));
        assert!(invite_missing_addresses(&[], &[public]));
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
        let public = format!("/ip4/203.0.113.7/udp/9000/quic-v1/p2p/{ID}");
        let out = external_addrs(&[public]);
        assert_eq!(
            out.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
            vec!["/ip4/203.0.113.7/udp/9000/quic-v1"]
        );

        // A mixed list keeps only the routable entries; no fallback to loopback any more.
        let mixed = vec![
            format!("/ip4/127.0.0.1/tcp/9/p2p/{ID}"),
            format!("/ip6/2001:db8::5/tcp/9/p2p/{ID}"),
        ];
        assert_eq!(external_addrs(&mixed).len(), 1);
    }

    #[test]
    fn the_listen_port_prefers_the_seed_derived_home_port() {
        let net = ServerNet {
            key_seed: [42u8; 32],
            port: 0,
            advertise: String::new(),
            relay: String::new(),
            rendezvous: String::new(),
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
    }
}
