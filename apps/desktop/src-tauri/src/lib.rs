//! The Tauri command/event bridge — a thin shell over the `catcoms-app` event-stream
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
    channel_id, peer_addrs_from_snapshot, spawn, AppEvent, Profile, Server, ServerActor,
    ServerRecord, ServerStore, MAX_AVATAR_BYTES,
};
use catcoms_discovery::{Candidate, DiscoveryPolicy, PolicyConfig, Source};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_net::{
    phase0_peer_id, target_peer_in_multiaddr, validate_rendezvous_addrs, MeshHandle, MeshService,
    RendezvousTarget,
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
    /// separately persisted — on reload it is recovered from the persisted invite's `rendezvous`.
    rendezvous: Vec<String>,
    /// A clonable handle to this server's live transport, kept so the bridge can register a
    /// freshly-minted invite's namespace at the rendezvous *after* the `Server` was moved into its
    /// actor. `None` for a joiner (never registers) or a server without rendezvous.
    mesh: Option<MeshHandle>,
}

/// App state managed by Tauri: every running server keyed by a bridge-assigned id, plus the
/// on-disk store once the user has unlocked it with a passphrase (`None` = in-memory only).
#[derive(Default)]
struct AppState {
    servers: Mutex<HashMap<u64, ServerEntry>>,
    next_id: Mutex<u64>,
    store: Mutex<Option<ServerStore>>,
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
}

/// A chat message as serialized to the frontend.
#[derive(Serialize, Clone)]
struct UiMessage {
    author: String,
    text: String,
    ts: u64,
}

/// A roster member as serialized to the frontend.
#[derive(Serialize, Clone)]
struct UiMember {
    fingerprint: String,
    you: bool,
}

/// A member profile as serialized to the frontend (keyed by fingerprint). `avatar` is
/// base64-encoded JPEG bytes (empty = no avatar).
#[derive(Serialize, Clone)]
struct UiProfile {
    fingerprint: String,
    name: String,
    color: String,
    font: String,
    effect: String,
    avatar: String,
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
}

// Event payloads — every event is tagged with its server id.
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

/// Forward one server actor's event stream to the frontend, tagging each with `server`.
/// How often the bridge nudges a server's actor to drive steady-state rendezvous discovery. The
/// real-time interval lives HERE (in the bridge / `apps`, off the deterministic-time seam the
/// `crates` ambient gate enforces); the actor's pass is a no-op for a server without rendezvous.
const DISCOVERY_INTERVAL_SECS: u64 = 60;

/// Spawn a per-server timer that periodically drives steady-state rendezvous discovery, so the
/// group re-finds itself after a restart. Exits once the actor stops (`drive_discovery` errors).
fn spawn_discovery_timer(actor: ServerActor) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(DISCOVERY_INTERVAL_SECS));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if actor.drive_discovery().await.is_err() {
                break; // the actor stopped
            }
        }
    });
}

fn forward_events(app: AppHandle, server: u64, mut events: mpsc::Receiver<AppEvent>) {
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            match ev {
                AppEvent::ChannelUpdated { channel } => {
                    // Channel ids are u128 — send as a string (JS numbers lose precision).
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
                AppEvent::FilesUpdated => {
                    let _ = app.emit("files-updated", ServerEvt { server });
                }
                AppEvent::StatusUpdated => {
                    let _ = app.emit("status-updated", ServerEvt { server });
                }
                AppEvent::WikiUpdated => {
                    let _ = app.emit("wiki-updated", ServerEvt { server });
                }
                AppEvent::RolesUpdated => {
                    let _ = app.emit("roles-updated", ServerEvt { server });
                }
                AppEvent::Closed => {
                    let _ = app.emit("server-closed", ServerEvt { server });
                    break;
                }
            }
        }
    });
}

/// Extract the TCP port from a multiaddr (the OS-assigned listen port).
fn tcp_port(addr: &Multiaddr) -> Option<u16> {
    addr.iter().find_map(|p| match p {
        Protocol::Tcp(port) => Some(port),
        _ => None,
    })
}

/// Build a dialable bootstrap multiaddr from a user-entered reachable address, so peers on
/// a LAN or the internet can join. Accepts a bare IPv4 (`1.2.3.4` — uses the bound `port`),
/// `host:port` (e.g. a forwarded port), or a full multiaddr starting with `/` (e.g. a relay
/// circuit address). Appends this node's `/p2p/<id>` if absent. (IPv4/multiaddr only; a
/// hostname would need `/dns4/`.)
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
    let (host, p) = match input.rsplit_once(':') {
        Some((h, ps)) => (h, ps.parse().map_err(|_| format!("bad port in '{input}'"))?),
        None => (input, port),
    };
    if host.is_empty() {
        return Err(format!("bad address '{input}'"));
    }
    Ok(format!("/ip4/{host}/tcp/{p}/p2p/{peer_id}"))
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
) -> u64 {
    let id = {
        let mut n = state.next_id.lock().await;
        *n += 1;
        *n
    };
    forward_events(app.clone(), id, events);
    spawn_discovery_timer(actor.clone());
    state.servers.lock().await.insert(
        id,
        ServerEntry {
            actor,
            invite,
            name,
            bootstrap,
            rendezvous,
            mesh,
        },
    );
    id
}

/// Snapshot a running server through its actor and seal it to disk (best-effort: a missing
/// store, a stopped actor, or an I/O error is logged, not fatal — the app keeps running).
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

/// Whether a multiaddr is a loopback address (`127.0.0.0/8` or `::1`).
fn addr_is_loopback(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| match p {
        Protocol::Ip4(ip) => ip.is_loopback(),
        Protocol::Ip6(ip) => ip.is_loopback(),
        _ => false,
    })
}

/// The reachable external addresses to advertise for rendezvous registration, from the bootstrap
/// list. Loopback is advertised **only** when nothing else is reachable (same-machine testing) —
/// otherwise it's dropped so we don't pollute a shared rendezvous namespace with a record a remote
/// joiner can't reach (it would instead use the invite's real bootstrap addrs as a fallback).
fn external_addrs(bootstrap: &[String]) -> Vec<Multiaddr> {
    let all: Vec<Multiaddr> = bootstrap.iter().filter_map(|s| external_addr(s)).collect();
    let routable: Vec<Multiaddr> = all.iter().filter(|a| !addr_is_loopback(a)).cloned().collect();
    if routable.is_empty() {
        all
    } else {
        routable
    }
}

/// Register a `(group_id, nonce)` invite's pre-join namespace at the rendezvous `rz` via `handle`
/// (fire-and-forget — the grant is internally deferred + flushed once an external address exists,
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

/// The discover-on-join path (no hard-coded inviter address): build a transport, dial the invite's
/// rendezvous node(s), discover the inviter's records under the pre-join namespace, rank them
/// through the [`DiscoveryPolicy`] (never auto-dial), then dial the chosen addresses — plus the
/// invite's `bootstrap` addrs as direct fallbacks — and return the connected transport + the
/// inviter's peer id. Mirrors `tcp_rendezvous_e2e.rs`.
async fn discover_and_connect(
    invite: &InviteToken,
) -> Result<(MeshService, PeerId, Vec<(String, Vec<u8>)>), String> {
    let targets = validate_rendezvous_addrs(&invite.rendezvous).map_err(|e| e.to_string())?;
    if targets.is_empty() {
        return Err("invite carries no rendezvous address".into());
    }
    let rz_addrs: Vec<Multiaddr> = targets.iter().map(|t| t.addr.clone()).collect();
    // Bind a listen port so the joiner is itself dialable — post-join steady-state discovery has
    // members register/discover + dial each other — then dial the rendezvous nodes.
    let listen: Multiaddr = "/ip4/0.0.0.0/tcp/0"
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| e.to_string())?;
    let (mesh, _id) = MeshService::new_tcp(Some(listen), &rz_addrs).map_err(|e| e.to_string())?;
    // Advertise our reachable (loopback) address so our steady-state rendezvous registration
    // carries a dialable record (same-machine; a LAN/relay advertise for joiners is a follow-up).
    if let Ok(Some(bound)) = timeout(Duration::from_secs(10), mesh.next_listen_addr()).await {
        if let Some(port) = tcp_port(&bound) {
            if let Ok(addr) = format!("/ip4/127.0.0.1/tcp/{port}").parse::<Multiaddr>() {
                let _ = mesh.add_external_address(addr).await;
            }
        }
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
    .map_err(|_| "timed out connecting to the rendezvous".to_string())?;

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
                // Placeholder pre-join: we can't read the registrant's own signed seq here, so the
                // policy's anti-replay freshness is inert; the backstop is request_join's Welcome-
                // signature + group-id check, which fails closed if we dial the wrong peer.
                seq: 1,
                tag_verified: false, // pre-join: no group secret to recompute the member tag
            });
            if candidates.len() >= 8 {
                break;
            }
        }
    })
    .await;
    if candidates.is_empty() {
        return Err("could not discover the server at the rendezvous".into());
    }

    // The DiscoveryPolicy alone decides what to dial (eclipse-resistance — never auto-dial).
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

    // Dial the policy-chosen addresses plus the invite's bootstrap addrs (direct fallbacks).
    for a in dialed.addresses.iter().chain(invite.bootstrap.iter()) {
        if let Ok(m) = a.parse::<Multiaddr>() {
            let _ = mesh.dial(m).await;
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
    .map_err(|_| "timed out connecting to the discovered server".to_string())?;
    // The rendezvous config the joiner keeps for steady-state discovery (re-finding the group).
    let rz_config: Vec<(String, Vec<u8>)> = targets
        .iter()
        .map(|t| (t.addr.to_string(), t.peer.to_bytes()))
        .collect();
    Ok((mesh, inviter, rz_config))
}

/// Found a new server: bind all interfaces (so LAN/internet peers can reach it, not just
/// loopback), found the group, mint a single-use invite carrying the reachable address(es),
/// spawn the actor, and register it. `advertise` is an optional user-supplied reachable
/// address (LAN or public IP); `relay` is an optional relay-node multiaddr — when given, we
/// reserve a circuit there and put the **relayed** address first in the invite, so a joiner
/// reaches us through the relay with **no port-forward** (zero-config NAT traversal).
/// `rendezvous` is an optional zero-knowledge rendezvous multiaddr — when given, we register at it
/// so a joiner can discover us with **no hard-coded address at all** (just the pasted invite).
#[tauri::command]
async fn found_server(
    app: AppHandle,
    state: State<'_, AppState>,
    display_name: String,
    advertise: String,
    relay: String,
    rendezvous: String,
) -> Result<FoundResult, String> {
    let listen: Multiaddr = "/ip4/0.0.0.0/tcp/0"
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| e.to_string())?;
    let relay = relay.trim().to_string();
    let relay_dial: Vec<Multiaddr> = if relay.is_empty() {
        Vec::new()
    } else {
        vec![relay
            .parse()
            .map_err(|e: libp2p::multiaddr::Error| format!("bad relay address: {e}"))?]
    };
    let (mesh, libp2p_id) =
        MeshService::new_tcp(Some(listen), &relay_dial).map_err(|e| e.to_string())?;

    // Discover the OS-assigned port; advertise loopback (same-machine) plus the user's
    // reachable address if given, so the invite works for same-machine, LAN, or internet.
    let bound = timeout(Duration::from_secs(10), mesh.next_listen_addr())
        .await
        .map_err(|_| "listen-addr timeout".to_string())?
        .ok_or_else(|| "transport stopped".to_string())?;
    let port = tcp_port(&bound).ok_or_else(|| "no bound TCP port".to_string())?;
    let id = libp2p_id.to_string();
    let mut bootstrap = vec![format!("/ip4/127.0.0.1/tcp/{port}/p2p/{id}")];
    if !advertise.trim().is_empty() {
        bootstrap.push(build_advertised(&advertise, port, &id)?);
    }

    // Reserve a relay circuit and prefer the relayed address (NAT traversal, no port-forward).
    if !relay.is_empty() {
        // Wait for the relay connection before reserving (the relay-client transport needs
        // a live connection to reserve over).
        timeout(Duration::from_secs(20), async {
            loop {
                if let Some(TransportEvent::PeerConnected(_)) = mesh.next_event().await {
                    break;
                }
            }
        })
        .await
        .map_err(|_| "could not connect to the relay".to_string())?;
        let circuit: Multiaddr = format!("{relay}/p2p-circuit")
            .parse()
            .map_err(|e: libp2p::multiaddr::Error| e.to_string())?;
        mesh.listen_on(circuit).await.map_err(|e| e.to_string())?;
        let circuit_addr = timeout(Duration::from_secs(20), async {
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
        bootstrap.insert(0, circuit_addr.to_string()); // prefer the relayed address
    }

    // Optional rendezvous: connect to it + advertise our reachable address(es) on the raw mesh
    // (so the deferred registration can flush), keeping a handle to register each invite's
    // namespace after the server is spawned. The founder is then discoverable with no hard-coded
    // address — a joiner needs only the pasted invite.
    let rendezvous = rendezvous.trim().to_string();
    let (rz_target, rz_handle): (Option<RendezvousTarget>, Option<MeshHandle>) = if rendezvous
        .is_empty()
    {
        (None, None)
    } else {
        let rz = validate_rendezvous_addrs(std::slice::from_ref(&rendezvous))
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or_else(|| "no rendezvous address".to_string())?;
        mesh.dial(rz.addr.clone()).await.map_err(|e| e.to_string())?;
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
        // Advertise our routable addresses so the deferred registration can flush. A relay
        // *circuit* address is intentionally not advertised here — it auto-promotes to an external
        // address on reservation (in the transport actor), so the rendezvous still learns it.
        for addr in external_addrs(&bootstrap) {
            mesh.add_external_address(addr)
                .await
                .map_err(|e| e.to_string())?;
        }
        (Some(rz), Some(mesh.handle()))
    };

    let device = MlsDevice::generate().map_err(|e| e.to_string())?;
    let name = display_name.clone();
    let mut server = Server::found(mesh, device, OsCryptoRng, Box::new(SystemClock), display_name)
        .map_err(|e| e.to_string())?;
    server.subscribe_control().await.map_err(|e| e.to_string())?;
    attach_blob_store(&state, &mut server).await;
    // Steady-state discovery: tell the server which rendezvous to re-register/discover at, so the
    // actor re-finds the group after a restart. (The founder already advertised + connected above.)
    if let Some(rz) = &rz_target {
        server.set_rendezvous_nodes(vec![(rz.addr.to_string(), rz.peer.to_bytes())]);
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
    let server_id = register_server(
        &app,
        &state,
        actor,
        events,
        Some(invite_hex),
        name,
        bootstrap,
        rz_vec,
        rz_handle,
    )
    .await;
    // Seal the new server + the registry to disk (if the store is unlocked).
    persist_server(&state, server_id).await;
    persist_registry(&state).await;
    Ok(FoundResult {
        server: server_id,
        channel: general.to_string(),
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
) -> Result<FoundResult, String> {
    let bytes = hex::decode(invite_hex.trim()).map_err(|e| e.to_string())?;
    let invite = InviteToken::decode(&bytes).map_err(|e| e.to_string())?;

    // If the invite points at a rendezvous, discover the inviter there (no hard-coded address);
    // otherwise dial the invite's bootstrap addresses directly (loopback / LAN / relayed).
    let (mesh, inviter, rz_config) = if !invite.rendezvous.is_empty() {
        discover_and_connect(&invite).await?
    } else {
        let addrs: Vec<Multiaddr> = invite
            .bootstrap
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        if addrs.is_empty() {
            return Err("invite carries no usable bootstrap address".to_string());
        }
        let inviter_lp = addrs
            .iter()
            .find_map(target_peer_in_multiaddr)
            .ok_or_else(|| "bootstrap has no peer id".to_string())?;
        let inviter = phase0_peer_id(&inviter_lp);
        let (mesh, _id) = MeshService::new_tcp(None, &addrs).map_err(|e| e.to_string())?;
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
        .map_err(|_| "timed out connecting to the server".to_string())?;
        (mesh, inviter, Vec::new())
    };

    let device = MlsDevice::generate().map_err(|e| e.to_string())?;
    let name = display_name.clone();
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
    .map_err(|e| e.to_string())?;
    attach_blob_store(&state, &mut server).await;
    // Steady-state discovery: the joiner keeps the invite's rendezvous so the actor re-registers/
    // re-discovers there (re-finding the group after a restart, no fresh invite).
    if !rz_config.is_empty() {
        server.set_rendezvous_nodes(rz_config);
    }

    let general = channel_id("general");
    let (actor, events, _task) = spawn(server);
    actor.open_channel(general).await;
    actor.catch_up(inviter, general).await;
    actor.catch_up_profiles(inviter).await;
    actor.catch_up_files(inviter).await;
    actor.catch_up_status(inviter).await;
    actor.catch_up_wiki(inviter).await;
    actor.catch_up_roles(inviter).await;
    // A joiner mints no invites (owner-scoped), so it carries no bootstrap/rendezvous of its own.
    let server_id = register_server(
        &app,
        &state,
        actor,
        events,
        None,
        name,
        Vec::new(),
        Vec::new(),
        None,
    )
    .await;
    // Seal the joined server + the registry to disk (if the store is unlocked).
    persist_server(&state, server_id).await;
    persist_registry(&state).await;
    Ok(FoundResult {
        server: server_id,
        channel: general.to_string(),
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
    let id = channel_id(&name);
    let actor = actor_of(&state, server).await?;
    actor.open_channel(id).await;
    actor.catch_up_any(id).await;
    Ok(id.to_string())
}

/// The single-use invite to share (founder only); `None` for a joiner.
#[tauri::command]
async fn get_invite(state: State<'_, AppState>, server: u64) -> Result<Option<String>, String> {
    Ok(state
        .servers
        .lock()
        .await
        .get(&server)
        .and_then(|e| e.invite.clone()))
}

/// Mint a **fresh** single-use invite on demand (owner/admin only — gated in `Server::mint_invite`),
/// carrying the live bootstrap address captured at found/reload. If the server registered at a
/// rendezvous, the fresh invite is also discovery-enabled and its new (nonce-keyed) namespace is
/// registered there via the stored transport handle, so the new joiner can discover us with no
/// hard-coded address. Replaces the server's stored invite and re-seals the registry.
#[tauri::command]
async fn mint_invite_fresh(state: State<'_, AppState>, server: u64) -> Result<String, String> {
    let (bootstrap, rendezvous, handle) = {
        let servers = state.servers.lock().await;
        let e = servers.get(&server).ok_or_else(|| "unknown server".to_string())?;
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
        // Register the fresh invite's namespace so the new joiner can discover us.
        if let (Some(handle), Some(rz)) = (
            &handle,
            validate_rendezvous_addrs(&rendezvous)
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

/// Rename a server — a **local** display label in this client's rail (server names are not
/// shared between members), persisted to the registry.
#[tauri::command]
async fn rename_server(state: State<'_, AppState>, server: u64, name: String) -> Result<(), String> {
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

/// Set this member's own profile (name + styling + optional avatar). `avatar` is
/// base64-encoded JPEG bytes (empty = no avatar).
#[tauri::command]
async fn set_profile(
    state: State<'_, AppState>,
    server: u64,
    name: String,
    color: String,
    font: String,
    effect: String,
    avatar: String,
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
    let actor = actor_of(&state, server).await?;
    actor
        .set_profile(Profile {
            name,
            color,
            font,
            effect,
            avatar,
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
            avatar: if p.avatar.is_empty() {
                String::new()
            } else {
                B64.encode(&p.avatar)
            },
        })
        .collect())
}

/// Share a file (base64-encoded bytes); returns its content-address hex.
#[tauri::command]
async fn add_file(
    state: State<'_, AppState>,
    server: u64,
    name: String,
    mime: String,
    path: String,
    data: String,
) -> Result<String, String> {
    let bytes = B64
        .decode(data.as_bytes())
        .map_err(|e| format!("bad file data: {e}"))?;
    let actor = actor_of(&state, server).await?;
    let cid = actor.add_file(name, mime, path, bytes).await?;
    persist_server(&state, server).await;
    Ok(cid)
}

/// The shared file list (metadata; bytes are fetched on download).
#[tauri::command]
async fn get_files(state: State<'_, AppState>, server: u64) -> Result<Vec<UiFile>, String> {
    let actor = actor_of(&state, server).await?;
    Ok(actor
        .files()
        .await
        .into_iter()
        .map(|f| UiFile {
            name: f.name,
            size: f.size,
            mime: f.mime,
            cid: hex::encode(&f.cid),
            author: f.author,
            path: f.path,
        })
        .collect())
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

/// Download a shared file by content-address hex; returns base64-encoded bytes.
#[tauri::command]
async fn download_file(
    state: State<'_, AppState>,
    server: u64,
    cid: String,
) -> Result<String, String> {
    let raw = hex::decode(cid.trim()).map_err(|e| format!("bad cid: {e}"))?;
    let actor = actor_of(&state, server).await?;
    let bytes = actor.download_file(raw).await?;
    Ok(B64.encode(&bytes))
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
        .map(|m| UiMessage {
            author: m.author,
            text: m.text,
            ts: m.ts,
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

/// Create or update a wiki page.
#[tauri::command]
async fn save_wiki_page(
    state: State<'_, AppState>,
    server: u64,
    name: String,
    body: String,
) -> Result<(), String> {
    let actor = actor_of(&state, server).await?;
    actor.write_wiki_page(name, body).await;
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
) -> Result<(), String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let actor = actor_of(&state, server).await?;
    actor.send_message(id, text).await;
    persist_server(&state, server).await;
    Ok(())
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
        .map(|m| UiMessage {
            author: m.author,
            text: m.text,
            ts: m.ts,
        })
        .collect())
}

/// One server reloaded from disk, returned to the UI to repopulate the rail.
#[derive(Serialize, Clone)]
struct ReloadedServer {
    server: u64,
    name: String,
    invite: String,
    channel: String,
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
    let listen: Multiaddr = "/ip4/0.0.0.0/tcp/0"
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| e.to_string())?;
    // 9g: dial the last-known peers (from the persisted peer records) at construction, so a
    // reloaded joiner reconnects on its own as those peers come online.
    let redial: Vec<Multiaddr> = peer_addrs_from_snapshot(snapshot)
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    let (mesh, libp2p_id) =
        MeshService::new_tcp(Some(listen), &redial).map_err(|e| e.to_string())?;
    // Capture the current loopback bootstrap. The OS-assigned port changes across reloads (the
    // 9f caveat), so a freshly-minted invite must carry the *new* address — capture it here
    // rather than reuse the persisted (stale) one. Best-effort same-machine reach; re-advertising
    // a LAN/relay address on reload is a networking follow-up (tied to rendezvous).
    let bootstrap = match timeout(Duration::from_secs(10), mesh.next_listen_addr()).await {
        Ok(Some(addr)) => tcp_port(&addr)
            .map(|p| vec![format!("/ip4/127.0.0.1/tcp/{p}/p2p/{libp2p_id}")])
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    // Advertise our (new-port) reachable address unconditionally, so that if this server has a
    // persisted steady-state rendezvous config (restored from the snapshot), the actor's discovery
    // tick can re-register a dialable record. Harmless for a server without rendezvous.
    for addr in external_addrs(&bootstrap) {
        let _ = mesh.add_external_address(addr).await;
    }

    // If the persisted invite was discovery-enabled, re-connect to its rendezvous, re-advertise
    // our (new-port) address, and re-register the invite's namespace there — so the founder is
    // discoverable again after restart, and fresh invites can register via the kept handle. The
    // rz addrs ride in the persisted invite (not separately persisted). Best-effort.
    let persisted_invite = (!record.invite.is_empty())
        .then(|| hex::decode(&record.invite).ok().map(|b| InviteToken::decode(&b).ok()))
        .flatten()
        .flatten();
    let mut rz_vec: Vec<String> = Vec::new();
    let mut rz_handle: Option<MeshHandle> = None;
    if let Some(invite) = &persisted_invite {
        if let Some(rz) = validate_rendezvous_addrs(&invite.rendezvous)
            .ok()
            .and_then(|v| v.into_iter().next())
        {
            if mesh.dial(rz.addr.clone()).await.is_ok() {
                let rz_peer = phase0_peer_id(&rz.peer);
                let connected = timeout(Duration::from_secs(15), async {
                    loop {
                        if let Some(TransportEvent::PeerConnected(p)) = mesh.next_event().await {
                            if p == rz_peer {
                                break;
                            }
                        }
                    }
                })
                .await
                .is_ok();
                if connected {
                    for addr in external_addrs(&bootstrap) {
                        let _ = mesh.add_external_address(addr).await;
                    }
                    let handle = mesh.handle();
                    let _ = register_join_ns(
                        &handle,
                        &invite.group_id,
                        &invite.invite_nonce,
                        &rz,
                    )
                    .await;
                    rz_vec = invite.rendezvous.clone();
                    rz_handle = Some(handle);
                }
            }
        }
    }

    let mut server = Server::restore(
        snapshot,
        mesh,
        OsCryptoRng,
        Box::new(SystemClock),
        &record.display_name,
    )
    .map_err(|e| e.to_string())?;
    server.subscribe_control().await.map_err(|e| e.to_string())?;
    attach_blob_store(state, &mut server).await;

    // If the persisted invite is discovery-enabled but we could NOT re-register its namespace
    // (rendezvous infra was down at reload), drop it: it would not resolve — no registration, and
    // after a reload the only bootstrap is a stale new-port loopback. The rail then prompts a fresh
    // invite (which re-registers). A direct (non-rendezvous) invite is presented unchanged.
    let discovery_unregistered =
        persisted_invite.as_ref().is_some_and(|i| !i.rendezvous.is_empty()) && rz_handle.is_none();
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
    spawn_discovery_timer(actor.clone());
    state.servers.lock().await.insert(
        record.id,
        ServerEntry {
            actor,
            invite: presented_invite,
            name: record.display_name.clone(),
            bootstrap,
            rendezvous: rz_vec,
            mesh: rz_handle,
        },
    );
    Ok(())
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
    // process kept running), don't reload from disk — that would spawn a duplicate actor +
    // transport per server. Return the servers already registered so the rail repopulates.
    if state.store.lock().await.is_some() {
        let servers = state.servers.lock().await;
        return Ok(servers
            .iter()
            .map(|(id, e)| ReloadedServer {
                server: *id,
                name: e.name.clone(),
                invite: e.invite.clone().unwrap_or_default(),
                channel: channel_id("general").to_string(),
            })
            .collect());
    }

    let records = store.load_registry().map_err(|e| e.to_string())?;

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
        });
    }
    Ok(reloaded)
}

/// Build and run the Tauri application.
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            unlock,
            found_server,
            join_server,
            leave_server,
            open_channel,
            get_invite,
            mint_invite_fresh,
            rename_server,
            get_members,
            set_profile,
            get_profiles,
            add_file,
            get_files,
            download_file,
            file_available,
            delete_file,
            post_status,
            get_statuses,
            get_wiki_pages,
            get_wiki_map,
            get_wiki_page,
            save_wiki_page,
            get_roles,
            set_admin,
            remove_member,
            send_message,
            get_messages
        ])
        .run(tauri::generate_context!())
        .expect("error while running the CatComs desktop app");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcp_port_is_extracted() {
        let a: Multiaddr = "/ip4/192.168.1.5/tcp/54321".parse().unwrap();
        assert_eq!(tcp_port(&a), Some(54321));
        let b: Multiaddr = "/ip4/0.0.0.0/tcp/0".parse().unwrap();
        assert_eq!(tcp_port(&b), Some(0));
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
        // Empty / malformed are rejected.
        assert!(build_advertised("", 9000, id).is_err());
        assert!(build_advertised("1.2.3.4:notaport", 9000, id).is_err());
    }
}
