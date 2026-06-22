//! The Tauri command/event bridge — a thin shell over the `catcoms-app` event-stream
//! actor (slices 8b-2 / 8c). The frontend `invoke`s these commands and `listen`s for the
//! forwarded events; all the real work lives in the tested `catcoms-app` actor, which
//! itself wraps the protocol stack. The GUI never touches MLS or automerge.
//!
//! Scope: found a server (and mint a single-use invite carrying its loopback address),
//! or join an existing server by pasting that invite; then #general — send + read
//! messages. The discovery/relay wiring and multi-server are later slices.

use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use catcoms_app::{channel_id, spawn, AppEvent, Profile, Server, ServerActor, MAX_AVATAR_BYTES};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_net::{phase0_peer_id, target_peer_in_multiaddr, MeshService};
use catcoms_rt::{Clock, MeshTransport, OsCryptoRng, RngCore, SystemClock, TransportEvent};
use libp2p::Multiaddr;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;

/// App state managed by Tauri: the running server actor, plus (for a founder) the
/// single-use invite to share.
#[derive(Default)]
struct AppState {
    actor: Mutex<Option<ServerActor>>,
    invite: Mutex<Option<String>>,
}

/// A chat message as serialized to the frontend.
#[derive(Serialize, Clone)]
struct UiMessage {
    author: String,
    text: String,
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

/// Forward an actor's event stream to the frontend as Tauri events.
fn forward_events(app: AppHandle, mut events: mpsc::Receiver<AppEvent>) {
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            match ev {
                AppEvent::ChannelUpdated { channel } => {
                    // Channel ids are u128 — send as a string (JS numbers lose precision).
                    let _ = app.emit("channel-updated", channel.to_string());
                }
                AppEvent::MembersChanged { count } => {
                    let _ = app.emit("members-changed", count);
                }
                AppEvent::ProfilesUpdated => {
                    let _ = app.emit("profiles-updated", ());
                }
                AppEvent::Closed => {
                    let _ = app.emit("server-closed", ());
                    break;
                }
            }
        }
    });
}

/// Found a new server: bind a loopback TCP port, found the group, mint a single-use
/// invite carrying that address, spawn the actor, and forward its events.
#[tauri::command]
async fn found_server(
    app: AppHandle,
    state: State<'_, AppState>,
    display_name: String,
) -> Result<String, String> {
    let listen: Multiaddr = "/ip4/127.0.0.1/tcp/0"
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| e.to_string())?;
    let (mesh, libp2p_id) = MeshService::new_tcp(Some(listen), &[]).map_err(|e| e.to_string())?;

    // Discover the OS-assigned port and build a dialable bootstrap address.
    let bound = timeout(Duration::from_secs(10), mesh.next_listen_addr())
        .await
        .map_err(|_| "listen-addr timeout".to_string())?
        .ok_or_else(|| "transport stopped".to_string())?;
    let bootstrap = format!("{bound}/p2p/{libp2p_id}");

    let device = MlsDevice::generate().map_err(|e| e.to_string())?;
    let mut server = Server::found(mesh, device, OsCryptoRng, Box::new(SystemClock), display_name)
        .map_err(|e| e.to_string())?;
    server.subscribe_control().await.map_err(|e| e.to_string())?;

    // Mint a single-use invite (1h) carrying the bootstrap address.
    let mut nonce = [0u8; 16];
    let mut rng = OsCryptoRng;
    rng.fill_bytes(&mut nonce);
    let expires = SystemClock.now_ms() + 3_600_000;
    let invite = server
        .mint_invite(nonce, expires, vec![bootstrap])
        .map_err(|e| e.to_string())?;
    *state.invite.lock().await = Some(hex::encode(invite.encode()));

    let general = channel_id("general");
    let (actor, events, _task) = spawn(server);
    actor.open_channel(general).await;
    forward_events(app, events);
    *state.actor.lock().await = Some(actor);
    Ok(general.to_string())
}

/// Join an existing server by pasting its invite: decode it, dial the bootstrap
/// address, run the MLS join, then catch up #general.
#[tauri::command]
async fn join_server(
    app: AppHandle,
    state: State<'_, AppState>,
    invite_hex: String,
    display_name: String,
) -> Result<String, String> {
    let bytes = hex::decode(invite_hex.trim()).map_err(|e| e.to_string())?;
    let invite = InviteToken::decode(&bytes).map_err(|e| e.to_string())?;
    let boot = invite
        .bootstrap
        .first()
        .ok_or_else(|| "invite carries no bootstrap address".to_string())?;
    let addr: Multiaddr = boot
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| e.to_string())?;
    let inviter_lp =
        target_peer_in_multiaddr(&addr).ok_or_else(|| "bootstrap has no peer id".to_string())?;
    let inviter = phase0_peer_id(&inviter_lp);

    let (mesh, _id) = MeshService::new_tcp(None, std::slice::from_ref(&addr))
        .map_err(|e| e.to_string())?;
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
    let server = Server::join(
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

    let general = channel_id("general");
    let (actor, events, _task) = spawn(server);
    actor.open_channel(general).await;
    actor.catch_up(inviter, general).await;
    actor.catch_up_profiles(inviter).await;
    forward_events(app, events);
    *state.actor.lock().await = Some(actor);
    Ok(general.to_string())
}

/// Open (create/subscribe) a channel by name; returns its id. Members who open the same
/// name converge on the same channel.
#[tauri::command]
async fn open_channel(state: State<'_, AppState>, name: String) -> Result<String, String> {
    let id = channel_id(&name);
    if let Some(actor) = state.actor.lock().await.as_ref() {
        actor.open_channel(id).await;
    }
    Ok(id.to_string())
}

/// The single-use invite to share (founder only); `None` for a joiner.
#[tauri::command]
async fn get_invite(state: State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(state.invite.lock().await.clone())
}

/// The current roster (member fingerprints; `you` marks the local device).
#[tauri::command]
async fn get_members(state: State<'_, AppState>) -> Result<Vec<UiMember>, String> {
    let guard = state.actor.lock().await;
    let Some(actor) = guard.as_ref() else {
        return Ok(Vec::new());
    };
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
    if let Some(actor) = state.actor.lock().await.as_ref() {
        actor
            .set_profile(Profile {
                name,
                color,
                font,
                effect,
                avatar,
            })
            .await;
    }
    Ok(())
}

/// All known member profiles (keyed by fingerprint).
#[tauri::command]
async fn get_profiles(state: State<'_, AppState>) -> Result<Vec<UiProfile>, String> {
    let guard = state.actor.lock().await;
    let Some(actor) = guard.as_ref() else {
        return Ok(Vec::new());
    };
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

/// Send a chat message to a channel (by id).
#[tauri::command]
async fn send_message(
    state: State<'_, AppState>,
    channel: String,
    text: String,
) -> Result<(), String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    if let Some(actor) = state.actor.lock().await.as_ref() {
        actor.send_message(id, text).await;
    }
    Ok(())
}

/// Read a channel's current messages (by id).
#[tauri::command]
async fn get_messages(
    state: State<'_, AppState>,
    channel: String,
) -> Result<Vec<UiMessage>, String> {
    let id: u128 = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let guard = state.actor.lock().await;
    let Some(actor) = guard.as_ref() else {
        return Ok(Vec::new());
    };
    Ok(actor
        .messages(id)
        .await
        .into_iter()
        .map(|m| UiMessage {
            author: m.author,
            text: m.text,
        })
        .collect())
}

/// Build and run the Tauri application.
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            found_server,
            join_server,
            open_channel,
            get_invite,
            get_members,
            set_profile,
            get_profiles,
            send_message,
            get_messages
        ])
        .run(tauri::generate_context!())
        .expect("error while running the CatComs desktop app");
}
