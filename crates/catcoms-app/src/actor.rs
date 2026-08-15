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

use catcoms_crypto::{DeviceCertificate, DeviceId};
use catcoms_rt::{CryptoRngCore, MeshTransport, PeerId};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use catcoms_storage::Cid;

use crate::{
    ChatMessage, DeliveryState, DeviceEntry, FileEntry, FilesView, InboxItem, Livery, MemberBadge,
    MemberView, MessageStats, Profile, Server, ServerEvent,
};

/// Per drive: how long to wait for a discovered record before concluding the queue is drained.
const DISCOVERY_DRAIN_MS: u64 = 500;
/// Minimum gap between delivery snapshots for one channel (ms, on the injected clock). Delivery
/// evidence is derived by walking the channel document's change graph, so it is recomputed on a
/// timer rather than on every inbound op — and the event only fires when the result actually
/// changed.
const DELIVERY_THROTTLE_MS: u64 = 1_000;
/// Per-tick cap on discovered records ingested, so one tick can't block the actor unboundedly.
const MAX_DISCOVERED_PER_TICK: usize = 16;

/// A fetched + decrypted file chunk: its plaintext bytes plus the provider that served it (or an
/// error string). One chunk per command keeps the actor responsive during a large download.
type ChunkResult = Result<(Vec<u8>, Option<String>), String>;

/// A command from the UI to a running server actor.
#[derive(Debug)]
pub enum AppCommand {
    /// Open a channel (subscribe + create locally). Acked once subscribed, so a caller
    /// can avoid racing a subsequent publish ahead of the subscription.
    OpenChannel {
        channel: u128,
        ack: oneshot::Sender<()>,
    },
    /// Send a chat message to a channel (`reply_to` = the parent message id, or empty).
    SendMessage {
        channel: u128,
        text: String,
        reply_to: String,
    },
    /// Edit the text of one of your own messages (by id) in a channel.
    EditMessage {
        channel: u128,
        id: String,
        text: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Delete one of your own messages (by id) from a channel.
    DeleteMessage {
        channel: u128,
        id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Toggle this member's emoji reaction on a message (by id) in a channel.
    ToggleReaction {
        channel: u128,
        id: String,
        emoji: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Pin or unpin a message (by id) in a channel (owner/admin).
    SetPin {
        channel: u128,
        id: String,
        pinned: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Set (or clear) a channel's topic (any member).
    SetChannelTopic {
        channel: u128,
        topic: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Query a channel's current topic.
    ChannelTopic {
        channel: u128,
        reply: oneshot::Sender<String>,
    },
    /// Pull a channel's history from `peer` (e.g. right after joining).
    CatchUp { peer: PeerId, channel: u128 },
    /// Pull a channel's history from the best known peer (no peer named).
    CatchUpAny { channel: u128 },
    /// Query a channel's current materialized messages.
    Messages {
        channel: u128,
        reply: oneshot::Sender<Vec<ChatMessage>>,
    },
    /// Query a channel's lightweight activity stats (count + timestamps; no text).
    MessageStats {
        channel: u128,
        reply: oneshot::Sender<MessageStats>,
    },
    /// Query this server's mention/reply inbox (newest first, capped at `limit`).
    Inbox {
        limit: usize,
        reply: oneshot::Sender<Vec<InboxItem>>,
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
    /// Publish the server livery (owner/admin only).
    SetLivery {
        livery: Livery,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Set (or clear, with `""`) the shared server icon (owner/admin only).
    SetServerIcon {
        icon: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Set (or clear, with `""`) the shared server cursor (owner/admin only).
    SetServerCursor {
        cursor: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Query the server's published livery.
    Livery { reply: oneshot::Sender<Livery> },
    /// Pull the livery document from `peer` (e.g. right after joining).
    CatchUpLivery { peer: PeerId },
    /// Assign (or clear, with an empty label) a member's custom badge (owner/admin only).
    SetMemberBadge {
        fp: String,
        label: String,
        color: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Query every assigned member badge, keyed by fingerprint.
    Badges {
        reply: oneshot::Sender<HashMap<String, MemberBadge>>,
    },
    /// Pull the badge document from `peer` (e.g. right after joining).
    CatchUpBadges { peer: PeerId },
    /// Query the companion-device registry (multi-device M3), keyed by companion fingerprint.
    Devices {
        reply: oneshot::Sender<HashMap<String, DeviceEntry>>,
    },
    /// Pull the companion-device registry from `peer` (e.g. right after joining).
    CatchUpDevices { peer: PeerId },
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
    /// Query the shared file list with per-file local-availability counts + a reachable-peer flag.
    FilesView { reply: oneshot::Sender<FilesView> },
    /// Query the fingerprints of members reachable right now (presence).
    OnlineMembers { reply: oneshot::Sender<Vec<String>> },
    /// Query delivery state for this device's recent messages in a channel, so a UI can paint it
    /// on open instead of waiting for the next throttled `DeliveryChanged`.
    DeliverySnapshot {
        channel: u128,
        reply: oneshot::Sender<Vec<DeliveryState>>,
    },
    /// Query pending incoming DM (friend) requests: `(sender fp, sender name, invite bytes)`.
    DmRequests {
        reply: oneshot::Sender<Vec<(String, String, Vec<u8>)>>,
    },
    /// Dismiss a pending DM request by the sender's fingerprint (accepted or declined).
    DismissDmRequest { from_fp: String },
    /// Deliver a DM (friend) invite to a member over this group ("Add friend"); `true` if reached.
    SendDmInvite {
        target_fp: String,
        invite: Vec<u8>,
        reply: oneshot::Sender<Result<bool, String>>,
    },
    /// Push a call-signalling message (opaque payload) to a member; `true` if reached.
    SendCallSignal {
        target_fp: String,
        payload: Vec<u8>,
        reply: oneshot::Sender<Result<bool, String>>,
    },
    /// This call's E2E media base key + epoch (derived from MLS; never sent on the wire).
    MediaKey {
        call_id: u128,
        reply: oneshot::Sender<Result<(Vec<u8>, u64), String>>,
    },
    /// The download plan for a file by content address: `(total chunks, total size)`, or `None`.
    /// The bridge fetches the chunks one per command so the actor stays responsive between them.
    FileDownloadPlan {
        cid: Vec<u8>,
        reply: oneshot::Sender<Option<(usize, u64)>>,
    },
    /// Fetch + decrypt a single chunk (`idx`) of a file: `(plaintext bytes, provider)`, or an error.
    FetchFileChunk {
        cid: Vec<u8>,
        idx: usize,
        reply: oneshot::Sender<ChunkResult>,
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
    /// Create a server event (any member); replies with its id, or a validation error.
    CreateEvent {
        title: String,
        body: String,
        start_ts: u64,
        end_ts: u64,
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Delete a server event by id (its author, or an owner/admin).
    DeleteEvent {
        id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Query the server events, sorted by start time ascending.
    Events {
        reply: oneshot::Sender<Vec<ServerEvent>>,
    },
    /// Pull the calendar document from `peer` (e.g. right after joining).
    CatchUpCalendar { peer: PeerId },
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
    /// Query the wiki's per-page render formats (name -> "md" | "wiki"); absent = markdown.
    WikiMeta {
        reply: oneshot::Sender<HashMap<String, String>>,
    },
    /// Set a wiki page's render format ("md" or "wiki"); replies with a validation error.
    SetWikiFormat {
        name: String,
        format: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Delete a wiki page (and its format metadata); replies with an error if it is missing.
    DeleteWikiPage {
        name: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Rename a wiki page, carrying its body and format; replies with an error if `from` is
    /// missing or `to` is taken.
    RenameWikiPage {
        from: String,
        to: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
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
    /// Revoke one of your own linked devices (M5): the origin signs a revocation; the owner
    /// enforces the MLS Remove.
    RevokeDevice {
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
    /// This member's origin identity on this server: its device id plus the group id.
    /// Read-only — the grant ceremony (multi-device M2) needs both to anchor the SAS
    /// and to tell the new device which group a grant is for.
    OriginIdentity {
        reply: oneshot::Sender<(DeviceId, Vec<u8>)>,
    },
    /// The **server owner's** signature key, which a grant pins so the new device can
    /// authenticate the owner's Welcome before it has a roster (multi-device M3).
    OwnerPublicKey {
        reply: oneshot::Sender<Option<[u8; 32]>>,
    },
    /// Sign a device certificate for a companion device with **this** server's origin
    /// key (multi-device M2). Deliberately narrow: the key never leaves the actor, so
    /// there is no command that exports it.
    SignDeviceCert {
        new_device_id: DeviceId,
        device_name: String,
        reply: oneshot::Sender<Result<DeviceCertificate, String>>,
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
    /// The server livery changed — the UI should re-fetch it (`livery`) and re-apply it.
    LiveryUpdated,
    /// A custom member badge changed — the UI should re-fetch badges (`badges`).
    BadgesUpdated,
    /// The companion-device registry changed — the UI should re-fetch it (`devices`) and
    /// re-resolve message attribution (multi-device M3/M4).
    DevicesUpdated,
    /// The shared file list changed — the UI should re-fetch it (`files`).
    FilesUpdated,
    /// The status feed changed — the UI should re-fetch it (`statuses`).
    StatusUpdated,
    /// The server events (calendar) changed — the UI should re-fetch them (`events`).
    EventsUpdated,
    /// The wiki changed — the UI should re-fetch pages / the open page.
    WikiUpdated,
    /// Member roles changed — the UI should re-fetch roles.
    RolesUpdated,
    /// The advisory eclipse verdict changed: `true` = the node may be isolated (verify a member
    /// out of band). Surfaced as a UI hint; never gates anything.
    EclipseChanged { caution: bool },
    /// The set of members reachable right now (a live connection) changed — `online` is their
    /// fingerprints, for the roster's presence indicators + the file-availability hint.
    ConnectivityChanged { online: Vec<String> },
    /// Delivery state changed for this device's recent messages in `channel` (oldest first).
    /// Recomputed at most once a second per channel and emitted only on a real change, so a UI
    /// can render it directly without polling.
    DeliveryChanged {
        channel: u128,
        states: Vec<DeliveryState>,
    },
    /// The set of pending incoming DM (friend) requests changed — the UI should re-fetch them.
    DmRequestsChanged,
    /// An inbound call-signalling message arrived: `(sender fingerprint, opaque payload)`. One event
    /// per signal; the UI decodes the payload (`{callId, type, data}`) and drives WebRTC.
    CallSignal { from_fp: String, payload: Vec<u8> },
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
                reply_to: String::new(),
            })
            .await;
    }

    /// Send a chat message replying to `reply_to` (the parent message's id).
    pub async fn send_reply(
        &self,
        channel: u128,
        text: impl Into<String>,
        reply_to: impl Into<String>,
    ) {
        let _ = self
            .cmd_tx
            .send(AppCommand::SendMessage {
                channel,
                text: text.into(),
                reply_to: reply_to.into(),
            })
            .await;
    }

    /// Edit the text of one of your own messages (by id) in a channel.
    pub async fn edit_message(
        &self,
        channel: u128,
        id: String,
        text: String,
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::EditMessage {
                channel,
                id,
                text,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Delete one of your own messages (by id) from a channel.
    pub async fn delete_message(&self, channel: u128, id: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::DeleteMessage { channel, id, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Toggle this member's emoji reaction on a message (by id) in a channel.
    pub async fn toggle_reaction(
        &self,
        channel: u128,
        id: String,
        emoji: String,
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::ToggleReaction {
                channel,
                id,
                emoji,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Pin or unpin a message (by id) in a channel (owner/admin).
    pub async fn set_pin(&self, channel: u128, id: String, pinned: bool) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetPin {
                channel,
                id,
                pinned,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Set (or clear, with `""`) a channel's topic. Any member may — see
    /// [`Server::set_channel_topic`]. A `ChannelUpdated` event follows if it changed.
    pub async fn set_channel_topic(&self, channel: u128, topic: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetChannelTopic {
                channel,
                topic,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Fetch a channel's current topic (empty if unset).
    pub async fn channel_topic(&self, channel: u128) -> String {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::ChannelTopic { channel, reply })
            .await
            .is_err()
        {
            return String::new();
        }
        rx.await.unwrap_or_default()
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

    /// Fetch a channel's lightweight activity stats (count + timestamps; no text).
    pub async fn message_stats(&self, channel: u128) -> MessageStats {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::MessageStats { channel, reply })
            .await
            .is_err()
        {
            return MessageStats::default();
        }
        rx.await.unwrap_or_default()
    }

    /// Fetch this server's mention/reply inbox (newest first, capped at `limit`).
    pub async fn inbox(&self, limit: usize) -> Vec<InboxItem> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Inbox { limit, reply })
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

    /// This member's origin identity on this server: `(device id, group id)`.
    /// Returns `None` if the actor has stopped.
    pub async fn origin_identity(&self) -> Option<(DeviceId, Vec<u8>)> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::OriginIdentity { reply })
            .await
            .is_err()
        {
            return None;
        }
        rx.await.ok()
    }

    /// The server owner's signature key, to pin into a device grant (multi-device M3).
    /// `None` if the actor has stopped or the group has no readable owner key.
    pub async fn owner_public_key(&self) -> Option<[u8; 32]> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::OwnerPublicKey { reply })
            .await
            .is_err()
        {
            return None;
        }
        rx.await.ok().flatten()
    }

    /// Sign a device certificate for a companion device with this server's origin key
    /// (multi-device M2). The key never leaves the actor — only the certificate does.
    pub async fn sign_device_cert(
        &self,
        new_device_id: DeviceId,
        device_name: String,
    ) -> Result<DeviceCertificate, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SignDeviceCert {
                new_device_id,
                device_name,
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

    /// Publish the server livery (owner/admin only; a `LiveryUpdated` event follows).
    pub async fn set_livery(&self, livery: Livery) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetLivery { livery, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Set (or clear, with `""`) the shared server icon — base64 image bytes (owner/admin
    /// only; a `LiveryUpdated` event follows). Publishing colours never disturbs it.
    pub async fn set_server_icon(&self, icon: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetServerIcon { icon, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Set (or clear, with `""`) the shared server cursor — base64 image bytes (owner/admin
    /// only; a `LiveryUpdated` event follows). Publishing colours, or the icon, never
    /// disturbs it.
    pub async fn set_server_cursor(&self, cursor: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetServerCursor { cursor, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Fetch the server's published livery.
    pub async fn livery(&self) -> Livery {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Livery { reply })
            .await
            .is_err()
        {
            return Livery::default();
        }
        rx.await.unwrap_or_default()
    }

    /// Pull the livery document from `peer`.
    pub async fn catch_up_livery(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpLivery { peer }).await;
    }

    /// Assign (or clear, with an empty `label`) a member's custom badge (owner/admin only; a
    /// `BadgesUpdated` event follows).
    pub async fn set_member_badge(
        &self,
        fp: String,
        label: String,
        color: String,
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetMemberBadge {
                fp,
                label,
                color,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Fetch every assigned member badge, keyed by fingerprint.
    pub async fn badges(&self) -> HashMap<String, MemberBadge> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Badges { reply })
            .await
            .is_err()
        {
            return HashMap::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Pull the badge document from `peer`.
    pub async fn catch_up_badges(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpBadges { peer }).await;
    }

    /// Fetch the companion-device registry, keyed by companion fingerprint (multi-device M3).
    pub async fn devices(&self) -> HashMap<String, DeviceEntry> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Devices { reply })
            .await
            .is_err()
        {
            return HashMap::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Pull the companion-device registry from `peer`.
    pub async fn catch_up_devices(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpDevices { peer }).await;
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

    /// Fetch the shared file list with per-file local-availability counts + the reachable-peer flag.
    pub async fn files_view(&self) -> FilesView {
        let (reply, rx) = oneshot::channel();
        let empty = || FilesView {
            files: Vec::new(),
            has_peers: false,
        };
        if self
            .cmd_tx
            .send(AppCommand::FilesView { reply })
            .await
            .is_err()
        {
            return empty();
        }
        rx.await.unwrap_or_else(|_| empty())
    }

    /// Fetch the fingerprints of members reachable right now (presence).
    pub async fn online_members(&self) -> Vec<String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::OnlineMembers { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Fetch delivery state for this device's recent messages in a channel (oldest first).
    pub async fn delivery_snapshot(&self, channel: u128) -> Vec<DeliveryState> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::DeliverySnapshot { channel, reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Fetch pending incoming DM (friend) requests: `(sender fp, sender name, invite bytes)`.
    pub async fn dm_requests(&self) -> Vec<(String, String, Vec<u8>)> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::DmRequests { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Dismiss a pending DM request by the sender's fingerprint.
    pub async fn dismiss_dm_request(&self, from_fp: String) {
        let _ = self
            .cmd_tx
            .send(AppCommand::DismissDmRequest { from_fp })
            .await;
    }

    /// Deliver a DM (friend) invite to a member over this group; `Ok(true)` if reached.
    pub async fn send_dm_invite(&self, target_fp: String, invite: Vec<u8>) -> Result<bool, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SendDmInvite {
                target_fp,
                invite,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Push a call-signalling message (opaque payload) to a member; `Ok(true)` if reached.
    pub async fn send_call_signal(
        &self,
        target_fp: String,
        payload: Vec<u8>,
    ) -> Result<bool, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SendCallSignal {
                target_fp,
                payload,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// This call's E2E media base key (raw bytes) + the epoch it's keyed to.
    pub async fn media_key(&self, call_id: u128) -> Result<(Vec<u8>, u64), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::MediaKey { call_id, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// The download plan for a file by content address: `(total chunks, total size)`, or `None` if
    /// not listed / corrupt / implausibly large.
    pub async fn file_download_plan(&self, cid: Vec<u8>) -> Option<(usize, u64)> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::FileDownloadPlan { cid, reply })
            .await
            .is_err()
        {
            return None;
        }
        rx.await.unwrap_or(None)
    }

    /// Fetch + decrypt a single chunk (`idx`) of a file: `(plaintext bytes, provider)`. One chunk
    /// per call so the actor interleaves other work between chunks (the orchestrator reassembles).
    pub async fn fetch_file_chunk(&self, cid: Vec<u8>, idx: usize) -> ChunkResult {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::FetchFileChunk { cid, idx, reply })
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

    /// Create a server event (any member); replies with its id. An `EventsUpdated` event follows
    /// on success.
    pub async fn create_event(
        &self,
        title: String,
        body: String,
        start_ts: u64,
        end_ts: u64,
    ) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::CreateEvent {
                title,
                body,
                start_ts,
                end_ts,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Delete a server event by id (its author, or an owner/admin). An `EventsUpdated` event
    /// follows on success.
    pub async fn delete_event(&self, id: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::DeleteEvent { id, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Fetch the server events, sorted by start time ascending.
    pub async fn events(&self) -> Vec<ServerEvent> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Events { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Pull the calendar document from `peer`.
    pub async fn catch_up_calendar(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpCalendar { peer }).await;
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

    /// Revoke one of your own linked (companion) devices by fingerprint.
    pub async fn revoke_device(&self, fp: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::RevokeDevice { fp, reply })
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

    /// Fetch the wiki's per-page render formats (name -> "md" | "wiki"); a page absent from the
    /// map has no declared format and renders as markdown.
    pub async fn wiki_meta(&self) -> HashMap<String, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::WikiMeta { reply })
            .await
            .is_err()
        {
            return HashMap::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Set a wiki page's render format — "md" or "wiki" (a `WikiUpdated` event follows).
    pub async fn set_wiki_format(
        &self,
        name: impl Into<String>,
        format: impl Into<String>,
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetWikiFormat {
                name: name.into(),
                format: format.into(),
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Delete a wiki page and its format metadata (a `WikiUpdated` event follows).
    pub async fn delete_wiki_page(&self, name: impl Into<String>) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::DeleteWikiPage {
                name: name.into(),
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Rename a wiki page, carrying its body and format (a `WikiUpdated` event follows).
    pub async fn rename_wiki_page(
        &self,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::RenameWikiPage {
                from: from.into(),
                to: to.into(),
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
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
        // Per open channel: a content signature of its messages (see `channel_changed`), so an
        // edit/delete/add all surface a `ChannelUpdated`.
        let mut counts: HashMap<u128, u64> = HashMap::new();
        let mut members = server.member_count();
        // Open the per-server profile document and seed this member's name from the
        // display name, so the roster/messages show a name immediately (the user can
        // customize color/font/effect later via SetProfile). Seed ONLY when this device has no
        // profile entry yet (first founding/join) — otherwise a reload would overwrite the
        // customized name/color/font the user saved with the founding display name + defaults.
        if let Err(e) = server.open_profiles().await {
            tracing::warn!(error = %e, "open_profiles failed");
        }
        if !server.profiles().contains_key(&server.my_fingerprint()) {
            let seed = Profile {
                name: server.display_name().to_string(),
                ..Profile::default()
            };
            if let Err(e) = server.set_profile(seed).await {
                tracing::warn!(error = %e, "seed profile failed");
            }
        }
        let mut last_profiles = server.profiles();
        // …and the livery doc, so an owner/admin's published scheme reaches this client.
        if let Err(e) = server.open_livery().await {
            tracing::warn!(error = %e, "open_livery failed");
        }
        let mut last_livery = server.livery();
        // …and the badge doc, so an owner/admin's assigned badges reach this client.
        if let Err(e) = server.open_badges().await {
            tracing::warn!(error = %e, "open_badges failed");
        }
        let mut last_badges = server.badges();
        // …and the companion-device registry, so a member's second device is attributable to the
        // member (multi-device M3) and the owner's admission gate sees the current map.
        if let Err(e) = server.open_devices().await {
            tracing::warn!(error = %e, "open_devices failed");
        }
        let mut last_devices = server.devices();
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
        // …and the calendar, so the server's scheduled events reach this client.
        if let Err(e) = server.open_calendar().await {
            tracing::warn!(error = %e, "open_calendar failed");
        }
        let mut last_events = server.events();
        // …and the wiki.
        if let Err(e) = server.open_wiki().await {
            tracing::warn!(error = %e, "open_wiki failed");
        }
        let mut last_wiki = wiki_snapshot(&server);
        // …and subscribe the member-roles doc so admin grants propagate. The *owner* role is
        // not stored here — it is derived from the MLS designated committer (lowest leaf
        // index), so every member computes the owner identically with no roles op present.
        if let Err(e) = server.open_roles().await {
            tracing::warn!(error = %e, "open_roles failed");
        }
        let mut last_roles = server.roles();
        let mut last_eclipse = false;
        let mut last_online = server.online_members();
        let mut last_dm_requests = server.dm_requests();
        // Per channel: when delivery state was last recomputed, and what it was — the throttle
        // plus the change detector for `DeliveryChanged`.
        let mut delivery: HashMap<u128, (u64, Vec<DeliveryState>)> = HashMap::new();
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => match cmd {
                    Some(AppCommand::OpenChannel { channel, ack }) => {
                        if let Err(e) = server.open_channel(channel).await {
                            tracing::warn!(error = %e, channel, "open_channel failed");
                        }
                        // Seed (and start tracking) the channel's current content signature WITHOUT
                        // emitting — the UI fetches messages on open (switchTo → refresh); only a
                        // later add/edit/delete should fire ChannelUpdated. (A non-empty channel's
                        // signature is non-zero, so the old `or_insert(0)` would have spuriously
                        // signalled "changed" on open under the content-hash detector.)
                        channel_changed(&server, channel, &mut counts);
                        let _ = ack.send(());
                    }
                    Some(AppCommand::SendMessage {
                        channel,
                        text,
                        reply_to,
                    }) => {
                        if let Err(e) = server.send_reply(channel, &text, &reply_to).await {
                            tracing::warn!(error = %e, channel, "send_message failed");
                        }
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::EditMessage {
                        channel,
                        id,
                        text,
                        reply,
                    }) => {
                        let res = server
                            .edit_message(channel, &id, &text)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::DeleteMessage { channel, id, reply }) => {
                        let res = server
                            .delete_message(channel, &id)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::ToggleReaction {
                        channel,
                        id,
                        emoji,
                        reply,
                    }) => {
                        let res = server
                            .toggle_reaction(channel, &id, &emoji)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::SetPin {
                        channel,
                        id,
                        pinned,
                        reply,
                    }) => {
                        let res = server
                            .set_pin(channel, &id, pinned)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::SetChannelTopic {
                        channel,
                        topic,
                        reply,
                    }) => {
                        let res = server
                            .set_channel_topic(channel, &topic)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::ChannelTopic { channel, reply }) => {
                        let _ = reply.send(server.channel_topic(channel));
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
                    Some(AppCommand::MessageStats { channel, reply }) => {
                        let _ = reply.send(server.message_stats(channel));
                    }
                    Some(AppCommand::Inbox { limit, reply }) => {
                        let _ = reply.send(server.inbox(limit));
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
                    Some(AppCommand::SetLivery { livery, reply }) => {
                        let res = server.set_livery(livery).await.map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if livery_changed(&server, &mut last_livery) {
                            let _ = event_tx.send(AppEvent::LiveryUpdated).await;
                        }
                    }
                    Some(AppCommand::SetServerIcon { icon, reply }) => {
                        let res = server.set_server_icon(icon).await.map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if livery_changed(&server, &mut last_livery) {
                            let _ = event_tx.send(AppEvent::LiveryUpdated).await;
                        }
                    }
                    Some(AppCommand::SetServerCursor { cursor, reply }) => {
                        let res = server
                            .set_server_cursor(cursor)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if livery_changed(&server, &mut last_livery) {
                            let _ = event_tx.send(AppEvent::LiveryUpdated).await;
                        }
                    }
                    Some(AppCommand::Livery { reply }) => {
                        let _ = reply.send(server.livery());
                    }
                    Some(AppCommand::CatchUpLivery { peer }) => {
                        if let Err(e) = server.request_livery_catchup(peer).await {
                            tracing::warn!(error = %e, "livery catch-up failed");
                        }
                        if livery_changed(&server, &mut last_livery) {
                            let _ = event_tx.send(AppEvent::LiveryUpdated).await;
                        }
                    }
                    Some(AppCommand::SetMemberBadge { fp, label, color, reply }) => {
                        let res = server
                            .set_member_badge(fp, label, color)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if badges_changed(&server, &mut last_badges) {
                            let _ = event_tx.send(AppEvent::BadgesUpdated).await;
                        }
                    }
                    Some(AppCommand::Badges { reply }) => {
                        let _ = reply.send(server.badges());
                    }
                    Some(AppCommand::CatchUpBadges { peer }) => {
                        if let Err(e) = server.request_badges_catchup(peer).await {
                            tracing::warn!(error = %e, "badges catch-up failed");
                        }
                        if badges_changed(&server, &mut last_badges) {
                            let _ = event_tx.send(AppEvent::BadgesUpdated).await;
                        }
                    }
                    Some(AppCommand::Devices { reply }) => {
                        let _ = reply.send(server.devices());
                    }
                    Some(AppCommand::CatchUpDevices { peer }) => {
                        if let Err(e) = server.request_devices_catchup(peer).await {
                            tracing::warn!(error = %e, "devices catch-up failed");
                        }
                        if devices_changed(&server, &mut last_devices) {
                            let _ = event_tx.send(AppEvent::DevicesUpdated).await;
                        }
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
                    Some(AppCommand::FilesView { reply }) => {
                        let _ = reply.send(server.files_view());
                    }
                    Some(AppCommand::OnlineMembers { reply }) => {
                        let _ = reply.send(server.online_members());
                    }
                    Some(AppCommand::DeliverySnapshot { channel, reply }) => {
                        let _ = reply.send(server.delivery_snapshot(channel));
                    }
                    Some(AppCommand::DmRequests { reply }) => {
                        let _ = reply.send(server.dm_requests());
                    }
                    Some(AppCommand::DismissDmRequest { from_fp }) => {
                        server.dismiss_dm_request(&from_fp);
                        if dm_requests_changed(&server, &mut last_dm_requests) {
                            let _ = event_tx.send(AppEvent::DmRequestsChanged).await;
                        }
                    }
                    Some(AppCommand::SendDmInvite {
                        target_fp,
                        invite,
                        reply,
                    }) => {
                        let res = server
                            .send_dm_invite(&target_fp, &invite)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                    }
                    Some(AppCommand::SendCallSignal {
                        target_fp,
                        payload,
                        reply,
                    }) => {
                        let res = server
                            .send_call_signal(&target_fp, &payload)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                    }
                    Some(AppCommand::MediaKey { call_id, reply }) => {
                        let res = server
                            .media_key(call_id)
                            .map(|(k, epoch)| (k.to_vec(), epoch))
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                    }
                    Some(AppCommand::FileDownloadPlan { cid, reply }) => {
                        let plan = <[u8; 32]>::try_from(cid.as_slice())
                            .ok()
                            .and_then(|arr| server.file_download_plan(&Cid::from_bytes(arr)));
                        let _ = reply.send(plan);
                    }
                    // Fetch ONE chunk, then return to the select! loop — so a large download no
                    // longer pins the actor: other commands + sync_once interleave between chunks
                    // (the bridge orchestrates the per-chunk loop + reassembly + progress).
                    Some(AppCommand::FetchFileChunk { cid, idx, reply }) => {
                        let res = match <[u8; 32]>::try_from(cid.as_slice()) {
                            Ok(arr) => server
                                .fetch_file_chunk(&Cid::from_bytes(arr), idx)
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
                    Some(AppCommand::CreateEvent { title, body, start_ts, end_ts, reply }) => {
                        let res = server
                            .create_event(&title, &body, start_ts, end_ts)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if events_changed(&server, &mut last_events) {
                            let _ = event_tx.send(AppEvent::EventsUpdated).await;
                        }
                    }
                    Some(AppCommand::DeleteEvent { id, reply }) => {
                        let res = server.delete_event(&id).await.map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if events_changed(&server, &mut last_events) {
                            let _ = event_tx.send(AppEvent::EventsUpdated).await;
                        }
                    }
                    Some(AppCommand::Events { reply }) => {
                        let _ = reply.send(server.events());
                    }
                    Some(AppCommand::CatchUpCalendar { peer }) => {
                        if let Err(e) = server.request_calendar_catchup(peer).await {
                            tracing::warn!(error = %e, "calendar catch-up failed");
                        }
                        if events_changed(&server, &mut last_events) {
                            let _ = event_tx.send(AppEvent::EventsUpdated).await;
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
                    Some(AppCommand::RevokeDevice { fp, reply }) => {
                        let res = server.revoke_device(&fp).await.map_err(|e| e.to_string());
                        let _ = reply.send(res);
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
                    Some(AppCommand::WikiMeta { reply }) => {
                        let _ = reply.send(server.wiki_meta());
                    }
                    Some(AppCommand::SetWikiFormat { name, format, reply }) => {
                        let res = server
                            .set_wiki_page_format(&name, &format)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if wiki_changed(&server, &mut last_wiki) {
                            let _ = event_tx.send(AppEvent::WikiUpdated).await;
                        }
                    }
                    Some(AppCommand::DeleteWikiPage { name, reply }) => {
                        let res = server.delete_wiki_page(&name).await.map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if wiki_changed(&server, &mut last_wiki) {
                            let _ = event_tx.send(AppEvent::WikiUpdated).await;
                        }
                    }
                    Some(AppCommand::RenameWikiPage { from, to, reply }) => {
                        let res = server
                            .rename_wiki_page(&from, &to)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
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
                        // Re-evaluate the advisory eclipse verdict each pass; surface a change.
                        let caution = server.observe_eclipse();
                        if caution != last_eclipse {
                            last_eclipse = caution;
                            let _ = event_tx.send(AppEvent::EclipseChanged { caution }).await;
                        }
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
                    Some(AppCommand::OriginIdentity { reply }) => {
                        let _ = reply.send((server.device_id(), server.group_id()));
                    }
                    Some(AppCommand::OwnerPublicKey { reply }) => {
                        let _ = reply.send(server.owner_public_key());
                    }
                    Some(AppCommand::SignDeviceCert { new_device_id, device_name, reply }) => {
                        let res = server
                            .issue_device_certificate(new_device_id, &device_name)
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
                        if livery_changed(&server, &mut last_livery) {
                            let _ = event_tx.send(AppEvent::LiveryUpdated).await;
                        }
                        if badges_changed(&server, &mut last_badges) {
                            let _ = event_tx.send(AppEvent::BadgesUpdated).await;
                        }
                        if devices_changed(&server, &mut last_devices) {
                            let _ = event_tx.send(AppEvent::DevicesUpdated).await;
                        }
                        if files_changed(&server, &mut file_count) {
                            let _ = event_tx.send(AppEvent::FilesUpdated).await;
                        }
                        if status_changed(&server, &mut status_count) {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                        if events_changed(&server, &mut last_events) {
                            let _ = event_tx.send(AppEvent::EventsUpdated).await;
                        }
                        if wiki_changed(&server, &mut last_wiki) {
                            let _ = event_tx.send(AppEvent::WikiUpdated).await;
                        }
                        if roles_changed(&server, &mut last_roles) {
                            let _ = event_tx.send(AppEvent::RolesUpdated).await;
                        }
                        // Presence: emit when the set of currently-reachable members changes
                        // (a peer connected or dropped). `online_members` is sorted, so the Vec
                        // compare is order-stable.
                        let online = server.online_members();
                        if online != last_online {
                            last_online = online.clone();
                            let _ = event_tx
                                .send(AppEvent::ConnectivityChanged { online })
                                .await;
                        }
                        // Delivery: a peer's inbound op may be the evidence that it received one of
                        // our messages. Recomputing walks the channel's change graph, so it is
                        // rate-limited per channel; the event then fires only on a real change.
                        for channel in counts.keys().copied().collect::<Vec<_>>() {
                            let now = server.now_ms();
                            if let Some((at, _)) = delivery.get(&channel) {
                                if now.saturating_sub(*at) < DELIVERY_THROTTLE_MS {
                                    continue;
                                }
                            }
                            let states = server.delivery_snapshot(channel);
                            let changed = match delivery.insert(channel, (now, states.clone())) {
                                Some((_, previous)) => previous != states,
                                None => !states.is_empty(),
                            };
                            if changed {
                                let _ = event_tx
                                    .send(AppEvent::DeliveryChanged { channel, states })
                                    .await;
                            }
                        }
                        // A DM (friend) request may have arrived over this group — surface a change.
                        if dm_requests_changed(&server, &mut last_dm_requests) {
                            let _ = event_tx.send(AppEvent::DmRequestsChanged).await;
                        }
                        // Drain any inbound call-signalling messages, one event each, so the UI's
                        // WebRTC layer can process offers/answers/ICE promptly.
                        for (from_fp, payload) in server.take_call_signals() {
                            let _ = event_tx
                                .send(AppEvent::CallSignal { from_fp, payload })
                                .await;
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

/// Whether a channel's rendered content (its messages or its topic) changed since last seen
/// (updating the record).
/// Synchronous — the `&Server` borrow ends before the caller awaits the event send, so
/// the actor future stays `Send` (a `&Server` held across an await would require
/// `Server: Sync`, which it is not).
fn channel_changed<T, R>(
    server: &Server<T, R>,
    channel: u128,
    sigs: &mut HashMap<u128, u64>,
) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    use std::hash::{Hash, Hasher};
    // A content signature (not just the count) so an EDIT — which doesn't change the count — is
    // detected too, both locally and when a peer's edit arrives. `DefaultHasher::new()` has a fixed
    // seed, so the same content hashes the same across ticks. Cheap over a channel's message list.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // The topic is part of the channel's rendered state, so a peer's topic change refreshes the
    // UI through the same `ChannelUpdated` the message list uses.
    server.channel_topic(channel).hash(&mut h);
    for m in &server.messages(channel) {
        m.id.hash(&mut h);
        m.text.hash(&mut h);
        m.edited.hash(&mut h);
        m.pinned.hash(&mut h);
        // Reactions change the rendered message too (count unchanged), so fold them in.
        for r in &m.reactions {
            r.emoji.hash(&mut h);
            r.by.hash(&mut h);
        }
    }
    let sig = h.finish();
    if sigs.get(&channel).copied() != Some(sig) {
        sigs.insert(channel, sig);
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

/// Whether the server events changed since last seen (updating the record). A count alone would
/// miss a concurrent create+delete converging in one tick, so this compares the full list — which
/// `Server::events` already returns in a deterministic order, and whose entries are size-bounded
/// (`MAX_EVENT_TITLE_BYTES` / `MAX_EVENT_BODY_BYTES`).
fn events_changed<T, R>(server: &Server<T, R>, last: &mut Vec<ServerEvent>) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = server.events();
    if now != *last {
        *last = now;
        true
    } else {
        false
    }
}

/// What [`wiki_changed`] compares: the page map **and** the per-page format metadata. A format
/// toggle leaves every body byte-identical, so the bodies alone would miss it.
type WikiSnapshot = (HashMap<String, String>, HashMap<String, String>);

/// The wiki's current bodies + formats.
fn wiki_snapshot<T, R>(server: &Server<T, R>) -> WikiSnapshot
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    (server.wiki_map(), server.wiki_meta())
}

/// Whether the wiki changed since last seen (a page added/removed/renamed, a body edited or a
/// format toggled — a count alone misses edits, so this compares the full snapshot). Updates
/// the record.
fn wiki_changed<T, R>(server: &Server<T, R>, last: &mut WikiSnapshot) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = wiki_snapshot(server);
    if now != *last {
        *last = now;
        true
    } else {
        false
    }
}

/// Whether the server livery changed since last seen (updating the record). Compares the
/// whole materialized [`Livery`], so an **icon-only** or **cursor-only** write is caught like a
/// colour change. The colour fields are a handful of short strings; the icon and cursor are
/// bounded base64 blobs (≤ `MAX_SERVER_ICON_BYTES` / `MAX_SERVER_CURSOR_BYTES` decoded), so
/// this stays a cheap memcmp per convergence.
fn livery_changed<T, R>(server: &Server<T, R>, last: &mut Livery) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = server.livery();
    if now != *last {
        *last = now;
        true
    } else {
        false
    }
}

/// Whether any custom member badge changed since last seen (updating the record). The map is
/// bounded (`MAX_BADGES` short entries), so comparing it per convergence is cheap.
fn badges_changed<T, R>(server: &Server<T, R>, last: &mut HashMap<String, MemberBadge>) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = server.badges();
    if now != *last {
        *last = now;
        true
    } else {
        false
    }
}

/// Whether the companion-device registry changed since last seen (updating the record).
/// Bounded by `MAX_DEVICES` short entries, like the badge map.
fn devices_changed<T, R>(server: &Server<T, R>, last: &mut HashMap<String, DeviceEntry>) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = server.devices();
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

/// Whether the pending incoming DM (friend) requests changed since last seen. The list is small
/// (bounded), so comparing it per tick is cheap.
#[allow(clippy::type_complexity)]
fn dm_requests_changed<T, R>(
    server: &Server<T, R>,
    last: &mut Vec<(String, String, Vec<u8>)>,
) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = server.dm_requests();
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
