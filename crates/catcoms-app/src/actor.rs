//! The async **event-stream actor** around a [`Server`] (slice 8b-1).
//!
//! A GUI can't poll `sync_once` by hand — it needs a live thing it sends *commands* to
//! and gets *events* from. [`spawn`] moves a `Server` into a background task that owns
//! it, drives the network, and translates between a command channel and an event
//! channel. The Tauri command bridge (8b-2) is a thin shell over this; tests drive it
//! directly over the in-memory transport.
//!
//! The task `select!`s between the command channel and `Server::sync_once`. When a
//! command arrives mid-`sync_once`, the in-flight `sync_once` is cancelled — safe at its
//! only real suspension point (`next_event`, which leaves the event queued); a cancel
//! during the brief pre-event recovery work may at worst drop an in-flight catch-up,
//! which the recovery machinery re-detects on the next inbound event (self-healing).

use std::collections::HashMap;

use catcoms_rt::{CryptoRngCore, MeshTransport, PeerId};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use catcoms_storage::Cid;

use crate::{ChatMessage, FileEntry, MemberView, Profile, Server};

/// Per drive: how long to wait for a discovered record before concluding the queue is drained.
const DISCOVERY_DRAIN_MS: u64 = 500;
/// Per-tick cap on discovered records ingested, so one tick can't block the actor unboundedly.
const MAX_DISCOVERED_PER_TICK: usize = 16;

/// A command from the UI to a running server actor.
#[derive(Debug)]
pub enum AppCommand {
    /// Open a channel (subscribe + create locally). Acked once subscribed, so a caller
    /// can avoid racing a subsequent publish ahead of the subscription.
    OpenChannel {
        channel: u128,
        ack: oneshot::Sender<()>,
    },
    /// Send a chat message to a channel.
    SendMessage { channel: u128, text: String },
    /// Pull a channel's history from `peer` (e.g. right after joining).
    CatchUp { peer: PeerId, channel: u128 },
    /// Pull a channel's history from the best known peer (no peer named).
    CatchUpAny { channel: u128 },
    /// Query a channel's current materialized messages.
    Messages {
        channel: u128,
        reply: oneshot::Sender<Vec<ChatMessage>>,
    },
    /// Query the current member count.
    MemberCount { reply: oneshot::Sender<usize> },
    /// Query the roster (member fingerprints + which one is self).
    Members {
        reply: oneshot::Sender<Vec<MemberView>>,
    },
    /// Set this member's own profile (name + styling).
    SetProfile { profile: Profile },
    /// Pull the profile document from `peer` (e.g. right after joining).
    CatchUpProfiles { peer: PeerId },
    /// Query all known member profiles, keyed by fingerprint.
    Profiles {
        reply: oneshot::Sender<HashMap<String, Profile>>,
    },
    /// Share a file under folder `path`; replies with its content-address hex, or an error.
    AddFile {
        name: String,
        mime: String,
        path: String,
        bytes: Vec<u8>,
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Query the shared file list.
    Files {
        reply: oneshot::Sender<Vec<FileEntry>>,
    },
    /// Download a file's bytes by content address (raw CID bytes); a precise error otherwise.
    DownloadFile {
        cid: Vec<u8>,
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
    },
    /// Whether the file's blob is already held locally (no network fetch needed to open it).
    FileAvailable {
        cid: Vec<u8>,
        reply: oneshot::Sender<bool>,
    },
    /// Remove a file from the shared index by content address (owner/admin only).
    DeleteFile {
        cid: Vec<u8>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Pull the file index from `peer` (e.g. right after joining).
    CatchUpFiles { peer: PeerId },
    /// Post to the server status feed.
    PostStatus { text: String },
    /// Query the status feed.
    Statuses {
        reply: oneshot::Sender<Vec<ChatMessage>>,
    },
    /// Pull the status feed from `peer` (e.g. right after joining).
    CatchUpStatus { peer: PeerId },
    /// Query the wiki page names (sorted).
    WikiPages { reply: oneshot::Sender<Vec<String>> },
    /// Query the whole wiki as a name -> body map (for backlinks / link existence).
    WikiMap {
        reply: oneshot::Sender<HashMap<String, String>>,
    },
    /// Read a wiki page's body.
    ReadWikiPage {
        name: String,
        reply: oneshot::Sender<String>,
    },
    /// Create or update a wiki page.
    WriteWikiPage { name: String, body: String },
    /// Pull the wiki from `peer` (e.g. right after joining).
    CatchUpWiki { peer: PeerId },
    /// Query every member's role, keyed by fingerprint (owner/admin/member).
    Roles {
        reply: oneshot::Sender<HashMap<String, String>>,
    },
    /// Grant or revoke admin for a member fingerprint (owner only).
    SetAdmin {
        fp: String,
        admin: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Pull the roles document from `peer` (e.g. right after joining).
    CatchUpRoles { peer: PeerId },
    /// Remove a member by fingerprint (owner only).
    RemoveMember {
        fp: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Mint a fresh single-use invite (owner/admin only) carrying `bootstrap`; replies with the
    /// encoded `InviteToken` bytes, or an error.
    MintInvite {
        nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
    },
    /// Mint a fresh single-use invite that ALSO embeds `rendezvous` infra addrs (signed-over), so a
    /// joiner can discover the server with no hard-coded address. Owner/admin only. The caller
    /// (bridge) is responsible for registering the new namespace at the rendezvous via a
    /// `MeshHandle` afterwards.
    MintInviteWithRendezvous {
        nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
        rendezvous: Vec<String>,
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
    },
    /// Serialize the server's durable state for sealing to disk (Phase 9f).
    Snapshot {
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
    },
    /// Drive one steady-state rendezvous-discovery pass (re-register + re-discover + dial newly
    /// found members). Fire-and-forget; sent periodically by the bridge's per-server timer (the
    /// real-time interval lives there, off the deterministic-time seam). No-op without rendezvous.
    DriveDiscovery,
    /// Stop the actor.
    Shutdown,
}

/// An event from a running server actor to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /// A channel's message list changed — the UI should re-fetch it (`messages`). Using
    /// a re-fetch signal (rather than diffed deltas) keeps ordering robust under CRDT
    /// merges of concurrent messages.
    ChannelUpdated { channel: u128 },
    /// The roster size changed (a member joined or was removed).
    MembersChanged { count: usize },
    /// A member profile changed — the UI should re-fetch profiles (`profiles`).
    ProfilesUpdated,
    /// The shared file list changed — the UI should re-fetch it (`files`).
    FilesUpdated,
    /// The status feed changed — the UI should re-fetch it (`statuses`).
    StatusUpdated,
    /// The wiki changed — the UI should re-fetch pages / the open page.
    WikiUpdated,
    /// Member roles changed — the UI should re-fetch roles.
    RolesUpdated,
    /// The actor has stopped (transport closed or shutdown requested).
    Closed,
}

/// A handle to a running server actor: send commands, run queries.
#[derive(Debug, Clone)]
pub struct ServerActor {
    cmd_tx: mpsc::Sender<AppCommand>,
}

impl ServerActor {
    /// Open a channel and wait until it is subscribed.
    pub async fn open_channel(&self, channel: u128) {
        let (ack, done) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::OpenChannel { channel, ack })
            .await
            .is_ok()
        {
            let _ = done.await;
        }
    }

    /// Send a chat message to a channel (fire-and-forget; a `ChannelUpdated` event follows).
    pub async fn send_message(&self, channel: u128, text: impl Into<String>) {
        let _ = self
            .cmd_tx
            .send(AppCommand::SendMessage {
                channel,
                text: text.into(),
            })
            .await;
    }

    /// Pull a channel's history from `peer`.
    pub async fn catch_up(&self, peer: PeerId, channel: u128) {
        let _ = self
            .cmd_tx
            .send(AppCommand::CatchUp { peer, channel })
            .await;
    }

    /// Pull a channel's history from the best known peer (no peer named).
    pub async fn catch_up_any(&self, channel: u128) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpAny { channel }).await;
    }

    /// Fetch a channel's current messages.
    pub async fn messages(&self, channel: u128) -> Vec<ChatMessage> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Messages { channel, reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Serialize the server's durable state (to be sealed to disk by the bridge). Returns an
    /// error string if the actor has stopped or the snapshot failed.
    pub async fn snapshot(&self) -> Result<Vec<u8>, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Snapshot { reply })
            .await
            .is_err()
        {
            return Err("server actor stopped".into());
        }
        rx.await
            .unwrap_or_else(|_| Err("server actor dropped".into()))
    }

    /// Mint a fresh single-use invite (owner/admin only) carrying `bootstrap`; returns the
    /// encoded `InviteToken` bytes, or an error string.
    pub async fn mint_invite(
        &self,
        nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
    ) -> Result<Vec<u8>, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::MintInvite {
                nonce,
                expires_at_ms,
                bootstrap,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server actor stopped".into());
        }
        rx.await
            .unwrap_or_else(|_| Err("server actor dropped".into()))
    }

    /// Mint a fresh single-use invite that ALSO embeds `rendezvous` infra addrs (owner/admin only);
    /// returns the encoded `InviteToken` bytes. The caller registers the new namespace separately.
    pub async fn mint_invite_with_rendezvous(
        &self,
        nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
        rendezvous: Vec<String>,
    ) -> Result<Vec<u8>, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::MintInviteWithRendezvous {
                nonce,
                expires_at_ms,
                bootstrap,
                rendezvous,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server actor stopped".into());
        }
        rx.await
            .unwrap_or_else(|_| Err("server actor dropped".into()))
    }

    /// Fetch the current member count.
    pub async fn member_count(&self) -> usize {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::MemberCount { reply })
            .await
            .is_err()
        {
            return 0;
        }
        rx.await.unwrap_or(0)
    }

    /// Fetch the roster (member fingerprints).
    pub async fn members(&self) -> Vec<MemberView> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Members { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Set this member's own profile (a `ProfilesUpdated` event follows).
    pub async fn set_profile(&self, profile: Profile) {
        let _ = self.cmd_tx.send(AppCommand::SetProfile { profile }).await;
    }

    /// Pull the profile document from `peer`.
    pub async fn catch_up_profiles(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpProfiles { peer }).await;
    }

    /// Fetch all known member profiles, keyed by fingerprint.
    pub async fn profiles(&self) -> HashMap<String, Profile> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Profiles { reply })
            .await
            .is_err()
        {
            return HashMap::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Share a file (bytes) under folder `path`; returns its content-address hex, or an error.
    pub async fn add_file(
        &self,
        name: String,
        mime: String,
        path: String,
        bytes: Vec<u8>,
    ) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::AddFile {
                name,
                mime,
                path,
                bytes,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Fetch the shared file list.
    pub async fn files(&self) -> Vec<FileEntry> {
        let (reply, rx) = oneshot::channel();
        if self.cmd_tx.send(AppCommand::Files { reply }).await.is_err() {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Download a file's bytes by content address (raw CID bytes); a precise error string if it
    /// can't be produced (not listed / held-but-unreadable / no peer has it / undecryptable).
    pub async fn download_file(&self, cid: Vec<u8>) -> Result<Vec<u8>, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::DownloadFile { cid, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Whether the file's blob is held locally (openable without a network fetch).
    pub async fn file_available(&self, cid: Vec<u8>) -> bool {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::FileAvailable { cid, reply })
            .await
            .is_err()
        {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    /// Remove a file from the shared index by content address (owner/admin only). A
    /// `FilesUpdated` event follows on success.
    pub async fn delete_file(&self, cid: Vec<u8>) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::DeleteFile { cid, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Pull the file index from `peer`.
    pub async fn catch_up_files(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpFiles { peer }).await;
    }

    /// Post to the status feed (a `StatusUpdated` event follows).
    pub async fn post_status(&self, text: impl Into<String>) {
        let _ = self
            .cmd_tx
            .send(AppCommand::PostStatus { text: text.into() })
            .await;
    }

    /// Fetch the status feed.
    pub async fn statuses(&self) -> Vec<ChatMessage> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Statuses { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Pull the status feed from `peer`.
    pub async fn catch_up_status(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpStatus { peer }).await;
    }

    /// Fetch the wiki page names (sorted).
    pub async fn wiki_pages(&self) -> Vec<String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::WikiPages { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Fetch the whole wiki as a name -> body map.
    pub async fn wiki_map(&self) -> HashMap<String, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::WikiMap { reply })
            .await
            .is_err()
        {
            return HashMap::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Fetch every member's role (fingerprint -> owner/admin/member).
    pub async fn roles(&self) -> HashMap<String, String> {
        let (reply, rx) = oneshot::channel();
        if self.cmd_tx.send(AppCommand::Roles { reply }).await.is_err() {
            return HashMap::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Grant or revoke admin for a member fingerprint (owner only).
    pub async fn set_admin(&self, fp: String, admin: bool) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetAdmin { fp, admin, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Pull the roles document from `peer`.
    pub async fn catch_up_roles(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpRoles { peer }).await;
    }

    /// Remove a member by fingerprint (owner only).
    pub async fn remove_member(&self, fp: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::RemoveMember { fp, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Read a wiki page's body.
    pub async fn read_wiki_page(&self, name: impl Into<String>) -> String {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::ReadWikiPage {
                name: name.into(),
                reply,
            })
            .await
            .is_err()
        {
            return String::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Create or update a wiki page (a `WikiUpdated` event follows).
    pub async fn write_wiki_page(&self, name: impl Into<String>, body: impl Into<String>) {
        let _ = self
            .cmd_tx
            .send(AppCommand::WriteWikiPage {
                name: name.into(),
                body: body.into(),
            })
            .await;
    }

    /// Pull the wiki from `peer`.
    pub async fn catch_up_wiki(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpWiki { peer }).await;
    }

    /// Stop the actor.
    pub async fn shutdown(&self) {
        let _ = self.cmd_tx.send(AppCommand::Shutdown).await;
    }

    /// Drive one steady-state rendezvous-discovery pass. Fire-and-forget — the bridge calls this on
    /// a timer. Returns `Err` once the actor has stopped (so the bridge's timer task can exit).
    pub async fn drive_discovery(&self) -> Result<(), ()> {
        self.cmd_tx
            .send(AppCommand::DriveDiscovery)
            .await
            .map_err(|_| ())
    }
}

/// Move `server` into a background task. Returns a [`ServerActor`] handle, a receiver of
/// [`AppEvent`]s, and the task's [`JoinHandle`].
pub fn spawn<T, R>(
    mut server: Server<T, R>,
) -> (ServerActor, mpsc::Receiver<AppEvent>, JoinHandle<()>)
where
    T: MeshTransport + Send + 'static,
    R: CryptoRngCore + Send + 'static,
{
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<AppCommand>(64);
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>(256);
    let handle = tokio::spawn(async move {
        let mut counts: HashMap<u128, usize> = HashMap::new();
        let mut members = server.member_count();
        // Open the per-server profile document and seed this member's name from the
        // display name, so the roster/messages show a name immediately (the user can
        // customize color/font/effect later via SetProfile).
        if let Err(e) = server.open_profiles().await {
            tracing::warn!(error = %e, "open_profiles failed");
        }
        let seed = Profile {
            name: server.display_name().to_string(),
            ..Profile::default()
        };
        if let Err(e) = server.set_profile(seed).await {
            tracing::warn!(error = %e, "seed profile failed");
        }
        let mut last_profiles = server.profiles();
        // Open the per-server file index too.
        if let Err(e) = server.open_files().await {
            tracing::warn!(error = %e, "open_files failed");
        }
        let mut file_count = server.files().len();
        // …and the status feed.
        if let Err(e) = server.open_status().await {
            tracing::warn!(error = %e, "open_status failed");
        }
        let mut status_count = server.statuses().len();
        // …and the wiki.
        if let Err(e) = server.open_wiki().await {
            tracing::warn!(error = %e, "open_wiki failed");
        }
        let mut last_wiki = server.wiki_map();
        // …and subscribe the member-roles doc so admin grants propagate. The *owner* role is
        // not stored here — it is derived from the MLS designated committer (lowest leaf
        // index), so every member computes the owner identically with no roles op present.
        if let Err(e) = server.open_roles().await {
            tracing::warn!(error = %e, "open_roles failed");
        }
        let mut last_roles = server.roles();
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => match cmd {
                    Some(AppCommand::OpenChannel { channel, ack }) => {
                        if let Err(e) = server.open_channel(channel).await {
                            tracing::warn!(error = %e, channel, "open_channel failed");
                        }
                        counts.entry(channel).or_insert(0);
                        let _ = ack.send(());
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::SendMessage { channel, text }) => {
                        if let Err(e) = server.send_message(channel, &text).await {
                            tracing::warn!(error = %e, channel, "send_message failed");
                        }
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::CatchUp { peer, channel }) => {
                        if let Err(e) = server.request_channel_catchup(peer, channel).await {
                            tracing::warn!(error = %e, channel, "catch-up failed");
                        }
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::CatchUpAny { channel }) => {
                        if let Err(e) = server.request_channel_catchup_any(channel).await {
                            tracing::warn!(error = %e, channel, "any-peer catch-up failed");
                        }
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::Messages { channel, reply }) => {
                        let _ = reply.send(server.messages(channel));
                    }
                    Some(AppCommand::MemberCount { reply }) => {
                        let _ = reply.send(server.member_count());
                    }
                    Some(AppCommand::Members { reply }) => {
                        let _ = reply.send(server.members_view());
                    }
                    Some(AppCommand::SetProfile { profile }) => {
                        if let Err(e) = server.set_profile(profile).await {
                            tracing::warn!(error = %e, "set_profile failed");
                        }
                        sync_profiles(&mut server, &mut last_profiles, &event_tx).await;
                    }
                    Some(AppCommand::CatchUpProfiles { peer }) => {
                        if let Err(e) = server.request_profiles_catchup(peer).await {
                            tracing::warn!(error = %e, "profiles catch-up failed");
                        }
                        sync_profiles(&mut server, &mut last_profiles, &event_tx).await;
                    }
                    Some(AppCommand::Profiles { reply }) => {
                        let _ = reply.send(server.profiles());
                    }
                    Some(AppCommand::AddFile { name, mime, path, bytes, reply }) => {
                        let res = server
                            .add_file(&name, &mime, &path, &bytes)
                            .await
                            .map(|cid| cid.to_hex())
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if files_changed(&server, &mut file_count) {
                            let _ = event_tx.send(AppEvent::FilesUpdated).await;
                        }
                    }
                    Some(AppCommand::Files { reply }) => {
                        let _ = reply.send(server.files());
                    }
                    Some(AppCommand::DownloadFile { cid, reply }) => {
                        let res = match <[u8; 32]>::try_from(cid.as_slice()) {
                            Ok(arr) => server
                                .download_file(&Cid::from_bytes(arr))
                                .await
                                .map_err(|e| e.to_string()),
                            Err(_) => Err("bad content address".to_string()),
                        };
                        let _ = reply.send(res);
                    }
                    Some(AppCommand::FileAvailable { cid, reply }) => {
                        let avail = match <[u8; 32]>::try_from(cid.as_slice()) {
                            Ok(arr) => server.file_available(&Cid::from_bytes(arr)),
                            Err(_) => false,
                        };
                        let _ = reply.send(avail);
                    }
                    Some(AppCommand::DeleteFile { cid, reply }) => {
                        let res = match <[u8; 32]>::try_from(cid.as_slice()) {
                            Ok(arr) => server
                                .delete_file(&Cid::from_bytes(arr))
                                .await
                                .map_err(|e| e.to_string()),
                            Err(_) => Err("bad content address".to_string()),
                        };
                        let _ = reply.send(res);
                        if files_changed(&server, &mut file_count) {
                            let _ = event_tx.send(AppEvent::FilesUpdated).await;
                        }
                    }
                    Some(AppCommand::CatchUpFiles { peer }) => {
                        if let Err(e) = server.request_files_catchup(peer).await {
                            tracing::warn!(error = %e, "files catch-up failed");
                        }
                        if files_changed(&server, &mut file_count) {
                            let _ = event_tx.send(AppEvent::FilesUpdated).await;
                        }
                    }
                    Some(AppCommand::PostStatus { text }) => {
                        if let Err(e) = server.post_status(&text).await {
                            tracing::warn!(error = %e, "post_status failed");
                        }
                        if status_changed(&server, &mut status_count) {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                    }
                    Some(AppCommand::Statuses { reply }) => {
                        let _ = reply.send(server.statuses());
                    }
                    Some(AppCommand::CatchUpStatus { peer }) => {
                        if let Err(e) = server.request_status_catchup(peer).await {
                            tracing::warn!(error = %e, "status catch-up failed");
                        }
                        if status_changed(&server, &mut status_count) {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                    }
                    Some(AppCommand::WikiPages { reply }) => {
                        let _ = reply.send(server.wiki_pages());
                    }
                    Some(AppCommand::WikiMap { reply }) => {
                        let _ = reply.send(server.wiki_map());
                    }
                    Some(AppCommand::Roles { reply }) => {
                        let _ = reply.send(server.roles());
                    }
                    Some(AppCommand::SetAdmin { fp, admin, reply }) => {
                        let res = server.set_admin(&fp, admin).await.map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if roles_changed(&server, &mut last_roles) {
                            let _ = event_tx.send(AppEvent::RolesUpdated).await;
                        }
                    }
                    Some(AppCommand::CatchUpRoles { peer }) => {
                        if let Err(e) = server.request_roles_catchup(peer).await {
                            tracing::warn!(error = %e, "roles catch-up failed");
                        }
                        if roles_changed(&server, &mut last_roles) {
                            let _ = event_tx.send(AppEvent::RolesUpdated).await;
                        }
                    }
                    Some(AppCommand::RemoveMember { fp, reply }) => {
                        let res = server.remove_member(&fp).await.map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        let mc = server.member_count();
                        if mc != members {
                            members = mc;
                            let _ = event_tx.send(AppEvent::MembersChanged { count: mc }).await;
                        }
                        if roles_changed(&server, &mut last_roles) {
                            let _ = event_tx.send(AppEvent::RolesUpdated).await;
                        }
                    }
                    Some(AppCommand::ReadWikiPage { name, reply }) => {
                        let _ = reply.send(server.read_wiki_page(&name));
                    }
                    Some(AppCommand::WriteWikiPage { name, body }) => {
                        if let Err(e) = server.write_wiki_page(&name, &body).await {
                            tracing::warn!(error = %e, "write_wiki_page failed");
                        }
                        if wiki_changed(&server, &mut last_wiki) {
                            let _ = event_tx.send(AppEvent::WikiUpdated).await;
                        }
                    }
                    Some(AppCommand::CatchUpWiki { peer }) => {
                        if let Err(e) = server.request_wiki_catchup(peer).await {
                            tracing::warn!(error = %e, "wiki catch-up failed");
                        }
                        if wiki_changed(&server, &mut last_wiki) {
                            let _ = event_tx.send(AppEvent::WikiUpdated).await;
                        }
                    }
                    Some(AppCommand::Snapshot { reply }) => {
                        let _ = reply.send(server.snapshot().map(|z| z.to_vec()).map_err(|e| e.to_string()));
                    }
                    // Steady-state rendezvous discovery: re-register + re-discover at the rendezvous,
                    // then drain the records that arrive in a bounded window and dial each
                    // (policy-gated). Driven by a periodic command from the bridge (the real-time
                    // timer lives there, off the deterministic-time seam). A no-op without rendezvous.
                    Some(AppCommand::DriveDiscovery) => {
                        if server.has_rendezvous() {
                            server.drive_discovery().await;
                            // ONE overall timeout bounds the whole drain to a single window, so an
                            // attacker drip-feeding records can't extend the actor's block; the
                            // count cap bounds the work.
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_millis(DISCOVERY_DRAIN_MS),
                                async {
                                    for _ in 0..MAX_DISCOVERED_PER_TICK {
                                        match server.next_discovered().await {
                                            Some(d) => server.ingest_discovered(d).await,
                                            None => break, // transport closed
                                        }
                                    }
                                },
                            )
                            .await;
                        }
                    }
                    Some(AppCommand::MintInvite { nonce, expires_at_ms, bootstrap, reply }) => {
                        let res = server
                            .mint_invite(nonce, expires_at_ms, bootstrap)
                            .map(|t| t.encode())
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                    }
                    Some(AppCommand::MintInviteWithRendezvous { nonce, expires_at_ms, bootstrap, rendezvous, reply }) => {
                        let res = server
                            .mint_invite_with_rendezvous(nonce, expires_at_ms, bootstrap, rendezvous)
                            .map(|t| t.encode())
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                    }
                    Some(AppCommand::Shutdown) | None => {
                        let _ = event_tx.send(AppEvent::Closed).await;
                        break;
                    }
                },
                cont = server.sync_once() => match cont {
                    Ok(true) => {
                        for channel in counts.keys().copied().collect::<Vec<_>>() {
                            if channel_changed(&server, channel, &mut counts) {
                                let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                            }
                        }
                        let mc = server.member_count();
                        if mc != members {
                            members = mc;
                            let _ = event_tx.send(AppEvent::MembersChanged { count: mc }).await;
                        }
                        sync_profiles(&mut server, &mut last_profiles, &event_tx).await;
                        if files_changed(&server, &mut file_count) {
                            let _ = event_tx.send(AppEvent::FilesUpdated).await;
                        }
                        if status_changed(&server, &mut status_count) {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                        if wiki_changed(&server, &mut last_wiki) {
                            let _ = event_tx.send(AppEvent::WikiUpdated).await;
                        }
                        if roles_changed(&server, &mut last_roles) {
                            let _ = event_tx.send(AppEvent::RolesUpdated).await;
                        }
                    }
                    _ => {
                        let _ = event_tx.send(AppEvent::Closed).await;
                        break;
                    }
                },
            }
        }
    });
    (ServerActor { cmd_tx }, event_rx, handle)
}

/// Whether a channel's message count changed since last seen (updating the record).
/// Synchronous — the `&Server` borrow ends before the caller awaits the event send, so
/// the actor future stays `Send` (a `&Server` held across an await would require
/// `Server: Sync`, which it is not).
fn channel_changed<T, R>(
    server: &Server<T, R>,
    channel: u128,
    counts: &mut HashMap<u128, usize>,
) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let n = server.messages(channel).len();
    if counts.get(&channel).copied() != Some(n) {
        counts.insert(channel, n);
        true
    } else {
        false
    }
}

/// Whether the shared file count changed since last seen (updating the record).
fn files_changed<T, R>(server: &Server<T, R>, last: &mut usize) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let n = server.files().len();
    if *last != n {
        *last = n;
        true
    } else {
        false
    }
}

/// Whether the status feed count changed since last seen (updating the record).
fn status_changed<T, R>(server: &Server<T, R>, last: &mut usize) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let n = server.statuses().len();
    if *last != n {
        *last = n;
        true
    } else {
        false
    }
}

/// Whether the wiki changed since last seen (a page added or a body edited — a count
/// alone misses edits, so this compares the full page map). Updates the record.
fn wiki_changed<T, R>(server: &Server<T, R>, last: &mut HashMap<String, String>) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = server.wiki_map();
    if now != *last {
        *last = now;
        true
    } else {
        false
    }
}

/// Whether member roles changed since last seen (updating the record).
fn roles_changed<T, R>(server: &Server<T, R>, last: &mut HashMap<String, String>) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = server.roles();
    if now != *last {
        *last = now;
        true
    } else {
        false
    }
}

/// Whether the profile document changed since last seen (updating the record). Compares
/// the materialized map; profiles are small, so this is cheap to run per tick.
fn profiles_changed<T, R>(server: &Server<T, R>, last: &mut HashMap<String, Profile>) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = server.profiles();
    if now != *last {
        *last = now;
        true
    } else {
        false
    }
}

/// Emit `ProfilesUpdated` if the profile set changed, then fetch any avatar blobs not yet
/// held from a peer and emit again if that resolved new avatars — so a member's picture
/// renders shortly after their profile arrives (avatars travel by content address, not
/// inline). Synchronous fetch (blocks the actor briefly per missing avatar); fine for the
/// small downscaled avatars, a concurrent fetch is a later refinement.
async fn sync_profiles<T, R>(
    server: &mut Server<T, R>,
    last_profiles: &mut HashMap<String, Profile>,
    event_tx: &mpsc::Sender<AppEvent>,
) where
    T: MeshTransport,
    R: CryptoRngCore,
{
    if profiles_changed(server, last_profiles) {
        let _ = event_tx.send(AppEvent::ProfilesUpdated).await;
    }
    // Always attempt to resolve missing avatars — the peer holding one is often only known
    // (via gossip's remember_peer) *after* the profile already arrived by catch-up, so a
    // change-gated fetch would never retry. Held avatars short-circuit on `has_blob`; only
    // genuinely-missing ones hit the network. (Caching unavailable CIDs to avoid re-asking
    // a peer that lacks one is a later refinement.)
    match server.fetch_missing_avatars().await {
        Ok(n) if n > 0 => {
            let _ = profiles_changed(server, last_profiles);
            let _ = event_tx.send(AppEvent::ProfilesUpdated).await;
        }
        Err(e) => tracing::warn!(error = %e, "avatar fetch failed"),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Server;
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    use std::time::Duration;
    use tokio::time::timeout;

    const GENERAL: u128 = 1;

    fn founder(
        hub: &std::sync::Arc<Hub>,
        peer: PeerId,
        name: &str,
        seed: u64,
    ) -> Server<MemNetwork, ChaCha20Rng> {
        Server::found(
            hub.join(peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(seed),
            Box::new(ManualClock::new(1_000)),
            name,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn the_actor_signals_a_channel_update_on_send_and_serves_queries() {
        let hub = Hub::new();
        let (actor, mut events, handle) = spawn(founder(&hub, PeerId::from_u64(1), "alice", 1));

        actor.open_channel(GENERAL).await;
        actor.send_message(GENERAL, "hi there").await;

        let ev = timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("event timeout")
            .expect("actor closed");
        assert_eq!(ev, AppEvent::ChannelUpdated { channel: GENERAL });

        let msgs = actor.messages(GENERAL).await;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "hi there");
        assert_eq!(actor.member_count().await, 1);

        actor.shutdown().await;
        let _ = handle.await;
    }

    #[tokio::test]
    async fn the_actor_snapshots_for_persistence_and_the_bytes_restore() {
        let hub = Hub::new();
        let (actor, mut events, handle) = spawn(founder(&hub, PeerId::from_u64(1), "alice", 1));

        actor.open_channel(GENERAL).await;
        actor.send_message(GENERAL, "remember me").await;
        let _ = timeout(Duration::from_secs(5), events.recv()).await; // drain the update

        let bytes = actor.snapshot().await.expect("snapshot");
        actor.shutdown().await;
        let _ = handle.await;

        // The snapshot restores into a working Server (fresh transport) with history intact.
        let hub2 = Hub::new();
        let restored = Server::restore(
            &bytes,
            hub2.join(PeerId::from_u64(2)),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(2_000)),
            "alice",
        )
        .expect("restore");
        let msgs = restored.messages(GENERAL);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "remember me");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_actors_converge_on_a_channel() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice_srv = founder(&hub, alice_peer, "alice", 1);
        alice_srv.subscribe_control().await.unwrap();
        alice_srv.open_channel(GENERAL).await.unwrap();
        let invite = alice_srv.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (alice, _alice_events, alice_handle) = spawn(alice_srv);

        // Bob joins — Alice's actor serves the join via its own sync loop.
        let bob_srv = Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &invite,
        )
        .await
        .unwrap();
        let (bob, mut bob_events, bob_handle) = spawn(bob_srv);
        bob.open_channel(GENERAL).await; // subscribes before Alice publishes

        alice.send_message(GENERAL, "hello bob").await;

        // Bob's actor should signal the channel changed; then re-fetch shows the message.
        timeout(Duration::from_secs(60), async {
            loop {
                match bob_events.recv().await {
                    Some(AppEvent::ChannelUpdated { channel }) if channel == GENERAL => break,
                    Some(_) => continue,
                    None => panic!("bob actor closed"),
                }
            }
        })
        .await
        .expect("bob did not observe the channel update");

        let msgs = bob.messages(GENERAL).await;
        assert!(
            msgs.iter().any(|m| m.text == "hello bob"),
            "bob converged on Alice's message: {msgs:?}"
        );

        alice.shutdown().await;
        bob.shutdown().await;
        let _ = alice_handle.await;
        let _ = bob_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_actors_converge_on_profiles() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice_srv = founder(&hub, alice_peer, "alice", 1);
        alice_srv.subscribe_control().await.unwrap();
        let invite = alice_srv.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (alice, _alice_events, alice_handle) = spawn(alice_srv);

        let bob_srv = Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &invite,
        )
        .await
        .unwrap();
        let (bob, mut bob_events, bob_handle) = spawn(bob_srv);

        // Alice customizes her profile; Bob catches the profile document up and converges
        // (the distinctive capitalized "Alice" + effect proves it is her *custom* profile,
        // not just the seeded lowercase display name).
        alice
            .set_profile(Profile {
                name: "Alice".into(),
                color: "#ffcc00".into(),
                font: "serif".into(),
                effect: "wave".into(),
                ..Default::default()
            })
            .await;
        bob.catch_up_profiles(alice_peer).await;

        timeout(Duration::from_secs(60), async {
            loop {
                if bob.profiles().await.values().any(|p| p.name == "Alice") {
                    break;
                }
                match bob_events.recv().await {
                    Some(_) => continue,
                    None => panic!("bob actor closed"),
                }
            }
        })
        .await
        .expect("bob did not converge on Alice's profile");

        let alice_profile = bob
            .profiles()
            .await
            .into_values()
            .find(|p| p.name == "Alice")
            .expect("Alice's profile present");
        assert_eq!(alice_profile.effect, "wave");
        assert_eq!(alice_profile.font, "serif");

        alice.shutdown().await;
        bob.shutdown().await;
        let _ = alice_handle.await;
        let _ = bob_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_founder_catches_up_a_channel_the_joiner_created() {
        const SECRET: u128 = 0xBEEF;
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice_srv = founder(&hub, alice_peer, "alice", 1);
        alice_srv.subscribe_control().await.unwrap();
        let invite = alice_srv.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (alice, mut alice_events, alice_handle) = spawn(alice_srv);

        let bob_srv = Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &invite,
        )
        .await
        .unwrap();
        let (bob, mut bob_events, bob_handle) = spawn(bob_srv);

        // Bob creates a channel Alice has never opened and posts to it.
        bob.open_channel(SECRET).await;
        bob.send_message(SECRET, "from bob").await;
        timeout(Duration::from_secs(60), async {
            loop {
                if bob
                    .messages(SECRET)
                    .await
                    .iter()
                    .any(|m| m.text == "from bob")
                {
                    break;
                }
                match bob_events.recv().await {
                    Some(_) => continue,
                    None => panic!("bob actor closed"),
                }
            }
        })
        .await
        .expect("bob has his own message");

        // Alice opens the same channel and pulls the backlog with no named peer — the
        // founder catching up a joiner-created channel (the symmetric case 8i could not do).
        alice.open_channel(SECRET).await;
        alice.catch_up_any(SECRET).await;
        timeout(Duration::from_secs(60), async {
            loop {
                if alice
                    .messages(SECRET)
                    .await
                    .iter()
                    .any(|m| m.text == "from bob")
                {
                    break;
                }
                match alice_events.recv().await {
                    Some(_) => continue,
                    None => panic!("alice actor closed"),
                }
            }
        })
        .await
        .expect("alice caught up the joiner-created channel from the best peer");

        alice.shutdown().await;
        bob.shutdown().await;
        let _ = alice_handle.await;
        let _ = bob_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_avatar_is_fetched_over_the_blob_layer() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice_srv = founder(&hub, alice_peer, "alice", 1);
        alice_srv.subscribe_control().await.unwrap();
        let invite = alice_srv.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (alice, _alice_events, alice_handle) = spawn(alice_srv);

        let bob_srv = Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &invite,
        )
        .await
        .unwrap();
        let (bob, mut bob_events, bob_handle) = spawn(bob_srv);

        // Alice sets an avatar: its bytes go to the blob store, only the CID into her
        // profile. Bob converges her profile, then his actor fetches the avatar blob over
        // the mesh — proving an image travels by content address, not inline in gossip.
        let avatar = vec![0xABu8; 1234];
        alice
            .set_profile(Profile {
                name: "Alice".into(),
                avatar: avatar.clone(),
                ..Default::default()
            })
            .await;
        bob.catch_up_profiles(alice_peer).await;

        timeout(Duration::from_secs(60), async {
            loop {
                if bob.profiles().await.values().any(|p| p.avatar == avatar) {
                    break;
                }
                match bob_events.recv().await {
                    Some(_) => continue,
                    None => panic!("bob actor closed"),
                }
            }
        })
        .await
        .expect("bob fetched Alice's avatar over the blob layer");

        alice.shutdown().await;
        bob.shutdown().await;
        let _ = alice_handle.await;
        let _ = bob_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_shared_file_appears_in_another_members_index() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice_srv = founder(&hub, alice_peer, "alice", 1);
        alice_srv.subscribe_control().await.unwrap();
        let invite = alice_srv.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (alice, _alice_events, alice_handle) = spawn(alice_srv);

        let bob_srv = Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &invite,
        )
        .await
        .unwrap();
        let (bob, mut bob_events, bob_handle) = spawn(bob_srv);

        // Alice shares a file: bytes to the blob store, only the metadata into the index.
        let data = b"shared document contents".to_vec();
        alice
            .add_file(
                "doc.txt".into(),
                "text/plain".into(),
                "".into(),
                data.clone(),
            )
            .await
            .unwrap();

        // Bob pulls the file index from Alice (request/response — what the real app does on
        // join) until the shared file appears. The loop retries the catch-up every ~50ms
        // and drains bob_events when present (so a full event channel can't stall his
        // actor) — fully deterministic, no reliance on gossip timing or peer discovery.
        // (Fetching the blob bytes over the mesh is covered deterministically by
        //  catcoms-sync::tests::a_blob_is_fetched_from_a_member_over_the_mesh.)
        let file = timeout(Duration::from_secs(60), async {
            loop {
                bob.catch_up_files(alice_peer).await;
                if let Some(f) = bob.files().await.into_iter().find(|f| f.name == "doc.txt") {
                    break f;
                }
                let _ = timeout(Duration::from_millis(50), bob_events.recv()).await;
            }
        })
        .await
        .expect("bob sees the shared file in the index");
        assert_eq!(file.size, data.len() as u64);
        assert_eq!(
            file.cid.len(),
            32,
            "the index carries the file's content address"
        );

        alice.shutdown().await;
        bob.shutdown().await;
        let _ = alice_handle.await;
        let _ = bob_handle.await;
    }
}
