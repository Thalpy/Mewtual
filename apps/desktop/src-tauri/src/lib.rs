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
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_net::{phase0_peer_id, target_peer_in_multiaddr, MeshService};
use catcoms_rt::{Clock, MeshTransport, OsCryptoRng, RngCore, SystemClock, TransportEvent};
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
async fn register_server(
    app: &AppHandle,
    state: &AppState,
    actor: ServerActor,
    events: mpsc::Receiver<AppEvent>,
    invite: Option<String>,
    name: String,
) -> u64 {
    let id = {
        let mut n = state.next_id.lock().await;
        *n += 1;
        *n
    };
    forward_events(app.clone(), id, events);
    state
        .servers
        .lock()
        .await
        .insert(id, ServerEntry { actor, invite, name });
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

/// Found a new server: bind all interfaces (so LAN/internet peers can reach it, not just
/// loopback), found the group, mint a single-use invite carrying the reachable address(es),
/// spawn the actor, and register it. `advertise` is an optional user-supplied reachable
/// address (LAN or public IP); `relay` is an optional relay-node multiaddr — when given, we
/// reserve a circuit there and put the **relayed** address first in the invite, so a joiner
/// reaches us through the relay with **no port-forward** (zero-config NAT traversal).
#[tauri::command]
async fn found_server(
    app: AppHandle,
    state: State<'_, AppState>,
    display_name: String,
    advertise: String,
    relay: String,
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

    let device = MlsDevice::generate().map_err(|e| e.to_string())?;
    let name = display_name.clone();
    let mut server = Server::found(mesh, device, OsCryptoRng, Box::new(SystemClock), display_name)
        .map_err(|e| e.to_string())?;
    server.subscribe_control().await.map_err(|e| e.to_string())?;
    attach_blob_store(&state, &mut server).await;

    // Mint a single-use invite (1h) carrying the bootstrap address.
    let mut nonce = [0u8; 16];
    let mut rng = OsCryptoRng;
    rng.fill_bytes(&mut nonce);
    let expires = SystemClock.now_ms() + 3_600_000;
    let invite = server
        .mint_invite(nonce, expires, bootstrap)
        .map_err(|e| e.to_string())?;
    let invite_hex = hex::encode(invite.encode());

    let general = channel_id("general");
    let (actor, events, _task) = spawn(server);
    actor.open_channel(general).await;
    let server_id = register_server(&app, &state, actor, events, Some(invite_hex), name).await;
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
    // Dial every bootstrap address the invite carries — loopback (same machine), a LAN IP,
    // or a public/relayed address; whichever reaches the inviter first wins.
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

    let general = channel_id("general");
    let (actor, events, _task) = spawn(server);
    actor.open_channel(general).await;
    actor.catch_up(inviter, general).await;
    actor.catch_up_profiles(inviter).await;
    actor.catch_up_files(inviter).await;
    actor.catch_up_status(inviter).await;
    actor.catch_up_wiki(inviter).await;
    actor.catch_up_roles(inviter).await;
    let server_id = register_server(&app, &state, actor, events, None, name).await;
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
    match actor.download_file(raw).await {
        Some(bytes) => Ok(B64.encode(&bytes)),
        None => Err("file unavailable (no peer has it yet)".into()),
    }
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
    let (mesh, _id) = MeshService::new_tcp(Some(listen), &redial).map_err(|e| e.to_string())?;
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

    let general = channel_id("general");
    let (actor, events, _task) = spawn(server);
    actor.open_channel(general).await;
    // Register under the SAME id as on disk (don't allocate a new one).
    forward_events(app.clone(), record.id, events);
    state.servers.lock().await.insert(
        record.id,
        ServerEntry {
            actor,
            invite: if record.invite.is_empty() {
                None
            } else {
                Some(record.invite.clone())
            },
            name: record.display_name.clone(),
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

    let mut reloaded = Vec::new();
    let mut max_id = 0u64;
    for record in &records {
        let bytes = match store.load_server(record.id) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("unlock: loading server {} failed: {e}", record.id);
                continue;
            }
        };
        if let Err(e) = reload_one(&app, &state, &bytes, record).await {
            eprintln!("unlock: restoring server {} failed: {e}", record.id);
            continue;
        }
        max_id = max_id.max(record.id);
        reloaded.push(ReloadedServer {
            server: record.id,
            name: record.display_name.clone(),
            invite: record.invite.clone(),
            channel: channel_id("general").to_string(),
        });
    }

    // Keep the unlocked store, and ensure new servers get ids past the reloaded ones.
    *state.store.lock().await = Some(store);
    {
        let mut n = state.next_id.lock().await;
        if *n < max_id {
            *n = max_id;
        }
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
