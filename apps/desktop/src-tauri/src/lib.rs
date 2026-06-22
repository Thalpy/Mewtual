//! The Tauri command/event bridge — a thin shell over the `catcoms-app` event-stream
//! actor (slice 8b-2). The frontend `invoke`s these commands and `listen`s for the
//! forwarded events; all the real work lives in the tested `catcoms-app` actor, which
//! itself wraps the protocol stack. The GUI never touches MLS or automerge.
//!
//! Scope (first cut): found a server, open #general, send + read messages. Joining via
//! an invite and the discovery/relay wiring are the next slices.

use catcoms_app::{spawn, AppEvent, Server, ServerActor};
use catcoms_mls::MlsDevice;
use catcoms_net::MeshService;
use catcoms_rt::{OsCryptoRng, SystemClock};
use libp2p::Multiaddr;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

/// The default channel id (the UI exposes one channel for now).
const GENERAL: u128 = 1;

/// App state managed by Tauri: the running server actor (if any).
#[derive(Default)]
struct AppState {
    actor: Mutex<Option<ServerActor>>,
}

/// A chat message as serialized to the frontend.
#[derive(Serialize, Clone)]
struct UiMessage {
    author: String,
    text: String,
}

/// Found a new server: bind a loopback TCP port, found the group, spawn the actor, and
/// forward its events to the frontend.
#[tauri::command]
async fn found_server(
    app: AppHandle,
    state: State<'_, AppState>,
    display_name: String,
) -> Result<(), String> {
    let listen: Multiaddr = "/ip4/127.0.0.1/tcp/0"
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| e.to_string())?;
    let (mesh, _id) = MeshService::new_tcp(Some(listen), &[]).map_err(|e| e.to_string())?;
    let device = MlsDevice::generate().map_err(|e| e.to_string())?;
    let mut server = Server::found(mesh, device, OsCryptoRng, Box::new(SystemClock), display_name)
        .map_err(|e| e.to_string())?;
    server.subscribe_control().await.map_err(|e| e.to_string())?;

    let (actor, mut events, _task) = spawn(server);
    actor.open_channel(GENERAL).await;

    // Forward actor events to the frontend.
    let app = app.clone();
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

    *state.actor.lock().await = Some(actor);
    Ok(())
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
            send_message,
            get_messages
        ])
        .run(tauri::generate_context!())
        .expect("error while running the CatComs desktop app");
}
