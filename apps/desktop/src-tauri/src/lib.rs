//! The Tauri command/event bridge — a thin shell over the `catcoms-app` event-stream
//! actor (slices 8b-2 / 8c). The frontend `invoke`s these commands and `listen`s for the
//! forwarded events; all the real work lives in the tested `catcoms-app` actor, which
//! itself wraps the protocol stack. The GUI never touches MLS or automerge.
//!
//! Scope: found a server (and mint a single-use invite carrying its loopback address),
//! or join an existing server by pasting that invite; then #general — send + read
//! messages. The discovery/relay wiring and multi-server are later slices.

use std::time::Duration;

use catcoms_app::{spawn, AppEvent, Server, ServerActor};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_net::{phase0_peer_id, target_peer_in_multiaddr, MeshService};
use catcoms_rt::{Clock, MeshTransport, OsCryptoRng, RngCore, SystemClock, TransportEvent};
use libp2p::Multiaddr;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;

/// The default channel id (the UI exposes one channel for now).
const GENERAL: u128 = 1;

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

/// Forward an actor's event stream to the frontend as Tauri events.
fn forward_events(app: AppHandle, mut events: mpsc::Receiver<AppEvent>) {
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            match ev {
                AppEvent::ChannelUpdated { channel } => {
                    let _ = app.emit("channel-updated", channel);
                }
                AppEvent::MembersChanged { count } => {
                    let _ = app.emit("members-changed", count);
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
) -> Result<(), String> {
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

    let (actor, events, _task) = spawn(server);
    actor.open_channel(GENERAL).await;
    forward_events(app, events);
    *state.actor.lock().await = Some(actor);
    Ok(())
}

/// Join an existing server by pasting its invite: decode it, dial the bootstrap
/// address, run the MLS join, then catch up #general.
#[tauri::command]
async fn join_server(
    app: AppHandle,
    state: State<'_, AppState>,
    invite_hex: String,
    display_name: String,
) -> Result<(), String> {
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

    let (actor, events, _task) = spawn(server);
    actor.open_channel(GENERAL).await;
    actor.catch_up(inviter, GENERAL).await;
    forward_events(app, events);
    *state.actor.lock().await = Some(actor);
    Ok(())
}

/// The single-use invite to share (founder only); `None` for a joiner.
#[tauri::command]
async fn get_invite(state: State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(state.invite.lock().await.clone())
}

/// Send a chat message to #general.
#[tauri::command]
async fn send_message(state: State<'_, AppState>, text: String) -> Result<(), String> {
    if let Some(actor) = state.actor.lock().await.as_ref() {
        actor.send_message(GENERAL, text).await;
    }
    Ok(())
}

/// Read #general's current messages.
#[tauri::command]
async fn get_messages(state: State<'_, AppState>) -> Result<Vec<UiMessage>, String> {
    let guard = state.actor.lock().await;
    let Some(actor) = guard.as_ref() else {
        return Ok(Vec::new());
    };
    Ok(actor
        .messages(GENERAL)
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
            get_invite,
            send_message,
            get_messages
        ])
        .run(tauri::generate_context!())
        .expect("error while running the CatComs desktop app");
}
