//! The async **event-stream actor** around a [`Server`] (slice 8b-1).
//!
//! A GUI can't poll `sync_once` by hand; it needs a live thing it sends *commands* to
//! and gets *events* from. [`spawn`] moves a `Server` into a background task that owns
//! it, drives the network, and translates between a command channel and an event
//! channel. The Tauri command bridge (8b-2) is a thin shell over this; tests drive it
//! directly over the in-memory transport.
//!
//! The task `select!`s between the command channel and `Server::sync_once`. When a
//! command arrives mid-`sync_once`, the in-flight `sync_once` is cancelled; safe at its
//! only real suspension point (`next_event`, which leaves the event queued); a cancel
//! during the brief pre-event recovery work may at worst drop an in-flight catch-up,
//! which the recovery machinery re-detects on the next inbound event (self-healing).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::time::Duration;

use catcoms_crypto::{DeviceCertificate, DeviceId};
use catcoms_rt::{Clock, CryptoRngCore, MeshTransport, PeerId, RequestCancellation};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use catcoms_storage::{Cid, FileRef};

use crate::{
    ChannelHead, ChannelInfo, ChatMessage, DeliverySnapshot, DeliveryState, DeviceEntry, FileEntry,
    FileMediaHead, FileRange, FileUsage, FilesView, InboxItem, JoinAttempt, JukeEntry, Livery,
    MemberBadge, MemberRecoveryApplied, MemberRecoveryCode, MemberRecoveryVerified, MemberView,
    MessageStats, ModerationState, Profile, Server, ServerEvent, StorageHealth, StorageRepair,
    StorageSnapshot, SwitchboardOffer, WikiPendingEdit, WikiRevision,
};

/// Per drive: how long to wait for a discovered record before concluding the queue is drained.
const DISCOVERY_DRAIN_MS: u64 = 500;
/// Arrival ids one channel delta carries. A notification needs the rows that arrived, not all of
/// them; a catch-up that lands a thousand messages says so with the flag and the count.
const MAX_REPORTED_ARRIVALS: usize = 32;
/// Minimum gap between delivery snapshots for one channel (ms, on the injected clock). Delivery
/// evidence is derived by walking the channel document's change graph, so it is recomputed on a
/// timer rather than on every inbound op; and the event only fires when the result actually
/// changed.
const DELIVERY_THROTTLE_MS: u64 = 1_000;
/// Per-tick cap on discovered records ingested, so one tick can't block the actor unboundedly.
const MAX_DISCOVERED_PER_TICK: usize = 16;
/// Ceiling on a **single** member-PEX request (ms). Deliberately per request rather than per
/// pass: a shared budget lets the first peer asked spend all of it, so a peer that accepts and
/// never answers silently starves every peer behind it, on every tick, forever. Whatever misses
/// its deadline is backed off (`note_pex_failure`) and retried on a later tick.
const PEX_REQUEST_MS: u64 = 3_000;

/// A fetched + decrypted file chunk: its plaintext bytes plus the provider that served it (or an
/// error string). One chunk per command keeps the actor responsive during a large download.
type ChunkResult = Result<(Vec<u8>, Option<String>), String>;

/// Await one actor-owned chunk fetch while retaining a native cancellation edge.
///
/// Dropping only the bridge's reply receiver does not cancel an [`AppCommand`] already executing
/// inside the actor. The cancellation receiver must therefore participate in the same `select!`
/// as the `Server` future so a stale room cannot keep this actor pinned behind a withholding peer.
async fn fetch_chunk_or_cancel<F>(mut cancel: Option<RequestCancellation>, fetch: F) -> ChunkResult
where
    F: Future<Output = ChunkResult>,
{
    let Some(cancel) = cancel.as_mut() else {
        return fetch.await;
    };
    if cancel.is_cancelled() {
        return Err("download cancelled".to_string());
    }
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("download cancelled".to_string()),
        result = fetch => result,
    }
}

/// The user-visible operation a command belongs to, carried across the actor boundary.
///
/// # Why this exists
///
/// Diagnostics could follow an operation from the webview to native persistence and no further.
/// Everything the actor did *in response* was a separate, uncorrelated record, so a
/// `ChannelUpdated` arriving two seconds after a send carried no evidence of being that send's
/// consequence. That is precisely the question the whole correlation architecture exists to
/// answer: given "my message did not arrive", which of the ten stages failed. Six of them were
/// past this line.
///
/// # Why it is an opaque integer
///
/// This crate emits diagnostics through the `tracing` facade and owns no diagnostic state, which
/// is what keeps it independent of whichever binary is observing it. A trace is therefore a number
/// it carries and never interprets; the binary that minted it knows what it means.
///
/// **Local only.** A trace identifies one device's own work. It is never put on the peer-to-peer
/// wire: doing so would let a remote peer correlate this device's operations, which is the exact
/// linkage the session-scoped reference model exists to prevent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Trace(pub u64);

impl Trace {
    /// No operation: internal work, or a command from a caller that did not mint one.
    pub const NONE: Trace = Trace(0);

    /// Whether this stands for an operation at all.
    ///
    /// Zero means absent rather than "operation zero". Correlating on it would gather every
    /// unrelated piece of internal work that also had none.
    pub fn is_set(self) -> bool {
        self.0 != 0
    }

    /// The sixteen hex characters a trace is quoted by, matching the diagnostics rendering.
    pub fn as_hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

/// A command plus the operation that issued it.
///
/// An envelope rather than a field on each of the fifty [`AppCommand`] variants. The variants
/// describe *what to do*; which operation asked is a property of the delivery, and threading it
/// through every variant would have meant fifty edits for one fact and fifty chances to forget it
/// on the next command somebody adds.
#[derive(Debug)]
pub struct Envelope {
    pub trace: Trace,
    pub command: AppCommand,
}

/// An event plus the operation that caused it, or [`Trace::NONE`] for spontaneous work.
///
/// An inbound message from a peer genuinely has no local operation behind it, and says so, which
/// is the distinction that separates "this appeared because I sent it" from "this appeared because
/// somebody else did".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracedEvent {
    pub trace: Trace,
    pub event: AppEvent,
}

/// The command channel, with the caller's operation attached to whatever goes down it.
///
/// A wrapper with the same `send` shape as the `mpsc::Sender` it replaces, so the fifty existing
/// call sites read exactly as they did. The alternative was fifty near-identical edits whose only
/// effect was to construct an envelope.
#[derive(Debug, Clone)]
struct CommandSender {
    tx: mpsc::Sender<Envelope>,
    trace: Trace,
}

impl CommandSender {
    async fn send(&self, command: AppCommand) -> Result<(), mpsc::error::SendError<Envelope>> {
        self.tx
            .send(Envelope {
                trace: self.trace,
                command,
            })
            .await
    }
}

/// The event channel, which stamps each event with the command being handled when it was sent.
///
/// The actor handles one command at a time, so "what is in progress" is a single value rather than
/// something that has to be threaded through every helper. It is atomic only because a `&EventSink`
/// crosses `.await` points inside a `Send` task; nothing contends for it.
#[derive(Debug)]
struct EventSink {
    tx: mpsc::Sender<TracedEvent>,
    current: std::sync::atomic::AtomicU64,
}

impl EventSink {
    fn new(tx: mpsc::Sender<TracedEvent>) -> Self {
        EventSink {
            tx,
            current: std::sync::atomic::AtomicU64::new(Trace::NONE.0),
        }
    }

    /// Take a command out of its envelope and adopt its operation for the duration of handling it.
    ///
    /// Called at the head of the command arm of the actor's `select!`, so every event the arm goes
    /// on to emit is attributed to the command that caused it without any arm having to say so.
    fn begin(&self, envelope: Option<Envelope>) -> Option<AppCommand> {
        let (trace, command) = match envelope {
            Some(envelope) => (envelope.trace, Some(envelope.command)),
            None => (Trace::NONE, None),
        };
        self.current
            .store(trace.0, std::sync::atomic::Ordering::Relaxed);
        if trace.is_set() {
            // The stage that separates a slow actor from a deep mailbox. Without it, a command that
            // took two seconds is indistinguishable from one that waited behind another for two
            // seconds, and those have completely different fixes.
            tracing::debug!(
                target: "catcoms_app",
                trace = %trace.as_hex(),
                "ACTOR.COMMAND.RECEIVED"
            );
        }
        command
    }

    /// Leave whatever operation was in progress.
    ///
    /// Called when the actor turns to work nobody asked for: an inbound op from a peer is not the
    /// consequence of the last local command, and attributing it to one would invent a causal link
    /// that a reader would then trust.
    fn idle(&self) {
        self.current
            .store(Trace::NONE.0, std::sync::atomic::Ordering::Relaxed);
    }

    async fn send(&self, event: AppEvent) -> Result<(), mpsc::error::SendError<TracedEvent>> {
        let trace = Trace(self.current.load(std::sync::atomic::Ordering::Relaxed));
        self.tx.send(TracedEvent { trace, event }).await
    }
}

/// A command from the UI to a running server actor.
#[derive(Debug)]
pub enum AppCommand {
    /// Create (or idempotently open) a channel and publish it to the shared directory.
    CreateChannel {
        name: String,
        reply: oneshot::Sender<Result<ChannelInfo, String>>,
    },
    /// Query the shared channel directory.
    Channels {
        reply: oneshot::Sender<Vec<ChannelInfo>>,
    },
    /// Pull the channel directory from the join contact, then subscribe/catch up every entry.
    CatchUpChannelIndex { peer: PeerId },
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
        reply: oneshot::Sender<Result<(), String>>,
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
    /// Queue a shared file in a channel's jukebox (any member); replies with the entry id.
    JukeboxAdd {
        channel: u128,
        cid: String,
        name: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Remove a jukebox entry (by id) from a channel (any member).
    JukeboxRemove {
        channel: u128,
        entry: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Query a channel's current jukebox queue.
    Jukebox {
        channel: u128,
        reply: oneshot::Sender<Vec<JukeEntry>>,
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
    /// Query the newest `limit` messages of a channel, each with whether it addresses me.
    MessageTail {
        channel: u128,
        limit: usize,
        after_id: String,
        after_ts: u64,
        reply: oneshot::Sender<crate::MessageTail>,
    },
    /// Query the named rows, wherever they sort, each with whether it addresses me.
    MessagesById {
        channel: u128,
        ids: Vec<String>,
        reply: oneshot::Sender<Vec<(ChatMessage, bool)>>,
    },
    /// Query a bounded slice of a channel around an anchor (see [`crate::MessagePageQuery`]).
    MessagePage {
        channel: u128,
        query: crate::MessagePageQuery,
        reply: oneshot::Sender<crate::MessagePage>,
    },
    /// Query every pinned message of a channel.
    PinnedMessages {
        channel: u128,
        reply: oneshot::Sender<Vec<ChatMessage>>,
    },
    /// Query a channel's lightweight activity stats (count + timestamps; no text).
    MessageStats {
        channel: u128,
        reply: oneshot::Sender<MessageStats>,
    },
    /// Query one activity head per channel, for rebuilding unread state after a lock or restart.
    ChannelHeads {
        reply: oneshot::Sender<Vec<ChannelHead>>,
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
    /// Query whether an exact DeviceId is in the live MLS roster.
    ContainsMemberDevice {
        device: DeviceId,
        reply: oneshot::Sender<bool>,
    },
    /// Resolve a current roster device through its self-signed PEX record.
    MemberTransportPeer {
        device: DeviceId,
        reply: oneshot::Sender<Option<PeerId>>,
    },
    /// Open one explicit, expiring pre-member handshake-helper capability.
    AuthorizeJoinHelper {
        joiner: PeerId,
        invite_nonce: [u8; 16],
        inviter: DeviceId,
        target: PeerId,
        expires_at_ms: u64,
        reply: oneshot::Sender<bool>,
    },
    /// Revoke a replaced one-time helper capability before authorizing its successor.
    RevokeJoinHelper {
        joiner: PeerId,
        invite_nonce: [u8; 16],
    },
    /// Enable/disable the local standing protocol gate; persistence and record publication are
    /// coordinated by the bridge.
    SetSwitchboardOffered { offered: bool },
    /// Query only fresh, connected and record-bound standing offers.
    SwitchboardOffers {
        reply: oneshot::Sender<Vec<SwitchboardOffer>>,
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
    /// Set (or clear, with `""`) the shared server name (owner/admin only).
    SetServerName {
        name: String,
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
        progress: Option<mpsc::Sender<(usize, usize)>>,
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Seal + store ONE chunk of a streamed upload; replies with the chunk's [`FileRef`].
    SealUploadChunk {
        bytes: Vec<u8>,
        mime: String,
        reply: oneshot::Sender<Result<FileRef, String>>,
    },
    /// Publish the index entry for a streamed upload whose chunks are already sealed and stored;
    /// replies with the file's content-address hex, or an error.
    PublishUpload {
        name: String,
        mime: String,
        path: String,
        plaintext_cid: [u8; 32],
        total_size: u64,
        chunks: Vec<FileRef>,
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Drop the sealed chunk blobs of a streamed upload that will never be published.
    DiscardUpload {
        chunks: Vec<FileRef>,
        reply: oneshot::Sender<()>,
    },
    /// Query the shared file list.
    Files {
        reply: oneshot::Sender<Vec<FileEntry>>,
    },
    /// Query the shared file list with per-file local-availability counts + a reachable-peer flag.
    FilesView { reply: oneshot::Sender<FilesView> },
    /// Verify every file chunk referenced by this server without network traffic.
    StorageHealth {
        reply: oneshot::Sender<StorageHealth>,
    },
    /// Capture file listings and their verified local-storage verdict in one actor turn.
    StorageSnapshot {
        reply: oneshot::Sender<StorageSnapshot>,
    },
    /// Re-fetch missing/unreadable file chunks, then return the verified result.
    RepairStorage {
        reply: oneshot::Sender<Result<StorageRepair, String>>,
    },
    /// Query the fingerprints of members reachable right now (presence).
    OnlineMembers { reply: oneshot::Sender<Vec<String>> },
    /// Query what this node knows about reaching each member (the debug console's network view).
    MemberRoutes {
        reply: oneshot::Sender<Vec<catcoms_sync::MemberRoute>>,
    },
    /// Query the recent inbound join attempts this node served, newest first (operator
    /// diagnostics; see `Server::join_attempts`).
    JoinAttempts {
        reply: oneshot::Sender<Vec<JoinAttempt>>,
    },
    /// Query delivery state for this device's recent messages in a channel, so a UI can paint it
    /// on open instead of waiting for the next throttled `DeliveryChanged`.
    DeliverySnapshot {
        channel: u128,
        reply: oneshot::Sender<DeliverySnapshot>,
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
        /// Present only for bounded inline operations that registered native cancellation.
        cancel: Option<RequestCancellation>,
        reply: oneshot::Sender<ChunkResult>,
    },
    /// The size and declared type of a listed file, read from the index without decrypting any
    /// of it: what the media protocol needs before it can answer a `Range` request at all.
    FileHead {
        cid: Vec<u8>,
        reply: oneshot::Sender<Option<FileMediaHead>>,
    },
    /// Read one window of a file's plaintext, for the media protocol: whole-file reads do not fit
    /// a player that wants to start on the first chunk and seek by the second.
    ReadFileRange {
        cid: Vec<u8>,
        expected_manifest_version: [u8; 32],
        start: u64,
        max_len: usize,
        reply: oneshot::Sender<Result<FileRange, String>>,
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
    /// Adjust ONE listing's circulation expiry (`None` = keep forever); uploader/owner/admin.
    SetFileExpiry {
        cid: Vec<u8>,
        path: String,
        expires: Option<u64>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Where a file is referenced: wiki pages + status/chat counts.
    FileUsage {
        cid: Vec<u8>,
        reply: oneshot::Sender<FileUsage>,
    },
    /// The wiki-pinned content addresses (lowercase hex); files that must never decay.
    WikiPinnedCids { reply: oneshot::Sender<Vec<String>> },
    /// Pull the file index from `peer` (e.g. right after joining).
    CatchUpFiles { peer: PeerId },
    /// Post to the server status feed (owner/admin, or anyone once the feed is opened to members).
    PostStatus {
        text: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Query the status feed.
    Statuses {
        reply: oneshot::Sender<Vec<ChatMessage>>,
    },
    /// Edit one of your own status posts (by id).
    EditStatus {
        id: String,
        text: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Delete a status post (by id): your own, or anyone's as an owner/admin.
    DeleteStatus {
        id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Toggle this member's emoji reaction on a status post (by id).
    ToggleStatusReaction {
        id: String,
        emoji: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Pin or unpin a status post (by id) (owner/admin).
    SetStatusPin {
        id: String,
        pinned: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Query whether plain members may post to the status feed.
    StatusMembersMayPost { reply: oneshot::Sender<bool> },
    /// Open or close the status feed to plain members (owner/admin only).
    SetStatusMembersMayPost {
        allow: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Pull the status feed from `peer` (e.g. right after joining).
    CatchUpStatus { peer: PeerId },
    /// Create a server event (any member); replies with its id, or a validation error.
    CreateEvent {
        title: String,
        body: String,
        start_ts: u64,
        end_ts: u64,
        image: String,
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
    /// Create or update a wiki page. Replies `Ok(true)` when review mode queued the edit
    /// for approval instead of publishing it.
    WriteWikiPage {
        name: String,
        body: String,
        reply: oneshot::Sender<Result<bool, String>>,
    },
    /// Query a page's revision history (oldest first; auto-accepted edits included).
    WikiHistory {
        page: String,
        reply: oneshot::Sender<Vec<WikiRevision>>,
    },
    /// Query the live review queue (pending edits still inside their window, oldest first).
    WikiPendingEdits {
        reply: oneshot::Sender<Vec<WikiPendingEdit>>,
    },
    /// Query the largest file this server accepts, in bytes.
    FileSizeLimit { reply: oneshot::Sender<u64> },
    /// Set the largest file this server accepts, in bytes (owner/admin only).
    SetFileSizeLimit {
        bytes: u64,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Query the wiki review window in days (0 = off).
    WikiReviewDays { reply: oneshot::Sender<u32> },
    /// Set the wiki review window in days, 0..=30 (owner/admin only).
    SetWikiReviewDays {
        days: u32,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Approve a pending wiki edit by id (owner/admin only).
    ApproveWikiEdit {
        id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Decline a pending wiki edit by id (owner/admin only; errors once auto-accepted).
    RejectWikiEdit {
        id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Restore a page to an earlier revision (replies `Ok(true)` when queued for review).
    RestoreWikiPage {
        page: String,
        rev: String,
        reply: oneshot::Sender<Result<bool, String>>,
    },
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
    /// Query the public signed moderation history and advisory votes.
    ModerationState {
        reply: oneshot::Sender<ModerationState>,
    },
    /// Preserve a message snapshot as a signed warning (owner/admin).
    WarnMessage {
        channel: u128,
        message_id: String,
        reason: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Open an advisory kick case citing warnings for the same target (owner/admin).
    CreateKickCase {
        target: String,
        reason: String,
        evidence_ids: Vec<String>,
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Cast/replace this member identity's advisory yes/no vote.
    CastKickVote {
        case_id: String,
        yes: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Owner decision; optionally runs the protocol-enforced member removal.
    ResolveKickCase {
        case_id: String,
        remove: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Pull moderation history from `peer` after joining.
    CatchUpModeration { peer: PeerId },
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
    /// Wrap a just-minted plain token with the inviter-signed live switchboard plan. The actor
    /// chooses the offers; the bridge cannot inject routes.
    WrapInviteWithSwitchboards {
        invite: Vec<u8>,
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
    },
    /// Lift every outstanding transport eviction (P6). Owner/admin only. Wired **only** to the
    /// explicit "Generate new invite" action: minting is also reached automatically by the
    /// invite panel's self-heal, and lifting there would silently re-admit every removed member.
    ReadmitEvictedPeers {
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// This member's origin identity on this server: its device id plus the group id.
    /// Read-only; the grant ceremony (multi-device M2) needs both to anchor the SAS
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
    /// found members), then one member-PEX pass and a refresh of the cross-session address cache.
    /// Fire-and-forget; sent periodically by the bridge's per-server timer (the real-time interval
    /// lives there, off the deterministic-time seam). The rendezvous half is a no-op without
    /// rendezvous configured; the PEX half always runs, because members exchange records over
    /// whatever connections they have with no infrastructure involved.
    DriveDiscovery,
    /// Replace the transient local-only reconnect hints after the bridge observes a currently
    /// live outbound member route. Validation and membership checks remain inside `ChannelSync`.
    SetLocalReconnectRoutes { routes: Vec<(PeerId, String)> },
    /// Mint a short-lived, member-signed recovery code containing only safe direct listener
    /// routes. The code is intended for an already-authorized group member over an out-of-band
    /// channel; it is not an invitation and cannot add a device to the roster.
    MintMemberRecovery {
        candidates: Vec<String>,
        reply: oneshot::Sender<Result<MemberRecoveryCode, String>>,
    },
    /// Authenticate a recovery code without performing its dial. The bridge persists this exact,
    /// expiring peer authority before it sends [`AppCommand::ApplyMemberRecovery`].
    VerifyMemberRecovery {
        code: String,
        reply: oneshot::Sender<Result<MemberRecoveryVerified, String>>,
    },
    /// Verify and submit the routes in another current member's recovery code. Authentication of
    /// the resulting socket still happens in the transport handshake before it becomes live.
    ApplyMemberRecovery {
        code: String,
        reply: oneshot::Sender<Result<MemberRecoveryApplied, String>>,
    },
    /// Explicit user-triggered isolation repair. The sync layer retains the anti-click cooldown
    /// and every shared egress limit; the reply distinguishes no routes from safety deferral.
    ManualFallbackRedial {
        reply: oneshot::Sender<catcoms_sync::ManualRedialOutcome>,
    },
    /// (Re)publish this device's own signed peer record with `addresses` at `seq`. Sent by the
    /// bridge when this node's reachability changes (a UPnP mapping arriving, say), so members
    /// learn the new address instead of holding a dead one.
    PublishSelfRecord { addresses: Vec<String>, seq: u64 },
    /// Serialize the cross-session address cache for sealing beside the snapshot (Phase 9f).
    AddressCacheBytes {
        integrity_key: [u8; 32],
        reply: oneshot::Sender<Vec<u8>>,
    },
    /// Stop the actor.
    Shutdown,
}

/// Which part of a channel document moved, carried by [`AppEvent::ChannelUpdated`].
///
/// One channel document holds the message log, the topic and the jukebox queue, and every one of
/// them renders. A single "it changed" signal is therefore ambiguous exactly where the UI needs
/// certainty: only a genuine arrival may raise an unread badge, while an edit, a reaction or a
/// queue add should refresh the view and nothing else.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelChange {
    /// At least one message id is present that was not there before: a real arrival. This is the
    /// only flag that may create unread state. Deliberately not inferred from the message count,
    /// which a concurrent append+delete batch (or a catch-up merge) can leave untouched.
    pub messages_appended: bool,
    /// The ids that were not present last time, in the order they now read, capped at
    /// [`MAX_REPORTED_ARRIVALS`].
    ///
    /// Rows are ordered by timestamp, so an arrival is not always the last row: a delayed message,
    /// or one from a device whose clock is behind, lands wherever its stamp says. Anything that
    /// wants to describe what arrived has to be told which rows those were; inferring it from the
    /// end of the list describes whatever is newest, which after such an arrival is somebody
    /// else's older message.
    pub arrivals: Vec<String>,
    /// The rendered message list moved without an arrival: an edit, a delete, a reaction or a pin.
    pub messages_changed: bool,
    /// The channel topic changed.
    pub topic: bool,
    /// The jukebox queue changed (an add or a remove).
    pub jukebox: bool,
}

impl ChannelChange {
    /// Did anything at all move? A delta with nothing set is never emitted.
    pub fn any(&self) -> bool {
        self.messages_appended || self.messages_changed || self.topic || self.jukebox
    }
}

/// An event from a running server actor to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /// The shared channel directory changed; the UI should re-fetch it (`channels`).
    ChannelsUpdated,
    /// A channel's rendered content changed; the UI should re-fetch it (`messages`). Using
    /// a re-fetch signal (rather than diffed deltas) keeps ordering robust under CRDT
    /// merges of concurrent messages.
    ///
    /// `change` says WHAT moved. The event used to carry only the channel id, so the UI had to
    /// read every mutation of the document as "a message arrived": a reaction, a topic edit or a
    /// jukebox add then raised an unread badge for a channel nobody had written to.
    ChannelUpdated {
        channel: u128,
        change: ChannelChange,
    },
    /// The roster size changed (a member joined or was removed).
    MembersChanged { count: usize },
    /// A member profile changed; the UI should re-fetch profiles (`profiles`).
    ProfilesUpdated,
    /// The server livery changed; the UI should re-fetch it (`livery`) and re-apply it.
    LiveryUpdated,
    /// A custom member badge changed; the UI should re-fetch badges (`badges`).
    BadgesUpdated,
    /// The companion-device registry changed; the UI should re-fetch it (`devices`) and
    /// re-resolve message attribution (multi-device M3/M4).
    DevicesUpdated,
    /// The shared file list changed; the UI should re-fetch it (`files`).
    FilesUpdated,
    /// The status feed changed; the UI should re-fetch it (`statuses`).
    StatusUpdated,
    /// The server events (calendar) changed; the UI should re-fetch them (`events`).
    EventsUpdated,
    /// The wiki changed; the UI should re-fetch pages / the open page.
    WikiUpdated,
    /// Member roles changed; the UI should re-fetch roles.
    RolesUpdated,
    /// Signed moderation evidence/cases/votes changed.
    ModerationUpdated,
    /// The advisory eclipse verdict changed: `true` = the node may be isolated (verify a member
    /// out of band). Surfaced as a UI hint; never gates anything.
    EclipseChanged { caution: bool },
    /// The set of members reachable right now (a live connection) changed; `online` is their
    /// fingerprints, for the roster's presence indicators + the file-availability hint.
    ConnectivityChanged { online: Vec<String> },
    /// Typed claimed-peer route evidence changed without necessarily changing aggregate presence.
    /// This catches relay/direct upgrades, partial closes, fresh records, and retry transitions.
    MemberRoutesChanged,
    /// The fresh, connected standing-switchboard offer set changed or expired. The UI should
    /// re-fetch its typed status instead of retaining an old host indefinitely.
    SwitchboardsChanged,
    /// Delivery state changed for this device's recent messages in `channel` (oldest first).
    /// Recomputed at most once a second per channel and emitted only on a real change, so a UI
    /// can render it directly without polling.
    DeliveryChanged {
        channel: u128,
        snapshot: DeliverySnapshot,
    },
    /// The set of pending incoming DM (friend) requests changed; the UI should re-fetch them.
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
    cmd_tx: CommandSender,
}

impl ServerActor {
    /// A handle whose commands belong to one operation.
    ///
    /// The join between the caller's diagnostics and the actor's. A caller that has minted a trace
    /// for "the user pressed send" uses this so the actor's work, and every event that work
    /// produces, lands under that same trace rather than in a separate record that has to be lined
    /// up by timestamp afterwards.
    ///
    /// A cheap clone, not a mutation: the untraced handle keeps working, and two operations can be
    /// in flight without either adopting the other's trace.
    pub fn with_trace(&self, trace: u64) -> ServerActor {
        ServerActor {
            cmd_tx: CommandSender {
                tx: self.cmd_tx.tx.clone(),
                trace: Trace(trace),
            },
        }
    }

    /// Create a channel in the shared directory.
    pub async fn create_channel(&self, name: impl Into<String>) -> Result<ChannelInfo, String> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(AppCommand::CreateChannel {
                name: name.into(),
                reply,
            })
            .await
            .map_err(|_| "server stopped".to_string())?;
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Read the shared channel directory.
    pub async fn channels(&self) -> Vec<ChannelInfo> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Channels { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Pull the shared channel directory from `peer` after joining.
    pub async fn catch_up_channel_index(&self, peer: PeerId) {
        let _ = self
            .cmd_tx
            .send(AppCommand::CatchUpChannelIndex { peer })
            .await;
    }

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

    /// Send a chat message to a channel; a `ChannelUpdated` event follows on success.
    pub async fn send_message(&self, channel: u128, text: impl Into<String>) {
        let _ = self.send_reply(channel, text, String::new()).await;
    }

    /// Send a chat message replying to `reply_to` (the parent message's id).
    pub async fn send_reply(
        &self,
        channel: u128,
        text: impl Into<String>,
        reply_to: impl Into<String>,
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SendMessage {
                channel,
                text: text.into(),
                reply_to: reply_to.into(),
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
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

    /// Set (or clear, with `""`) a channel's topic. Any member may; see
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

    /// Queue a shared file in a channel's jukebox; replies with the entry id. Any member may;
    /// see [`Server::jukebox_add`]. A `ChannelUpdated` event follows.
    pub async fn jukebox_add(
        &self,
        channel: u128,
        cid: String,
        name: String,
    ) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::JukeboxAdd {
                channel,
                cid,
                name,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Remove a jukebox entry (by id) from a channel. Any member may; see
    /// [`Server::jukebox_remove`]. A `ChannelUpdated` event follows if it changed.
    pub async fn jukebox_remove(&self, channel: u128, entry: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::JukeboxRemove {
                channel,
                entry,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Fetch a channel's jukebox queue (empty if none).
    pub async fn jukebox(&self, channel: u128) -> Vec<JukeEntry> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Jukebox { channel, reply })
            .await
            .is_err()
        {
            return Vec::new();
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

    /// The newest `limit` messages of a channel, oldest first, each paired with whether it is
    /// addressed to me (see [`Server::message_tail`]). Empty if the actor has stopped.
    pub async fn message_tail(
        &self,
        channel: u128,
        limit: usize,
        after_id: String,
        after_ts: u64,
    ) -> crate::MessageTail {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::MessageTail {
                channel,
                limit,
                after_id,
                after_ts,
                reply,
            })
            .await
            .is_err()
        {
            return crate::MessageTail::default();
        }
        rx.await.unwrap_or_default()
    }

    /// The named rows, wherever they sort, each with whether it addresses me (see
    /// [`Server::messages_by_id`]). Ids that name nothing are absent from the answer.
    pub async fn messages_by_id(
        &self,
        channel: u128,
        ids: Vec<String>,
    ) -> Vec<(ChatMessage, bool)> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::MessagesById {
                channel,
                ids,
                reply,
            })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// A bounded slice of a channel around an anchor (see [`Server::message_page`]). An empty
    /// default page if the actor has stopped.
    pub async fn message_page(
        &self,
        channel: u128,
        query: crate::MessagePageQuery,
    ) -> crate::MessagePage {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::MessagePage {
                channel,
                query,
                reply,
            })
            .await
            .is_err()
        {
            return crate::MessagePage::default();
        }
        rx.await.unwrap_or_default()
    }

    /// Every pinned message of a channel (see [`Server::pinned_messages`]).
    pub async fn pinned_messages(&self, channel: u128) -> Vec<ChatMessage> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::PinnedMessages { channel, reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Fetch one activity head per channel (no message text), for rebuilding unread state.
    pub async fn channel_heads(&self) -> Vec<ChannelHead> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::ChannelHeads { reply })
            .await
            .is_err()
        {
            return Vec::new();
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

    /// Lift every outstanding transport eviction (owner/admin only), so a previously removed
    /// member can reach this node to redeem an invite. Call from the **explicit** invite action
    /// only; see `Server::readmit_evicted_peers`.
    pub async fn readmit_evicted_peers(&self) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::ReadmitEvictedPeers { reply })
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
    /// (multi-device M2). The key never leaves the actor; only the certificate does.
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

    pub async fn contains_member_device(&self, device: DeviceId) -> bool {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::ContainsMemberDevice { device, reply })
            .await
            .is_err()
        {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    pub async fn member_transport_peer(&self, device: DeviceId) -> Option<PeerId> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::MemberTransportPeer { device, reply })
            .await
            .is_err()
        {
            return None;
        }
        rx.await.unwrap_or(None)
    }

    pub async fn authorize_join_helper(
        &self,
        joiner: PeerId,
        invite_nonce: [u8; 16],
        inviter: DeviceId,
        target: PeerId,
        expires_at_ms: u64,
    ) -> bool {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::AuthorizeJoinHelper {
                joiner,
                invite_nonce,
                inviter,
                target,
                expires_at_ms,
                reply,
            })
            .await
            .is_err()
        {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    pub async fn revoke_join_helper(&self, joiner: PeerId, invite_nonce: [u8; 16]) {
        let _ = self
            .cmd_tx
            .send(AppCommand::RevokeJoinHelper {
                joiner,
                invite_nonce,
            })
            .await;
    }

    pub async fn set_switchboard_offered(&self, offered: bool) -> Result<(), String> {
        self.cmd_tx
            .send(AppCommand::SetSwitchboardOffered { offered })
            .await
            .map_err(|_| "server stopped".to_string())
    }

    pub async fn switchboard_offers(&self) -> Vec<SwitchboardOffer> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SwitchboardOffers { reply })
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

    pub async fn wrap_invite_with_switchboards(&self, invite: Vec<u8>) -> Result<Vec<u8>, String> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(AppCommand::WrapInviteWithSwitchboards { invite, reply })
            .await
            .map_err(|_| "server stopped".to_string())?;
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Set (or clear, with `""`) the shared server icon; base64 image bytes (owner/admin
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

    /// Set (or clear, with `""`) the shared server name (owner/admin only; a `LiveryUpdated`
    /// event follows). Publishing colours or either image never disturbs it.
    pub async fn set_server_name(&self, name: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetServerName { name, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Set (or clear, with `""`) the shared server cursor; base64 image bytes (owner/admin
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
        self.add_file_with_progress(name, mime, path, bytes, None)
            .await
    }

    /// Share a file while reporting sealed/stored chunks plus final index publication.
    pub async fn add_file_with_progress(
        &self,
        name: String,
        mime: String,
        path: String,
        bytes: Vec<u8>,
        progress: Option<mpsc::Sender<(usize, usize)>>,
    ) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::AddFile {
                name,
                mime,
                path,
                bytes,
                progress,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Seal + store ONE chunk of a streamed upload, returning the chunk's [`FileRef`] for the
    /// manifest [`publish_upload`](Self::publish_upload) writes at the end.
    ///
    /// One chunk per command is the point: a whole-file `add_file` holds this actor for the entire
    /// upload, so the server stops syncing and every other command for it queues behind the
    /// transfer. Between these the actor returns to its loop and interleaves everything else.
    pub async fn seal_upload_chunk(&self, bytes: Vec<u8>, mime: String) -> Result<FileRef, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SealUploadChunk { bytes, mime, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Publish the index entry for a streamed upload whose chunks are already stored; returns the
    /// file's content-address hex.
    pub async fn publish_upload(
        &self,
        name: String,
        mime: String,
        path: String,
        plaintext_cid: [u8; 32],
        total_size: u64,
        chunks: Vec<FileRef>,
    ) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::PublishUpload {
                name,
                mime,
                path,
                plaintext_cid,
                total_size,
                chunks,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Drop the sealed chunk blobs of a streamed upload that was abandoned or failed, so an
    /// interrupted transfer does not leave bytes on disk no manifest names.
    ///
    /// Waits for the deletion rather than for the command to be queued. Cancel and lock tell the
    /// caller the upload has been cleaned up, and a queued deletion is not a deletion: the process
    /// can exit between the two and strand exactly the blobs that were reported gone.
    pub async fn discard_upload(&self, chunks: Vec<FileRef>) {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::DiscardUpload { chunks, reply })
            .await
            .is_ok()
        {
            let _ = rx.await;
        }
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

    /// Verify this server's referenced file chunks without network traffic.
    pub async fn storage_health(&self) -> StorageHealth {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::StorageHealth { reply })
            .await
            .is_err()
        {
            return StorageHealth::default();
        }
        rx.await.unwrap_or_default()
    }

    /// Capture file listings and their cryptographic health without an index mutation window.
    pub async fn storage_snapshot(&self) -> StorageSnapshot {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::StorageSnapshot { reply })
            .await
            .is_err()
        {
            return StorageSnapshot::default();
        }
        rx.await.unwrap_or_default()
    }

    /// Attempt repair of missing/unreadable referenced chunks.
    pub async fn repair_storage(&self) -> Result<StorageRepair, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::RepairStorage { reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
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

    /// Fetch what this node knows about reaching each member (the debug console's network view).
    pub async fn member_routes(&self) -> Vec<catcoms_sync::MemberRoute> {
        self.try_member_routes().await.unwrap_or_default()
    }

    /// Fallible member-route query for user-facing diagnostics.
    ///
    /// An empty live roster and a stopped actor are different facts. Callers that retain the last
    /// diagnostic snapshot use this form so actor failure becomes "snapshot unavailable" instead
    /// of a successful empty result that erases evidence.
    pub async fn try_member_routes(&self) -> Result<Vec<catcoms_sync::MemberRoute>, String> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(AppCommand::MemberRoutes { reply })
            .await
            .map_err(|_| "server stopped".to_string())?;
        rx.await.map_err(|_| "server stopped".to_string())
    }

    /// Request one explicit safety-bounded fallback redial pass.
    pub async fn manual_fallback_redial(
        &self,
    ) -> Result<catcoms_sync::ManualRedialOutcome, String> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(AppCommand::ManualFallbackRedial { reply })
            .await
            .map_err(|_| "server stopped".to_string())?;
        rx.await.map_err(|_| "server stopped".to_string())
    }

    /// Fetch the recent inbound join attempts this node served, newest first.
    pub async fn join_attempts(&self) -> Vec<JoinAttempt> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::JoinAttempts { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Fetch delivery state for this device's recent messages in a channel (oldest first).
    pub async fn delivery_snapshot(&self, channel: u128) -> Result<DeliverySnapshot, String> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(AppCommand::DeliverySnapshot { channel, reply })
            .await
            .map_err(|_| "server stopped".to_string())?;
        rx.await.map_err(|_| "server stopped".to_string())
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
        self.fetch_file_chunk_cancellable(cid, idx, None).await
    }

    /// As [`Self::fetch_file_chunk`], with cancellation observed inside the actor-owned fetch.
    pub async fn fetch_file_chunk_cancellable(
        &self,
        cid: Vec<u8>,
        idx: usize,
        cancel: Option<RequestCancellation>,
    ) -> ChunkResult {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::FetchFileChunk {
                cid,
                idx,
                cancel,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// The size and declared type of a listed file. See [`AppCommand::FileHead`].
    pub async fn file_head(&self, cid: Vec<u8>) -> Option<FileMediaHead> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::FileHead { cid, reply })
            .await
            .is_err()
        {
            return None;
        }
        rx.await.ok().flatten()
    }

    /// Read one window of a file's plaintext. See [`AppCommand::ReadFileRange`].
    pub async fn read_file_range(
        &self,
        cid: Vec<u8>,
        expected_manifest_version: [u8; 32],
        start: u64,
        max_len: usize,
    ) -> Result<FileRange, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::ReadFileRange {
                cid,
                expected_manifest_version,
                start,
                max_len,
                reply,
            })
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

    /// Adjust ONE listing's circulation expiry (`None` = keep forever). Uploader/owner/admin
    /// only. A `FilesUpdated` event follows on success.
    pub async fn set_file_expiry(
        &self,
        cid: Vec<u8>,
        path: String,
        expires: Option<u64>,
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetFileExpiry {
                cid,
                path,
                expires,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Where a file is referenced (wiki pages + status/chat counts).
    pub async fn file_usage(&self, cid: Vec<u8>) -> FileUsage {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::FileUsage { cid, reply })
            .await
            .is_err()
        {
            return FileUsage::default();
        }
        rx.await.unwrap_or_default()
    }

    /// The wiki-pinned content addresses (lowercase hex): files that must never decay.
    pub async fn wiki_pinned_cids(&self) -> Vec<String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::WikiPinnedCids { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Pull the file index from `peer`.
    pub async fn catch_up_files(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpFiles { peer }).await;
    }

    /// Post to the status feed (a `StatusUpdated` event follows on success). Refused for a plain
    /// member while the feed is closed to members, which is why the caller gets the answer back
    /// rather than a post that quietly never happened.
    pub async fn post_status(&self, text: impl Into<String>) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::PostStatus {
                text: text.into(),
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
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

    /// Edit one of your own status posts (by id).
    pub async fn edit_status(&self, id: String, text: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::EditStatus { id, text, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Delete a status post (by id): your own, or anyone's as an owner/admin.
    pub async fn delete_status(&self, id: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::DeleteStatus { id, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Toggle this member's emoji reaction on a status post (by id).
    pub async fn toggle_status_reaction(&self, id: String, emoji: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::ToggleStatusReaction { id, emoji, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Pin or unpin a status post (by id) (owner/admin).
    pub async fn set_status_pin(&self, id: String, pinned: bool) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetStatusPin { id, pinned, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Whether plain members may post to the status feed. A stopped server reads as `false`, the
    /// same answer an unread feed gives, so a UI that cannot reach the actor offers no posting box
    /// rather than one whose posts would be refused.
    pub async fn status_members_may_post(&self) -> bool {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::StatusMembersMayPost { reply })
            .await
            .is_err()
        {
            return false;
        }
        rx.await.unwrap_or_default()
    }

    /// Open or close the status feed to plain members; owner/admin only (a `StatusUpdated` event
    /// follows).
    pub async fn set_status_members_may_post(&self, allow: bool) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetStatusMembersMayPost { allow, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
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
        image: String,
    ) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::CreateEvent {
                title,
                body,
                start_ts,
                end_ts,
                image,
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

    /// Fetch the signed moderation history and votes.
    pub async fn moderation_state(&self) -> ModerationState {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::ModerationState { reply })
            .await
            .is_err()
        {
            return ModerationState::default();
        }
        rx.await.unwrap_or_default()
    }

    pub async fn warn_message(
        &self,
        channel: u128,
        message_id: String,
        reason: String,
    ) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::WarnMessage {
                channel,
                message_id,
                reason,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    pub async fn create_kick_case(
        &self,
        target: String,
        reason: String,
        evidence_ids: Vec<String>,
    ) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::CreateKickCase {
                target,
                reason,
                evidence_ids,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    pub async fn cast_kick_vote(&self, case_id: String, yes: bool) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::CastKickVote {
                case_id,
                yes,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    pub async fn resolve_kick_case(&self, case_id: String, remove: bool) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::ResolveKickCase {
                case_id,
                remove,
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    pub async fn catch_up_moderation(&self, peer: PeerId) {
        let _ = self
            .cmd_tx
            .send(AppCommand::CatchUpModeration { peer })
            .await;
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

    /// Create or update a wiki page (a `WikiUpdated` event follows). `Ok(true)` = review
    /// mode queued the edit for approval instead of publishing it.
    pub async fn write_wiki_page(
        &self,
        name: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<bool, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::WriteWikiPage {
                name: name.into(),
                body: body.into(),
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Fetch a page's revision history (oldest first; auto-accepted edits included).
    pub async fn wiki_history(&self, page: impl Into<String>) -> Vec<WikiRevision> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::WikiHistory {
                page: page.into(),
                reply,
            })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Fetch the live review queue (pending edits still inside their window, oldest first).
    pub async fn wiki_pending_edits(&self) -> Vec<WikiPendingEdit> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::WikiPendingEdits { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Fetch the wiki review window in days (0 = off).
    pub async fn wiki_review_days(&self) -> u32 {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::WikiReviewDays { reply })
            .await
            .is_err()
        {
            return 0;
        }
        rx.await.unwrap_or_default()
    }

    /// The largest file this server accepts, in bytes. Answers the default rather than a
    /// misleading zero if the actor has stopped.
    pub async fn file_size_limit(&self) -> u64 {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::FileSizeLimit { reply })
            .await
            .is_err()
        {
            return crate::DEFAULT_FILE_SIZE_LIMIT;
        }
        rx.await.unwrap_or(crate::DEFAULT_FILE_SIZE_LIMIT)
    }

    /// Set the largest file this server accepts, in bytes; owner/admin only (a `FilesChanged`
    /// event follows, because the limit lives in the file index document).
    pub async fn set_file_size_limit(&self, bytes: u64) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetFileSizeLimit { bytes, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Set the wiki review window in days, 0..=30; owner/admin only (a `WikiUpdated` event
    /// follows).
    pub async fn set_wiki_review_days(&self, days: u32) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::SetWikiReviewDays { days, reply })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Approve a pending wiki edit (owner/admin only; a `WikiUpdated` event follows).
    pub async fn approve_wiki_edit(&self, id: impl Into<String>) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::ApproveWikiEdit {
                id: id.into(),
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Decline a pending wiki edit (owner/admin only; a `WikiUpdated` event follows).
    pub async fn reject_wiki_edit(&self, id: impl Into<String>) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::RejectWikiEdit {
                id: id.into(),
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
    }

    /// Restore a page to an earlier revision; `Ok(true)` = queued for review (a `WikiUpdated`
    /// event follows either way).
    pub async fn restore_wiki_page(
        &self,
        page: impl Into<String>,
        rev: impl Into<String>,
    ) -> Result<bool, String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::RestoreWikiPage {
                page: page.into(),
                rev: rev.into(),
                reply,
            })
            .await
            .is_err()
        {
            return Err("server stopped".into());
        }
        rx.await.unwrap_or_else(|_| Err("server stopped".into()))
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

    /// Set a wiki page's render format; "md" or "wiki" (a `WikiUpdated` event follows).
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

    /// Drive one steady-state rendezvous-discovery pass. Fire-and-forget; the bridge calls this on
    /// a timer. Returns `Err` once the actor has stopped (so the bridge's timer task can exit).
    pub async fn drive_discovery(&self) -> Result<(), ()> {
        self.cmd_tx
            .send(AppCommand::DriveDiscovery)
            .await
            .map_err(|_| ())
    }

    /// Install bridge-observed, vault-sealed local reconnect hints into the running server.
    /// The actor owns the authoritative roster and reparses every route before retaining it.
    pub async fn set_local_reconnect_routes(
        &self,
        routes: Vec<(PeerId, String)>,
    ) -> Result<(), ()> {
        self.cmd_tx
            .send(AppCommand::SetLocalReconnectRoutes { routes })
            .await
            .map_err(|_| ())
    }

    /// Create an out-of-band recovery code for a member that has lost every usable route to this
    /// server. Candidate filtering and signing happen inside the server actor, where the current
    /// roster, transport identity and deterministic clock are authoritative.
    pub async fn mint_member_recovery(
        &self,
        candidates: Vec<String>,
    ) -> Result<MemberRecoveryCode, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(AppCommand::MintMemberRecovery {
                candidates,
                reply: tx,
            })
            .await
            .map_err(|_| "server actor stopped".to_string())?;
        rx.await.map_err(|_| "server actor stopped".to_string())?
    }

    /// Apply a current member's signed recovery code without granting membership or trusting the
    /// advertised route. The eventual Noise-authenticated connection remains the proof boundary.
    pub async fn apply_member_recovery(
        &self,
        code: String,
    ) -> Result<MemberRecoveryApplied, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(AppCommand::ApplyMemberRecovery { code, reply: tx })
            .await
            .map_err(|_| "server actor stopped".to_string())?;
        rx.await.map_err(|_| "server actor stopped".to_string())?
    }

    /// Authenticate a current member's signed recovery code without starting a socket attempt.
    /// Applying repeats the checks, so expiry or roster movement between phases fails safely.
    pub async fn verify_member_recovery(
        &self,
        code: String,
    ) -> Result<MemberRecoveryVerified, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(AppCommand::VerifyMemberRecovery { code, reply: tx })
            .await
            .map_err(|_| "server actor stopped".to_string())?;
        rx.await.map_err(|_| "server actor stopped".to_string())?
    }

    /// (Re)publish this device's signed peer record. `seq` must come from this launch's reserved
    /// peer-record sequence block (see `ServerNet::reserve_record_seq_block`).
    pub async fn publish_self_record(&self, addresses: Vec<String>, seq: u64) {
        let _ = self
            .cmd_tx
            .send(AppCommand::PublishSelfRecord { addresses, seq })
            .await;
    }

    /// Serialize the cross-session address cache so the bridge can seal it beside the snapshot.
    pub async fn address_cache_bytes(&self, integrity_key: [u8; 32]) -> Result<Vec<u8>, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(AppCommand::AddressCacheBytes {
                integrity_key,
                reply: tx,
            })
            .await
            .map_err(|_| "server actor stopped".to_string())?;
        rx.await.map_err(|_| "server actor stopped".to_string())
    }
}

/// Return the injected-clock delay until any dirty delivery snapshot can be recomputed.
///
/// A channel stays dirty when an acknowledgement arrives inside its throttle window. Without a
/// separately scheduled wake, that acknowledgement would remain invisible until some unrelated
/// later network event happened to drive the actor again.
fn next_delivery_delay(
    now_ms: u64,
    delivery: &HashMap<u128, (u64, Vec<DeliveryState>)>,
    dirty: &HashSet<u128>,
) -> Option<u64> {
    dirty
        .iter()
        .map(|channel| {
            delivery.get(channel).map_or(0, |(last_ms, _)| {
                DELIVERY_THROTTLE_MS.saturating_sub(now_ms.saturating_sub(*last_ms))
            })
        })
        .min()
}

/// Recompute every dirty channel whose throttle has elapsed and return only changed snapshots.
///
/// Channel ids are sorted before processing so tests and UI event ordering do not depend on the
/// randomized iteration order of a `HashSet`. Entries that are still throttled deliberately stay
/// dirty; the actor's next loop iteration schedules the earliest remaining injected-clock wake.
fn recompute_due_delivery<T, R>(
    server: &mut Server<T, R>,
    delivery: &mut HashMap<u128, (u64, Vec<DeliveryState>)>,
    dirty: &mut HashSet<u128>,
) -> Vec<(u128, DeliverySnapshot)>
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now_ms = server.runtime_clock().monotonic_ms();
    let mut channels = dirty.iter().copied().collect::<Vec<_>>();
    channels.sort_unstable();

    let mut changed = Vec::new();
    for channel in channels {
        if delivery
            .get(&channel)
            .is_some_and(|(last_ms, _)| now_ms.saturating_sub(*last_ms) < DELIVERY_THROTTLE_MS)
        {
            continue;
        }

        let snapshot = server.delivery_snapshot(channel);
        dirty.remove(&channel);
        let did_change = match delivery.insert(channel, (now_ms, snapshot.states.clone())) {
            Some((_, previous)) => previous != snapshot.states,
            None => !snapshot.states.is_empty(),
        };
        if did_change {
            changed.push((channel, snapshot));
        }
    }
    changed
}

/// Move `server` into a background task. Returns a [`ServerActor`] handle, a receiver of
/// [`AppEvent`]s, and the task's [`JoinHandle`].
pub fn spawn<T, R>(
    mut server: Server<T, R>,
) -> (ServerActor, mpsc::Receiver<TracedEvent>, JoinHandle<()>)
where
    T: MeshTransport + Send + 'static,
    R: CryptoRngCore + Send + 'static,
{
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<Envelope>(64);
    let (raw_events, event_rx) = mpsc::channel::<TracedEvent>(256);
    let event_tx = EventSink::new(raw_events);
    let handle = tokio::spawn(async move {
        // Per open channel: a content signature of its messages, topic and jukebox (see
        // `channel_delta`), so an edit/delete/add all surface a `ChannelUpdated` that says which
        // of the three it was.
        let mut counts: HashMap<u128, ChannelSignature> = HashMap::new();
        // Per document: the version it was last projected at. Every `*_changed` check below used
        // to re-materialize its whole document on every network event, so a long channel made
        // every gossip frame, presence blip and receipt cost a full walk of the history; the
        // version comparison is what lets an unchanged document cost nothing.
        let mut versions = DocVersions::default();
        let mut last_delivery_evidence = server.delivery_evidence_revision();
        let mut members = server.member_count();
        // The directory itself is shared. Seed `general` for legacy/new servers, then open every
        // known message document so later gossip and reconnect catch-up have somewhere to land.
        if let Err(e) = server.open_channel_index().await {
            tracing::warn!(error = %e, "open_channel_index failed");
        }
        if let Err(e) = server.create_channel("general").await {
            tracing::warn!(error = %e, "seed general channel failed");
        }
        let mut last_channels = server.channels();
        for channel in last_channels.iter().map(|c| c.id) {
            if let Err(e) = server.open_channel(channel).await {
                tracing::warn!(error = %e, channel, "open listed channel failed");
            }
            channel_delta_if_moved(&server, channel, &mut counts, &mut versions);
        }
        // Open the per-server profile document and seed this member's name from the
        // display name, so the roster/messages show a name immediately (the user can
        // customize color/font/effect later via SetProfile). Seed ONLY when this device has no
        // profile entry yet (first founding/join); otherwise a reload would overwrite the
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
        let mut last_statuses = status_snapshot(&server);
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
        // not stored here; it is derived from the MLS designated committer (lowest leaf
        // index), so every member computes the owner identically with no roles op present.
        if let Err(e) = server.open_roles().await {
            tracing::warn!(error = %e, "open_roles failed");
        }
        let mut last_roles = server.roles();
        // Moderation evidence is its own signed document so it survives edits/deletes of the live
        // chat post and so advisory votes never share a membership authorization path.
        if let Err(e) = server.open_moderation().await {
            tracing::warn!(error = %e, "open_moderation failed");
        }
        let mut last_moderation = server.moderation_state();
        let mut last_eclipse = false;
        let mut last_online = server.online_members();
        let mut last_member_route_revision = server.member_route_revision();
        let mut last_switchboards = server.connected_switchboard_offers();
        let mut last_dm_requests = server.dm_requests();
        // Per channel: when delivery state was last recomputed, and what it was; the throttle
        // plus the change detector for `DeliveryChanged`.
        let mut delivery: HashMap<u128, (u64, Vec<DeliveryState>)> = HashMap::new();
        // A sync event inside the throttle window must not be forgotten. Dirty channels are
        // coalesced here and revisited by an injected-clock timer even if the network goes idle.
        let mut delivery_dirty = HashSet::new();
        loop {
            let delivery_clock = server.runtime_clock();
            let delivery_delay =
                next_delivery_delay(delivery_clock.monotonic_ms(), &delivery, &delivery_dirty);
            let delivery_wake = async move {
                match delivery_delay {
                    Some(delay_ms) => {
                        delivery_clock.sleep(Duration::from_millis(delay_ms)).await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::pin!(delivery_wake);
            tokio::select! {
                biased;
                // `begin` unwraps the envelope and adopts the caller's operation for as long as
                // this arm runs, so every event the arm emits is attributed to the command that
                // caused it without any of the fifty arms below having to mention it.
                cmd = cmd_rx.recv() => match event_tx.begin(cmd) {
                    Some(AppCommand::CreateChannel { name, reply }) => {
                        let res = server.create_channel(&name).await.map_err(|e| e.to_string());
                        // The creator already opened this document as part of create_channel.
                        // Do not immediately pull that same empty/new document from a peer: a
                        // peer receiving the directory op may simultaneously be pulling it from
                        // us, and the two actors would otherwise wait on each other's request.
                        let locally_created = res.as_ref().ok().map(|channel| channel.id);
                        let _ = reply.send(res);
                        sync_channels(
                            &mut server,
                            &mut last_channels,
                            &mut counts,
                            &mut versions,
                            &event_tx,
                            None,
                            locally_created,
                        )
                        .await;
                    }
                    Some(AppCommand::Channels { reply }) => {
                        let _ = reply.send(server.channels());
                    }
                    Some(AppCommand::CatchUpChannelIndex { peer }) => {
                        if let Err(e) = server.request_channel_index_catchup(peer).await {
                            tracing::warn!(error = %e, "channel directory catch-up failed");
                        }
                        sync_channels(
                            &mut server,
                            &mut last_channels,
                            &mut counts,
                            &mut versions,
                            &event_tx,
                            Some(peer),
                            None,
                        )
                        .await;
                    }
                    Some(AppCommand::OpenChannel { channel, ack }) => {
                        if let Err(e) = server.open_channel(channel).await {
                            tracing::warn!(error = %e, channel, "open_channel failed");
                        }
                        // Seed (and start tracking) the channel's current content signature WITHOUT
                        // emitting; the UI fetches messages on open (switchTo → refresh); only a
                        // later add/edit/delete should fire ChannelUpdated. `channel_delta` reports
                        // nothing on first sight for exactly this reason.
                        channel_delta_if_moved(&server, channel, &mut counts, &mut versions);
                        let _ = ack.send(());
                    }
                    Some(AppCommand::SendMessage {
                        channel,
                        text,
                        reply_to,
                        reply,
                    }) => {
                        let res = server
                            .send_reply(channel, &text, &reply_to)
                            .await
                            .map_err(|e| e.to_string());
                        if let Err(e) = &res {
                            tracing::warn!(error = %e, channel, "send_message failed");
                        }
                        let change = res
                            .is_ok()
                            .then(|| channel_delta_if_moved(&server, channel, &mut counts, &mut versions))
                            .flatten();
                        let _ = reply.send(res);
                        if let Some(change) = change {
                            let _ = event_tx
                                .send(AppEvent::ChannelUpdated { channel, change })
                                .await;
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
                        if let Some(change) = channel_delta_if_moved(&server, channel, &mut counts, &mut versions) {
                            let _ = event_tx
                                .send(AppEvent::ChannelUpdated { channel, change })
                                .await;
                        }
                    }
                    Some(AppCommand::DeleteMessage { channel, id, reply }) => {
                        let res = server
                            .delete_message(channel, &id)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if let Some(change) = channel_delta_if_moved(&server, channel, &mut counts, &mut versions) {
                            let _ = event_tx
                                .send(AppEvent::ChannelUpdated { channel, change })
                                .await;
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
                        if let Some(change) = channel_delta_if_moved(&server, channel, &mut counts, &mut versions) {
                            let _ = event_tx
                                .send(AppEvent::ChannelUpdated { channel, change })
                                .await;
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
                        if let Some(change) = channel_delta_if_moved(&server, channel, &mut counts, &mut versions) {
                            let _ = event_tx
                                .send(AppEvent::ChannelUpdated { channel, change })
                                .await;
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
                        if let Some(change) = channel_delta_if_moved(&server, channel, &mut counts, &mut versions) {
                            let _ = event_tx
                                .send(AppEvent::ChannelUpdated { channel, change })
                                .await;
                        }
                    }
                    Some(AppCommand::ChannelTopic { channel, reply }) => {
                        let _ = reply.send(server.channel_topic(channel));
                    }
                    Some(AppCommand::JukeboxAdd {
                        channel,
                        cid,
                        name,
                        reply,
                    }) => {
                        let res = server
                            .jukebox_add(channel, &cid, &name)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if let Some(change) = channel_delta_if_moved(&server, channel, &mut counts, &mut versions) {
                            let _ = event_tx
                                .send(AppEvent::ChannelUpdated { channel, change })
                                .await;
                        }
                    }
                    Some(AppCommand::JukeboxRemove {
                        channel,
                        entry,
                        reply,
                    }) => {
                        let res = server
                            .jukebox_remove(channel, &entry)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if let Some(change) = channel_delta_if_moved(&server, channel, &mut counts, &mut versions) {
                            let _ = event_tx
                                .send(AppEvent::ChannelUpdated { channel, change })
                                .await;
                        }
                    }
                    Some(AppCommand::Jukebox { channel, reply }) => {
                        let _ = reply.send(server.jukebox(channel));
                    }
                    Some(AppCommand::CatchUp { peer, channel }) => {
                        if let Err(e) = server.request_channel_catchup(peer, channel).await {
                            tracing::warn!(error = %e, channel, "catch-up failed");
                        }
                        if let Some(change) = channel_delta_if_moved(&server, channel, &mut counts, &mut versions) {
                            let _ = event_tx
                                .send(AppEvent::ChannelUpdated { channel, change })
                                .await;
                        }
                    }
                    Some(AppCommand::CatchUpAny { channel }) => {
                        if let Err(e) = server.request_channel_catchup_any(channel).await {
                            tracing::warn!(error = %e, channel, "any-peer catch-up failed");
                        }
                        if let Some(change) = channel_delta_if_moved(&server, channel, &mut counts, &mut versions) {
                            let _ = event_tx
                                .send(AppEvent::ChannelUpdated { channel, change })
                                .await;
                        }
                    }
                    Some(AppCommand::Messages { channel, reply }) => {
                        let _ = reply.send(server.messages(channel));
                    }
                    Some(AppCommand::MessageTail {
                        channel,
                        limit,
                        after_id,
                        after_ts,
                        reply,
                    }) => {
                        let _ = reply.send(server.message_tail(channel, limit, &after_id, after_ts));
                    }
                    Some(AppCommand::MessagesById {
                        channel,
                        ids,
                        reply,
                    }) => {
                        let _ = reply.send(server.messages_by_id(channel, &ids));
                    }
                    Some(AppCommand::MessagePage {
                        channel,
                        query,
                        reply,
                    }) => {
                        let _ = reply.send(server.message_page(channel, &query));
                    }
                    Some(AppCommand::PinnedMessages { channel, reply }) => {
                        let _ = reply.send(server.pinned_messages(channel));
                    }
                    Some(AppCommand::MessageStats { channel, reply }) => {
                        let _ = reply.send(server.message_stats(channel));
                    }
                    Some(AppCommand::ChannelHeads { reply }) => {
                        let _ = reply.send(server.channel_heads());
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
                    Some(AppCommand::ContainsMemberDevice { device, reply }) => {
                        let _ = reply.send(server.contains_member_device(&device));
                    }
                    Some(AppCommand::MemberTransportPeer { device, reply }) => {
                        let _ = reply.send(server.member_transport_peer(&device));
                    }
                    Some(AppCommand::AuthorizeJoinHelper {
                        joiner,
                        invite_nonce,
                        inviter,
                        target,
                        expires_at_ms,
                        reply,
                    }) => {
                        let _ = reply.send(server.authorize_join_helper(
                            joiner,
                            invite_nonce,
                            inviter,
                            target,
                            expires_at_ms,
                        ));
                    }
                    Some(AppCommand::RevokeJoinHelper {
                        joiner,
                        invite_nonce,
                    }) => server.revoke_join_helper(joiner, invite_nonce),
                    Some(AppCommand::SetSwitchboardOffered { offered }) => {
                        server.set_switchboard_offered(offered);
                    }
                    Some(AppCommand::SwitchboardOffers { reply }) => {
                        let _ = reply.send(server.connected_switchboard_offers());
                    }
                    Some(AppCommand::SetProfile { profile }) => {
                        if let Err(e) = server.set_profile(profile).await {
                            tracing::warn!(error = %e, "set_profile failed");
                        }
                        sync_profiles(&mut server, &mut last_profiles, &event_tx, true).await;
                    }
                    Some(AppCommand::CatchUpProfiles { peer }) => {
                        if let Err(e) = server.request_profiles_catchup(peer).await {
                            tracing::warn!(error = %e, "profiles catch-up failed");
                        }
                        sync_profiles(&mut server, &mut last_profiles, &event_tx, true).await;
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
                    Some(AppCommand::SetServerName { name, reply }) => {
                        let res = server.set_server_name(name).await.map_err(|e| e.to_string());
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
                    Some(AppCommand::AddFile { name, mime, path, bytes, progress, reply }) => {
                        let res = server
                            .add_file_with_progress(&name, &mime, &path, &bytes, progress.as_ref())
                            .await
                            .map(|cid| cid.to_hex())
                            .map_err(|e| e.to_string());
                        // Close the progress stream before resolving the command. The desktop
                        // bridge drains that stream before returning from its invoke; retaining
                        // this sender until the end of the arm can otherwise leave the transfer
                        // UI waiting behind an unrelated (and potentially back-pressured) event.
                        drop(progress);
                        let _ = reply.send(res);
                        if files_changed(&server, &mut file_count) {
                            let _ = event_tx.send(AppEvent::FilesUpdated).await;
                        }
                    }
                    Some(AppCommand::SealUploadChunk { bytes, mime, reply }) => {
                        let res = server
                            .seal_upload_chunk(&bytes, &mime)
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                    }
                    Some(AppCommand::PublishUpload {
                        name,
                        mime,
                        path,
                        plaintext_cid,
                        total_size,
                        chunks,
                        reply,
                    }) => {
                        let res = server
                            .publish_upload(
                                &name,
                                &mime,
                                &path,
                                Cid::from_bytes(plaintext_cid),
                                total_size,
                                chunks,
                            )
                            .await
                            .map(|cid| cid.to_hex())
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if files_changed(&server, &mut file_count) {
                            let _ = event_tx.send(AppEvent::FilesUpdated).await;
                        }
                    }
                    Some(AppCommand::DiscardUpload { chunks, reply }) => {
                        server.discard_upload_chunks(&chunks);
                        let _ = reply.send(());
                    }
                    Some(AppCommand::Files { reply }) => {
                        let _ = reply.send(server.files());
                    }
                    Some(AppCommand::FilesView { reply }) => {
                        let _ = reply.send(server.files_view());
                    }
                    Some(AppCommand::StorageHealth { reply }) => {
                        let _ = reply.send(server.storage_health());
                    }
                    Some(AppCommand::StorageSnapshot { reply }) => {
                        let _ = reply.send(server.storage_snapshot());
                    }
                    Some(AppCommand::RepairStorage { reply }) => {
                        let res = server.repair_storage().await.map_err(|e| e.to_string());
                        let _ = reply.send(res);
                    }
                    Some(AppCommand::OnlineMembers { reply }) => {
                        let _ = reply.send(server.online_members());
                    }
                    Some(AppCommand::MemberRoutes { reply }) => {
                        let _ = reply.send(server.member_routes());
                    }
                    Some(AppCommand::JoinAttempts { reply }) => {
                        let _ = reply.send(server.join_attempts());
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
                    // Fetch ONE chunk, then return to the select! loop; so a large download no
                    // longer pins the actor: other commands + sync_once interleave between chunks
                    // (the bridge orchestrates the per-chunk loop + reassembly + progress).
                    Some(AppCommand::FetchFileChunk { cid, idx, cancel, reply }) => {
                        let res = match <[u8; 32]>::try_from(cid.as_slice()) {
                            Ok(arr) => {
                                let server_cancellation = cancel.clone();
                                fetch_chunk_or_cancel(
                                cancel,
                                async {
                                    server
                                        .fetch_file_chunk_cancellable(
                                            &Cid::from_bytes(arr),
                                            idx,
                                            server_cancellation,
                                        )
                                        .await
                                        .map_err(|e| e.to_string())
                                },
                            )
                            .await
                            },
                            Err(_) => Err("bad content address".to_string()),
                        };
                        let _ = reply.send(res);
                    }
                    Some(AppCommand::FileHead { cid, reply }) => {
                        let head = <[u8; 32]>::try_from(cid.as_slice())
                            .ok()
                            .and_then(|arr| server.file_head(&Cid::from_bytes(arr)));
                        let _ = reply.send(head);
                    }
                    Some(AppCommand::ReadFileRange {
                        cid,
                        expected_manifest_version,
                        start,
                        max_len,
                        reply,
                    }) => {
                        let res = match <[u8; 32]>::try_from(cid.as_slice()) {
                            Ok(arr) => server
                                .read_file_range(
                                    &Cid::from_bytes(arr),
                                    expected_manifest_version,
                                    start,
                                    max_len,
                                )
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
                    Some(AppCommand::SetFileExpiry { cid, path, expires, reply }) => {
                        let res = match <[u8; 32]>::try_from(cid.as_slice()) {
                            Ok(arr) => server
                                .set_file_expiry(&Cid::from_bytes(arr), &path, expires)
                                .await
                                .map_err(|e| e.to_string()),
                            Err(_) => Err("bad content address".to_string()),
                        };
                        let ok = res.is_ok();
                        let _ = reply.send(res);
                        // The listing count is unchanged, so `files_changed` can't see this;
                        // announce it directly so every surface repaints the new expiry.
                        if ok {
                            let _ = event_tx.send(AppEvent::FilesUpdated).await;
                        }
                    }
                    Some(AppCommand::FileUsage { cid, reply }) => {
                        let usage = match <[u8; 32]>::try_from(cid.as_slice()) {
                            Ok(arr) => server.file_usage(&Cid::from_bytes(arr)),
                            Err(_) => FileUsage::default(),
                        };
                        let _ = reply.send(usage);
                    }
                    Some(AppCommand::WikiPinnedCids { reply }) => {
                        let mut pinned: Vec<String> =
                            server.wiki_pinned_cids().into_iter().collect();
                        pinned.sort();
                        let _ = reply.send(pinned);
                    }
                    Some(AppCommand::CatchUpFiles { peer }) => {
                        if let Err(e) = server.request_files_catchup(peer).await {
                            tracing::warn!(error = %e, "files catch-up failed");
                        }
                        if files_changed(&server, &mut file_count) {
                            let _ = event_tx.send(AppEvent::FilesUpdated).await;
                        }
                    }
                    Some(AppCommand::PostStatus { text, reply }) => {
                        let res = server.post_status(&text).await.map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if status_changed(&server, &mut last_statuses) {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                    }
                    Some(AppCommand::Statuses { reply }) => {
                        let _ = reply.send(server.statuses());
                    }
                    Some(AppCommand::EditStatus { id, text, reply }) => {
                        let res = server
                            .edit_status(&id, &text)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if status_changed(&server, &mut last_statuses) {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                    }
                    Some(AppCommand::DeleteStatus { id, reply }) => {
                        let res = server.delete_status(&id).await.map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if status_changed(&server, &mut last_statuses) {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                    }
                    Some(AppCommand::ToggleStatusReaction { id, emoji, reply }) => {
                        let res = server
                            .toggle_status_reaction(&id, &emoji)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if status_changed(&server, &mut last_statuses) {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                    }
                    Some(AppCommand::SetStatusPin { id, pinned, reply }) => {
                        let res = server
                            .set_status_pin(&id, pinned)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if status_changed(&server, &mut last_statuses) {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                    }
                    Some(AppCommand::StatusMembersMayPost { reply }) => {
                        let _ = reply.send(server.status_members_may_post());
                    }
                    Some(AppCommand::SetStatusMembersMayPost { allow, reply }) => {
                        let res = server
                            .set_status_members_may_post(allow)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        // The policy rides the feed document, so a UI that re-reads the feed on
                        // this event also re-reads who may write to it.
                        if status_changed(&server, &mut last_statuses) {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                    }
                    Some(AppCommand::CatchUpStatus { peer }) => {
                        if let Err(e) = server.request_status_catchup(peer).await {
                            tracing::warn!(error = %e, "status catch-up failed");
                        }
                        if status_changed(&server, &mut last_statuses) {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                    }
                    Some(AppCommand::CreateEvent { title, body, start_ts, end_ts, image, reply }) => {
                        let res = server
                            .create_event(&title, &body, start_ts, end_ts, &image)
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
                        if moderation_changed(&server, &mut last_moderation) {
                            let _ = event_tx.send(AppEvent::ModerationUpdated).await;
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
                    Some(AppCommand::ModerationState { reply }) => {
                        let _ = reply.send(server.moderation_state());
                    }
                    Some(AppCommand::WarnMessage {
                        channel,
                        message_id,
                        reason,
                        reply,
                    }) => {
                        let res = server
                            .warn_message(channel, &message_id, &reason)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if moderation_changed(&server, &mut last_moderation) {
                            let _ = event_tx.send(AppEvent::ModerationUpdated).await;
                        }
                    }
                    Some(AppCommand::CreateKickCase {
                        target,
                        reason,
                        evidence_ids,
                        reply,
                    }) => {
                        let res = server
                            .create_kick_case(&target, &reason, &evidence_ids)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if moderation_changed(&server, &mut last_moderation) {
                            let _ = event_tx.send(AppEvent::ModerationUpdated).await;
                        }
                    }
                    Some(AppCommand::CastKickVote { case_id, yes, reply }) => {
                        let res = server
                            .cast_kick_vote(&case_id, yes)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if moderation_changed(&server, &mut last_moderation) {
                            let _ = event_tx.send(AppEvent::ModerationUpdated).await;
                        }
                    }
                    Some(AppCommand::ResolveKickCase { case_id, remove, reply }) => {
                        let res = server
                            .resolve_kick_case(&case_id, remove)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if moderation_changed(&server, &mut last_moderation) {
                            let _ = event_tx.send(AppEvent::ModerationUpdated).await;
                        }
                        let mc = server.member_count();
                        if mc != members {
                            members = mc;
                            let _ = event_tx.send(AppEvent::MembersChanged { count: mc }).await;
                        }
                        if roles_changed(&server, &mut last_roles) {
                            let _ = event_tx.send(AppEvent::RolesUpdated).await;
                        }
                    }
                    Some(AppCommand::CatchUpModeration { peer }) => {
                        if let Err(e) = server.request_moderation_catchup(peer).await {
                            tracing::warn!(error = %e, "moderation catch-up failed");
                        }
                        if moderation_changed(&server, &mut last_moderation) {
                            let _ = event_tx.send(AppEvent::ModerationUpdated).await;
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
                        if moderation_changed(&server, &mut last_moderation) {
                            let _ = event_tx.send(AppEvent::ModerationUpdated).await;
                        }
                    }
                    Some(AppCommand::ReadWikiPage { name, reply }) => {
                        let _ = reply.send(server.read_wiki_page(&name));
                    }
                    Some(AppCommand::WriteWikiPage { name, body, reply }) => {
                        let res = server
                            .write_wiki_page(&name, &body)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if wiki_changed(&server, &mut last_wiki) {
                            let _ = event_tx.send(AppEvent::WikiUpdated).await;
                        }
                    }
                    Some(AppCommand::WikiHistory { page, reply }) => {
                        let _ = reply.send(server.wiki_history(&page));
                    }
                    Some(AppCommand::WikiPendingEdits { reply }) => {
                        let _ = reply.send(server.wiki_pending_edits());
                    }
                    Some(AppCommand::FileSizeLimit { reply }) => {
                        let _ = reply.send(server.file_size_limit());
                    }
                    Some(AppCommand::SetFileSizeLimit { bytes, reply }) => {
                        let res = server
                            .set_file_size_limit(bytes)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        // The limit lives in the file index document, so this is a file change
                        // as far as every reader is concerned.
                        if files_changed(&server, &mut file_count) {
                            let _ = event_tx.send(AppEvent::FilesUpdated).await;
                        }
                    }
                    Some(AppCommand::WikiReviewDays { reply }) => {
                        let _ = reply.send(server.wiki_review_days());
                    }
                    Some(AppCommand::SetWikiReviewDays { days, reply }) => {
                        let res = server
                            .set_wiki_review_days(days)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if wiki_changed(&server, &mut last_wiki) {
                            let _ = event_tx.send(AppEvent::WikiUpdated).await;
                        }
                    }
                    Some(AppCommand::ApproveWikiEdit { id, reply }) => {
                        let res = server
                            .approve_wiki_edit(&id)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if wiki_changed(&server, &mut last_wiki) {
                            let _ = event_tx.send(AppEvent::WikiUpdated).await;
                        }
                    }
                    Some(AppCommand::RejectWikiEdit { id, reply }) => {
                        let res = server
                            .reject_wiki_edit(&id)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
                        if wiki_changed(&server, &mut last_wiki) {
                            let _ = event_tx.send(AppEvent::WikiUpdated).await;
                        }
                    }
                    Some(AppCommand::RestoreWikiPage { page, rev, reply }) => {
                        let res = server
                            .restore_wiki_page(&page, &rev)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(res);
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
                        // Same-LAN routes are deliberately absent from signed public peer records.
                        // Retry the exact outbound routes this installation previously proved,
                        // independent of rendezvous configuration, so simultaneous restarts heal
                        // when the member that owns the listener comes online second.
                        server.dial_local_reconnect_routes().await;
                        if server.has_rendezvous() {
                            server.drive_discovery().await;
                            // ONE overall timeout bounds the whole drain to a single window, so an
                            // attacker drip-feeding records can't extend the actor's block; the
                            // count cap bounds the work.
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_millis(DISCOVERY_DRAIN_MS),
                                async {
                                    for _ in 0..MAX_DISCOVERED_PER_TICK {
                                        match server.next_postjoin_discovery_event().await {
                                            Some(catcoms_sync::PostJoinDiscoveryEvent::Discovered(d)) => {
                                                server.ingest_discovered(d).await;
                                            }
                                            Some(catcoms_sync::PostJoinDiscoveryEvent::Registered(registration)) => {
                                                server.note_rendezvous_registered(registration);
                                            }
                                            None => break, // transport closed
                                        }
                                    }
                                },
                            )
                            .await;
                        }
                        // Member peer exchange, every pass and independent of rendezvous: this is
                        // how `peer_records` fills, and with it presence, the cross-session
                        // re-dial and the eclipse detector's reach term. Then offer the
                        // cross-session cache's previously-proven members to the dial policy
                        // (candidates only; the first-contact route past a hostile rendezvous),
                        // and fold whatever we now know back into the cache for next launch.
                        //
                        // Each request carries its own deadline, not the pass as a whole. One
                        // budget for the pass meant a peer that accepted the first request and
                        // then went quiet consumed the entire tick, every tick, and the peers
                        // behind it were never reached: a self-eclipse of the discovery layer
                        // for the price of one idle connection. A peer that misses its deadline
                        // is backed off, so it also stops being picked for a while.
                        for peer in server.pex_targets() {
                            match tokio::time::timeout(
                                std::time::Duration::from_millis(PEX_REQUEST_MS),
                                server.request_pex(peer),
                            )
                            .await
                            {
                                Ok(Ok(_)) => {
                                    // The role offer uses its own additive request kind so old
                                    // peers remain PEX-compatible. It is best-effort and bounded
                                    // by the same per-peer deadline as PEX.
                                    let _ = tokio::time::timeout(
                                        std::time::Duration::from_millis(PEX_REQUEST_MS),
                                        server.request_switchboard_offer(peer),
                                    )
                                    .await;
                                }
                                Ok(Err(e)) => {
                                    tracing::trace!(error = %e, "PEX request failed");
                                    server.note_pex_failure(peer);
                                }
                                Err(_) => {
                                    tracing::debug!("PEX request timed out; backing the peer off");
                                    server.note_pex_failure(peer);
                                }
                            }
                        }
                        // Fold records learned by the PEX requests above into the passive view
                        // before dialing it. The previous order delayed a newly learned member or
                        // dynamic-IP epoch until the *next* minute tick, even though its fresh
                        // signature is precisely the signal that should bypass retry backoff.
                        server.cache_known_records();
                        // SWIM observations, reciprocal signalling and HyParView promotion share
                        // one bounded repair pass. Helpers inspect only live proven paths; all
                        // resulting sockets remain behind the ordinary policy + endpoint budget.
                        server.drive_mesh_repair().await;
                        server.dial_cached_peers().await;
                        // PEX can authenticate the signed descriptor for a transport identity
                        // that was already connected before this pass. In that ordering there is
                        // no later transport event to announce the now-resolved member as online,
                        // so refresh presence here as well as in the transport-event arm. The UI
                        // must not have to wait for a message or another socket transition before
                        // it learns that the existing connection belongs to this roster member.
                        let online = server.online_members();
                        if online != last_online {
                            last_online = online.clone();
                            let _ = event_tx.send(AppEvent::ConnectivityChanged { online }).await;
                        }
                        let route_revision = server.member_route_revision();
                        if route_revision != last_member_route_revision {
                            last_member_route_revision = route_revision;
                            let _ = event_tx.send(AppEvent::MemberRoutesChanged).await;
                        }
                        let switchboards = server.connected_switchboard_offers();
                        if switchboards != last_switchboards {
                            last_switchboards = switchboards;
                            let _ = event_tx.send(AppEvent::SwitchboardsChanged).await;
                        }
                    }
                    Some(AppCommand::SetLocalReconnectRoutes { routes }) => {
                        server.set_local_reconnect_routes(routes);
                    }
                    Some(AppCommand::MintMemberRecovery { candidates, reply }) => {
                        let result = server
                            .mint_member_recovery_code(candidates)
                            .map_err(|error| error.to_string());
                        let _ = reply.send(result);
                    }
                    Some(AppCommand::VerifyMemberRecovery { code, reply }) => {
                        let result = server
                            .verify_member_recovery_code(&code)
                            .map_err(|error| error.to_string());
                        let _ = reply.send(result);
                    }
                    Some(AppCommand::ApplyMemberRecovery { code, reply }) => {
                        let result = server
                            .apply_member_recovery_code(&code)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = reply.send(result);
                    }
                    Some(AppCommand::ManualFallbackRedial { reply }) => {
                        let outcome = server.manual_fallback_redial().await;
                        let _ = reply.send(outcome);
                        let route_revision = server.member_route_revision();
                        if route_revision != last_member_route_revision {
                            last_member_route_revision = route_revision;
                            let _ = event_tx.send(AppEvent::MemberRoutesChanged).await;
                        }
                    }
                    Some(AppCommand::PublishSelfRecord { addresses, seq }) => {
                        if let Err(e) = server.publish_self_record(addresses, seq) {
                            tracing::warn!(error = %e, "publishing the peer record failed");
                        }
                    }
                    Some(AppCommand::AddressCacheBytes { integrity_key, reply }) => {
                        // Refresh before serializing, so what is sealed reflects the members this
                        // session actually proved rather than the set the last tick happened to see.
                        server.cache_known_records();
                        let _ = reply.send(server.address_cache_bytes(&integrity_key));
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
                    Some(AppCommand::WrapInviteWithSwitchboards { invite, reply }) => {
                        let result = server
                            .wrap_invite_with_switchboards(&invite)
                            .map_err(|error| error.to_string());
                        let _ = reply.send(result);
                    }
                    Some(AppCommand::ReadmitEvictedPeers { reply }) => {
                        let res = server.readmit_evicted_peers().map_err(|e| e.to_string());
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
                // A receipt may be the final network event in a quiet room. Wake from the same
                // injected clock used to start the throttle so the last coalesced state is still
                // surfaced without waiting for unrelated traffic.
                _ = &mut delivery_wake => {
                    event_tx.idle();
                    for (channel, snapshot) in recompute_due_delivery(
                        &mut server,
                        &mut delivery,
                        &mut delivery_dirty,
                    ) {
                        let _ = event_tx
                            .send(AppEvent::DeliveryChanged { channel, snapshot })
                            .await;
                    }
                },
                // Work nobody asked for. An op arriving from a peer is not the consequence of the
                // last local command, and attributing it to one would invent a causal link that a
                // reader would go on to trust.
                cont = server.sync_once() => { event_tx.idle(); match cont {
                    Ok(true) => {
                        if server.has_pending_reciprocal() {
                            server.drive_pending_reciprocal().await;
                        }
                        sync_channels(
                            &mut server,
                            &mut last_channels,
                            &mut counts,
                            &mut versions,
                            &event_tx,
                            None,
                            None,
                        )
                        .await;
                        // Only a channel whose document moved can have changed; the version
                        // check is O(1), the delta it guards walks the channel's history.
                        let mut moved_channels = Vec::new();
                        for channel in counts.keys().copied().collect::<Vec<_>>() {
                            if !versions.moved(&server, crate::DocType::Channel, channel) {
                                continue;
                            }
                            moved_channels.push(channel);
                            // The version was consumed just above; a second check here would
                            // read "unchanged" and swallow the very change it is meant to report.
                            if let Some(change) = channel_delta(&server, channel, &mut counts) {
                                let _ = event_tx
                                    .send(AppEvent::ChannelUpdated { channel, change })
                                    .await;
                            }
                        }
                        let mc = server.member_count();
                        // Roles derive from the roles document AND the group (the owner is the
                        // designated committer; the roster is the MLS membership), and moderation
                        // derives from roles plus the device registry; so those two projections
                        // are re-read when any of their inputs moved, not only their own document.
                        let membership_moved = mc != members || versions.epoch_moved(&server);
                        if mc != members {
                            members = mc;
                            let _ = event_tx.send(AppEvent::MembersChanged { count: mc }).await;
                        }
                        let profiles_moved =
                            versions.moved(&server, crate::DocType::Profile, crate::PROFILE_DOC);
                        sync_profiles(&mut server, &mut last_profiles, &event_tx, profiles_moved)
                            .await;
                        if versions.moved(&server, crate::DocType::Livery, crate::LIVERY_DOC)
                            && livery_changed(&server, &mut last_livery)
                        {
                            let _ = event_tx.send(AppEvent::LiveryUpdated).await;
                        }
                        if versions.moved(&server, crate::DocType::Badges, crate::BADGES_DOC)
                            && badges_changed(&server, &mut last_badges)
                        {
                            let _ = event_tx.send(AppEvent::BadgesUpdated).await;
                        }
                        let devices_moved =
                            versions.moved(&server, crate::DocType::Devices, crate::DEVICES_DOC);
                        if devices_moved && devices_changed(&server, &mut last_devices) {
                            let _ = event_tx.send(AppEvent::DevicesUpdated).await;
                        }
                        if versions.moved(
                            &server,
                            crate::DocType::FileIndex,
                            crate::FILE_INDEX_DOC,
                        ) && files_changed(&server, &mut file_count)
                        {
                            let _ = event_tx.send(AppEvent::FilesUpdated).await;
                        }
                        if versions.moved(&server, crate::DocType::Status, crate::STATUS_DOC)
                            && status_changed(&server, &mut last_statuses)
                        {
                            let _ = event_tx.send(AppEvent::StatusUpdated).await;
                        }
                        if versions.moved(&server, crate::DocType::Calendar, crate::CALENDAR_DOC)
                            && events_changed(&server, &mut last_events)
                        {
                            let _ = event_tx.send(AppEvent::EventsUpdated).await;
                        }
                        // The wiki's effective state is read-time: a pending edit auto-accepts at
                        // its deadline with no op written. While anything is pending the full
                        // compare keeps running so that moment is still noticed; with nothing
                        // pending, only the document moving can change what is rendered.
                        let wiki_moved =
                            versions.moved(&server, crate::DocType::Wiki, crate::WIKI_DOC);
                        if (wiki_moved || !last_wiki.2.is_empty())
                            && wiki_changed(&server, &mut last_wiki)
                        {
                            let _ = event_tx.send(AppEvent::WikiUpdated).await;
                        }
                        let roles_moved = versions.moved(
                            &server,
                            crate::DocType::MemberRoles,
                            crate::ROLES_DOC,
                        ) | membership_moved
                            | devices_moved;
                        if roles_moved && roles_changed(&server, &mut last_roles) {
                            let _ = event_tx.send(AppEvent::RolesUpdated).await;
                        }
                        // Re-validating moderation evidence costs one signature check per record,
                        // which is exactly the work that must not run on every gossip frame.
                        if (versions.moved(
                            &server,
                            crate::DocType::Moderation,
                            crate::moderation::MODERATION_DOC,
                        ) | roles_moved)
                            && moderation_changed(&server, &mut last_moderation)
                        {
                            let _ = event_tx.send(AppEvent::ModerationUpdated).await;
                        }
                        // Presence: emit when the set of currently-reachable members changes
                        // (a peer connected or dropped). `online_members` is sorted, so the Vec
                        // compare is order-stable.
                        let online = server.online_members();
                        let presence_changed = online != last_online;
                        if presence_changed {
                            last_online = online.clone();
                            let _ = event_tx
                                .send(AppEvent::ConnectivityChanged { online })
                                .await;
                        }
                        // A DCUtR upgrade or one partial close does not change aggregate presence,
                        // but it can change the Connectivity verdict from relay to direct (or
                        // back). The session-local revision is bumped only by mutations that can
                        // affect a current member row, avoiding an O(roster x addresses) compare
                        // and preventing unclaimed Internet-peer churn from driving UI refreshes.
                        let route_revision = server.member_route_revision();
                        if route_revision != last_member_route_revision {
                            last_member_route_revision = route_revision;
                            let _ = event_tx.send(AppEvent::MemberRoutesChanged).await;
                        }
                        let switchboards = server.connected_switchboard_offers();
                        if switchboards != last_switchboards {
                            last_switchboards = switchboards;
                            let _ = event_tx.send(AppEvent::SwitchboardsChanged).await;
                        }
                        // Delivery: a peer's inbound op may be the evidence that it received one of
                        // our messages. Recomputing walks the channel's change graph, so it is
                        // rate-limited per channel; throttled channels remain dirty and receive a
                        // deterministic timer wake if no later network event arrives.
                        //
                        // Only a channel whose document moved can carry new causal evidence. An
                        // explicit receipt lives outside the documents and a delivery row also
                        // names how many members are reachable, so either of those dirties
                        // everything. This gating was reverted once, because the timer it arms was
                        // the only thing cancelling a tick stuck in an outbound request; that is
                        // now the request's own deadline (`CATCHUP_REQUEST_MS`), so the wake is a
                        // wake again rather than a load-bearing accident.
                        let delivery_evidence = server.delivery_evidence_revision();
                        if delivery_evidence != last_delivery_evidence || presence_changed {
                            last_delivery_evidence = delivery_evidence;
                            delivery_dirty.extend(counts.keys().copied());
                        } else {
                            delivery_dirty.extend(moved_channels.iter().copied());
                        }
                        for (channel, snapshot) in recompute_due_delivery(
                            &mut server,
                            &mut delivery,
                            &mut delivery_dirty,
                        ) {
                            let _ = event_tx
                                .send(AppEvent::DeliveryChanged { channel, snapshot })
                                .await;
                        }
                        // A DM (friend) request may have arrived over this group; surface a change.
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
                } },
            }
        }
    });
    (
        ServerActor {
            cmd_tx: CommandSender {
                tx: cmd_tx,
                trace: Trace::NONE,
            },
        },
        event_rx,
        handle,
    )
}

/// Notice channel-directory changes, subscribe newly-discovered channel documents, and recover
/// their existing history. The catalog event is emitted only after those subscriptions exist, so
/// clicking a channel immediately cannot race its first catch-up.
async fn sync_channels<T, R>(
    server: &mut Server<T, R>,
    last: &mut Vec<ChannelInfo>,
    sigs: &mut HashMap<u128, ChannelSignature>,
    versions: &mut DocVersions,
    event_tx: &EventSink,
    catchup_peer: Option<PeerId>,
    locally_created: Option<u128>,
) where
    T: MeshTransport,
    R: CryptoRngCore,
{
    // The directory is one small document; comparing its version first keeps the common case
    // (nothing changed) from re-reading it on every network event.
    if !versions.moved(
        server,
        crate::DocType::ChannelIndex,
        crate::CHANNEL_INDEX_DOC,
    ) {
        return;
    }
    let next = server.channels();
    if next == *last {
        return;
    }
    let prior: std::collections::HashSet<u128> = last.iter().map(|c| c.id).collect();
    for channel in next.iter().filter(|c| !prior.contains(&c.id)) {
        if let Err(e) = server.open_channel(channel.id).await {
            tracing::warn!(error = %e, channel = channel.id, "open discovered channel failed");
            continue;
        }
        if locally_created != Some(channel.id) {
            let recovered = match catchup_peer {
                Some(peer) => server.request_channel_catchup(peer, channel.id).await,
                None => server.request_channel_catchup_any(channel.id).await,
            };
            if let Err(e) = recovered {
                tracing::warn!(
                    error = %e,
                    channel = channel.id,
                    "discovered channel catch-up failed"
                );
            }
        }
        channel_delta_if_moved(server, channel.id, sigs, versions);
    }
    *last = next;
    let _ = event_tx.send(AppEvent::ChannelsUpdated).await;
}

/// The last-seen version of every document the actor projects (see [`Server::doc_version`]),
/// plus the group epoch. A projection is only re-read when its inputs moved; before this record
/// existed, every one of them was re-materialized on every network event.
#[derive(Default)]
struct DocVersions {
    seen: HashMap<(crate::DocType, u128), u64>,
    epoch: Option<u64>,
}

impl DocVersions {
    /// Whether `(doc_type, doc_id)` changed since the last call for it (recording the current
    /// version). The first call for a document always reports movement, so a projection is
    /// seeded on first sight exactly as it was when every tick re-read everything.
    fn moved<T, R>(&mut self, server: &Server<T, R>, doc_type: crate::DocType, doc_id: u128) -> bool
    where
        T: MeshTransport,
        R: CryptoRngCore,
    {
        let version = server.doc_version(doc_type, doc_id);
        self.seen.insert((doc_type, doc_id), version) != Some(version)
    }

    /// Whether the MLS epoch advanced since the last call (recording it): the signal that the
    /// roster, and so everything derived from membership, may have changed.
    fn epoch_moved<T, R>(&mut self, server: &Server<T, R>) -> bool
    where
        T: MeshTransport,
        R: CryptoRngCore,
    {
        let epoch = server.epoch();
        self.epoch.replace(epoch) != Some(epoch)
    }
}

/// [`channel_delta`], skipped when the channel's document has not moved since the last look.
/// The version check is O(1); the delta walks the channel's whole history. A skipped check can
/// never hide a change, since every change to a channel is an op on its document.
fn channel_delta_if_moved<T, R>(
    server: &Server<T, R>,
    channel: u128,
    sigs: &mut HashMap<u128, ChannelSignature>,
    versions: &mut DocVersions,
) -> Option<ChannelChange>
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    if !versions.moved(server, crate::DocType::Channel, channel) {
        return None;
    }
    channel_delta(server, channel, sigs)
}

/// The last-seen fingerprint of one channel's rendered content, kept per open channel so a change
/// can be classified rather than merely detected. The parts are separate because they answer
/// separate questions, and `ids` is the set of message ids: an arrival is "an id we have never
/// seen", which is the one definition a concurrent append+delete cannot fool.
///
/// `jukebox` holds the queue itself rather than a digest of it. The message log is the only
/// unbounded part here, so a queue capped at [`crate::MAX_JUKEBOX_ENTRIES`] entries is cheap to
/// keep whole, and a digest can only ever lose: it can report "nothing moved" for a queue that
/// moved, which is the one answer this record must not give, since it exists to settle "the
/// queue changed but the UI did not".
#[derive(Default)]
struct ChannelSignature {
    topic: u64,
    jukebox: Vec<JukeEntry>,
    messages: u64,
    ids: std::collections::HashSet<u64>,
}

fn hash_of(value: impl std::hash::Hash) -> u64 {
    use std::hash::Hasher as _;
    // `DefaultHasher::new()` has a fixed seed, so the same content hashes the same across ticks.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// Classify what moved in a channel since it was last seen (updating the record). `None` means
/// nothing did, so no event is worth emitting.
///
/// Synchronous; the `&Server` borrow ends before the caller awaits the event send, so
/// the actor future stays `Send` (a `&Server` held across an await would require
/// `Server: Sync`, which it is not).
fn channel_delta<T, R>(
    server: &Server<T, R>,
    channel: u128,
    sigs: &mut HashMap<u128, ChannelSignature>,
) -> Option<ChannelChange>
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    use std::hash::{Hash as _, Hasher as _};
    // The topic and the jukebox queue ride the same channel document and the same rendered header,
    // so a peer's edit still has to reach the UI; it just no longer claims to be a message.
    let topic = hash_of(server.channel_topic(channel));
    // Compared whole. Folding each entry's id and name into a hash lost every other field, so a
    // peer re-pointing a queued track at a different content address (same id, same display name)
    // read as an unchanged queue and never reached the UI.
    let jukebox = server.jukebox(channel);
    // A content signature (not just the count) so an EDIT; which doesn't change the count; is
    // detected too, both locally and when a peer's edit arrives. Cheap over a channel's message
    // list, which this actor already materializes on every sync tick.
    let previous_ids = sigs.get(&channel).map(|sig| sig.ids.clone());
    let mut messages_hasher = std::collections::hash_map::DefaultHasher::new();
    let mut ids = std::collections::HashSet::new();
    // The ids that were not here last time, in the order they now read. Rows are ordered by
    // timestamp, so an arrival is not always the last row: a message can be delayed, or come from
    // a device whose clock is behind, and land anywhere in the list. Naming them is the only way
    // for anything downstream to describe what actually arrived rather than what happens to sit at
    // the end. Bounded, because this only has to carry an announcement.
    let mut arrivals: Vec<String> = Vec::new();
    server.with_messages(channel, |msgs| {
        for m in msgs {
            if let Some(seen) = &previous_ids {
                if !seen.contains(&hash_of(&m.id))
                    && !m.id.is_empty()
                    && arrivals.len() < MAX_REPORTED_ARRIVALS
                {
                    arrivals.push(m.id.clone());
                }
            }
            m.id.hash(&mut messages_hasher);
            m.text.hash(&mut messages_hasher);
            m.edited.hash(&mut messages_hasher);
            m.pinned.hash(&mut messages_hasher);
            // Reactions change the rendered message too (count unchanged), so fold them in.
            for r in &m.reactions {
                r.emoji.hash(&mut messages_hasher);
                r.by.hash(&mut messages_hasher);
            }
            ids.insert(hash_of(&m.id));
        }
    });
    let messages = messages_hasher.finish();

    let next = ChannelSignature {
        topic,
        jukebox,
        messages,
        ids,
    };
    let Some(previous) = sigs.get(&channel) else {
        // First sight of this channel: seed the record and report nothing. The UI fetches messages
        // when it opens a channel, so a "changed" here would be a spurious badge on every open.
        sigs.insert(channel, next);
        return None;
    };
    let appended = next.ids.iter().any(|id| !previous.ids.contains(id));
    let change = ChannelChange {
        messages_appended: appended,
        arrivals,
        messages_changed: !appended && next.messages != previous.messages,
        topic: next.topic != previous.topic,
        jukebox: next.jukebox != previous.jukebox,
    };
    sigs.insert(channel, next);
    change.any().then_some(change)
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

/// What [`status_changed`] compares: the feed's posts, and the policy deciding whether this
/// member is offered anywhere to write. Both ride the same document and both change what is
/// rendered, so both belong in the comparison.
type StatusSnapshot = (Vec<ChatMessage>, bool);

/// The status feed's current posts + posting policy.
fn status_snapshot<T, R>(server: &Server<T, R>) -> StatusSnapshot
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    (server.statuses(), server.status_members_may_post())
}

/// Whether the status feed changed since last seen (updating the record). A count of posts was
/// enough while the feed only grew; an edit, a reaction, a pin and a policy change each leave the
/// number of posts exactly where it was, and each still has to reach the UI. Both states are in
/// memory at this moment, so comparing them can only be more truthful than folding either into a
/// digest first.
fn status_changed<T, R>(server: &Server<T, R>, last: &mut StatusSnapshot) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = status_snapshot(server);
    if now != *last {
        *last = now;
        true
    } else {
        false
    }
}

/// Whether the server events changed since last seen (updating the record). A count alone would
/// miss a concurrent create+delete converging in one tick, so this compares the full list; which
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

/// What [`wiki_changed`] compares: the page map, the per-page format metadata, the review
/// queue, and the review window. A format toggle leaves every body byte-identical, a newly
/// queued proposal changes no page at all, and a review-window flip is pure settings; each
/// still must reach the UI as a `WikiUpdated`.
type WikiSnapshot = (
    HashMap<String, String>,
    HashMap<String, String>,
    Vec<WikiPendingEdit>,
    u32,
);

/// The wiki's current bodies + formats.
fn wiki_snapshot<T, R>(server: &Server<T, R>) -> WikiSnapshot
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    (
        server.wiki_map(),
        server.wiki_meta(),
        server.wiki_pending_edits(),
        server.wiki_review_days(),
    )
}

/// Whether the wiki changed since last seen (a page added/removed/renamed, a body edited or a
/// format toggled; a count alone misses edits, so this compares the full snapshot). Updates
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

/// Whether signed moderation evidence/cases/votes changed since last seen. The document is bounded
/// per record and returned in deterministic order, so a full comparison also catches a changed vote
/// without relying on a count.
fn moderation_changed<T, R>(server: &Server<T, R>, last: &mut ModerationState) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = server.moderation_state();
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
/// held from a peer and emit again if that resolved new avatars; so a member's picture
/// renders shortly after their profile arrives (avatars travel by content address, not
/// inline). Synchronous fetch (blocks the actor briefly per missing avatar); fine for the
/// small downscaled avatars, a concurrent fetch is a later refinement.
async fn sync_profiles<T, R>(
    server: &mut Server<T, R>,
    last_profiles: &mut HashMap<String, Profile>,
    event_tx: &EventSink,
    profiles_moved: bool,
) where
    T: MeshTransport,
    R: CryptoRngCore,
{
    if profiles_moved && profiles_changed(server, last_profiles) {
        let _ = event_tx.send(AppEvent::ProfilesUpdated).await;
    }
    // Always attempt to resolve missing avatars; the peer holding one is often only known
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
    use catcoms_mls::{InviteToken, MlsDevice};
    use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    use std::time::Duration;
    use tokio::time::timeout;

    const GENERAL: u128 = 1;

    /// A delta says which rows arrived, not which row sorts last.
    ///
    /// Rows are ordered by the sender's timestamp, so a message that was delayed, or written on a
    /// device whose clock is behind, lands wherever its stamp says. Reading the end of the list to
    /// find "the new one" then describes whatever is newest, which after such an arrival is a
    /// message somebody has very likely already seen.
    #[tokio::test]
    async fn a_channel_delta_names_the_rows_that_arrived_not_the_newest_row() {
        let hub = Hub::new();
        let mut server = founder(&hub, PeerId::from_u64(1), "alice", 1);
        server.open_channel(GENERAL).await.unwrap();
        async fn post_at(server: &mut Server<MemNetwork, ChaCha20Rng>, id: &str, ts: u64) {
            let id = id.to_string();
            server
                .sync
                .post(crate::DocType::Channel, GENERAL, move |d| {
                    crate::append_message(d, &id, "bob", "text", ts, "")
                })
                .await
                .unwrap();
        }
        let mut sigs = HashMap::new();

        // First sight seeds the record and reports nothing.
        post_at(&mut server, "newest", 2_000).await;
        assert!(channel_delta(&server, GENERAL, &mut sigs).is_none());

        // Now a message that was said earlier arrives. It sorts before the row already here, so
        // the last row is unchanged and only the arrival list can say what happened.
        post_at(&mut server, "delayed", 1_000).await;
        let change = channel_delta(&server, GENERAL, &mut sigs).expect("an arrival is a change");
        assert!(change.messages_appended);
        assert_eq!(
            change.arrivals,
            vec!["delayed".to_string()],
            "the row that arrived, not the row that sorts last"
        );
        let rows = server.with_messages(GENERAL, |m| {
            m.iter().map(|m| m.id.clone()).collect::<Vec<_>>()
        });
        assert_eq!(
            rows,
            vec!["delayed".to_string(), "newest".to_string()],
            "and it really did sort before the row that was already here"
        );

        // A pin re-renders the log without anything arriving, and names nothing.
        server.set_pin(GENERAL, "newest", true).await.unwrap();
        let change = channel_delta(&server, GENERAL, &mut sigs).expect("a pin is a change");
        assert!(!change.messages_appended);
        assert!(change.arrivals.is_empty());
    }

    /// An arrival is read by name, from wherever it sorts, and carries its own addressing.
    ///
    /// Both are what the tail could not give. A message ordered by its sender's timestamp can sit
    /// arbitrarily far back, so a bounded tail does not contain it; and whether it addresses this
    /// member is a property of that row, where the tail could only say whether anything *after the
    /// read mark* did, which a row sorting before that mark is not.
    #[tokio::test]
    async fn an_arrival_is_found_by_id_and_carries_its_own_addressing() {
        let hub = Hub::new();
        let mut server = founder(&hub, PeerId::from_u64(1), "alice", 1);
        server.open_channel(GENERAL).await.unwrap();
        server.open_profiles().await.unwrap();
        server
            .set_profile(crate::Profile {
                name: "Alice Cat".into(),
                ..crate::Profile::default()
            })
            .await
            .unwrap();
        async fn post_at(
            server: &mut Server<MemNetwork, ChaCha20Rng>,
            id: &str,
            text: &str,
            ts: u64,
        ) {
            let (id, text) = (id.to_string(), text.to_string());
            server
                .sync
                .post(crate::DocType::Channel, GENERAL, move |d| {
                    crate::append_message(d, &id, "bob", &text, ts, "")
                })
                .await
                .unwrap();
        }
        // A long conversation, and then a mention that was said before all of it.
        for i in 0..200u64 {
            post_at(&mut server, &format!("m{i}"), "chatter", 10_000 + i).await;
        }
        post_at(&mut server, "delayed", "@[Alice Cat] over here", 1_000).await;

        let rows = server.messages_by_id(GENERAL, &["delayed".to_string()]);
        assert_eq!(rows.len(), 1, "found by name, not by looking near the end");
        assert_eq!(rows[0].0.id, "delayed");
        assert!(rows[0].1, "and it says for itself that it addresses me");

        // The tail cannot answer either question about it.
        let tail = server.message_tail(GENERAL, 64, "m199", 10_199);
        assert!(
            !tail.rows.iter().any(|(m, _)| m.id == "delayed"),
            "the row is nowhere near the rows a notification would otherwise read"
        );
        assert!(
            !tail.addressed_after_cursor,
            "and the cursor scan cannot see a mention that sorts before the mark"
        );

        // Ids that name nothing are simply absent, rather than something else standing in.
        assert!(server
            .messages_by_id(GENERAL, &["no-such-row".to_string()])
            .is_empty());
    }

    #[tokio::test]
    async fn cancellation_interrupts_an_actor_owned_chunk_future() {
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        let cancellation = RequestCancellation::new(receiver, None);
        let stalled = fetch_chunk_or_cancel(Some(cancellation), std::future::pending());
        tokio::pin!(stalled);

        assert!(timeout(Duration::from_millis(10), &mut stalled)
            .await
            .is_err());
        cancel.send(true).unwrap();
        assert_eq!(
            timeout(Duration::from_secs(1), &mut stalled)
                .await
                .expect("cancellation must wake the actor")
                .unwrap_err(),
            "download cancelled"
        );
    }

    #[tokio::test]
    async fn fallible_member_routes_distinguishes_a_stopped_actor_from_an_empty_view() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let actor = ServerActor {
            cmd_tx: CommandSender {
                tx,
                trace: Trace::NONE,
            },
        };

        assert_eq!(
            actor.try_member_routes().await.unwrap_err(),
            "server stopped"
        );
        assert!(
            actor.member_routes().await.is_empty(),
            "the compatibility convenience remains an empty fallback"
        );
    }

    fn founder(
        hub: &std::sync::Arc<Hub>,
        peer: PeerId,
        name: &str,
        seed: u64,
    ) -> Server<MemNetwork, ChaCha20Rng> {
        founder_with_clock(hub, peer, name, seed, &ManualClock::new(1_000))
    }

    fn founder_with_clock(
        hub: &std::sync::Arc<Hub>,
        peer: PeerId,
        name: &str,
        seed: u64,
        clock: &ManualClock,
    ) -> Server<MemNetwork, ChaCha20Rng> {
        Server::found(
            hub.join(peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(seed),
            Box::new(clock.clone()),
            name,
        )
        .unwrap()
    }

    /// The version gate must be exactly as sensitive as the delta it guards: a quiet tick costs
    /// nothing, and no change `channel_delta` would report is ever swallowed; including an edit,
    /// which changes content without changing the count.
    #[tokio::test]
    async fn channel_delta_is_skipped_only_while_the_document_is_untouched() {
        let hub = Hub::new();
        let mut server = founder(&hub, PeerId::from_u64(1), "alice", 1);
        server.open_channel(GENERAL).await.unwrap();
        let mut sigs = HashMap::new();
        let mut versions = DocVersions::default();
        assert!(
            channel_delta_if_moved(&server, GENERAL, &mut sigs, &mut versions).is_none(),
            "first sight seeds the record and reports nothing"
        );
        assert!(
            sigs.contains_key(&GENERAL),
            "the record was seeded even though nothing was reported"
        );
        assert!(
            !versions.moved(&server, crate::DocType::Channel, GENERAL),
            "an untouched document has not moved"
        );

        server.send_message(GENERAL, "hi").await.unwrap();
        let change = channel_delta_if_moved(&server, GENERAL, &mut sigs, &mut versions)
            .expect("a send moves the document");
        assert!(change.messages_appended);
        assert!(
            channel_delta_if_moved(&server, GENERAL, &mut sigs, &mut versions).is_none(),
            "a change is reported once"
        );

        let id = server.messages(GENERAL)[0].id.clone();
        server.edit_message(GENERAL, &id, "hello").await.unwrap();
        let change = channel_delta_if_moved(&server, GENERAL, &mut sigs, &mut versions)
            .expect("an edit moves the document even though the count is unchanged");
        assert!(change.messages_changed && !change.messages_appended);
    }

    /// The cached materialization is keyed by document version, so a read after any write sees
    /// the new content, reactions included (they change a row without adding one).
    #[tokio::test]
    async fn message_reads_follow_every_document_version() {
        let hub = Hub::new();
        let mut server = founder(&hub, PeerId::from_u64(1), "alice", 1);
        server.open_channel(GENERAL).await.unwrap();
        assert!(server.messages(GENERAL).is_empty());
        let before = server.doc_version(crate::DocType::Channel, GENERAL);
        server.send_message(GENERAL, "one").await.unwrap();
        assert!(server.doc_version(crate::DocType::Channel, GENERAL) > before);
        assert_eq!(server.messages(GENERAL).len(), 1);
        let id = server.messages(GENERAL)[0].id.clone();
        server.toggle_reaction(GENERAL, &id, "👍").await.unwrap();
        assert_eq!(server.messages(GENERAL)[0].reactions.len(), 1);
        server.send_message(GENERAL, "two").await.unwrap();
        assert_eq!(
            server.with_messages(GENERAL, |m| m
                .iter()
                .map(|m| m.text.clone())
                .collect::<Vec<_>>()),
            vec!["one".to_string(), "two".to_string()]
        );
    }

    /// A tail read ships only the newest rows, but "addressed to me" is answered against the whole
    /// channel: a reply whose parent is older than the tail still counts, a mention counts, and my
    /// own rows never do.
    #[tokio::test]
    async fn message_tail_is_bounded_and_resolves_addressing_against_the_whole_channel() {
        let hub = Hub::new();
        let mut server = founder(&hub, PeerId::from_u64(1), "alice", 1);
        server.open_channel(GENERAL).await.unwrap();
        server.open_profiles().await.unwrap();
        server
            .set_profile(Profile {
                name: "Alice Cat".into(),
                ..Profile::default()
            })
            .await
            .unwrap();
        server.send_message(GENERAL, "my own opener").await.unwrap();
        let mine = server.messages(GENERAL)[0].id.clone();
        // The opener carries a real clock reading, and rows are ordered by timestamp now, so the
        // rows written by hand below have to sit after it rather than in a small range of their own.
        let base = server.messages(GENERAL)[0].ts + 1;

        // A peer's rows, written straight into the document with the canonical schema.
        async fn post_as(
            server: &mut Server<MemNetwork, ChaCha20Rng>,
            id: &str,
            text: &str,
            reply_to: &str,
            ts: u64,
        ) {
            let (id, text, reply_to) = (id.to_string(), text.to_string(), reply_to.to_string());
            server
                .sync
                .post(crate::DocType::Channel, GENERAL, move |d| {
                    crate::append_message(d, &id, "bob", &text, ts, &reply_to)
                })
                .await
                .unwrap();
        }
        for i in 0..5 {
            post_as(&mut server, &format!("filler-{i}"), "filler", "", base + i).await;
        }
        post_as(&mut server, "reply", "re: opener", &mine, base + 100).await;
        post_as(&mut server, "mention", "@[Alice Cat] ping", "", base + 200).await;
        post_as(&mut server, "plain", "nothing for alice", "", base + 300).await;

        let tail = server.message_tail(GENERAL, 3, "", 0);
        assert_eq!(
            tail.rows
                .iter()
                .map(|(m, _)| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["reply", "mention", "plain"],
            "the newest rows, oldest first"
        );
        assert_eq!(
            tail.rows
                .iter()
                .map(|(_, to_me)| *to_me)
                .collect::<Vec<_>>(),
            vec![true, true, false],
            "the reply's parent is outside the tail and is still resolved"
        );
        let whole = server.message_tail(GENERAL, 100, "", 0);
        assert_eq!(
            whole.rows.len(),
            9,
            "a wide limit is clamped to the history"
        );
        assert_eq!(whole.rows[0].0.id, mine);
        assert!(!whole.rows[0].1, "my own row is never addressed to me");

        // A burst longer than the tail. The mention is now far outside the rows a notification
        // would carry, and it still has to ring: the answer is computed over the whole channel
        // past this device's cursor, not over the window.
        for i in 0..10 {
            post_as(
                &mut server,
                &format!("burst-{i}"),
                "ordinary chatter",
                "",
                base + 400 + i,
            )
            .await;
        }
        let after_burst = server.message_tail(GENERAL, 3, "filler-0", base);
        assert!(
            after_burst.rows.iter().all(|(_, to_me)| !*to_me),
            "nothing in the carried window is addressed to me"
        );
        assert!(
            after_burst.addressed_after_cursor,
            "the mention behind the burst is still reported"
        );
        assert!(
            !server
                .message_tail(GENERAL, 3, "plain", base + 300)
                .addressed_after_cursor,
            "a cursor past the mention leaves only ordinary chatter"
        );

        // A read mark whose message has been deleted. The id no longer names anything, but the
        // timestamp still says how far this device had read, and it has to be believed: treating
        // the channel as unread from its first row makes an old mention ring again, and go on
        // ringing on every arrival, for a conversation the member finished with long ago.
        assert!(
            !server
                .message_tail(GENERAL, 3, "deleted-row", base + 300)
                .addressed_after_cursor,
            "a lost cursor still knows the time it was at, and the mention is behind it"
        );
        assert!(
            server
                .message_tail(GENERAL, 3, "deleted-row", base + 100)
                .addressed_after_cursor,
            "and it does not swallow a mention that came after that time"
        );
        assert!(
            server
                .message_tail(GENERAL, 3, "no-such-id", 0)
                .addressed_after_cursor,
            "no cursor at all is the one case where the whole channel is unaccounted for"
        );
    }

    /// A page is a contiguous slice around an anchor in the channel's current order, and every
    /// row carries what it needs from the rest of the channel (reply parent, reply count,
    /// addressing) so a caller can hold only the slice.
    #[tokio::test]
    async fn message_page_slices_around_an_anchor_and_resolves_row_context() {
        use crate::{MessagePageQuery, PageAnchor};
        let hub = Hub::new();
        let mut server = founder(&hub, PeerId::from_u64(1), "alice", 1);
        server.open_channel(GENERAL).await.unwrap();

        let empty = server.message_page(
            GENERAL,
            &MessagePageQuery {
                anchor: PageAnchor::Tail,
                before: 5,
                after: 5,
                unread: None,
            },
        );
        assert_eq!(empty.total, 0);
        assert!(empty.anchor_index.is_none() && empty.rows.is_empty());

        server.send_message(GENERAL, "my own opener").await.unwrap();
        let mine = server.messages(GENERAL)[0].id.clone();
        // Rows are ordered by timestamp, so these have to be stamped in the order the assertions
        // below expect to read them, and after the opener's real clock reading.
        let base = server.messages(GENERAL)[0].ts + 1;
        async fn post_as(
            server: &mut Server<MemNetwork, ChaCha20Rng>,
            id: &str,
            text: &str,
            reply_to: &str,
            ts: u64,
        ) {
            let (id, text, reply_to) = (id.to_string(), text.to_string(), reply_to.to_string());
            server
                .sync
                .post(crate::DocType::Channel, GENERAL, move |d| {
                    crate::append_message(d, &id, "bob", &text, ts, &reply_to)
                })
                .await
                .unwrap();
        }
        for i in 0..5 {
            post_as(
                &mut server,
                &format!("filler-{i}"),
                &"x".repeat(300),
                "",
                base + i,
            )
            .await;
        }
        post_as(&mut server, "reply", "re: opener", &mine, base + 100).await;
        post_as(&mut server, "reply-2", "re: filler", "filler-0", base + 200).await;
        post_as(&mut server, "plain", "nothing for alice", "", base + 300).await;
        // 9 rows: mine, filler-0..4, reply, reply-2, plain.

        let ids = |page: &crate::MessagePage| {
            page.rows
                .iter()
                .map(|r| r.message.id.clone())
                .collect::<Vec<_>>()
        };
        let tail = server.message_page(
            GENERAL,
            &MessagePageQuery {
                anchor: PageAnchor::Tail,
                before: 2,
                after: 0,
                unread: None,
            },
        );
        assert_eq!(tail.total, 9);
        assert_eq!(
            tail.version,
            server.doc_version(crate::DocType::Channel, GENERAL)
        );
        assert_eq!((tail.start, tail.anchor_index), (6, Some(8)));
        assert_eq!(ids(&tail), vec!["reply", "reply-2", "plain"]);

        let around = server.message_page(
            GENERAL,
            &MessagePageQuery {
                anchor: PageAnchor::Id("filler-2".into()),
                before: 1,
                after: 1,
                unread: None,
            },
        );
        assert_eq!((around.start, around.anchor_index), (2, Some(3)));
        assert_eq!(ids(&around), vec!["filler-1", "filler-2", "filler-3"]);

        let missing = server.message_page(
            GENERAL,
            &MessagePageQuery {
                anchor: PageAnchor::Id("nope".into()),
                before: 3,
                after: 3,
                unread: None,
            },
        );
        assert_eq!(missing.total, 9);
        assert!(missing.anchor_index.is_none() && missing.rows.is_empty());

        let clamped = server.message_page(
            GENERAL,
            &MessagePageQuery {
                anchor: PageAnchor::Index(999),
                before: 0,
                after: 50,
                unread: None,
            },
        );
        assert_eq!((clamped.start, clamped.anchor_index), (8, Some(8)));
        assert_eq!(ids(&clamped), vec!["plain"]);

        let whole = server.message_page(
            GENERAL,
            &MessagePageQuery {
                anchor: PageAnchor::Index(0),
                before: 0,
                after: 100,
                unread: None,
            },
        );
        assert_eq!(whole.rows.len(), 9);
        let row = |id: &str| whole.rows.iter().find(|r| r.message.id == id).unwrap();
        assert_eq!(row(&mine).reply_count, 1, "one reply names my opener");
        assert_eq!(row("filler-0").reply_count, 1);
        assert_eq!(row("plain").reply_count, 0);
        let reply = row("reply");
        assert!(reply.targets_me, "a reply to my message addresses me");
        let preview = reply.reply_to_preview.as_ref().expect("parent resolved");
        assert_eq!(
            (preview.id.as_str(), preview.author.as_str()),
            (mine.as_str(), server.my_fingerprint().as_str())
        );
        let reply_2 = row("reply-2");
        assert!(
            !reply_2.targets_me,
            "a reply to somebody else's message is not for me"
        );
        assert_eq!(
            reply_2
                .reply_to_preview
                .as_ref()
                .unwrap()
                .text
                .chars()
                .count(),
            crate::REPLY_PREVIEW_CHARS,
            "a long parent is cut to the preview length"
        );
        assert!(row(&mine).reply_to_preview.is_none());

        let first_reply = server.message_page(
            GENERAL,
            &MessagePageQuery {
                anchor: PageAnchor::FirstReplyTo("filler-0".into()),
                before: 0,
                after: 0,
                unread: None,
            },
        );
        assert_eq!(ids(&first_reply), vec!["reply-2"]);

        // Unread: with the divider at Bob's newest row nothing is past it; at my own opener's
        // stamp all eight of Bob's rows are, and the first is filler-0 at index 1. My own row
        // never counts. A far-future row is clamped to the ceiling, so it cannot count either.
        let newest = base + 300;
        let probe = |divider_ts: u64| MessagePageQuery {
            anchor: PageAnchor::Tail,
            before: 0,
            after: 0,
            unread: Some(crate::UnreadProbe {
                divider_id: String::new(),
                divider_ts,
                now_ms: 1_000,
            }),
        };
        // My own opener carries the (manual) clock's 1_000, which is the newest plausible stamp.
        let none = server.message_page(GENERAL, &probe(newest)).unread.unwrap();
        assert_eq!(
            (none.count, none.first_index, none.ceiling_ts),
            (0, None, newest)
        );
        let all = server
            .message_page(GENERAL, &probe(base - 1))
            .unread
            .unwrap();
        assert_eq!((all.count, all.first_index), (8, Some(1)));
        post_as_at(&mut server, "future", "from a broken clock", u64::MAX / 2).await;
        // Read from just under the newest real row, so the clamped row has somewhere to count.
        let clamped = server
            .message_page(GENERAL, &probe(newest - 1))
            .unread
            .unwrap();
        assert_eq!(
            (clamped.count, clamped.ceiling_ts),
            (2, newest),
            "an implausible timestamp is pulled down to the ceiling: it counts once, like the \
             other row past the divider, and can never move the boundary into the future"
        );
        // The cursor is the message id, because a timestamp cannot separate two messages sent in
        // the same millisecond and cannot order two senders' clocks at all. Both of those hid a
        // real arrival while the divider was a timestamp comparison.
        let same_ms = newest + 10;
        post_as_at(&mut server, "same-ms-a", "first of two", same_ms).await;
        post_as_at(&mut server, "same-ms-b", "second of two", same_ms).await;
        // Written last, stamped earlier. Rows are ordered by their stamps now, so this one sits
        // where its stamp says rather than where it was written, which is the whole point: it can
        // no longer appear after a mark the reader set at a later moment.
        post_as_at(&mut server, "backwards", "an older clock", newest + 5).await;
        let by_id = |id: &str| MessagePageQuery {
            anchor: PageAnchor::Tail,
            before: 0,
            after: 0,
            unread: Some(crate::UnreadProbe {
                divider_id: id.to_string(),
                divider_ts: same_ms,
                now_ms: u64::from(u32::MAX),
            }),
        };
        let after_first = server
            .message_page(GENERAL, &by_id("same-ms-a"))
            .unread
            .unwrap();
        // Two: the second message of the millisecond, which a timestamp cursor could not have
        // separated from the first, and the broken-clock row from earlier, which sorts last
        // because that is what its stamp says.
        assert_eq!(
            after_first.count, 2,
            "the second message of the millisecond is unread, which a timestamp could not say"
        );
        let after_second = server
            .message_page(GENERAL, &by_id("same-ms-b"))
            .unread
            .unwrap();
        assert_eq!(
            after_second.count, 1,
            "and after it only the broken-clock row is left"
        );
        let deleted_cursor = server
            .message_page(GENERAL, &by_id("no-such-row"))
            .unread
            .unwrap();
        assert_eq!(
            deleted_cursor.count,
            server
                .message_page(GENERAL, &probe(same_ms))
                .unread
                .unwrap()
                .count,
            "a cursor that names no row falls back to the timestamp rule it replaced"
        );

        assert_eq!(server.pinned_messages(GENERAL).len(), 0);
        server.set_pin(GENERAL, &mine, true).await.unwrap();
        assert_eq!(server.pinned_messages(GENERAL)[0].id, mine);

        async fn post_as_at(
            server: &mut Server<MemNetwork, ChaCha20Rng>,
            id: &str,
            text: &str,
            ts: u64,
        ) {
            let (id, text) = (id.to_string(), text.to_string());
            server
                .sync
                .post(crate::DocType::Channel, GENERAL, move |d| {
                    crate::append_message(d, &id, "bob", &text, ts, "")
                })
                .await
                .unwrap();
        }
    }

    /// Where the time between launching and being able to read a message actually goes.
    ///
    /// The desktop's startup budget has never been broken down, so the hardening plan has been
    /// trading in chunk sizes. This times the native half on one machine: the vault unlock
    /// (Argon2id, deliberately slow), reading and unsealing a server, and rebuilding it from its
    /// snapshot. Not a correctness test; run with
    /// `cargo test -p catcoms-app --release --lib startup_probe -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn startup_probe() {
        use crate::store::{ServerRecord, ServerStore};
        use catcoms_rt::OsCryptoRng;
        let clock = catcoms_rt::SystemClock;
        let timed = |f: &mut dyn FnMut()| {
            let start = clock.monotonic_ms();
            f();
            clock.monotonic_ms().saturating_sub(start)
        };

        for messages in [200usize, 5_000, 20_000] {
            let hub = Hub::new();
            let mut server = founder(&hub, PeerId::from_u64(1), "alice", 1);
            server.open_channel(GENERAL).await.unwrap();
            for i in 0..messages {
                server
                    .send_message(
                        GENERAL,
                        &format!("message {i} with a plausible amount of text"),
                    )
                    .await
                    .unwrap();
            }
            let snapshot = server.snapshot().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let mut rng = OsCryptoRng;

            let mut first = None;
            let create = timed(&mut || {
                first = Some(
                    ServerStore::open(dir.path(), b"a long enough passphrase", &mut rng).unwrap(),
                )
            });
            let store = first.unwrap();
            store.save_server(7, &snapshot, &mut rng).unwrap();
            store
                .save_registry(
                    &[ServerRecord {
                        id: 7,
                        display_name: "alice".into(),
                        invite: String::new(),
                        is_dm: false,
                    }],
                    &mut rng,
                )
                .unwrap();
            drop(store);

            // The launch path: unlock the vault, read the registry, unseal and rebuild a server.
            let mut reopened = None;
            let unlock = timed(&mut || {
                reopened = Some(
                    ServerStore::open(dir.path(), b"a long enough passphrase", &mut rng).unwrap(),
                )
            });
            let store = reopened.unwrap();
            let mut registry = Vec::new();
            let read_registry = timed(&mut || registry = store.load_registry().unwrap());
            let mut sealed = None;
            let read_server =
                timed(&mut || sealed = Some(store.load_server(registry[0].id).unwrap()));
            let sealed = sealed.unwrap();
            let hub2 = Hub::new();
            let mut restored = None;
            let restore = timed(&mut || {
                restored = Server::restore(
                    &sealed,
                    hub2.join(PeerId::from_u64(2)),
                    ChaCha20Rng::seed_from_u64(1),
                    Box::new(ManualClock::new(2_000)),
                    "alice",
                )
                .ok()
            });
            let restored = restored.expect("the snapshot restores");
            let open = timed(&mut || {
                let _ = restored.messages(GENERAL);
            });
            println!(
                "messages={messages} snapshot={} KiB | vault_create={create}ms vault_unlock={unlock}ms \
                 registry={read_registry}ms read_server={read_server}ms restore={restore}ms first_read={open}ms",
                snapshot.len() / 1024,
            );
        }
    }

    /// Timing probe for the per-tick costs that scale with history. Not a correctness test; run
    /// with `cargo test -p catcoms-app --release --lib scale_probe -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn scale_probe() {
        let hub = Hub::new();
        let mut server = founder(&hub, PeerId::from_u64(1), "alice", 1);
        server.open_channel(GENERAL).await.unwrap();
        for n in [1_000usize, 5_000, 20_000] {
            let have = server.messages(GENERAL).len();
            for i in have..n {
                server
                    .send_message(GENERAL, &format!("message number {i} with some body text"))
                    .await
                    .unwrap();
            }
            // Wall time through the sanctioned seam (the ambient gate forbids `Instant` here);
            // millisecond resolution is enough for costs that are meant to be visible.
            let clock = catcoms_rt::SystemClock;
            let timed = |f: &mut dyn FnMut()| {
                let start = clock.monotonic_ms();
                f();
                clock.monotonic_ms().saturating_sub(start)
            };
            // The last send left the cache current, so force the walk itself to be measured.
            server.messages_cache.borrow_mut().clear();
            let mut count = 0;
            let walk = timed(&mut || count = server.messages(GENERAL).len());
            let cached = timed(&mut || {
                let _ = server.with_messages(GENERAL, |m| m.len());
            });
            let mut sigs = HashMap::new();
            channel_delta(&server, GENERAL, &mut sigs);
            let delta = timed(&mut || {
                channel_delta(&server, GENERAL, &mut sigs);
            });
            let mut size = 0;
            let snapshot = timed(&mut || size = server.snapshot().unwrap().len());
            println!(
                "n={n} walk+clone={walk}ms cached_read={cached}ms channel_delta={delta}ms snapshot={snapshot}ms ({} KiB) count={count}",
                size / 1024,
            );
        }
    }

    /// Drain events until the next `ChannelUpdated` for `channel`, returning what it says moved.
    async fn next_change(events: &mut mpsc::Receiver<TracedEvent>, channel: u128) -> ChannelChange {
        next_traced_change(events, channel).await.1
    }

    async fn wait_for_appended(events: &mut mpsc::Receiver<TracedEvent>, channel: u128) {
        loop {
            match events.recv().await {
                Some(TracedEvent {
                    event:
                        AppEvent::ChannelUpdated {
                            channel: updated,
                            change,
                        },
                    ..
                }) if updated == channel && change.messages_appended => return,
                Some(_) => continue,
                None => panic!("recipient actor closed"),
            }
        }
    }

    /// As [`next_change`], but keeping the operation the event was attributed to.
    async fn next_traced_change(
        events: &mut mpsc::Receiver<TracedEvent>,
        channel: u128,
    ) -> (Trace, ChannelChange) {
        timeout(Duration::from_secs(5), async {
            loop {
                match events.recv().await {
                    Some(TracedEvent {
                        trace,
                        event: AppEvent::ChannelUpdated { channel: c, change },
                    }) if c == channel => return (trace, change),
                    Some(_) => continue,
                    None => panic!("actor closed"),
                }
            }
        })
        .await
        .expect("no channel update arrived")
    }

    /// The link P3-004 says is missing: an event has to name the operation that caused it.
    ///
    /// Without it a `ChannelUpdated` arriving two seconds after a send is indistinguishable from
    /// one caused by somebody else's message, so "did my send reach the UI" cannot be answered from
    /// the record at all. Every stage before this one was already correlated and none of them could
    /// establish the thing anybody actually wanted to know.
    #[tokio::test]
    async fn an_event_names_the_operation_that_caused_it() {
        let hub = Hub::new();
        let (actor, mut events, handle) = spawn(founder(&hub, PeerId::from_u64(1), "alice", 1));
        actor.open_channel(GENERAL).await;

        let mine = 0x7f2c_0000_0000_0001;
        actor.with_trace(mine).send_message(GENERAL, "hi").await;
        let (trace, change) = next_traced_change(&mut events, GENERAL).await;
        assert!(change.messages_appended);
        assert_eq!(
            trace,
            Trace(mine),
            "the update carries the send that caused it"
        );

        // A handle with no trace is not a handle carrying somebody else's.
        actor.send_message(GENERAL, "and again").await;
        let (trace, _) = next_traced_change(&mut events, GENERAL).await;
        assert_eq!(trace, Trace::NONE, "an untraced command adopts nothing");
        assert!(!trace.is_set());

        actor.shutdown().await;
        let _ = handle.await;
    }

    /// Two operations in flight at once must not borrow each other's identity.
    ///
    /// The failure this rules out is the one that would make the whole mechanism worse than
    /// nothing: a trace that gathers another operation's stages does not merely fail to explain a
    /// bug, it explains it wrongly, and the reader has no way to tell.
    #[tokio::test]
    async fn concurrent_operations_keep_their_own_stages() {
        let hub = Hub::new();
        let (actor, mut events, handle) = spawn(founder(&hub, PeerId::from_u64(1), "alice", 1));
        actor.open_channel(GENERAL).await;

        let first = actor.with_trace(0xaaaa);
        let second = actor.with_trace(0xbbbb);
        // Issued together, so the actor interleaves them however it likes.
        let (a, b) = tokio::join!(
            first.send_reply(GENERAL, "from a", String::new()),
            second.send_reply(GENERAL, "from b", String::new()),
        );
        a.unwrap();
        b.unwrap();

        let mut seen = Vec::new();
        for _ in 0..2 {
            let (trace, _) = next_traced_change(&mut events, GENERAL).await;
            seen.push(trace);
        }
        seen.sort();
        assert_eq!(
            seen,
            vec![Trace(0xaaaa), Trace(0xbbbb)],
            "each send produced exactly one update, under its own trace"
        );

        actor.shutdown().await;
        let _ = handle.await;
    }

    /// Somebody else's message must not be attributed to my last command.
    ///
    /// The dangerous direction. A trace that merely *misses* a stage leaves a gap a reader can see;
    /// a trace that gathers an unrelated one asserts a causal link that never existed, and the
    /// reader has no way to tell. Here Bob's arrival lands on Alice while her own traced send is
    /// the most recent thing she handled, which is exactly when a sticky trace would claim it.
    #[tokio::test]
    async fn a_peers_message_is_not_attributed_to_my_last_command() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice_srv = founder(&hub, alice_peer, "alice", 1);
        alice_srv.subscribe_control().await.unwrap();
        alice_srv.open_channel(GENERAL).await.unwrap();
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
        let (bob, bob_events, bob_handle) = spawn(bob_srv);
        bob.open_channel(GENERAL).await;

        // Alice's own send, under a trace she will recognise.
        let mine = 0x7f2c_0000_0000_0002;
        // The actor tracks a channel's content signature from the moment it is opened *through the
        // actor*, which is what decides whether a change is reported at all.
        alice.open_channel(GENERAL).await;
        alice
            .with_trace(mine)
            .send_reply(GENERAL, "hi bob", String::new())
            .await
            .expect("alice's own send");
        let (trace, _) = next_traced_change(&mut alice_events, GENERAL).await;
        assert_eq!(trace, Trace(mine), "her own send is hers");

        // Bob answers. Alice learns about it through her sync loop, not through a command, and she
        // sends nothing further, so the next arrival she reports can only be his.
        bob.send_message(GENERAL, "hi alice").await;
        let seen = timeout(Duration::from_secs(60), async {
            loop {
                match alice_events.recv().await {
                    Some(TracedEvent {
                        trace,
                        event: AppEvent::ChannelUpdated { channel, change },
                    }) if channel == GENERAL && change.messages_appended => return trace,
                    Some(_) => continue,
                    None => panic!("alice's actor closed"),
                }
            }
        })
        .await
        .expect("alice never saw bob's message");
        assert_eq!(
            seen,
            Trace::NONE,
            "an arrival from a peer belongs to no local operation, and must not borrow one"
        );

        alice.shutdown().await;
        bob.shutdown().await;
        let _ = alice_handle.await;
        let _ = bob_handle.await;
        drop(bob_events);
    }

    /// One channel document holds messages, the topic and the jukebox queue. The UI raises unread
    /// badges from arrivals only, so each of those has to be distinguishable at the event: an
    /// untyped "it changed" is what made a jukebox add look like an unread chat message.
    #[tokio::test]
    async fn a_channel_update_says_which_part_of_the_document_moved() {
        let hub = Hub::new();
        let (actor, mut events, handle) = spawn(founder(&hub, PeerId::from_u64(1), "alice", 1));
        actor.open_channel(GENERAL).await;

        actor.send_message(GENERAL, "hi there").await;
        let change = next_change(&mut events, GENERAL).await;
        assert!(change.messages_appended, "a new message is an arrival");
        assert!(!change.messages_changed && !change.topic && !change.jukebox);

        let id = actor.messages(GENERAL).await[0].id.clone();

        actor
            .toggle_reaction(GENERAL, id.clone(), "🐈".into())
            .await
            .unwrap();
        let change = next_change(&mut events, GENERAL).await;
        assert!(!change.messages_appended, "a reaction is not an arrival");
        assert!(change.messages_changed, "but the log still re-renders");

        actor
            .edit_message(GENERAL, id, "hi there!".into())
            .await
            .unwrap();
        let change = next_change(&mut events, GENERAL).await;
        assert!(!change.messages_appended, "an edit is not an arrival");
        assert!(change.messages_changed);

        actor
            .set_channel_topic(GENERAL, "cats only".into())
            .await
            .unwrap();
        let change = next_change(&mut events, GENERAL).await;
        assert!(change.topic);
        assert!(!change.messages_appended && !change.messages_changed);

        actor
            .jukebox_add(GENERAL, "ab".into(), "purr.mp3".into())
            .await
            .unwrap();
        let change = next_change(&mut events, GENERAL).await;
        assert!(change.jukebox, "queue edits still reach the UI");
        assert!(
            !change.messages_appended,
            "but a jukebox add must never read as a new message"
        );

        actor.shutdown().await;
        let _ = handle.await;
    }

    /// Every member may write any key of the shared channel document, so an entry already in the
    /// queue can come back pointing at a different file while its id, name and queue time stay
    /// put. A signature that folded only id and name called that queue unchanged, so the UI kept
    /// offering the previous track and the person debugging it saw "the event says nothing moved".
    #[tokio::test]
    async fn requeueing_an_entry_onto_a_different_file_reads_as_a_queue_change() {
        let hub = Hub::new();
        let mut server = founder(&hub, PeerId::from_u64(1), "alice", 1);
        server.open_channel(GENERAL).await.unwrap();
        server.jukebox_add(GENERAL, "ab", "purr.mp3").await.unwrap();

        let mut sigs = HashMap::new();
        assert!(
            channel_delta(&server, GENERAL, &mut sigs).is_none(),
            "first sight of a channel only seeds the record"
        );

        let mut entry = server.jukebox(GENERAL).remove(0);
        entry.cid = "cd".into();
        server
            .sync
            .post(crate::DocType::Channel, GENERAL, move |d| {
                crate::add_juke_entry_in_doc(d, &entry)
            })
            .await
            .unwrap();

        let change = channel_delta(&server, GENERAL, &mut sigs).expect(
            "a queue entry now naming another file is a change the UI has to be told about",
        );
        assert!(
            change.jukebox,
            "and it is a queue change, not a message one"
        );
        assert!(!change.messages_appended && !change.messages_changed && !change.topic);
        assert!(
            channel_delta(&server, GENERAL, &mut sigs).is_none(),
            "a quiet tick after it must not invent a second one"
        );
    }

    /// Unread state has to survive an explicit lock and a restart, neither of which the live event
    /// stream covers. The heads are the state it gets rebuilt from, so own messages must not count.
    #[tokio::test]
    async fn channel_heads_report_the_newest_message_somebody_else_wrote() {
        let hub = Hub::new();
        let (actor, _events, handle) = spawn(founder(&hub, PeerId::from_u64(1), "alice", 1));
        // The heads cover the shared channel directory, which is what the UI lists, so this uses
        // the directory's own id for `general` rather than the bare doc id the other tests open.
        let general = crate::channel_id("general");
        actor.open_channel(general).await;
        actor.send_message(general, "only mine").await;

        let heads = actor.channel_heads().await;
        let head = heads
            .iter()
            .find(|h| h.channel == general)
            .expect("general has a head");
        assert_eq!(head.count, 1);
        assert!(head.latest_ts > 0, "the message has a timestamp");
        assert_eq!(
            head.latest_incoming_ts, 0,
            "my own message can never make my own channel unread"
        );
        assert!(head.latest_incoming_id.is_empty());

        actor.shutdown().await;
        let _ = handle.await;
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
        // The delta names the row that arrived, whose id is generated, so it is compared by shape.
        let AppEvent::ChannelUpdated { channel, change } = &ev.event else {
            panic!("expected a channel update, got {:?}", ev.event);
        };
        assert_eq!(*channel, GENERAL);
        assert!(change.messages_appended);
        assert_eq!(change.arrivals.len(), 1, "the message that was just sent");
        assert!(!change.messages_changed && !change.topic && !change.jukebox);

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

        // Bob joins; Alice's actor serves the join via its own sync loop.
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
                    Some(TracedEvent {
                        event: AppEvent::ChannelUpdated { channel, change },
                        ..
                    }) if channel == GENERAL && change.messages_appended => break,
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

    /// Two acknowledgements can arrive during one throttle interval and then the room can go
    /// completely quiet. The second receipt still needs to reach the UI: relying on a third
    /// network event left the last sender status stuck indefinitely.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_throttled_delivery_receipt_gets_an_injected_clock_wake() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let alice_clock = ManualClock::new(1_000);
        let mut alice_srv = founder_with_clock(&hub, alice_peer, "alice", 1, &alice_clock);
        alice_srv.subscribe_control().await.unwrap();
        alice_srv.open_channel(GENERAL).await.unwrap();
        alice_srv
            .publish_self_record(vec!["/ip4/203.0.113.1/tcp/22487".into()], 1)
            .unwrap();
        let alice_record = alice_srv.sync.self_record().unwrap().clone();
        let bob_invite = alice_srv.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (alice, mut alice_events, alice_handle) = spawn(alice_srv);

        let mut bob_srv = Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &bob_invite,
        )
        .await
        .unwrap();
        bob_srv.subscribe_control().await.unwrap();
        assert!(bob_srv.sync.ingest_peer_record(alice_record.clone()));
        let mut bob_connections = catcoms_sync::PreOwnerConnectionHandoff::default();
        bob_connections.observe(&catcoms_rt::TransportEvent::PeerConnected(alice_peer));
        bob_srv.sync.adopt_pre_owner_connections(bob_connections);

        // Minting through the actor guarantees the second invite includes Bob's committed
        // membership rather than using Alice's pre-join group state.
        let carol_invite = alice
            .mint_invite([8u8; 16], u64::MAX, vec![])
            .await
            .unwrap();
        let carol_invite = InviteToken::decode(&carol_invite).unwrap();
        let mut carol_srv = Server::join(
            hub.join(PeerId::from_u64(3)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(3),
            Box::new(ManualClock::new(1_000)),
            "carol",
            alice_peer,
            &carol_invite,
        )
        .await
        .unwrap();
        assert!(carol_srv.sync.ingest_peer_record(alice_record));
        let mut carol_connections = catcoms_sync::PreOwnerConnectionHandoff::default();
        carol_connections.observe(&catcoms_rt::TransportEvent::PeerConnected(alice_peer));
        carol_srv
            .sync
            .adopt_pre_owner_connections(carol_connections);

        // Bob was intentionally not spawned yet, so Carol's membership commit is queued at his
        // transport and can be applied deterministically before Alice authors at the new epoch.
        timeout(Duration::from_secs(10), async {
            while bob_srv.member_count() != 3 {
                assert!(bob_srv.sync_once().await.unwrap());
            }
        })
        .await
        .expect("Bob did not learn Carol's membership");

        let (bob, mut bob_events, bob_handle) = spawn(bob_srv);
        bob.open_channel(GENERAL).await;
        let (carol, mut carol_events, carol_handle) = spawn(carol_srv);
        carol.open_channel(GENERAL).await;

        alice.send_message(GENERAL, "quiet acknowledgement").await;
        timeout(
            Duration::from_secs(10),
            wait_for_appended(&mut bob_events, GENERAL),
        )
        .await
        .expect("Bob did not receive Alice's message");
        timeout(
            Duration::from_secs(10),
            wait_for_appended(&mut carol_events, GENERAL),
        )
        .await
        .expect("Carol did not receive Alice's message");

        // Other post-send protocol traffic can establish the throttled baseline at zero before
        // the first receipt arrives. Poll the underlying authenticated state slowly enough to
        // leave the biased command arm idle between reads. This establishes that both requests
        // were accepted while the injected clock remains fixed, not by introducing a third
        // network event.
        timeout(Duration::from_secs(10), async {
            loop {
                if alice
                    .delivery_snapshot(GENERAL)
                    .await
                    .expect("Alice actor is live")
                    .states
                    .first()
                    .is_some_and(|state| state.delivered == 2)
                {
                    return;
                }
                // Yield without advancing either wall or monotonic time. The actor can service
                // its network arm, while the throttle remains pinned at the original instant.
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("Alice did not authenticate both receipts");

        while let Ok(event) = alice_events.try_recv() {
            if let AppEvent::DeliveryChanged { channel, snapshot } = event.event {
                if channel == GENERAL {
                    let delivered = snapshot.states.first().map_or(0, |state| state.delivered);
                    assert_ne!(delivered, 2, "the throttle elapsed before clock advance");
                }
            }
        }

        alice_clock.advance_ms(DELIVERY_THROTTLE_MS);
        let final_delivered = timeout(Duration::from_secs(10), async {
            loop {
                match alice_events.recv().await {
                    Some(TracedEvent {
                        event: AppEvent::DeliveryChanged { channel, snapshot },
                        ..
                    }) if channel == GENERAL => {
                        if let Some(state) = snapshot.states.first() {
                            if state.delivered == 2 {
                                return state.delivered;
                            }
                        }
                    }
                    Some(_) => continue,
                    None => panic!("Alice's actor closed"),
                }
            }
        })
        .await
        .expect("the dirty delivery snapshot did not receive its timer wake");
        assert_eq!(final_delivered, 2);

        alice.shutdown().await;
        bob.shutdown().await;
        carol.shutdown().await;
        let _ = alice_handle.await;
        let _ = bob_handle.await;
        let _ = carol_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_actor_surfaces_an_inbound_call_signal_with_its_verified_sender() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice_srv = founder(&hub, alice_peer, "alice", 1);
        alice_srv.subscribe_control().await.unwrap();
        alice_srv
            .publish_self_record(vec!["/ip4/203.0.113.1/tcp/1".into()], 1)
            .unwrap();
        let alice_record = alice_srv.sync.self_record().unwrap().clone();
        let alice_fp = alice_srv.my_fingerprint();
        let invite = alice_srv.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (alice, mut alice_events, alice_handle) = spawn(alice_srv);

        let mut bob_srv = Server::join(
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
        assert!(bob_srv.sync.ingest_peer_record(alice_record));
        let bob_fp = bob_srv.my_fingerprint();
        let (bob, _bob_events, bob_handle) = spawn(bob_srv);

        let payload = br#"{"type":"offer","sdp":"opaque-to-actor"}"#.to_vec();
        assert!(bob
            .send_call_signal(alice_fp, payload.clone())
            .await
            .unwrap());

        let received = timeout(Duration::from_secs(5), async {
            loop {
                match alice_events.recv().await {
                    Some(TracedEvent {
                        event: AppEvent::CallSignal { from_fp, payload },
                        ..
                    }) => break (from_fp, payload),
                    Some(_) => continue,
                    None => panic!("alice actor closed"),
                }
            }
        })
        .await
        .expect("Alice did not surface the call signal");
        assert_eq!(received, (bob_fp, payload));

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
    async fn a_new_channel_and_its_messages_reach_existing_members() {
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

        // Bob creates a named channel. The shared directory must make it appear for Alice and
        // subscribe her before any message is sent; users should not need to know or manually
        // open the channel's derived document id.
        let plans = bob.create_channel("plans").await.unwrap();
        timeout(Duration::from_secs(60), async {
            loop {
                if alice.channels().await.iter().any(|c| c == &plans) {
                    break;
                }
                match alice_events.recv().await {
                    Some(_) => continue,
                    None => panic!("alice actor closed"),
                }
            }
        })
        .await
        .expect("Alice did not discover Bob's channel");

        bob.send_message(plans.id, "from bob").await;
        timeout(Duration::from_secs(60), async {
            loop {
                if bob
                    .messages(plans.id)
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

        // Posting to the discovered channel must then update Alice automatically too.
        timeout(Duration::from_secs(60), async {
            loop {
                if alice
                    .messages(plans.id)
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
        .expect("Alice did not receive a message in Bob's channel");

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
        // the mesh; proving an image travels by content address, not inline in gossip.
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

        // Bob pulls the file index from Alice (request/response; what the real app does on
        // join) until the shared file appears. The loop retries the catch-up every ~50ms
        // and drains bob_events when present (so a full event channel can't stall his
        // actor); fully deterministic, no reliance on gossip timing or peer discovery.
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
